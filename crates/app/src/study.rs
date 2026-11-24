//! Oefenen: one panel per card. Read the Dutch, test it by speaking or writing,
//! hear the sentence, look up the meaning, and score how easily it came back.

use dutch_flashcards_core::schedule::{Scheduler, describe_due};
use dutch_flashcards_core::speech::{Assessment, missed_ranges};
use dutch_flashcards_core::store::{Counts, Mode, Review};
use dutch_flashcards_core::voice::Speed;
use dutch_flashcards_core::writing::{Writing, Written};
use dutch_flashcards_core::{Card, CardId, Grade};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{ActiveTheme as _, Colorize as _, Disableable as _, Icon, Selectable as _, Sizable as _, h_flex, v_flex};
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
use crate::app::go_buttons;
use crate::ui::{
    card_box, empty_state, explained, hint, icon_button, keycap, pill, segment_button, segmented_bar, stat, surface,
};

pub struct StudyState {
    pub focus: FocusHandle,
    pub mode: Mode,
    pub card: Option<Card>,
    pub counts: Counts,
    pub reviewed_today: usize,
    pub attempt: Attempt,
    pub scheduler: Scheduler,
    /// Counts writings, so each one's letters animate afresh.
    pub write_round: u64,
    /// Cards skipped this session; they come back when nothing else is waiting.
    pub skipped: Vec<CardId>,
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
    // Writing the card.
    pub writing: Option<Writing>,
    /// The first finished writing; writing it again doesn't change it.
    pub written: Option<Written>,
}

impl Attempt {
    /// The speak button's icon, label and name on hover.
    fn speak_action(&self) -> (Icon, &'static str, &'static str) {
        if self.capture.is_on() {
            (icons::stop(), "Stop", "Stop de opname (Spatie)")
        } else if self.judging {
            (icons::speak(), "Luisteren…", "De app luistert naar je opname")
        } else if self.speech.is_some() || self.speech_note.is_some() {
            (icons::speak(), "Opnieuw", "Spreek de zin opnieuw (Spatie)")
        } else {
            (icons::speak(), "Spreek", "Spreek de zin uit (Spatie)")
        }
    }

    /// The write button's icon, label and name on hover.
    fn write_action(&self) -> (Icon, &'static str, &'static str) {
        match &self.writing {
            Some(writing) if !writing.is_done() => (icons::check(), "Klaar", "Toon de verbetering (Enter) · Esc stopt"),
            Some(_) => (icons::write(), "Opnieuw", "Schrijf de zin opnieuw (W)"),
            None => (icons::write(), "Schrijf", "Schrijf de zin (W)"),
        }
    }
}

