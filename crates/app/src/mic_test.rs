//! Instellingen → Microfoon: record a few words, see how loud they came in,
//! hear them back and, when Whisper is installed, read what the app understood.

use std::path::PathBuf;
use std::time::Duration;

use dutch_flashcards_core::speech::write_wav_16k;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{ActiveTheme as _, Disableable as _, Sizable as _, h_flex, v_flex};
use gpui_kit::*;

use crate::app::FlashcardsApp;
use crate::icons;
use crate::recorder::{self, Captured, Loudness, Recording};
use crate::ui::{heading, hint, level_meter};

/// A sentence to say during the test.
const TEST_SENTENCE: &str = "Goedemorgen, hoe gaat het met je?";

#[derive(Default)]
pub struct MicTest {
    recording: Option<Recording>,
    level: f32,
    result: Option<MicResult>,
    transcribing: bool,
    /// Why the test could not run, such as no microphone at all.
    error: Option<SharedString>,
}

struct MicResult {
    loudness: Loudness,
    device: Option<String>,
    /// Whether a recording was saved that can be played back.
    saved: bool,
    /// What Whisper heard, once it has listened.
    heard: Option<String>,
}

impl MicTest {
    pub fn is_busy(&self) -> bool {
        self.recording.is_some() || self.transcribing
    }
}

impl FlashcardsApp {
    fn mic_test_path(&self) -> PathBuf {
        self.paths.recordings().join("microfoontest.wav")
    }

    pub fn start_mic_test(&mut self, cx: &mut Context<Self>) {
        if self.mic_test.is_busy() || self.busy_listening() {
            return;
        }
        self.stop_audio();
        self.mic_test.result = None;
        self.mic_test.error = None;
        if let Some(path) = self.fake_recording.clone() {
            match recorder::from_file(&path) {
                Ok(captured) => self.finish_mic_test(captured, cx),
                Err(err) => self.mic_test.error = Some(format!("{err:#}").into()),
            }
            cx.notify();
            return;
        }
        self.mic_test.recording = Some(Recording::start());
        cx.notify();
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_millis(50)).await;
                let done = this.update(cx, |this, cx| {
                    let test = &mut this.mic_test;
                    let Some(recording) = &test.recording else { return true };
                    if recording.is_finished() {
                        let recording = test.recording.take().unwrap();
                        test.level = 0.0;
                        match recording.finish() {
                            Ok(captured) => this.finish_mic_test(captured, cx),
                            Err(err) => this.mic_test.error = Some(format!("{err:#}").into()),
                        }
                        cx.notify();
                        return true;
                    }
                    test.level = recording.level();
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

    fn stop_mic_test(&mut self) {
        if let Some(recording) = &self.mic_test.recording {
            recording.request_stop();
        }
    }

    fn finish_mic_test(&mut self, captured: Captured, cx: &mut Context<Self>) {
        let loudness = recorder::loudness(&captured);
        let path = self.mic_test_path();
        let saved = !captured.samples.is_empty()
            && match write_wav_16k(&path, &captured.samples, captured.rate, captured.channels) {
                Ok(()) => true,
                Err(err) => {
                    self.mic_test.error = Some(format!("{err:#}").into());
                    false
                }
            };
        self.mic_test.result = Some(MicResult { loudness, device: captured.device, saved, heard: None });
        if !saved || loudness == Loudness::Silent || !self.listener.is_installed() {
            return;
        }
        self.mic_test.transcribing = true;
        let listener = self.listener.clone();
        let job = cx.background_executor().spawn(async move { listener.transcribe(&path) });
        cx.spawn(async move |this, cx| {
            let heard = job.await;
            let _ = this.update(cx, |this, cx| {
                let test = &mut this.mic_test;
                test.transcribing = false;
                match heard {
                    Ok(heard) => {
                        if let Some(result) = &mut test.result {
                            result.heard = Some(heard);
                        }
                    }
                    Err(err) => test.error = Some(format!("{err:#}").into()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn play_mic_test(&mut self, cx: &mut Context<Self>) {
        if self.mic_test.recording.is_some() {
            return;
        }
        match self.audio.play(&self.mic_test_path()) {
            Ok(_) => self.info("Je testopname speelt af."),
            Err(err) => self.error(format!("{err:#}")),
        }
        cx.notify();
    }

    pub fn render_mic_test(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let test = &self.mic_test;
        let recording = test.recording.is_some();

        let button = Button::new("mic-test")
            .outline()
            .icon(icons::speak())
            .label(if recording {
                "Stop"
            } else if test.result.is_some() {
                "Opnieuw testen"
            } else {
                "Test de microfoon"
            })
            .disabled(test.transcribing || (!recording && self.busy_listening()))
            .on_click(cx.listener(move |this, _, _, cx| {
                if recording { this.stop_mic_test() } else { this.start_mic_test(cx) }
            }));

        let status = if let Some(rec) = &test.recording {
            Some(
                level_meter(test.level, cx)
                    .child(hint(if rec.heard_speech() { "Opname… stopt vanzelf als je klaar bent." } else { "Zeg iets." }, cx))
                    .into_any_element(),
            )
        } else {
            test.result.as_ref().map(|result| self.render_mic_result(result, cx))
        };

        v_flex()
            .gap_3()
            .max_w(px(640.))
            .child(heading("Microfoon"))
            .child(hint(
                format!("Neem een paar woorden op om te horen of de app je goed verstaat. Zeg bijvoorbeeld: “{TEST_SENTENCE}”"),
                cx,
            ))
            .child(h_flex().child(button))
            .children(status)
            .children(test.error.clone().map(|text| hint(text, cx).text_color(theme.danger)))
    }

    fn render_mic_result(&self, result: &MicResult, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let (color, verdict) = match result.loudness {
            Loudness::Good => (theme.success, "Je microfoon werkt."),
            Loudness::Quiet => (theme.warning, "Je klinkt zacht. Spreek dichter bij de microfoon of zet het invoervolume hoger."),
            Loudness::Clipped => (theme.warning, "Je klinkt te hard en vervormd. Zet het invoervolume lager."),
            Loudness::Silent => (
                theme.danger,
                "Ik hoorde niets. Kijk of de juiste microfoon is gekozen in je systeeminstellingen en niet gedempt is.",
            ),
        };
        let heard = if self.mic_test.transcribing {
            Some("Even luisteren…".to_string())
        } else {
            result.heard.as_ref().map(|heard| {
                if heard.is_empty() { "De app hoorde geen woorden.".to_string() } else { format!("De app hoorde: {heard}") }
            })
        };
        v_flex()
            .gap_2()
            .p_3()
            .rounded(px(6.))
            .border_1()
            .border_color(theme.border)
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(div().flex_none().size(px(10.)).rounded_full().bg(color))
                    .child(div().font_weight(FontWeight::MEDIUM).child(verdict)),
            )
            .children(result.device.as_ref().map(|name| hint(format!("Microfoon: {name}"), cx)))
            .children(heard.map(|text| hint(text, cx)))
            .child(
                h_flex().child(
                    Button::new("mic-test-play")
                        .ghost()
                        .small()
                        .icon(icons::listen())
                        .label("Beluister de opname")
                        .disabled(!result.saved || result.loudness == Loudness::Silent)
                        .on_click(cx.listener(|this, _, _, cx| this.play_mic_test(cx))),
                ),
            )
            .into_any_element()
    }
}
