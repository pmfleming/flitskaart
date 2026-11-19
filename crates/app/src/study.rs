//! Oefenen: show, read, score, hear.

use dutch_flashcards_core::schedule::describe;
use dutch_flashcards_core::speech::{Assessment, combine};
use dutch_flashcards_core::store::{Counts, Mode};
use dutch_flashcards_core::voice::Speed;
use dutch_flashcards_core::{Card, Grade, Status};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{ActiveTheme as _, h_flex, v_flex, Disableable as _, Selectable as _, Sizable as _};
use gpui_kit::*;
use gpui_kit::prelude::FluentBuilder as _;

use crate::app::{FlashcardsApp, Screen};
use crate::recorder::Recording;
use crate::ui::{empty_state, hint, keycap};

/// Where the learner is within one card.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// The Dutch is shown; the learner reads it (aloud or silently).
    Read,
    /// Listening mode: the audio plays with the text hidden.
    Listen,
    /// The learner scores themselves.
    Score,
    /// Speaking mode: after scoring, the learner asks to hear the card.
    Hear,
}

pub struct StudyState {
    pub focus: FocusHandle,
    pub mode: Mode,
    pub card: Option<Card>,
    pub step: Step,
    pub grade: Option<Grade>,
    pub heard: bool,
    pub english_shown: bool,
    pub peeked: bool,
    /// How far away the card would be scheduled for each grade.
    pub previews: [chrono::Duration; 4],
    pub counts: Counts,
    pub reviewed_today: usize,
    // Reading aloud (Spreken mode).
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
            step: Step::Read,
            grade: None,
            heard: false,
            english_shown: false,
            peeked: false,
            previews: Default::default(),
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

    fn first_step(&self) -> Step {
        if self.mode == Mode::Listen { Step::Listen } else { Step::Read }
    }

