//! Oefenen: one panel per card. Read the Dutch, test your reading, hear the
//! sentence, look up the meaning, and score how easily it came back.

use dutch_flashcards_core::schedule::{Scheduler, describe_due};
use dutch_flashcards_core::speech::{Assessment, missed_ranges};
use dutch_flashcards_core::store::{Counts, Mode, Review};
use dutch_flashcards_core::voice::Speed;
use dutch_flashcards_core::{Card, Grade};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{ActiveTheme as _, Disableable as _, Icon, Selectable as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, Context, FocusHandle, FontWeight, HighlightStyle, InteractiveElement as _, IntoElement, KeyDownEvent,
    MouseButton, ParentElement as _, SharedString, StatefulInteractiveElement as _, Styled as _, StyledText, UnderlineStyle,
    Window, div, px, relative,
};

use crate::app::{FlashcardsApp, Screen};
use crate::capture::Capture;
use crate::icons;
use crate::speaking::grade_color;
use crate::ui::{empty_state, hint, keycap};

pub struct StudyState {
    pub focus: FocusHandle,
    pub mode: Mode,
    pub card: Option<Card>,
    pub counts: Counts,
    pub reviewed_today: usize,
    pub attempt: Attempt,
    pub scheduler: Scheduler,
}

/// What the learner has done with the current card so far.
#[derive(Default)]
pub struct Attempt {
    /// Luisteren: the text stays hidden until the learner asks for it.
    pub revealed: bool,
    pub english_shown: bool,
    /// The meaning was opened during this review, even if hidden again.
    pub peeked: bool,
    // Testing the reading aloud (Lezen).
    pub capture: Capture,
    pub judging: bool,
    pub speech: Option<Assessment>,
    pub speech_note: Option<SharedString>,
    pub has_recording: bool,
}

impl Attempt {
    /// The reading test button's label.
    fn test_label(&self) -> &'static str {
        if self.capture.is_on() {
            "Stop"
        } else if self.judging {
            "Even luisteren…"
        } else if self.speech.is_some() || self.speech_note.is_some() {
            "Opnieuw"
        } else {
            "Test mijn uitspraak"
        }
    }
}

impl StudyState {
    pub fn new(mode: Mode, cx: &mut App) -> Self {
        StudyState { focus: cx.focus_handle(), mode, card: None, counts: Counts::default(), reviewed_today: 0, attempt: Attempt::default(), scheduler: Scheduler::default() }
    }

    /// Scores are open from the start in Lezen, and once the text is shown in Luisteren.
    fn scoring_open(&self) -> bool {
        self.card.is_some() && (self.mode == Mode::Read || self.attempt.revealed)
    }

    /// Luisteren hides the text until the learner asks for it.
    fn veiled(&self) -> bool {
        self.mode == Mode::Listen && !self.attempt.revealed
    }
}

/// What Oefenen says when no card is waiting, and where its button leads.
struct Rest {
    title: &'static str,
    detail: String,
    action: Option<(&'static str, Screen)>,
}

impl Rest {
    fn new(deck_size: usize, drafts: usize, reviewed: usize, next_due: Option<String>) -> Rest {
        let to_drafts = (drafts > 0).then_some(("Naar concepten", Screen::Drafts));
        match (deck_size, drafts) {
            (0, 0) => Rest {
                title: "Je stapel is nog leeg",
                detail: "Plak een stuk van een gesprek met Claude, ChatGPT of Gemini bij Toevoegen. Daar worden kaarten van gemaakt.".into(),
                action: Some(("Kaarten toevoegen", Screen::Add)),
            },
            (0, _) => Rest {
                title: "Controleer eerst je concepten",
                detail: "Er staan kaarten klaar. Bewaar ze, dan kun je ze hier oefenen.".into(),
                action: to_drafts,
            },
            _ => {
                let parts = [
                    (reviewed > 0).then(|| format!("Je hebt {reviewed} kaarten geoefend.")),
                    next_due.map(|when| format!("De volgende kaart is {when}.")),
                    (drafts > 0).then(|| format!("Er wachten {drafts} concepten op controle.")),
                ];
                Rest { title: "Klaar voor nu", detail: parts.into_iter().flatten().collect::<Vec<_>>().join(" "), action: to_drafts }
            }
        }
    }
}

impl FlashcardsApp {
    pub fn load_next_card(&mut self, cx: &mut Context<Self>) {
        let (now, per_day) = (Self::now(), self.settings.new_per_day);
        self.study.counts = self.store.counts(now, per_day).unwrap_or_default();
        match self.store.next_card(now, per_day) {
            Ok(card) => self.start_card(card, cx),
            Err(err) => self.error(format!("{err:#}")),
        }
    }

