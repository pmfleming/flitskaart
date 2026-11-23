//! Oefenen: one panel per card. Read the Dutch, test your reading, hear the
//! sentence, look up the meaning, and score how easily it came back.

use dutch_flashcards_core::speech::{Assessment, missed_ranges};
use dutch_flashcards_core::store::{Counts, Mode};
use dutch_flashcards_core::voice::Speed;
use dutch_flashcards_core::{Card, Grade, Status};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{ActiveTheme as _, Disableable as _, Icon, Selectable as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::app::{FlashcardsApp, Screen};
use crate::icons;
use crate::recorder::Recording;
use crate::ui::{empty_state, hint, keycap};

pub struct StudyState {
    pub focus: FocusHandle,
    pub mode: Mode,
    pub card: Option<Card>,
    /// Luisteren: the text stays hidden until the learner asks for it.
    pub revealed: bool,
    pub english_shown: bool,
    /// The meaning was opened during this review, even if hidden again.
    pub peeked: bool,
    pub counts: Counts,
    pub reviewed_today: usize,
    // Testing the reading aloud (Lezen).
    pub recording: Option<Recording>,
    pub level: f32,
    pub judging: bool,
    pub speech: Option<Assessment>,
    pub speech_note: Option<SharedString>,
    pub has_recording: bool,
}

impl StudyState {
    pub fn new(mode: Mode, cx: &mut App) -> Self {
        StudyState {
            focus: cx.focus_handle(),
            mode,
            card: None,
            revealed: false,
            english_shown: false,
            peeked: false,
            counts: Counts::default(),
            reviewed_today: 0,
            recording: None,
            level: 0.0,
            judging: false,
            speech: None,
            speech_note: None,
            has_recording: false,
        }
    }

    /// Scores are open from the start in Lezen, and once the text is shown in Luisteren.
    fn scoring_open(&self) -> bool {
        self.card.is_some() && (self.mode == Mode::Read || self.revealed)
    }
}

impl FlashcardsApp {
    pub fn load_next_card(&mut self, cx: &mut Context<Self>) {
        let now = Self::now();
        let per_day = self.settings.new_per_day;
        let next = self.store.next_card(now, per_day);
        self.study.counts = self.store.counts(now, per_day).unwrap_or_default();
        match next {
            Ok(card) => self.start_card(card, cx),
            Err(err) => self.error(format!("{err:#}")),
        }
    }

    fn start_card(&mut self, card: Option<Card>, cx: &mut Context<Self>) {
        self.stop_audio();
        let study = &mut self.study;
        study.revealed = false;
        study.english_shown = false;
        study.peeked = false;
        study.recording = None;
        study.level = 0.0;
        study.judging = false;
        study.speech = None;
        study.speech_note = None;
        study.has_recording = false;
        study.card = card;
        if let Some(card) = &self.study.card {
            let text = card.dutch.clone();
            match self.study.mode {
                Mode::Listen => self.speak(text, self.current_voice(), Speed::Normal, cx),
                Mode::Read => self.prepare_audio(text, cx),
            }
        }
        cx.notify();
    }

    fn set_mode(&mut self, mode: Mode, cx: &mut Context<Self>) {
        if self.study.mode == mode {
            return;
        }
        self.study.mode = mode;
        self.settings.mode = mode;
        self.save_settings();
        let card = self.study.card.take();
        self.start_card(card, cx);
    }

    /// Space: test the reading (or stop the recording) in Lezen; show the text in Luisteren.
    fn advance(&mut self, cx: &mut Context<Self>) {
        if self.study.card.is_none() {
            return;
        }
        match self.study.mode {
            Mode::Read if self.study.recording.is_some() => self.stop_recording(),
            Mode::Read => self.start_recording(cx),
            Mode::Listen => self.study.revealed = true,
        }
        cx.notify();
    }

    /// Plays the sentence in the Dutch voice chosen in the settings.
    fn hear(&mut self, speed: Speed, cx: &mut Context<Self>) {
        // The microphone would pick the voice up.
        if self.busy_listening() {
            return;
        }
        let Some(card) = &self.study.card else { return };
        let text = card.dutch.clone();
        self.speak(text, self.current_voice(), speed, cx);
    }

    fn toggle_english(&mut self, cx: &mut Context<Self>) {
        if self.study.card.is_none() {
            return;
        }
        self.study.english_shown = !self.study.english_shown;
        if self.study.english_shown {
            self.study.peeked = true;
        }
        cx.notify();
    }

    /// Saves the learner's score and moves straight to the next card.
    fn score(&mut self, grade: Grade, cx: &mut Context<Self>) {
        if !self.study.scoring_open() || self.busy_listening() {
            return;
        }
        let Some(card) = &self.study.card else { return };
        let result = self.store.record_review(
            card.id,
            grade,
            self.study.speech.as_ref(),
            self.study.mode,
            self.study.peeked,
            &self.scheduler,
            Self::now(),
        );
        if let Err(err) = result {
            self.error(format!("De beoordeling is niet bewaard: {err:#}"));
            return;
        }
        self.study.reviewed_today += 1;
        self.load_next_card(cx);
    }

    fn edit_current(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(card) = self.study.card.clone() {
            self.show(Screen::Deck, window, cx);
            self.start_editing(&card, window, cx);
        }
    }

    fn on_study_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let keystroke = &event.keystroke;
        if keystroke.modifiers.control || keystroke.modifiers.alt || keystroke.modifiers.platform || event.is_held {
            return;
        }
        if self.study_key(&keystroke.key, window, cx) {
            cx.stop_propagation();
        }
    }

    /// Handles one study shortcut. Returns false for keys it doesn't use.
    pub fn study_key(&mut self, key: &str, window: &mut Window, cx: &mut Context<Self>) -> bool {
        match key {
            "space" => self.advance(cx),
            "escape" if self.study.recording.is_some() => self.cancel_recording(cx),
            "r" => self.start_recording(cx),
            "o" => self.play_own_recording(cx),
            "1" | "2" | "3" | "4" => {
                if let Some(grade) = key.parse().ok().and_then(Grade::from_number) {
                    self.score(grade, cx);
                }
            }
            "v" => self.toggle_english(cx),
            "h" => self.hear(Speed::Normal, cx),
            "l" => self.hear(Speed::Slow, cx),
            "b" => self.edit_current(window, cx),
            _ => return false,
        }
        true
    }

    pub fn render_study(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let focus = self.study.focus.clone();
        let content = if self.study.card.is_some() {
            self.render_card(cx).into_any_element()
        } else {
            self.render_study_empty(cx).into_any_element()
        };
        v_flex()
            .id("study")
            .key_context("Study")
            .track_focus(&focus)
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
        let counts = self.study.counts;
        let has_card = self.study.card.is_some();
        let summary = if has_card || counts.due + counts.new > 0 {
            format!("{} te herhalen · {} nieuw", counts.due, counts.new)
        } else {
            String::new()
        };
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
            .child(
                h_flex().gap_3().child(hint(summary, cx)).when(has_card, |d| {
                    d.child(
                        Button::new("edit")
                            .ghost()
                            .small()
                            .tab_stop(false)
                            .child(h_flex().gap_1p5().child("Bewerken").child(keycap("B", cx)))
                            .on_click(cx.listener(|this, _, window, cx| this.edit_current(window, cx))),
                    )
                }),
            )
    }

    fn render_card(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let study = &self.study;
        let card = study.card.as_ref().unwrap();
        let veiled = study.mode == Mode::Listen && !study.revealed;

        // Words the app missed are marked in the sentence itself.
        let missed = HighlightStyle {
            color: Some(theme.danger),
            underline: Some(UnderlineStyle { thickness: px(2.), color: Some(theme.danger), wavy: false }),
            ..Default::default()
        };
        let highlights = match &study.speech {
            Some(result) if !veiled => missed_ranges(&card.dutch, &result.words).into_iter().map(|range| (range, missed)).collect(),
            _ => Vec::new(),
        };
        let text: SharedString = if veiled { "• • •".into() } else { card.dutch.clone().into() };
        let phrase = div()
            .max_w(px(720.))
            .text_center()
            .text_size(px(34.))
            .line_height(relative(1.25))
            .font_weight(FontWeight::MEDIUM)
            .when(veiled, |d| d.text_color(theme.muted_foreground))
            .child(StyledText::new(text).with_highlights(highlights));

        let sound = if self.preparing_audio {
            "stem wordt gemaakt…"
        } else if self.playing {
            "speelt af"
        } else {
            ""
        };

        let meaning = study.english_shown.then(|| {
            v_flex()
                .items_center()
                .gap_1()
                .max_w(px(560.))
                .pt_2()
                .border_t_1()
                .border_color(theme.border)
                .text_center()
                .child(div().italic().text_color(theme.muted_foreground).child(if card.english.is_empty() {
                    "(geen vertaling)".to_string()
                } else {
                    card.english.clone()
                }))
                .when(!card.note.is_empty(), |d| d.child(hint(card.note.clone(), cx)))
        });

        let reading = if study.mode == Mode::Read { self.render_reading_status(cx) } else { None };
        let bottom = if study.scoring_open() {
            self.render_scores(cx).into_any_element()
        } else {
            Button::new("reveal")
                .primary()
                .tab_stop(false)
                .child(h_flex().gap_2().child("Toon tekst").child(keycap("Spatie", cx)))
                .on_click(cx.listener(|this, _, window, cx| {
                    this.advance(cx);
                    this.study.focus.focus(window, cx);
                }))
                .into_any_element()
        };

        v_flex()
            .flex_1()
            .min_h_0()
            .items_center()
            .justify_center()
            .gap_4()
            .child(phrase)
            .child(div().h_4().text_xs().text_color(theme.primary).child(sound))
            .children(reading)
            .children(meaning)
            .child(self.render_tools(cx))
            .child(bottom)
    }

    /// Test mijn uitspraak, Hoor de zin and Betekenis.
    fn render_tools(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let study = &self.study;
        let busy = self.busy_listening();
        let tool = |id: &'static str, icon: Icon, label: &str, key: &str, cx: &App| {
            Button::new(id)
                .outline()
                .tab_stop(false)
                .icon(icon)
                .child(h_flex().gap_2().child(label.to_string()).child(keycap(key, cx)))
        };
        let test = (study.mode == Mode::Read).then(|| {
            let label = if study.recording.is_some() {
                "Stop"
            } else if study.judging {
                "Even luisteren…"
            } else if study.speech.is_some() || study.speech_note.is_some() {
                "Opnieuw"
            } else {
                "Test mijn uitspraak"
            };
            tool("test-reading", icons::speak(), label, "Spatie", cx)
                .disabled(!self.listener.is_installed() || study.judging)
                .on_click(cx.listener(|this, _, window, cx| {
                    this.advance(cx);
                    this.study.focus.focus(window, cx);
                }))
        });
        h_flex()
            .flex_wrap()
            .justify_center()
            .gap_2()
            .children(test)
            .child(tool("hear", icons::listen(), "Hoor de zin", "H", cx).disabled(busy).on_click(cx.listener(
                |this, _, window, cx| {
                    this.hear(Speed::Normal, cx);
                    this.study.focus.focus(window, cx);
                },
            )))
            .child(
                tool("meaning", icons::meaning(), "Betekenis", "V", cx)
                    .selected(study.english_shown)
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.toggle_english(cx);
                        this.study.focus.focus(window, cx);
                    })),
            )
    }

    /// The four scores: how easily the sentence came back. Nothing else.
    fn render_scores(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let open = !self.busy_listening();
        let colors = [theme.danger, theme.warning, theme.blue, theme.success];
        h_flex().gap_2().w_full().max_w(px(600.)).children(Grade::ALL.iter().enumerate().map(|(i, grade)| {
            let grade = *grade;
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
                .child(div().w_full().h(px(4.)).mb_1p5().bg(colors[i]))
                .child(div().font_weight(FontWeight::MEDIUM).child(grade.label()))
                .child(keycap(&grade.number().to_string(), cx))
                .when(!open, |d| d.opacity(0.5))
                .when(open, |d| {
                    d.cursor_pointer().hover(|s| s.bg(theme.secondary_hover)).on_click(cx.listener(
                        move |this, _, window, cx| {
                            this.score(grade, cx);
                            this.study.focus.focus(window, cx);
                        },
                    ))
                })
        }))
    }

    fn render_study_empty(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let deck_size = self.deck.cards.iter().filter(|c| c.status != Status::Draft).count();
        let drafts = self.drafts.len();
        let (title, detail, button) = if deck_size == 0 && drafts == 0 {
            (
                "Je stapel is nog leeg",
                "Plak een stuk van een gesprek met Claude, ChatGPT of Gemini bij Toevoegen. Daar worden kaarten van gemaakt.",
                Some(("Kaarten toevoegen", Screen::Add)),
            )
        } else if drafts > 0 && deck_size == 0 {
            (
                "Controleer eerst je concepten",
                "Er staan kaarten klaar. Bewaar ze, dan kun je ze hier oefenen.",
                Some(("Naar concepten", Screen::Drafts)),
            )
        } else {
            ("Klaar voor nu", "", if drafts > 0 { Some(("Naar concepten", Screen::Drafts)) } else { None })
        };
        let detail = if detail.is_empty() {
            let next = self.store.next_due_after(Self::now()).ok().flatten();
            let reviewed = self.study.reviewed_today;
            let mut text = if reviewed > 0 { format!("Je hebt {reviewed} kaarten geoefend. ") } else { String::new() };
            if let Some(next) = next {
                text.push_str(&format!(
                    "De volgende kaart is {}.",
                    dutch_flashcards_core::schedule::describe_due(next, Self::now())
                ));
            }
            if drafts > 0 {
                text.push_str(&format!(" Er wachten {drafts} concepten op controle."));
            }
            text
        } else {
            detail.to_string()
        };
        v_flex()
            .flex_1()
            .items_center()
            .justify_center()
            .child(empty_state(title, &detail, cx))
            .when_some(button, |d, (label, screen)| {
                d.child(Button::new("empty-action").primary().label(label).on_click(cx.listener(
                    move |this, _, window, cx| this.show(screen, window, cx),
                )))
            })
    }
}