    /// Steps shown in the strip above the card, for the current mode.
    fn steps(&self) -> &'static [(Step, &'static str)] {
        match self.mode {
            Mode::Speak => &[(Step::Read, "Lezen"), (Step::Score, "Beoordelen"), (Step::Hear, "Luisteren")],
            Mode::Silent => &[(Step::Read, "Lezen"), (Step::Score, "Beoordelen")],
            Mode::Listen => &[(Step::Listen, "Luisteren"), (Step::Score, "Beoordelen")],
        }
    }

    /// Sound is only offered after scoring in speaking mode, never in silent mode.
    pub fn audio_allowed(&self) -> bool {
        self.card.is_some()
            && match self.mode {
                Mode::Speak => self.step == Step::Hear,
                Mode::Silent => false,
                Mode::Listen => true,
            }
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
        study.step = study.first_step();
        study.grade = None;
        study.heard = false;
        study.english_shown = false;
        study.peeked = false;
        study.recording = None;
        study.level = 0.0;
        study.judging = false;
        study.speech = None;
        study.speech_note = None;
        study.has_recording = false;
        study.previews = match &card {
            Some(card) => self.scheduler.preview(&card.schedule, Self::now()),
            None => Default::default(),
        };
        study.card = card;
        if let Some(card) = &self.study.card {
            let text = card.dutch.clone();
            match self.study.mode {
                Mode::Listen => self.speak(text, self.current_voice(), Speed::Normal, cx),
                Mode::Speak => self.prepare_audio(text, cx),
                Mode::Silent => {}
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

    /// Space: the main action of the current step.
    fn advance(&mut self, cx: &mut Context<Self>) {
        if self.study.card.is_none() {
            return;
        }
        if self.study.recording.is_some() {
            self.stop_recording();
            return;
        }
        if self.study.judging {
            return;
        }
        match self.study.step {
            Step::Read if self.can_record() => self.start_recording(cx),
            Step::Read | Step::Listen => self.study.step = Step::Score,
            Step::Score => {}
            Step::Hear if !self.study.heard => self.hear(Speed::Normal, cx),
            Step::Hear => self.commit(cx),
        }
        cx.notify();
    }

    fn grade(&mut self, grade: Grade, cx: &mut Context<Self>) {
        if self.study.card.is_none() || self.busy_listening() || !matches!(self.study.step, Step::Score | Step::Hear) {
            return;
        }
        self.study.grade = Some(grade);
        if self.study.mode == Mode::Speak {
            self.study.step = Step::Hear;
            cx.notify();
        } else {
            self.commit(cx);
        }
    }

    fn hear(&mut self, speed: Speed, cx: &mut Context<Self>) {
        if !self.study.audio_allowed() {
            return;
        }
        let Some(card) = &self.study.card else { return };
        let text = card.dutch.clone();
        self.study.heard = true;
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

    /// Saves the score and moves to the next card.
    pub(crate) fn commit(&mut self, cx: &mut Context<Self>) {
        if self.busy_listening() {
            return;
        }
        let (Some(card), Some(grade)) = (self.study.card.as_ref(), self.study.grade) else { return };
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
            "enter" if self.study.step == Step::Hear => self.commit(cx),
            "enter" if self.study.step == Step::Read && self.study.recording.is_none() && !self.study.judging => {
                self.study.step = Step::Score;
                cx.notify();
            }
            "escape" if self.study.recording.is_some() => self.cancel_recording(cx),
            "r" => self.record_again(cx),
            "o" => self.play_own_recording(cx),
            "1" | "2" | "3" | "4" => {
                if let Some(grade) = key.parse().ok().and_then(Grade::from_number) {
                    self.grade(grade, cx);
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

    pub fn render_study(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let focus = self.study.focus.clone();
        let content = if self.study.card.is_some() {
            self.render_card(window, cx).into_any_element()
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
        let modes = [(Mode::Speak, "Spreken"), (Mode::Silent, "Stil"), (Mode::Listen, "Luisteren")];
        let counts = self.study.counts;
        let summary = if self.study.card.is_some() || counts.due + counts.new > 0 {
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
            .child(hint(summary, cx))
    }

    fn render_card(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let study = &self.study;
        let card = study.card.as_ref().unwrap();
        let theme = cx.theme().clone();
        let veiled = study.step == Step::Listen;
        let show_note = !card.note.is_empty() && matches!(study.step, Step::Score | Step::Hear);

        let steps = h_flex().gap_5().justify_center().children(study.steps().iter().enumerate().map(|(i, (step, label))| {
            let current = *step == study.step;
            div()
                .pb_0p5()
                .border_b_2()
                .border_color(if current { theme.primary } else { transparent_black() })
                .text_sm()
                .text_color(if current { theme.foreground } else { theme.muted_foreground })
                .when(current, |d| d.font_weight(FontWeight::MEDIUM))
                .child(format!("{}  {label}", i + 1))
        }));

        let phrase = div()
            .max_w(px(720.))
            .text_center()
            .text_size(px(34.))
            .line_height(relative(1.25))
            .font_weight(FontWeight::MEDIUM)
            .when(veiled, |d| d.text_color(theme.muted_foreground))
            .child(if veiled { "• • •".to_string() } else { card.dutch.clone() });

        let sound = if self.preparing_audio {
            "stem wordt gemaakt…"
        } else if self.playing {
            "speelt af"
        } else {
            ""
        };

        let recording_controls = (study.step == Step::Read && self.can_record()).then(|| self.render_recording_controls(cx));
        let (prompt, primary) = match study.step {
            Step::Read if recording_controls.is_some() => ("", None),
            Step::Read if study.mode == Mode::Speak => ("Lees de kaart hardop.", Some("Gelezen")),
            Step::Read => ("Lees de kaart in stilte.", Some("Gelezen")),
            Step::Listen => ("Luister. Wat hoor je?", Some("Toon tekst")),
            Step::Score => ("Hoe ging het?", None),
            Step::Hear if !study.heard => ("Hoor hoe het klinkt.", Some("Luister")),
            Step::Hear => ("", Some("Volgende")),
        };

        let actions = h_flex()
            .when(recording_controls.is_some(), |d| d.hidden())
            .gap_3()
            .justify_center()
            .when(!prompt.is_empty(), |d| d.child(hint(prompt, cx)))
            .when_some(primary, |d, label| {
                d.child(
                    Button::new("primary")
                        .primary()
                        .tab_stop(false)
                        .child(h_flex().gap_2().child(label).child(keycap("Spatie", cx)))
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.advance(cx);
                            this.study.focus.focus(window, cx);
                        })),
                )
            })
            .when(study.step == Step::Hear && !study.heard, |d| {
                d.child(
                    Button::new("skip")
                        .outline()
                        .tab_stop(false)
                        .child(h_flex().gap_2().child("Volgende").child(keycap("Enter", cx)))
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.commit(cx);
                            this.study.focus.focus(window, cx);
                        })),
                )
            });

        let speech_grade = study.speech.as_ref().map(|s| s.grade);
        let speech_panel = study.speech.is_some().then(|| self.render_speech_panel(cx));
        let grades = matches!(study.step, Step::Score | Step::Hear).then(|| {
            let colors = [theme.danger, theme.warning, theme.blue, theme.success];
            h_flex().gap_2().w_full().max_w(px(600.)).children(Grade::ALL.iter().enumerate().map(|(i, grade)| {
                let grade = *grade;
                let chosen = study.grade == Some(grade);
                v_flex()
                    .id(SharedString::from(format!("grade-{}", grade.number())))
                    .flex_1()
                    .items_center()
                    .gap_0p5()
                    .pb_2()
                    .overflow_hidden()
                    .rounded(px(6.))
                    .border_1()
                    .border_color(if chosen { colors[i] } else { theme.border })
                    .bg(if chosen { theme.accent } else { theme.secondary })
                    .hover(|s| s.bg(theme.secondary_hover))
                    .cursor_pointer()
                    .child(div().w_full().h(px(4.)).mb_1p5().bg(colors[i]))
                    .child(div().font_weight(FontWeight::MEDIUM).child(grade.label()))
                    .child(
                        h_flex()
                            .gap_1()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(keycap(&grade.number().to_string(), cx))
                            .child(describe(study.previews[combine(grade, speech_grade).number() as usize - 1])),
                    )
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.grade(grade, cx);
                        this.study.focus.focus(window, cx);
                    }))
            }))
        });

        let english = study.english_shown.then(|| {
            div()
                .max_w(px(560.))
                .pt_2()
                .border_t_1()
                .border_color(theme.border)
                .text_color(theme.muted_foreground)
                .italic()
                .text_center()
                .child(if card.english.is_empty() { "(geen vertaling)".to_string() } else { card.english.clone() })
        });

        let audio_ok = study.audio_allowed();
        let tool = |id: &'static str, label: &'static str, key: &'static str, enabled: bool, cx: &mut Context<Self>| {
            Button::new(id)
                .ghost()
                .small()
                .tab_stop(false)
                .disabled(!enabled)
                .child(h_flex().gap_1p5().child(label).child(keycap(key, cx)))
        };
        let toolbar = h_flex()
            .gap_2()
            .justify_center()
            .pt_3()
            .border_t_1()
            .border_color(theme.border)
            .child(tool("translation", "Vertaling", "V", true, cx).on_click(cx.listener(|this, _, window, cx| {
                this.toggle_english(cx);
                this.study.focus.focus(window, cx);
            })))
            .child(tool("again", "Herhaal", "H", audio_ok, cx).on_click(cx.listener(|this, _, window, cx| {
                this.hear(Speed::Normal, cx);
                this.study.focus.focus(window, cx);
            })))
            .child(tool("slow", "Langzaam", "L", audio_ok, cx).on_click(cx.listener(|this, _, window, cx| {
                this.hear(Speed::Slow, cx);
                this.study.focus.focus(window, cx);
            })))
            .child(tool("edit", "Bewerken", "B", true, cx).on_click(cx.listener(|this, _, window, cx| {
                this.edit_current(window, cx);
            })));

        v_flex()
            .flex_1()
            .min_h_0()
            .child(
                v_flex()
                    .flex_1()
                    .items_center()
                    .justify_center()
                    .gap_4()
                    .child(steps)
                    .child(phrase)
                    .child(div().h_5().text_xs().text_color(theme.primary).child(sound))
                    .when(show_note, |d| {
                        d.child(div().max_w(px(560.)).text_center().text_color(theme.muted_foreground).child(card.note.clone()))
                    })
                    .children(english)
                    .children(speech_panel)
                    .children(recording_controls)
                    .child(actions)
                    .children(grades),
            )
            .child(toolbar)
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
