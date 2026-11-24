//! Gesprek: a conversation in Dutch about a topic of the learner's choice.
//! Claude plays the other person and the Dutch voice speaks its lines. The
//! learner answers by typing or speaking; each answer comes back corrected
//! with the next line. At the end, what they said in English or didn't
//! understand, and their corrected mistakes, become draft cards.

use std::ops::Range;

use anyhow::Result;
use dutch_flashcards_core::conversation::{self, Conversation, Line, Reply, Said, Who};
use dutch_flashcards_core::drafting;
use dutch_flashcards_core::model::NewCard;
use dutch_flashcards_core::speech::{assess, missed_ranges, write_wav_16k};
use dutch_flashcards_core::voice::Speed;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{ActiveTheme as _, Colorize as _, Disableable as _, Icon, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::TestSupportExt as _;
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, Div, ElementId, Entity, Focusable as _, FontWeight, HighlightStyle, InteractiveElement as _, IntoElement,
    ParentElement as _, ScrollHandle, SharedString, StatefulInteractiveElement as _, Styled as _, StyledText, Subscription,
    UnderlineStyle, Window, div, px,
};

use crate::app::FlashcardsApp;
use crate::capture::Capture;
use crate::icons;
use crate::recorder::Captured;
use crate::ui::{card_box, heading, icon_button, pill, surface, titled};

