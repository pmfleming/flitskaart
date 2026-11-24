//! Testing the reading: record the learner reading the card aloud, let Whisper
//! listen, and show how understandable they were.

use std::path::PathBuf;

use anyhow::Result;
use dutch_flashcards_core::Grade;
use dutch_flashcards_core::speech::{Assessment, assess, speech_label, write_wav_16k};
use dutch_flashcards_core::store::Mode;
use gpui_kit::component::{ActiveTheme as _, Disableable as _, h_flex, v_flex};
use gpui_kit::{AnyElement, App, Context, Div, FontWeight, Hsla, IntoElement, ParentElement as _, Styled as _, div, px};

use crate::app::FlashcardsApp;
use crate::icons;
use crate::recorder::Captured;
use crate::study::Attempt;
use crate::ui::{explained, hint, icon_button};

const NOT_HEARD: &str = "Ik hoorde niets. Spreek wat luider of dichter bij de microfoon.";

impl Attempt {
    /// Shows why the reading could not be judged, and logs it.
    fn fail(&mut self, err: impl std::fmt::Display) {
        self.speech_note = Some(crate::logging::error("opname", format!("{err:#}")).into());
    }
}

impl FlashcardsApp {
    /// The reading can be tested in Lezen once the speech recognizer is installed.
    pub fn can_record(&self) -> bool {
        self.study.mode == Mode::Read && self.study.card.is_some() && self.listener.is_installed()
    }

    pub fn busy_listening(&self) -> bool {
        self.study.attempt.capture.is_on() || self.study.attempt.judging
    }

    fn last_recording(&self) -> PathBuf {
        self.paths.recordings().join("laatste.wav")
    }

    pub fn start_recording(&mut self, cx: &mut Context<Self>) {
        if self.can_record() && !self.busy_listening() {
            self.study.attempt.speech_note = None;
            self.start_capture(|this| &mut this.study.attempt.capture, Self::judge, cx);
        }
    }

    pub fn cancel_recording(&mut self, cx: &mut Context<Self>) {
        self.study.attempt.capture.cancel();
        cx.notify();
    }

    pub fn play_own_recording(&mut self, cx: &mut Context<Self>) {
        if self.study.attempt.has_recording && !self.busy_listening() {
            self.play_file(&self.last_recording(), "Je eigen opname speelt af.", cx);
        }
    }

    /// Saves the recording and lets Whisper judge it in the background.
    /// The latest try replaces any earlier one.
    fn judge(&mut self, captured: Result<Captured>, cx: &mut Context<Self>) {
        let path = self.last_recording();
        let Some(card) = &self.study.card else { return };
        let (card_id, text) = (card.id, card.dutch.clone());
        let attempt = &mut self.study.attempt;
        let saved = captured.and_then(|captured| {
            captured.had_speech.then(|| write_wav_16k(&path, &captured.samples, captured.rate, captured.channels)).transpose()
        });
        match saved {
            Ok(Some(())) => (attempt.has_recording, attempt.judging) = (true, true),
            Ok(None) => return attempt.speech_note = Some(NOT_HEARD.into()),
            Err(err) => return attempt.fail(err),
        }
        let listener = self.listener.clone();
        let job = move || listener.transcribe(&path).map(|heard| assess(&text, &heard));
        self.in_background(cx, job, move |this, result, _| {
            // The learner may have moved on while Whisper was listening.
            if this.study.card.as_ref().is_some_and(|card| card.id == card_id) {
                let attempt = &mut this.study.attempt;
                attempt.judging = false;
                match result {
                    Ok(assessment) => attempt.speech = Some(assessment),
                    Err(err) => attempt.fail(err),
                }
            }
        });
    }

    /// Under the card in Lezen: the level meter while recording, "listening"
    /// while Whisper works, then the result. Nothing before the first try.
    pub fn render_reading_status(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let attempt = &self.study.attempt;
        let body = if let Some(meter) = attempt.capture.render("Lees hardop.", cx) {
            let cancel = icon_button("cancel-recording", icons::close(), "Annuleren (Esc)").on_click(cx.listener(|this, _, window, cx| {
                this.cancel_recording(cx);
                this.study.focus.focus(window, cx);
            }));
            Some(meter.child(cancel).into_any_element())
        } else if attempt.judging {
            Some(hint("Even luisteren…", cx).into_any_element())
        } else if let Some(result) = &attempt.speech {
            Some(self.render_speech_result(result, cx))
        } else {
            (!self.listener.is_installed())
                .then(|| hint("Spraakherkenning ontbreekt: scripts/fetch-models.sh", cx).into_any_element())
        };
        let note = attempt.speech_note.clone().map(|text| hint(text, cx).text_color(cx.theme().warning));
        (body.is_some() || note.is_some())
            .then(|| v_flex().items_center().gap_1p5().max_w(px(600.)).children(body).children(note).into_any_element())
    }

    /// How understandable the learner was. The missed words are marked in the card text.
    fn render_speech_result(&self, result: &Assessment, cx: &mut Context<Self>) -> AnyElement {
        let chip = grade_chip(result.grade, cx);
        let own_recording = icon_button("own-recording", icons::playback(), "Mijn opname (O)")
            .disabled(!self.study.attempt.has_recording)
            .on_click(cx.listener(|this, _, window, cx| {
                this.play_own_recording(cx);
                this.study.focus.focus(window, cx);
            }));
        let heard = if result.heard.is_empty() { "…".to_string() } else { format!("“{}”", result.heard) };
        let muted = cx.theme().muted_foreground;
        v_flex()
            .items_center()
            .gap_1()
            .child(
                h_flex()
                    .flex_wrap()
                    .justify_center()
                    .items_center()
                    .gap_2()
                    .child(explained("speech-grade", "Uitspraak").child(chip))
                    .child(div().font_weight(FontWeight::MEDIUM).child(speech_label(result.grade)))
                    .child(explained("speech-accuracy", "Zoveel van de woorden verstond de app").child(hint(format!("{:.0}%", result.accuracy * 100.0), cx)))
                    .child(own_recording),
            )
            .child(
                explained("speech-heard", "De app hoorde")
                    .flex()
                    .items_start()
                    .gap_1p5()
                    .text_sm()
                    .text_color(muted)
                    .child(icons::transcript().size_4().mt(px(1.)).flex_none())
                    .child(div().italic().text_center().child(heard)),
            )
            .into_any_element()
    }
}

pub fn grade_color(grade: Grade, cx: &App) -> Hsla {
    let theme = cx.theme();
    [theme.danger, theme.warning, theme.blue, theme.success][grade.index()]
}

/// A test's grade, 1–4, in its colour.
pub fn grade_chip(grade: Grade, cx: &App) -> Div {
    div()
        .px_2()
        .py(px(1.))
        .rounded(px(5.))
        .bg(grade_color(grade, cx))
        .text_color(cx.theme().primary_foreground)
        .text_sm()
        .font_weight(FontWeight::SEMIBOLD)
        .child(grade.number().to_string())
}
