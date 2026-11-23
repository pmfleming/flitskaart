//! Gesprek: a conversation in Dutch about a topic of the learner's choice.
//! Claude plays the other person and the Dutch voice speaks its lines. The
//! learner answers by typing or speaking; each answer comes back corrected
//! with the next line. At the end the difficult lines become draft cards.

use anyhow::Result;
use dutch_flashcards_core::conversation::{self, Conversation, Line, Reply, Said, Who};
use dutch_flashcards_core::drafting;
use dutch_flashcards_core::model::NewCard;
use dutch_flashcards_core::speech::{assess, missed_ranges, write_wav_16k};
use dutch_flashcards_core::voice::Speed;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{ActiveTheme as _, Colorize as _, Disableable as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, Div, Entity, Focusable as _, FontWeight, HighlightStyle, InteractiveElement as _, IntoElement,
    ParentElement as _, ScrollHandle, SharedString, StatefulInteractiveElement as _, Styled as _, StyledText, Subscription,
    UnderlineStyle, Window, div, px,
};

use crate::app::FlashcardsApp;
use crate::capture::Capture;
use crate::icons;
use crate::recorder::Captured;
use crate::ui::{card_box, field_label, heading, hint, surface};

/// Topics offered on the start screen; any other can be typed.
pub const TOPICS: [&str; 6] = ["Het weekend", "Boodschappen doen", "Op het werk", "In een restaurant", "Bij de dokter", "Op vakantie"];

const NOT_HEARD: &str = "Ik hoorde niets. Spreek wat luider of dichter bij de microfoon, of typ je antwoord.";

pub struct TalkState {
    pub topic: Entity<InputState>,
    pub message: Entity<InputState>,
    pub current: Option<Conversation>,
    /// The learner's sentence on its way to Claude, or the one that failed to arrive.
    pub pending: Option<Said>,
    /// Waiting for Claude's next line.
    pub thinking: bool,
    /// Claude is turning the difficult lines into cards.
    pub making_cards: bool,
    pub capture: Capture,
    /// Whisper is writing down what the learner said.
    pub hearing: bool,
    pub error: Option<SharedString>,
    pub scroll: ScrollHandle,
    /// Bumped for each new conversation, so a late answer can't land in the next one.
    ticket: u64,
    _entered: [Subscription; 2],
}

impl TalkState {
    pub fn new(window: &mut Window, cx: &mut Context<FlashcardsApp>) -> Self {
        let topic = cx.new(|cx| InputState::new(window, cx).placeholder("Een eigen onderwerp, of laat het open"));
        let message = cx.new(|cx| InputState::new(window, cx).placeholder("Typ je antwoord en druk op Enter, of klik op Spreek"));
        let on_enter = |action: fn(&mut FlashcardsApp, &mut Window, &mut Context<FlashcardsApp>)| {
            move |this: &mut FlashcardsApp, _: &Entity<InputState>, event: &InputEvent, window: &mut Window, cx: &mut Context<FlashcardsApp>| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    action(this, window, cx);
                }
            }
        };
        let entered = [
            cx.subscribe_in(&topic, window, on_enter(FlashcardsApp::start_conversation)),
            cx.subscribe_in(&message, window, on_enter(FlashcardsApp::send_typed)),
        ];
        TalkState {
            topic,
            message,
            current: None,
            pending: None,
            thinking: false,
            making_cards: false,
            capture: Capture::default(),
            hearing: false,
            error: None,
            scroll: ScrollHandle::new(),
            ticket: 0,
            _entered: entered,
        }
    }

    /// The topic field before the conversation starts, the answer field after.
    pub fn focus(&self, window: &mut Window, cx: &mut App) {
        let field = if self.current.is_some() { &self.message } else { &self.topic };
        field.focus_handle(cx).focus(window, cx);
    }

    /// Recording, Whisper or Claude is busy with the learner's turn.
    pub fn busy(&self) -> bool {
        self.capture.is_on() || self.hearing || self.thinking || self.making_cards
    }

    /// The learner can answer: the partner has spoken and nothing is under way.
    fn can_answer(&self) -> bool {
        self.current.as_ref().is_some_and(|c| !c.lines.is_empty()) && !self.busy()
    }
}