    fn start_card(&mut self, card: Option<Card>, cx: &mut Context<Self>) {
        self.stop_audio();
        self.study.attempt = Attempt::default();
        let text = card.as_ref().map(|card| card.dutch.clone());
        self.study.card = card;
        match (text, self.study.mode) {
            (Some(text), Mode::Listen) => self.say(text, cx),
            (Some(text), Mode::Read) => self.prepare_audio(text, cx),
            (None, _) => {}
        }
        cx.notify();
    }

    fn set_mode(&mut self, mode: Mode, cx: &mut Context<Self>) {
        if self.study.mode == mode {
            return;
        }
        (self.study.mode, self.settings.mode) = (mode, mode);
        self.save_settings();
        let card = self.study.card.take();
        self.start_card(card, cx);
    }

    /// Space: test the reading (or stop the recording) in Lezen; show the text in Luisteren.
    fn advance(&mut self, cx: &mut Context<Self>) {
        match self.study.mode {
            _ if self.study.card.is_none() => {}
            Mode::Read if self.study.attempt.capture.is_on() => self.study.attempt.capture.stop(),
            Mode::Read => self.start_recording(cx),
            Mode::Listen => self.study.attempt.revealed = true,
        }
        cx.notify();
    }

    /// Plays the sentence in the Dutch voice chosen in the settings.
    fn hear(&mut self, speed: Speed, cx: &mut Context<Self>) {
        // The microphone would pick the voice up.
        let Some(card) = self.study.card.as_ref().filter(|_| !self.busy_listening()) else { return };
        self.speak(card.dutch.clone(), self.current_voice(), speed, cx);
    }

    fn toggle_english(&mut self, cx: &mut Context<Self>) {
        if self.study.card.is_some() {
            let attempt = &mut self.study.attempt;
            attempt.english_shown = !attempt.english_shown;
            attempt.peeked |= attempt.english_shown;
        }
        cx.notify();
    }

    /// Saves the learner's score and moves straight to the next card.
    fn score(&mut self, own: Grade, cx: &mut Context<Self>) {
        let Some(card) = self.study.card.as_ref().filter(|_| self.study.scoring_open() && !self.busy_listening()) else { return };
        let attempt = &self.study.attempt;
        let review = Review { own, speech: attempt.speech.as_ref(), mode: self.study.mode, peeked: attempt.peeked };
        if let Err(err) = self.store.record_review(card.id, review, &self.study.scheduler, Self::now()) {
            return self.error(format!("De beoordeling is niet bewaard: {err:#}"));
        }
        self.study.reviewed_today += 1;
        self.load_next_card(cx);
    }

    fn edit_current(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(id) = self.study.card.as_ref().map(|card| card.id) {
            self.show(Screen::Deck, window, cx);
            self.start_editing(id, window, cx);
        }
    }

    fn on_study_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let modifiers = &event.keystroke.modifiers;
        let plain = !(modifiers.control || modifiers.alt || modifiers.platform || event.is_held);
        if plain && self.study_key(&event.keystroke.key, window, cx) {
            cx.stop_propagation();
        }
    }

    /// Handles one study shortcut. Returns false for keys it doesn't use.
    pub fn study_key(&mut self, key: &str, window: &mut Window, cx: &mut Context<Self>) -> bool {
        match key {
            "space" => self.advance(cx),
            "escape" if self.study.attempt.capture.is_on() => self.cancel_recording(cx),
            "r" => self.start_recording(cx),
            "o" => self.play_own_recording(cx),
            "v" => self.toggle_english(cx),
            "h" => self.hear(Speed::Normal, cx),
            "l" => self.hear(Speed::Slow, cx),
            "b" => self.edit_current(window, cx),
            _ => match key.parse().ok().and_then(Grade::from_number) {
                Some(grade) => self.score(grade, cx),
                None => return false,
            },
        }
        true
    }