/// A topic's name and its icon.
pub type Topic = (&'static str, fn() -> Icon);

/// Topics offered on the start screen; any other can be typed.
pub const TOPICS: [Topic; 6] = [
    ("Het weekend", icons::weekend),
    ("Boodschappen doen", icons::shopping),
    ("Op het werk", icons::work),
    ("In een restaurant", icons::restaurant),
    ("Bij de dokter", icons::doctor),
    ("Op vakantie", icons::holiday),
];

const NOT_HEARD: &str = "Ik hoorde niets. Spreek wat luider of dichter bij de microfoon, of typ je antwoord.";

pub struct TalkState {
    pub topic: Entity<InputState>,
    pub message: Entity<InputState>,
    pub current: Option<Conversation>,
    /// The learner's sentence on its way to Claude, or the one that failed to arrive.
    pub pending: Option<Said>,
    /// Waiting for Claude's next line.
    pub thinking: bool,
    /// Claude is turning what the learner didn't know into cards.
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
        let message = cx.new(|cx| InputState::new(window, cx).placeholder("Typ je antwoord…"));
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

    /// Plays a piece of the conversation, unless the microphone is listening.
    fn play(&mut self, text: SharedString, speed: Speed, cx: &mut Context<Self>) {
        if !self.talk.capture.is_on() {
            self.speak(text.to_string(), self.current_voice(), speed, cx);
        }
    }

    /// A button that plays `text`: the ear at normal speed, the snail slowly.
    fn play_button(&self, id: impl Into<ElementId>, text: &str, speed: Speed, cx: &mut Context<Self>) -> Button {
        let text = SharedString::from(text.to_string());
        let (icon, tip) = if speed == Speed::Slow { (icons::slow(), "Langzaam") } else { (icons::listen(), "Luister") };
        icon_button(id, icon, tip).on_click(cx.listener(move |this, _, _, cx| this.play(text.clone(), speed, cx)))
    }

    /// A sentence with the ear before it and `tools` after it; clicking the
    /// sentence plays it too. `marks` highlights parts of it.
    fn playable(&self, id: &str, text: &str, marks: Vec<(Range<usize>, HighlightStyle)>, tools: impl IntoIterator<Item = Button>, cx: &mut Context<Self>) -> Div {
        let said = SharedString::from(text.to_string());
        let primary = cx.theme().primary;
        h_flex()
            .gap_1()
            .items_start()
            .child(self.play_button(SharedString::from(format!("{id}-play")), text, Speed::Normal, cx))
            .child(
                div()
                    .id(SharedString::from(id.to_string()))
                    .flex_1()
                    .min_w_0()
                    .cursor_pointer()
                    .text_lg()
                    .hover(move |style| style.text_color(primary))
                    .on_click(cx.listener(move |this, _, _, cx| this.play(said.clone(), Speed::Normal, cx)))
                    .child(StyledText::new(text.to_string()).with_highlights(marks))
                    .test_support(),
            )
            .children(tools)
    }


    /// Afronden: keeps the conversation and turns what the learner didn't know into
    /// drafts, then opens Concepten. Nothing said yet: just closes it.
    fn finish_conversation(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(current) = self.talk.current.as_ref().filter(|_| !self.talk.busy()) else { return };
        if !current.has_learner_lines() {
            return self.close_conversation(cx);
        }
        let transcript = current.transcript();
        let Some((steer, limit)) = current.card_request() else {
            return match self.keep_conversation(&transcript, "", &[], window, cx) {
                Ok(()) => self.info("Het gesprek is bewaard. Je hoefde niets in het Engels te zeggen en begreep alles, dus er zijn geen kaarten gemaakt.", cx),
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

    /// The start: a tile per topic, or one of your own, and how it works in three icons.
    fn render_topics(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let tiles = TOPICS.map(|(topic, icon)| {
            v_flex()
                .id(SharedString::from(format!("topic-{topic}")))
                .items_center()
                .justify_center()
                .gap_2()
                .h(px(92.))
                .rounded(px(10.))
                .border_1()
                .border_color(theme.border)
                .bg(theme.background)
                .cursor_pointer()
                .hover(|style| style.border_color(theme.primary).text_color(theme.primary))
                .on_click(cx.listener(move |this, _, window, cx| this.start_with_topic(topic, window, cx)))
                .child(icon().size_7())
                .child(div().text_sm().child(topic))
                .test_support()
        });
        let own = h_flex().gap_2().child(div().flex_1().child(Input::new(&self.talk.topic))).child(
            Button::new("talk-start")
                .primary()
                .icon(icons::send())
                .label("Begin")
                .on_click(cx.listener(|this, _, window, cx| this.start_conversation(window, cx))),
        );
        let how = [(icons::speak(), "Spreek of typ"), (icons::english(), "Engels mag"), (icons::cards(), "Lastig wordt een kaart")].map(|(icon, text)| {
            h_flex().gap_1p5().items_center().child(icon.size_4()).child(div().text_sm().child(text))
        });
        v_flex().flex_1().items_center().justify_center().child(
            card_box(cx)
                .w_full()
                .max_w(px(620.))
                .p_6()
                .gap_5()
                .child(titled(icons::talk(), "Waar wil je over praten?", cx))
                .child(div().grid().grid_cols(3).gap_3().children(tiles))
                .child(own)
                .child(h_flex().flex_wrap().justify_center().gap_5().text_color(theme.muted_foreground).children(how))
                .children(self.render_talk_error(cx)),
        )
    }

    fn render_conversation(&self, current: &Conversation, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let (gaps, making) = (current.gaps(), self.talk.making_cards);
        let topic = if current.topic.is_empty() { "Vrij gesprek".to_string() } else { current.topic.clone() };
        let finish = Button::new("talk-finish")
            .flex_none()
            .primary()
            .loading(making)
            .disabled(self.talk.busy())
            .tooltip("Klaar: maak kaarten van wat lastig was")
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .when(!making, |d| d.child(icons::cards().size_4()))
                    .child(if making { "Kaarten maken…" } else { "Afronden" })
                    .when(gaps > 0 && !making, |d| d.child(pill(gaps.to_string(), theme.primary_foreground, theme.primary))),
            )
            .on_click(cx.listener(|this, _, window, cx| this.finish_conversation(window, cx)));
        let header = h_flex()
            .gap_2p5()
            .items_center()
            .pb_3()
            .border_b_1()
            .border_color(theme.border)
            .child(topic_icon(&current.topic).size_5().text_color(theme.muted_foreground))
            .child(div().flex_1().min_w_0().child(heading(topic)))
            .child(finish);
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
        let partner = line.who == Who::Partner;
        let content = if partner { self.render_partner_text(index, line, cx) } else { self.render_learner_text(index, line, cx) };
        let row = h_flex().w_full().when(!partner, |d| d.justify_end());
        row.child(bubble(partner, cx).child(content)).into_any_element()
    }

    /// The partner's Dutch with its tools, its English once opened, and the
    /// words the learner didn't understand.
    fn render_partner_text(&self, index: usize, line: &Line, cx: &mut Context<Self>) -> Div {
        let slow = self.play_button(("line-slow", index as u64), &line.dutch, Speed::Slow, cx);
        let meaning = icon_button(("line-meaning", index as u64), icons::meaning(), "Betekenis")
            .when(line.meaning_shown, |button| button.outline().primary())
            .on_click(cx.listener(move |this, _, _, cx| this.change_line(index, Conversation::toggle_meaning, cx)));
        let unclear: Vec<_> = line
            .not_understood
            .iter()
            .enumerate()
            .map(|(k, words)| self.gap_chip(&format!("unclear-{index}-{k}"), icons::unclear(), words, None, cx))
            .collect();
        v_flex()
            .gap_1p5()
            .child(self.playable(&format!("line-{index}"), &line.dutch, Vec::new(), [slow, meaning], cx))
            .when(line.meaning_shown, |d| d.child(div().pl(INDENT).italic().text_color(cx.theme().muted_foreground).child(line.english.clone())))
            .when(!unclear.is_empty(), |d| d.child(h_flex().pl(INDENT).flex_wrap().gap_1p5().children(unclear)))
    }

    /// What the learner said: as it was, or struck through above the
    /// correction with its changed words marked. Then why, and the Dutch for
    /// what they said in English.
    fn render_learner_text(&self, index: usize, line: &Line, cx: &mut Context<Self>) -> Div {
        let theme = cx.theme().clone();
        let meant = line.meant();
        let slow = self.play_button(("line-slow", index as u64), meant, Speed::Slow, cx);
        let marks = match &line.corrected {
            None => Vec::new(),
            Some(corrected) => {
                let changed = HighlightStyle {
                    color: Some(theme.success),
                    font_weight: Some(FontWeight::SEMIBOLD),
                    underline: Some(UnderlineStyle { thickness: px(1.5), color: Some(theme.success), wavy: false }),
                    ..Default::default()
                };
                // The words of the correction that weren't in what the learner said.
                missed_ranges(corrected, &assess(corrected, &line.dutch).words).into_iter().map(|range| (range, changed)).collect()
            }
        };
        let english: Vec<_> = line
            .english_used
            .iter()
            .enumerate()
            .map(|(k, used)| self.gap_chip(&format!("english-{index}-{k}"), icons::english(), &used.dutch, Some(&used.english), cx))
            .collect();
        let muted = theme.muted_foreground;
        v_flex()
            .gap_1p5()
            .when(line.corrected.is_some(), |d| d.child(div().pl(INDENT).text_sm().line_through().text_color(muted).child(line.dutch.clone())))
            .child(self.playable(&format!("line-{index}"), meant, marks, [slow], cx))
            .when(!line.explanation.is_empty(), |d| {
                d.child(
                    h_flex()
                        .pl(INDENT)
                        .gap_1p5()
                        .items_start()
                        .text_color(muted)
                        .child(icons::tip().size_4().mt(px(1.)))
                        .child(div().flex_1().min_w_0().text_sm().child(line.explanation.clone())),
                )
            })
            .when(!english.is_empty(), |d| d.child(h_flex().pl(INDENT).flex_wrap().gap_1p5().children(english)))
    }

    /// Dutch the learner didn't know, marked with `icon` and, for English, what
    /// they said instead. Clicking it plays the Dutch.
    fn gap_chip(&self, id: &str, icon: Icon, dutch: &str, english: Option<&str>, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let said = SharedString::from(dutch.to_string());
        h_flex()
            .id(SharedString::from(id.to_string()))
            .gap_1p5()
            .items_center()
            .px_2()
            .py_0p5()
            .rounded(px(6.))
            .border_1()
            .border_color(theme.warning.opacity(0.6))
            .cursor_pointer()
            .hover(|style| style.bg(theme.secondary_hover))
            .on_click(cx.listener(move |this, _, _, cx| this.play(said.clone(), Speed::Normal, cx)))
            .child(icon.size_4().text_color(theme.warning))
            .child(div().text_sm().font_weight(FontWeight::MEDIUM).child(dutch.to_string()))
            .children(english.map(|english| div().text_xs().text_color(theme.muted_foreground).child(english.to_string())))
            .test_support()
            .into_any_element()
    }

    /// While waiting: the sentence on its way, and dots where the answer will come.
    fn render_pending(&self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        let theme = cx.theme().clone();
        let said = self.talk.pending.as_ref().map(|said| {
            let retry = (!self.talk.thinking)
                .then(|| icon_button("talk-resend", icons::retry(), "Opnieuw versturen").outline().on_click(cx.listener(|this, _, _, cx| this.resend(cx))));
            h_flex()
                .w_full()
                .justify_end()
                .gap_2()
                .items_center()
                .children(retry)
                .child(bubble(false, cx).opacity(0.6).child(div().text_lg().child(said.text.clone())))
        });
        let waiting = if self.talk.thinking {
            Some(h_flex().w_full().child(bubble(true, cx).child(dots(&theme))))
        } else if self.talk.hearing {
            let heard = h_flex().gap_2().items_center().child(icons::listen().small().text_color(theme.muted_foreground)).child(dots(&theme));
            Some(h_flex().w_full().justify_end().child(bubble(false, cx).child(heard)))
        } else {
            None
        };
        (said.is_some() || waiting.is_some()).then(|| v_flex().gap_3().children(said).children(waiting))
    }

    fn render_talk_error(&self, cx: &App) -> Option<Div> {
        let error = self.talk.error.clone()?;
        Some(div().text_sm().text_color(cx.theme().danger).child(error))
    }

    /// The answer field with send and speak, or the level meter while recording.
    fn render_composer(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let open = self.talk.can_answer();
        let recording = self.talk.capture.is_on();
        let speak = Button::new("talk-speak")
            .icon(if recording { icons::stop() } else { icons::speak() })
            .tooltip(if recording { "Stop" } else { "Spreek je antwoord" })
            .disabled(!(open || recording) || !self.listener.is_installed())
            .map(|button| if recording { button.danger() } else { button.outline() })
            .on_click(cx.listener(|this, _, _, cx| this.speak_answer(cx)));
        let field = match self.talk.capture.render("Zeg je antwoord.", cx) {
            Some(meter) => div().flex_1().child(meter),
            None => div().flex_1().child(Input::new(&self.talk.message).disabled(!open)),
        };
        let send = Button::new("talk-send")
            .primary()
            .icon(icons::send())
            .tooltip("Verstuur (Enter)")
            .disabled(!open || recording)
            .on_click(cx.listener(|this, _, window, cx| this.send_typed(window, cx)));
        h_flex().gap_2().items_center().pt_3().border_t_1().border_color(cx.theme().border).child(field).child(send).child(speak)
    }
}

/// How far text under a sentence is indented, to line up with the sentence after its ear.
const INDENT: gpui_kit::Pixels = px(30.);

/// The topic's own icon, or the speech bubble for any other topic.
fn topic_icon(topic: &str) -> Icon {
    TOPICS.iter().find(|(name, _)| name.eq_ignore_ascii_case(topic)).map_or_else(icons::speak, |(_, icon)| icon())
}

/// A line's balloon: the partner's on the surface colour, the learner's in the accent.
fn bubble(partner: bool, cx: &App) -> Div {
    let theme = cx.theme();
    v_flex()
        .max_w(px(600.))
        .px_4()
        .py_3()
        .rounded(px(12.))
        .border_1()
        .border_color(theme.border)
        .bg(if partner { surface(cx) } else { theme.accent })
}

/// Three dots: an answer is on its way.
fn dots(theme: &gpui_kit::component::theme::Theme) -> Div {
    h_flex().gap_1().py_1p5().children((0..3).map(|_| div().size(px(6.)).rounded_full().bg(theme.muted_foreground)))
}