impl StudyState {
    pub fn new(mode: Mode, cx: &mut App) -> Self {
        StudyState { focus: cx.focus_handle(), mode, card: None, counts: Counts::default(), reviewed_today: 0, attempt: Attempt::default(), scheduler: Scheduler::default(), write_round: 0, skipped: Vec::new() }
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

/// What Oefenen shows when no card is waiting, and where its buttons lead.
struct Rest {
    icon: Icon,
    title: &'static str,
    detail: &'static str,
    /// What was done and what comes next: an icon, a value and what it counts.
    stats: Vec<(Icon, String, &'static str)>,
    actions: Vec<Screen>,
}

impl Rest {
    fn new(deck_size: usize, drafts: usize, reviewed: usize, next_due: Option<String>) -> Rest {
        let to_drafts = (drafts > 0).then_some(Screen::Drafts).into_iter().collect();
        match (deck_size, drafts) {
            (0, 0) => Rest {
                icon: icons::deck(),
                title: "Je stapel is nog leeg",
                detail: "Begin een gesprek of plak er een.",
                stats: Vec::new(),
                actions: vec![Screen::Talk, Screen::Add],
            },
            (0, _) => Rest { icon: icons::drafts(), title: "Controleer eerst je concepten", detail: "", stats: Vec::new(), actions: to_drafts },
            _ => {
                let stats = [
                    (reviewed > 0).then(|| (icons::check(), reviewed.to_string(), "Vandaag geoefend")),
                    next_due.map(|when| (icons::due(), when, "Volgende kaart")),
                    (drafts > 0).then(|| (icons::drafts(), drafts.to_string(), "Concepten om te controleren")),
                ];
                Rest { icon: icons::done(), title: "Klaar voor nu", detail: "", stats: stats.into_iter().flatten().collect(), actions: to_drafts }
            }
        }
    }
}

impl FlashcardsApp {
    pub fn load_next_card(&mut self, cx: &mut Context<Self>) {
        let (now, per_day) = (Self::now(), self.settings.new_per_day);
        self.study.counts = self.store.counts(now, per_day).unwrap_or_default();
        let mut next = self.store.next_card(now, per_day, &self.study.skipped);
        // Skipped cards come back once nothing else is waiting.
        if matches!(next, Ok(None)) && !self.study.skipped.is_empty() {
            self.study.skipped.clear();
            next = self.store.next_card(now, per_day, &[]);
        }
        match next {
            Ok(card) => self.start_card(card, cx),
            Err(err) => self.error(format!("{err:#}")),
        }
    }

    /// Moves on without saving anything about this card; it comes back later.
    fn skip(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.study.card.as_ref().map(|card| card.id).filter(|_| !self.busy_listening()) else { return };
        self.study.skipped.push(id);
        self.load_next_card(cx);
        if self.study.card.as_ref().is_some_and(|card| card.id == id) {
            self.info("Er is geen andere kaart om te oefenen.", cx);
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
        let Some(card) = self.study.card.as_ref().filter(|_| self.study.scoring_open() && !self.busy_listening() && !self.writing_now()) else { return };
        let attempt = &self.study.attempt;
        let review = Review { own, speech: attempt.speech.as_ref(), written: attempt.written, mode: self.study.mode, peeked: attempt.peeked };
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
        // While writing, the typed character counts: a capital, or an accent.
        let typed = event.keystroke.key_char.clone().filter(|c| self.writing_now() && c.chars().count() == 1);
        if plain && self.study_key(typed.as_deref().unwrap_or(&event.keystroke.key), window, cx) {
            cx.stop_propagation();
        }
    }

    /// Handles one study shortcut. Returns false for keys it doesn't use.
    pub fn study_key(&mut self, key: &str, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.writing_now() {
            self.write_key(key, cx);
            return true;
        }
        match key {
            "w" => self.start_writing(cx),
            "s" => self.skip(cx),
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
        let modes = [(Mode::Read, "Lezen", icons::read()), (Mode::Listen, "Luisteren", icons::listen())];
        let Counts { due, new } = self.study.counts;
        let has_card = self.study.card.is_some();
        let done = self.study.reviewed_today;
        let summary = h_flex()
            .items_center()
            .gap_3()
            .when(has_card || due + new > 0, |d| {
                d.child(stat("count-due", icons::due(), due.to_string(), "Te herhalen", cx))
                    .child(stat("count-new", icons::new(), new.to_string(), "Nieuw", cx))
            })
            .when(done > 0, |d| d.child(stat("count-done", icons::check(), done.to_string(), "Vandaag geoefend", cx)));
        h_flex()
            .w_full()
            .max_w(px(760.))
            .mx_auto()
            .justify_between()
            .items_center()
            .flex_wrap()
            .gap_3()
            .child(segmented_bar(cx).children(modes.map(|(mode, label, icon)| {
                segment_button(SharedString::from(format!("mode-{label}")), label, self.study.mode == mode, cx)
                    .icon(icon)
                    .tab_stop(false)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.set_mode(mode, cx);
                        this.study.focus.focus(window, cx);
                    }))
            })))
            .child(h_flex().items_center().gap_3().child(summary).when(has_card, |d| {
                d.child(
                    icon_button("edit", icons::edit(), "Bewerken (B)")
                        .on_click(cx.listener(|this, _, window, cx| this.edit_current(window, cx))),
                )
            }))
    }

    fn render_card(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let reading = if self.study.mode == Mode::Read { self.render_reading_status(cx) } else { None };
        let written = self.render_written(cx);
        let meaning = self.render_meaning(cx);

        // Fixed-minimum feedback area keeps the Dutch phrase and controls anchored
        // when opening the meaning or starting a recording.
        let feedback = v_flex()
            .w_full()
            .min_h(px(64.))
            .items_center()
            .justify_center()
            .gap_2p5()
            .children(reading)
            .children(written)
            .children(meaning);

        let phrase = match &self.study.attempt.writing {
            Some(writing) => self.render_writing(writing, cx).into_any_element(),
            None => self.render_phrase(cx).into_any_element(),
        };
        let stage = card_box(cx)
            .w_full()
            .max_w(px(680.))
            .min_h(px(250.))
            .px_8()
            .pt_4()
            .pb_6()
            .gap_4()
            .items_center()
            .justify_between()
            .child(self.render_card_meta(cx))
            .child(v_flex().flex_1().w_full().items_center().justify_center().child(phrase))
            .child(feedback);

        v_flex()
            .flex_1()
            .min_h_0()
            .w_full()
            .items_center()
            .justify_center()
            .gap_5()
            .child(stage)
            .child(self.render_actions(cx))
            .when(self.study.scoring_open(), |d| d.child(self.render_scores(cx)))
    }

    /// The topic, the new-card mark, and the helpers that never decide the
    /// review: hear it, slowly, and the meaning.
    fn render_card_meta(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let Some(card) = self.study.card.as_ref() else {
            return h_flex().h(px(26.));
        };
        let has_tag = !card.tags.trim().is_empty();
        let audio_active = self.sound.playing || self.sound.preparing;
        let hear_tip = if self.sound.preparing { "Even wachten…" } else { "Hoor de zin (H)" };
        let hear = icon_button("hear", icons::listen(), hear_tip)
            .selected(audio_active)
            .disabled(self.busy_listening())
            .on_click(self.study_click(|this, cx| this.hear(Speed::Normal, cx), cx));
        let slow = icon_button("hear-slow", icons::slow(), "Langzaam (L)")
            .disabled(self.busy_listening())
            .on_click(self.study_click(|this, cx| this.hear(Speed::Slow, cx), cx));
        let meaning = icon_button("meaning", icons::meaning(), "Betekenis (V)")
            .selected(self.study.attempt.english_shown)
            .on_click(self.study_click(Self::toggle_english, cx));
        h_flex()
            .w_full()
            .min_h(px(26.))
            .justify_between()
            .items_center()
            .gap_2()
            .child(
                h_flex()
                    .items_center()
                    .gap_2()
                    .when(has_tag, |d| d.child(pill(card.tags.clone(), theme.secondary, theme.muted_foreground)))
                    .when(card.is_new(), |d| d.child(explained("card-new", "Nieuwe kaart").text_color(theme.primary).child(icons::new().size_4()))),
            )
            .child(h_flex().items_center().gap_0p5().text_color(theme.muted_foreground).child(hear).child(slow).child(meaning))
    }

    /// The Dutch, with the words the app missed underlined; dots while veiled.
    fn render_phrase(&self, cx: &App) -> gpui_kit::Div {
        let theme = cx.theme();
        let Some(card) = self.study.card.as_ref() else { return div() };
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
        div()
            .max_w(px(620.))
            .text_center()
            .text_size(px(32.))
            .line_height(relative(1.3))
            .font_weight(FontWeight::MEDIUM)
            .when(veiled, |d| d.text_color(theme.muted_foreground))
            .child(StyledText::new(text).with_highlights(highlights))
    }

    /// The English and the note, once the learner opens them.
    fn render_meaning(&self, cx: &App) -> Option<gpui_kit::Div> {
        let card = self.study.card.as_ref().filter(|_| self.study.attempt.english_shown)?;
        let theme = cx.theme();
        let english = if card.english.is_empty() { "(geen vertaling)".to_string() } else { card.english.clone() };
        Some(
            v_flex()
                .w_full()
                .items_center()
                .gap_1()
                .max_w(px(540.))
                .pt_2p5()
                .border_t_1()
                .border_color(theme.border)
                .text_center()
                .child(div().italic().text_color(theme.foreground).child(english))
                .when(!card.note.is_empty(), |d| d.child(hint(card.note.clone(), cx))),
        )
    }

    /// The two ways to test the card: say it (in Luisteren: show it) and write it.
    fn render_actions(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let attempt = &self.study.attempt;
        let writing = self.writing_now();
        let action = |id: &'static str, icon: Icon, label: &'static str, tip: &'static str| {
            Button::new(id).primary().large().min_w(px(150.)).tab_stop(false).icon(icon).label(label).tooltip(tip)
        };
        let first = match self.study.mode {
            Mode::Read => {
                let recording = attempt.capture.is_on();
                let (icon, label, tip) = attempt.speak_action();
                let button = action("test-reading", icon, label, tip)
                    .disabled(!self.listener.is_installed() || attempt.judging || writing)
                    .on_click(self.study_click(Self::advance, cx));
                Some(if recording { button.danger() } else { button })
            }
            Mode::Listen if !attempt.revealed => Some(
                action("reveal", icons::reveal(), "Toon tekst", "Toon de tekst (Spatie)").disabled(writing).on_click(self.study_click(Self::advance, cx)),
            ),
            Mode::Listen => None,
        };
        let (icon, label, tip) = attempt.write_action();
        let write = action("write", icon, label, tip)
            .disabled(self.busy_listening())
            .on_click(self.study_click(|this, cx| if this.writing_now() { this.finish_writing(cx) } else { this.start_writing(cx) }, cx));
        let skip = Button::new("skip")
            .outline()
            .large()
            .tab_stop(false)
            .icon(icons::skip())
            .label("Overslaan")
            .tooltip("Later in deze sessie; telt niet mee (S)")
            .disabled(self.busy_listening())
            .on_click(self.study_click(Self::skip, cx));
        h_flex().gap_3().justify_center().children(first).child(write).child(skip)
    }

    /// The four scores: how easily the sentence came back. Nothing else.
    fn render_scores(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let card_bg = surface(cx);
        let open = !self.busy_listening() && !self.writing_now();
        h_flex().gap_2p5().w_full().max_w(px(640.)).children(Grade::ALL.map(|grade| {
            let tone = grade_color(grade, cx);
            v_flex()
                .id(SharedString::from(format!("grade-{}", grade.number())))
                .flex_1()
                .items_center()
                .gap_1p5()
                .pb_2p5()
                .overflow_hidden()
                .rounded(px(10.))
                .border_1()
                .border_color(theme.border)
                .bg(card_bg)
                .child(div().w_full().h(px(4.)).mb_1().bg(tone))
                .child(div().font_weight(FontWeight::MEDIUM).child(grade.label()))
                .child(keycap(&grade.number().to_string(), cx))
                .when(!open, |d| d.opacity(0.5))
                .when(open, |d| {
                    d.cursor_pointer()
                        .hover(move |s| s.border_color(tone).bg(tone.opacity(0.08)))
                        .on_click(cx.listener(move |this, _, window, cx| {
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
        let stats: Vec<_> = rest.stats.into_iter().enumerate().map(|(i, (icon, value, tip))| stat(("rest-stat", i as u64), icon, value, tip, cx)).collect();
        v_flex()
            .flex_1()
            .items_center()
            .justify_center()
            .child(
                card_box(cx)
                    .w_full()
                    .max_w(px(520.))
                    .px_6()
                    .pb_6()
                    .gap_4()
                    .items_center()
                    .child(empty_state(rest.icon, rest.title, rest.detail, cx))
                    .when(!stats.is_empty(), |d| d.child(h_flex().flex_wrap().justify_center().gap_5().children(stats)))
                    .child(h_flex().gap_2().children(go_buttons(rest.actions, cx))),
            )
    }
}
