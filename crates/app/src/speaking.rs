//! Testing the reading: record the learner reading the card aloud, let Whisper
//! listen, and show how understandable they were.

use std::time::Duration;

use dutch_flashcards_core::speech::{self, Assessment, speech_label, write_wav_16k};
use dutch_flashcards_core::store::Mode;
use dutch_flashcards_core::Grade;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{ActiveTheme as _, Disableable as _, Sizable as _, h_flex, v_flex};
use gpui_kit::*;

use crate::app::FlashcardsApp;
use crate::recorder::{self, Captured, Recording};
use crate::ui::{hint, keycap, level_meter};

impl FlashcardsApp {
    /// The reading can be tested in Lezen once the speech recognizer is installed.
    pub fn can_record(&self) -> bool {
        self.study.mode == Mode::Read && self.study.card.is_some() && self.listener.is_installed()
    }

    pub fn busy_listening(&self) -> bool {
        self.study.recording.is_some() || self.study.judging
    }

    fn last_recording(&self) -> std::path::PathBuf {
        self.paths.recordings().join("laatste.wav")
    }

    pub fn start_recording(&mut self, cx: &mut Context<Self>) {
        if !self.can_record() || self.busy_listening() {
            return;
        }
        self.stop_audio();
        self.study.speech_note = None;
        if let Some(path) = self.fake_recording.clone() {
            match recorder::from_file(&path, self.settings.input_boost) {
                Ok(captured) => self.judge(captured, cx),
                Err(err) => self.study.speech_note = Some(format!("{err:#}").into()),
            }
            cx.notify();
            return;
        }
        self.study.recording = Some(Recording::start(self.settings.input_boost));
        cx.notify();
        // Refresh the level meter until the recording ends by itself or is stopped.
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_millis(50)).await;
                let done = this.update(cx, |this, cx| {
                    let Some(recording) = &this.study.recording else { return true };
                    if recording.is_finished() {
                        let recording = this.study.recording.take().unwrap();
                        this.study.level = 0.0;
                        match recording.finish() {
                            Ok(captured) => this.judge(captured, cx),
                            Err(err) => this.study.speech_note = Some(format!("{err:#}").into()),
                        }
                        cx.notify();
                        return true;
                    }
                    this.study.level = recording.level();
                    cx.notify();
                    false
                });
                if done.unwrap_or(true) {
                    break;
                }
            }
        })
        .detach();
    }

    pub fn stop_recording(&mut self) {
        if let Some(recording) = &self.study.recording {
            recording.request_stop();
        }
    }

    pub fn cancel_recording(&mut self, cx: &mut Context<Self>) {
        self.study.recording = None;
        self.study.level = 0.0;
        cx.notify();
    }

    pub fn play_own_recording(&mut self, cx: &mut Context<Self>) {
        if !self.study.has_recording || self.busy_listening() {
            return;
        }
        match self.audio.play(&self.last_recording()) {
            Ok(_) => self.info("Je eigen opname speelt af."),
            Err(err) => self.error(format!("{err:#}")),
        }
        cx.notify();
    }

    /// Saves the recording and lets Whisper transcribe it in the background.
    /// The latest try replaces any earlier one.
    fn judge(&mut self, captured: Captured, cx: &mut Context<Self>) {
        if !captured.had_speech {
            self.study.speech_note = Some("Ik hoorde niets. Spreek wat luider of dichter bij de microfoon.".into());
            return;
        }
        let path = self.last_recording();
        if let Err(err) = write_wav_16k(&path, &captured.samples, captured.rate, captured.channels) {
            self.study.speech_note = Some(format!("{err:#}").into());
            return;
        }
        let Some(card) = &self.study.card else { return };
        let (card_id, text) = (card.id, card.dutch.clone());
        self.study.has_recording = true;
        self.study.judging = true;
        let listener = self.listener.clone();
        let job = cx.background_executor().spawn(async move {
            listener.transcribe(&path).map(|heard| speech::assess(&text, &heard))
        });
        cx.spawn(async move |this, cx| {
            let result = job.await;
            let _ = this.update(cx, |this, cx| {
                this.study.judging = false;
                // The learner may have moved on while Whisper was listening.
                if this.study.card.as_ref().map(|c| c.id) != Some(card_id) {
                    return;
                }
                match result {
                    Ok(assessment) => this.study.speech = Some(assessment),
                    Err(err) => this.study.speech_note = Some(format!("{err:#}").into()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Under the card in Lezen: the level meter while recording, "listening"
    /// while Whisper works, then the result. Nothing before the first try.
    pub fn render_reading_status(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let theme = cx.theme().clone();
        let study = &self.study;
        let body = if let Some(recording) = &study.recording {
            let started = recording.heard_speech();
            Some(
                level_meter(study.level, cx)
                    .child(hint(if started { "Opname… stopt vanzelf als je klaar bent." } else { "Lees de kaart hardop." }, cx))
                    .into_any_element(),
            )
        } else if study.judging {
            Some(hint("Even luisteren…", cx).into_any_element())
        } else if let Some(result) = &study.speech {
            Some(self.render_speech_result(result, cx))
        } else if !self.listener.is_installed() {
            Some(hint("Spraakherkenning ontbreekt. Installeer die met scripts/fetch-models.sh.", cx).into_any_element())
        } else {
            None
        };
        let note = study.speech_note.clone().map(|text| hint(text, cx).text_color(theme.warning));
        if body.is_none() && note.is_none() {
            return None;
        }
        Some(v_flex().items_center().gap_1p5().max_w(px(600.)).children(body).children(note).into_any_element())
    }

    /// How understandable the learner was. The missed words are marked in the card text.
    fn render_speech_result(&self, result: &Assessment, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        v_flex()
            .items_center()
            .gap_1()
            .child(
                h_flex()
                    .flex_wrap()
                    .justify_center()
                    .items_center()
                    .gap_2()
                    .child(hint("Uitspraak", cx))
                    .child(
                        div()
                            .px_2()
                            .rounded(px(4.))
                            .bg(grade_color(result.grade, cx))
                            .text_color(theme.background)
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(result.grade.number().to_string()),
                    )
                    .child(div().font_weight(FontWeight::MEDIUM).child(speech_label(result.grade)))
                    .child(hint(format!("{:.0}% verstaan", result.accuracy * 100.0), cx))
                    .child(
                        Button::new("own-recording")
                            .ghost()
                            .small()
                            .tab_stop(false)
                            .disabled(!self.study.has_recording)
                            .child(h_flex().gap_1p5().child("Mijn opname").child(keycap("O", cx)))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.play_own_recording(cx);
                                this.study.focus.focus(window, cx);
                            })),
                    ),
            )
            .child(hint(
                if result.heard.is_empty() { "De app hoorde niets.".to_string() } else { format!("De app hoorde: {}", result.heard) },
                cx,
            ))
            .into_any_element()
    }
}

pub fn grade_color(grade: Grade, cx: &App) -> Hsla {
    let theme = cx.theme();
    match grade {
        Grade::DontKnow => theme.danger,
        Grade::Hard => theme.warning,
        Grade::Comfortable => theme.blue,
        Grade::Easy => theme.success,
    }
}