    /// A click handler that runs `action` and gives the keyboard back to the study panel.
    fn study_click<E>(&self, action: fn(&mut Self, &mut Context<Self>), cx: &mut Context<Self>) -> impl Fn(&E, &mut Window, &mut App) + 'static {
        cx.listener(move |this, _: &E, window, cx| {
            action(this, cx);
            this.study.focus.focus(window, cx);
        })
    }

    pub fn render_study(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let content = if self.study.card.is_some() {
            self.render_card(cx).into_any_element()
        } else {
            self.render_rest(cx).into_any_element()
        };
        v_flex()
            .id("study")
            .key_context("Study")
            .track_focus(&self.study.focus)
            .on_key_down(cx.listener(Self::on_study_key))
            .on_mouse_down(MouseButton::Left, cx.listener(|this, _, window, cx| this.study.focus.focus(window, cx)))
            .size_full()
            .p_5()
            .gap_4()
            .child(self.render_study_header(cx))
            .child(content)
    }

    fn render_study_header(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let modes = [(Mode::Read, "Lezen"), (Mode::Listen, "Luisteren")];
        let Counts { due, new } = self.study.counts;
        let has_card = self.study.card.is_some();
        let summary = if has_card || due + new > 0 { format!("{due} te herhalen · {new} nieuw") } else { String::new() };
        h_flex()
            .justify_between()
            .flex_wrap()
            .gap_3()
            .child(h_flex().gap_1().children(modes.map(|(mode, label)| {
                Button::new(SharedString::from(format!("mode-{label}")))
                    .label(label)
                    .small()
                    .outline()
                    .tab_stop(false)
                    .selected(self.study.mode == mode)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.set_mode(mode, cx);
                        this.study.focus.focus(window, cx);
                    }))
            })))
            .child(h_flex().gap_3().child(hint(summary, cx)).when(has_card, |d| {
                d.child(
                    Button::new("edit")
                        .ghost()
                        .small()
                        .tab_stop(false)
                        .child(h_flex().gap_1p5().child("Bewerken").child(keycap("B", cx)))
                        .on_click(cx.listener(|this, _, window, cx| this.edit_current(window, cx))),
                )
            }))
    }

    fn render_card(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let sound = match (self.sound.preparing, self.sound.playing) {
            (true, _) => "stem wordt gemaakt…",
            (_, true) => "speelt af",
            _ => "",
        };
        let reading = if self.study.mode == Mode::Read { self.render_reading_status(cx) } else { None };
        let bottom = if self.study.scoring_open() {
            self.render_scores(cx).into_any_element()
        } else {
            Button::new("reveal")
                .primary()
                .tab_stop(false)
                .child(h_flex().gap_2().child("Toon tekst").child(keycap("Spatie", cx)))
                .on_click(self.study_click(Self::advance, cx))
                .into_any_element()
        };
        v_flex()
            .flex_1()
            .min_h_0()
            .items_center()
            .justify_center()
            .gap_4()
            .children(self.render_phrase(cx))
            .child(div().h_4().text_xs().text_color(theme.primary).child(sound))
            .children(reading)
            .children(self.render_meaning(cx))
            .child(self.render_tools(cx))
            .child(bottom)
    }

    /// The Dutch, with the words the app missed underlined; dots while veiled.
    fn render_phrase(&self, cx: &App) -> Option<impl IntoElement> {
        let theme = cx.theme();
        let card = self.study.card.as_ref()?;
        let veiled = self.study.veiled();
        let missed = HighlightStyle {
            color: Some(theme.danger),
            underline: Some(UnderlineStyle { thickness: px(2.), color: Some(theme.danger), wavy: false }),
            ..Default::default()
        };
        let highlights = match &self.study.attempt.speech {
            Some(result) if !veiled => missed_ranges(&card.dutch, &result.words).into_iter().map(|range| (range, missed)).collect(),
            _ => Vec::new(),
        };
        let text: SharedString = if veiled { "• • •".into() } else { card.dutch.clone().into() };
        Some(
            div()
                .max_w(px(720.))
                .text_center()
                .text_size(px(34.))
                .line_height(relative(1.25))
                .font_weight(FontWeight::MEDIUM)
                .when(veiled, |d| d.text_color(theme.muted_foreground))
                .child(StyledText::new(text).with_highlights(highlights)),
        )
    }

    /// The English and the note, once the learner opens them.
    fn render_meaning(&self, cx: &App) -> Option<impl IntoElement> {
        let card = self.study.card.as_ref().filter(|_| self.study.attempt.english_shown)?;
        let theme = cx.theme();
        let english = if card.english.is_empty() { "(geen vertaling)".to_string() } else { card.english.clone() };
        Some(
            v_flex()
                .items_center()
                .gap_1()
                .max_w(px(560.))
                .pt_2()
                .border_t_1()
                .border_color(theme.border)
                .text_center()
                .child(div().italic().text_color(theme.muted_foreground).child(english))
                .when(!card.note.is_empty(), |d| d.child(hint(card.note.clone(), cx))),
        )
    }

    /// Test mijn uitspraak, Hoor de zin and Betekenis.
    fn render_tools(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let attempt = &self.study.attempt;
        let tool = |id: &'static str, icon: Icon, label: &str, key: &str, cx: &App| {
            Button::new(id).outline().tab_stop(false).icon(icon).child(h_flex().gap_2().child(label.to_string()).child(keycap(key, cx)))
        };
        let test = (self.study.mode == Mode::Read).then(|| {
            tool("test-reading", icons::speak(), attempt.test_label(), "Spatie", cx)
                .disabled(!self.listener.is_installed() || attempt.judging)
                .on_click(self.study_click(Self::advance, cx))
        });
        h_flex()
            .flex_wrap()
            .justify_center()
            .gap_2()
            .children(test)
            .child(
                tool("hear", icons::listen(), "Hoor de zin", "H", cx)
                    .disabled(self.busy_listening())
                    .on_click(self.study_click(|this, cx| this.hear(Speed::Normal, cx), cx)),
            )
            .child(
                tool("meaning", icons::meaning(), "Betekenis", "V", cx)
                    .selected(attempt.english_shown)
                    .on_click(self.study_click(Self::toggle_english, cx)),
            )
    }

    /// The four scores: how easily the sentence came back. Nothing else.
    fn render_scores(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let open = !self.busy_listening();
        h_flex().gap_2().w_full().max_w(px(600.)).children(Grade::ALL.map(|grade| {
            v_flex()
                .id(SharedString::from(format!("grade-{}", grade.number())))
                .flex_1()
                .items_center()
                .gap_1()
                .pb_2()
                .overflow_hidden()
                .rounded(px(6.))
                .border_1()
                .border_color(theme.border)
                .bg(theme.secondary)
                .child(div().w_full().h(px(4.)).mb_1p5().bg(grade_color(grade, cx)))
                .child(div().font_weight(FontWeight::MEDIUM).child(grade.label()))
                .child(keycap(&grade.number().to_string(), cx))
                .when(!open, |d| d.opacity(0.5))
                .when(open, |d| {
                    d.cursor_pointer().hover(|s| s.bg(theme.secondary_hover)).on_click(cx.listener(move |this, _, window, cx| {
                        this.score(grade, cx);
                        this.study.focus.focus(window, cx);
                    }))
                })
        }))
    }

    /// No card waiting: why, and what to do next.
    fn render_rest(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let now = Self::now();
        let next_due = self.store.next_due_after(now).ok().flatten().map(|due| describe_due(due, now));
        let rest = Rest::new(self.deck.cards.len(), self.drafts.len(), self.study.reviewed_today, next_due);
        v_flex()
            .flex_1()
            .items_center()
            .justify_center()
            .child(empty_state(rest.title, &rest.detail, cx))
            .when_some(rest.action, |d, (label, screen)| {
                d.child(
                    Button::new("empty-action")
                        .primary()
                        .label(label)
                        .on_click(cx.listener(move |this, _, window, cx| this.show(screen, window, cx))),
                )
            })
    }
}
