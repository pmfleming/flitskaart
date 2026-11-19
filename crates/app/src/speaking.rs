//! Reading a card aloud: record, let Whisper listen, show how understandable it was.

use std::time::Duration;

use dutch_flashcards_core::speech::{self, speech_label, write_wav_16k};
use dutch_flashcards_core::store::Mode;
use dutch_flashcards_core::Grade;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{ActiveTheme as _, Disableable as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::app::FlashcardsApp;
use crate::recorder::{self, Captured, Recording};
use crate::study::Step;
use crate::ui::{hint, keycap};

impl FlashcardsApp {
    /// Reading aloud is judged in Spreken mode once the speech recognizer is installed.
    pub fn can_record(&self) -> bool {
        self.study.mode == Mode::Speak && self.study.card.is_some() && self.listener.is_installed()
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
            match recorder::from_file(&path) {
                Ok(captured) => self.judge(captured, cx),
                Err(err) => self.study.speech_note = Some(format!("{err:#}").into()),
            }
            cx.notify();
            return;
        }
        self.study.recording = Some(Recording::start());
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

    /// Records again after scoring, keeping the learner's own grade.
    pub fn record_again(&mut self, cx: &mut Context<Self>) {
        if matches!(self.study.step, Step::Score | Step::Hear) && self.can_record() {
            self.start_recording(cx);
        }
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
                    Ok(assessment) => {
                        this.study.speech = Some(assessment);
                        if this.study.step == Step::Read {
                            this.study.step = Step::Score;
                        }
                    }
                    Err(err) => this.study.speech_note = Some(format!("{err:#}").into()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Shown on the read step in Spreken mode: record, the level meter, or "listening".
    pub fn render_recording_controls(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let study = &self.study;
        let note = study.speech_note.clone().map(|text| hint(text, cx).text_color(theme.warning));
        let body = if study.judging {
            h_flex().gap_2().child(hint("Even luisteren…", cx)).into_any_element()
        } else if let Some(recording) = &study.recording {
            let started = recording.heard_speech();
            h_flex()
                .gap_3()
                .items_center()
                .child(div().size(px(10.)).rounded_full().bg(theme.danger))
                .child(
                    div()
                        .w(px(160.))
                        .h(px(6.))
                        .rounded_full()
                        .bg(theme.secondary)
                        .child(div().h_full().rounded_full().bg(theme.danger).w(px(160. * study.level))),
                )
                .child(hint(if started { "Opname… stopt vanzelf als je klaar bent." } else { "Lees de kaart hardop." }, cx))
                .child(
                    Button::new("stop-recording")
                        .outline()
                        .tab_stop(false)
                        .child(h_flex().gap_2().child("Stop").child(keycap("Spatie", cx)))
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.stop_recording();
                            this.study.focus.focus(window, cx);
                        })),
                )
                .into_any_element()
        } else {
            h_flex()
                .gap_3()
                .items_center()
                .child(hint("Lees de kaart hardop.", cx))
                .child(
                    Button::new("record")
                        .primary()
                        .tab_stop(false)
                        .child(h_flex().gap_2().child("Opnemen").child(keycap("Spatie", cx)))
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.start_recording(cx);
                            this.study.focus.focus(window, cx);
                        })),
                )
                .child(
                    Button::new("no-recording")
                        .outline()
                        .tab_stop(false)
                        .child(h_flex().gap_2().child("Zonder opname").child(keycap("Enter", cx)))
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.study.step = Step::Score;
                            this.study.focus.focus(window, cx);
                            cx.notify();
                        })),
                )
                .into_any_element()
        };
        v_flex().items_center().gap_2().child(body).children(note).into_any_element()
    }

    /// How understandable the learner was, with the words the app missed.
    pub fn render_speech_panel(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let study = &self.study;
        let Some(result) = &study.speech else { return div().into_any_element() };
        let color = grade_color(result.grade, cx);
        let words = h_flex().flex_wrap().justify_center().gap_x_1p5().children(result.words.iter().map(|mark| {
            div()
                .when(!mark.heard, |d| d.text_color(theme.danger).underline())
                .child(mark.word.clone())
        }));
        let lowered = study.grade.is_some_and(|own| result.grade.number() < own.number());
        v_flex()
            .w_full()
            .max_w(px(600.))
            .gap_2()
            .p_3()
            .rounded(px(8.))
            .border_1()
            .border_color(theme.border)
            .bg(theme.secondary)
            .child(
                h_flex()
                    .justify_between()
                    .gap_3()
                    .child(
                        h_flex()
                            .gap_2()
                            .items_center()
                            .child(div().text_sm().text_color(theme.muted_foreground).child("Uitspraak"))
                            .child(
                                div()
                                    .px_2()
                                    .rounded(px(4.))
                                    .bg(color)
                                    .text_color(theme.background)
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(result.grade.number().to_string()),
                            )
                            .child(div().font_weight(FontWeight::MEDIUM).child(speech_label(result.grade)))
                            .child(hint(format!("{:.0}% verstaan", result.accuracy * 100.0), cx)),
                    )
                    .child(
                        h_flex()
                            .gap_1()
                            .child(
                                Button::new("own-recording")
                                    .ghost()
                                    .small()
                                    .tab_stop(false)
                                    .disabled(!study.has_recording)
                                    .child(h_flex().gap_1p5().child("Mijn opname").child(keycap("O", cx)))
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.play_own_recording(cx);
                                        this.study.focus.focus(window, cx);
                                    })),
                            )
                            .child(
                                Button::new("record-again")
                                    .ghost()
                                    .small()
                                    .tab_stop(false)
                                    .disabled(self.busy_listening())
                                    .child(h_flex().gap_1p5().child("Opnieuw").child(keycap("R", cx)))
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.record_again(cx);
                                        this.study.focus.focus(window, cx);
                                    })),
                            ),
                    ),
            )
            .child(words)
            .child(hint(
                if result.heard.is_empty() { "De app hoorde niets.".to_string() } else { format!("De app hoorde: {}", result.heard) },
                cx,
            ))
            .when(study.judging, |d| d.child(hint("Even luisteren…", cx)))
            .when_some(study.speech_note.clone(), |d, note| d.child(hint(note, cx).text_color(theme.warning)))
            .when(lowered, |d| {
                d.child(hint(
                    format!("Telt als {}: je uitspraak scoort lager dan je eigen beoordeling.", result.grade.label()),
                    cx,
                ))
            })
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