impl FlashcardsApp {
    /// Starts a conversation on the topic typed (or none), and asks for its opening line.
    pub fn start_conversation(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.talk.current.is_some() {
            return;
        }
        let topic = self.talk.topic.read(cx).value().to_string();
        self.talk.topic.update(cx, |state, cx| state.set_value("", window, cx));
        self.talk.ticket += 1;
        self.talk.current = Some(Conversation::new(&topic));
        self.talk.message.update(cx, |state, cx| state.focus(window, cx));
        self.ask_partner(None, cx);
    }

    fn start_with_topic(&mut self, topic: &'static str, window: &mut Window, cx: &mut Context<Self>) {
        self.talk.topic.update(cx, |state, cx| state.set_value(topic, window, cx));
        self.start_conversation(window, cx);
    }

    /// Sends what is typed in the answer field.
    fn send_typed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.talk.message.read(cx).value().trim().to_string();
        if text.is_empty() || !self.talk.can_answer() {
            return;
        }
        self.talk.message.update(cx, |state, cx| state.set_value("", window, cx));
        self.ask_partner(Some(Said { text, spoken: false }), cx);
    }

    /// Sends the learner's sentence (none for the opening) and waits for the
    /// correction and the partner's next line, which is then spoken.
    fn ask_partner(&mut self, said: Option<Said>, cx: &mut Context<Self>) {
        let Some(current) = &self.talk.current else { return };
        let request = current.request(said.as_ref());
        let (claude, ticket) = (self.claude.clone(), self.talk.ticket);
        (self.talk.pending, self.talk.thinking, self.talk.error) = (said, true, None);
        self.talk.scroll.scroll_to_bottom();
        let job = move || conversation::next_turn(&claude, &request);
        self.in_background(cx, job, move |this, reply, cx| {
            if this.talk.ticket == ticket {
                this.talk.thinking = false;
                this.take_reply(reply, cx);
            }
        });
        cx.notify();
    }

    fn take_reply(&mut self, reply: Result<Reply>, cx: &mut Context<Self>) {
        let Some(current) = &mut self.talk.current else { return };
        match reply {
            Ok(reply) => {
                let line = reply.reply.clone();
                current.add_turn(self.talk.pending.take(), reply);
                self.talk.scroll.scroll_to_bottom();
                self.say(line, cx);
            }
            Err(err) if current.lines.is_empty() => {
                // The opening failed: back to choosing a topic.
                self.talk.current = None;
                self.talk.error = Some(crate::logging::error("gesprek", format!("{err:#}")).into());
            }
            Err(err) => self.talk.error = Some(crate::logging::error("gesprek", format!("{err:#}")).into()),
        }
    }

    /// Sends the sentence that failed to arrive once more.
    fn resend(&mut self, cx: &mut Context<Self>) {
        if let Some(said) = self.talk.pending.take().filter(|_| !self.talk.busy()) {
            self.ask_partner(Some(said), cx);
        }
    }

    /// Spreek: records the answer, or stops a recording under way.
    pub fn speak_answer(&mut self, cx: &mut Context<Self>) {
        if self.talk.capture.is_on() {
            self.talk.capture.stop();
        } else if self.talk.can_answer() && self.listener.is_installed() {
            self.talk.error = None;
            self.start_capture(|this| &mut this.talk.capture, Self::heard_answer, cx);
        }
        cx.notify();
    }

    /// Lets Whisper write down the spoken answer, then sends it.
    fn heard_answer(&mut self, captured: Result<Captured>, cx: &mut Context<Self>) {
        let path = self.paths.recordings().join("gesprek.wav");
        let saved = captured.and_then(|captured| {
            captured.had_speech.then(|| write_wav_16k(&path, &captured.samples, captured.rate, captured.channels)).transpose()
        });
        match saved {
            Ok(Some(())) => self.talk.hearing = true,
            Ok(None) => return self.talk.error = Some(NOT_HEARD.into()),
            Err(err) => return self.talk.error = Some(crate::logging::error("gesprek", format!("{err:#}")).into()),
        }
        let (listener, ticket) = (self.listener.clone(), self.talk.ticket);
        self.in_background(cx, move || listener.transcribe(&path), move |this, heard, cx| {
            this.talk.hearing = false;
            match heard {
                _ if this.talk.ticket != ticket => {}
                Ok(text) if text.trim().is_empty() => this.talk.error = Some(NOT_HEARD.into()),
                Ok(text) => this.ask_partner(Some(Said { text, spoken: true }), cx),
                Err(err) => this.talk.error = Some(crate::logging::error("gesprek", format!("{err:#}")).into()),
            }
        });
    }

    fn change_line(&mut self, index: usize, change: fn(&mut Conversation, usize), cx: &mut Context<Self>) {
        if let Some(current) = &mut self.talk.current {
            change(current, index);
        }
        cx.notify();
    }

    /// Plays a line: the partner's, or what the learner meant to say.
    fn hear_line(&mut self, index: usize, speed: Speed, cx: &mut Context<Self>) {
        let text = self.talk.current.as_ref().and_then(|c| c.lines.get(index)).map(|line| line.meant().to_string());
        if let Some(text) = text.filter(|_| !self.talk.capture.is_on()) {
            self.speak(text, self.current_voice(), speed, cx);
        }
    }

    /// Afronden: keeps the conversation and turns its difficult lines into
    /// drafts, then opens Concepten. Nothing said yet: just closes it.
    fn finish_conversation(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(current) = self.talk.current.as_ref().filter(|_| !self.talk.busy()) else { return };
        if !current.has_learner_lines() {
            return self.close_conversation(cx);
        }
        let transcript = current.transcript();
        let Some((steer, limit)) = current.card_request() else {
            return match self.keep_conversation(&transcript, "", &[], window, cx) {
                Ok(()) => self.info("Het gesprek is bewaard. Je markeerde niets als moeilijk, dus er zijn geen kaarten gemaakt.", cx),
                Err(err) => self.talk.error = Some(crate::logging::error("gesprek", format!("{err:#}")).into()),
            };
        };
        self.stop_audio();
        (self.talk.making_cards, self.talk.error) = (true, None);
        let claude = self.claude.clone();
        let job = cx.background_executor().spawn(async move {
            let cards = drafting::draft_cards(&claude, &transcript, steer, limit);
            (transcript, cards)
        });
        cx.spawn_in(window, async move |this, cx| {
            let (transcript, cards) = job.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.talk.making_cards = false;
                let kept = cards.and_then(|cards| this.keep_conversation(&transcript, steer, &cards, window, cx));
                if let Err(err) = kept {
                    this.talk.error = Some(crate::logging::error("gesprek", format!("Er zijn geen kaarten gemaakt: {err:#}")).into());
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Stores the conversation with its drafts (if any) and closes it.
    fn keep_conversation(&mut self, transcript: &str, steer: &str, cards: &[NewCard], window: &mut Window, cx: &mut Context<Self>) -> Result<()> {
        if cards.is_empty() {
            self.store.add_snippet(transcript, steer, Self::now())?;
        } else {
            self.keep_drafts(transcript, steer, cards, window, cx)?;
        }
        self.close_conversation(cx);
        Ok(())
    }

    fn close_conversation(&mut self, cx: &mut Context<Self>) {
        self.talk.ticket += 1;
        (self.talk.current, self.talk.pending, self.talk.error) = (None, None, None);
        self.talk.capture.cancel();
        cx.notify();
    }

    pub fn render_talk(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let body = match &self.talk.current {
            None => self.render_topics(cx).into_any_element(),
            Some(current) => self.render_conversation(current, cx).into_any_element(),
        };
        v_flex().size_full().p_5().gap_4().child(body)
    }

    /// The start: a topic to talk about.
    fn render_topics(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let topics = TOPICS.map(|topic| {
            Button::new(SharedString::from(format!("topic-{topic}")))
                .outline()
                .label(topic)
                .on_click(cx.listener(move |this, _, window, cx| this.start_with_topic(topic, window, cx)))
        });
        v_flex().flex_1().items_center().justify_center().child(
            card_box(cx)
                .w_full()
                .max_w(px(680.))
                .p_6()
                .gap_5()
                .child(
                    v_flex()
                        .gap_1p5()
                        .child(heading("Een gesprek voeren"))
                        .child(hint(
                            "Kies een onderwerp. Claude praat met je in het Nederlands. Antwoord door te typen of te spreken: je krijgt elke zin verbeterd terug, met de volgende vraag. Zinnen die je lastig vond worden aan het eind kaarten.",
                            cx,
                        )),
                )
                .child(v_flex().gap_2().child(field_label("Kies een onderwerp", cx)).child(h_flex().flex_wrap().gap_2().children(topics)))
                .child(
                    v_flex().gap_1p5().child(field_label("Of typ je eigen onderwerp", cx)).child(
                        h_flex()
                            .gap_2()
                            .child(div().flex_1().child(Input::new(&self.talk.topic)))
                            .child(
                                Button::new("talk-start")
                                    .primary()
                                    .label("Begin gesprek")
                                    .on_click(cx.listener(|this, _, window, cx| this.start_conversation(window, cx))),
                            ),
                    ),
                )
                .children(self.render_talk_error(cx)),
        )
    }

    fn render_conversation(&self, current: &Conversation, cx: &mut Context<Self>) -> impl IntoElement {
        let marked = current.difficult();
        let topic = if current.topic.is_empty() { "Vrij gesprek".to_string() } else { current.topic.clone() };
        let finish = if self.talk.making_cards { "Kaarten maken…".to_string() } else { format!("Afronden · {marked} moeilijk") };
        let header = h_flex()
            .justify_between()
            .items_center()
            .gap_3()
            .pb_3()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(v_flex().gap_0p5().child(heading(topic)).child(hint("Markeer wat je lastig vond: dat worden kaarten.", cx)))
            .child(
                Button::new("talk-finish")
                    .primary()
                    .label(finish)
                    .loading(self.talk.making_cards)
                    .disabled(self.talk.busy())
                    .on_click(cx.listener(|this, _, window, cx| this.finish_conversation(window, cx))),
            );
        let lines: Vec<AnyElement> = current.lines.iter().enumerate().map(|(index, line)| self.render_line(index, line, cx)).collect();
        let log = v_flex()
            .id("talk-log")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .track_scroll(&self.talk.scroll)
            .gap_3()
            .py_2()
            .children(lines)
            .children(self.render_pending(cx));
        v_flex()
            .size_full()
            .max_w(px(760.))
            .mx_auto()
            .gap_3()
            .child(header)
            .child(log)
            .children(self.render_talk_error(cx))
            .child(self.render_composer(cx))
    }

    fn render_line(&self, index: usize, line: &Line, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let partner = line.who == Who::Partner;
        let small = |name: &'static str, label: &str| Button::new((name, index as u64)).ghost().xsmall().tab_stop(false).label(label.to_string());
        let mut meaning_btn = small("line-meaning", "Betekenis")
            .icon(icons::meaning())
            .on_click(cx.listener(move |this, _, _, cx| this.change_line(index, Conversation::toggle_meaning, cx)));
        if line.meaning_opened {
            meaning_btn = meaning_btn.outline().primary();
        }
        let mut difficult_btn = small("line-difficult", if line.difficult { "Moeilijk ✓" } else { "Moeilijk" })
            .on_click(cx.listener(move |this, _, _, cx| this.change_line(index, Conversation::toggle_difficult, cx)));
        if line.difficult {
            difficult_btn = difficult_btn.outline().warning();
        }
        let tools = h_flex()
            .gap_1()
            .flex_wrap()
            .child(small("line-hear", "Hoor").icon(icons::listen()).on_click(cx.listener(move |this, _, _, cx| this.hear_line(index, Speed::Normal, cx))))
            .child(small("line-slow", "Langzaam").on_click(cx.listener(move |this, _, _, cx| this.hear_line(index, Speed::Slow, cx))))
            .when(partner, |d| d.child(meaning_btn))
            .child(difficult_btn);
        let content = if partner { self.render_partner_text(line, cx) } else { render_learner_text(line, cx) };
        let bubble = v_flex()
            .max_w(px(600.))
            .gap_2()
            .px_4()
            .py_3()
            .rounded(px(12.))
            .border_1()
            .border_color(if line.difficult { theme.warning.opacity(0.75) } else { theme.border })
            .bg(if partner { surface(cx) } else { theme.accent })
            .child(content)
            .child(tools);
        let row = h_flex().w_full().when(!partner, |d| d.justify_end());
        row.child(bubble).into_any_element()
    }

    fn render_partner_text(&self, line: &Line, cx: &App) -> Div {
        v_flex()
            .gap_1()
            .child(div().text_lg().child(line.dutch.clone()))
            .when(line.meaning_opened, |d| d.child(div().italic().text_color(cx.theme().muted_foreground).child(line.english.clone())))
    }

    /// The sentence on its way to Claude, and what is happening to it.
    fn render_pending(&self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        let status = if self.talk.thinking {
            Some(hint(if self.talk.pending.is_some() { "Even nakijken…" } else { "Het gesprek begint…" }, cx))
        } else if self.talk.hearing {
            Some(hint("Even luisteren…", cx))
        } else {
            None
        };
        let said = self.talk.pending.as_ref().map(|said| {
            let retry = (!self.talk.thinking).then(|| {
                Button::new("talk-resend").small().outline().label("Opnieuw versturen").on_click(cx.listener(|this, _, _, cx| this.resend(cx)))
            });
            h_flex()
                .w_full()
                .justify_end()
                .child(v_flex().items_end().gap_1().child(div().text_color(cx.theme().muted_foreground).child(said.text.clone())).children(retry))
        });
        (status.is_some() || said.is_some()).then(|| v_flex().gap_2().children(said).children(status))
    }

    fn render_talk_error(&self, cx: &App) -> Option<Div> {
        let error = self.talk.error.clone()?;
        Some(div().text_sm().text_color(cx.theme().danger).child(error))
    }

    /// The answer field, Verstuur and Spreek, or the level meter while recording.
    fn render_composer(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let open = self.talk.can_answer();
        let recording = self.talk.capture.is_on();
        let mut speak = Button::new("talk-speak")
            .outline()
            .icon(icons::speak())
            .label(if recording { "Stop" } else { "Spreek" })
            .disabled(!(open || recording) || !self.listener.is_installed())
            .on_click(cx.listener(|this, _, _, cx| this.speak_answer(cx)));
        if recording {
            speak = speak.danger();
        }
        let field = match self.talk.capture.render("Zeg je antwoord.", cx) {
            Some(meter) => div().flex_1().child(meter),
            None => div().flex_1().child(Input::new(&self.talk.message).disabled(!open)),
        };
        let send = Button::new("talk-send")
            .primary()
            .label("Verstuur")
            .disabled(!open || recording)
            .on_click(cx.listener(|this, _, window, cx| this.send_typed(window, cx)));
        h_flex().gap_2().items_center().pt_3().border_t_1().border_color(cx.theme().border).child(field).child(send).child(speak)
    }
}

/// What the learner said: unchanged with a tick, or struck through with the
/// correction under it, its changed words marked, and why.
fn render_learner_text(line: &Line, cx: &App) -> Div {
    let theme = cx.theme();
    let how = if line.spoken { "gesproken" } else { "getypt" };
    let Some(corrected) = &line.corrected else {
        return v_flex()
            .gap_0p5()
            .child(div().text_lg().child(line.dutch.clone()))
            .child(hint(format!("Goed zo ✓ · {how}"), cx).text_color(theme.success));
    };
    let changed = HighlightStyle {
        color: Some(theme.success),
        font_weight: Some(FontWeight::SEMIBOLD),
        underline: Some(UnderlineStyle { thickness: px(1.5), color: Some(theme.success), wavy: false }),
        ..Default::default()
    };
    // The words of the correction that weren't in what the learner said.
    let marks = missed_ranges(corrected, &assess(corrected, &line.dutch).words).into_iter().map(|range| (range, changed));
    v_flex()
        .gap_0p5()
        .child(div().line_through().text_color(theme.muted_foreground).child(line.dutch.clone()))
        .child(div().text_lg().child(StyledText::new(corrected.clone()).with_highlights(marks.collect::<Vec<_>>())))
        .when(!line.explanation.is_empty(), |d| d.child(hint(line.explanation.clone(), cx)))
        .child(hint(how, cx))
}
