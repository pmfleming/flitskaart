//! Instellingen → Microfoon: which microphone the sound comes from, and a test
//! that records a few words, says how loud they came in, plays back exactly
//! what the computer heard and, when Whisper is installed, what it understood.

use std::path::PathBuf;
use std::time::Duration;

use dutch_flashcards_core::inputs::{Input, Inputs};
use dutch_flashcards_core::speech::write_wav_16k;
use dutch_flashcards_core::store::{INPUT_BOOSTS, snap_input_boost};
use gpui_kit::component::button::Button;
use gpui_kit::component::slider::{Slider, SliderEvent};
use gpui_kit::component::{ActiveTheme as _, Disableable as _, h_flex, v_flex};
use gpui_kit::*;
use gpui_kit::prelude::FluentBuilder as _;

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
    /// The system's microphones; `None` until read, or when PipeWire can't be asked.
    inputs: Option<Inputs>,
    inputs_read: bool,
}

struct MicResult {
    loudness: Loudness,
    /// The device name the recorder reported, used when PipeWire can't be asked.
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

    /// Reads the system's microphones again in the background.
    pub fn refresh_inputs(&mut self, cx: &mut Context<Self>) {
        let job = cx.background_executor().spawn(async { Inputs::current() });
        cx.spawn(async move |this, cx| {
            let inputs = job.await;
            let _ = this.update(cx, |this, cx| {
                this.mic_test.inputs = inputs;
                this.mic_test.inputs_read = true;
                cx.notify();
            });
        })
        .detach();
    }

    pub fn start_mic_test(&mut self, cx: &mut Context<Self>) {
        if self.mic_test.is_busy() || self.busy_listening() {
            return;
        }
        self.stop_audio();
        self.refresh_inputs(cx);
        self.mic_test.result = None;
        self.mic_test.error = None;
        if let Some(path) = self.fake_recording.clone() {
            match recorder::from_file(&path, self.settings.input_boost) {
                Ok(captured) => self.finish_mic_test(captured, cx),
                Err(err) => self.mic_test.error = Some(format!("{err:#}").into()),
            }
            cx.notify();
            return;
        }
        self.mic_test.recording = Some(Recording::start(self.settings.input_boost));
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
        self.show_mic_test_result();
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
                this.show_mic_test_result();
                cx.notify();
            });
        })
        .detach();
    }

    /// Scrolls Instellingen so the whole microphone section is in view.
    fn show_mic_test_result(&self) {
        self.settings_scroll.scroll_to_item(crate::settings::MICROPHONE_SECTION);
    }

    /// Follows the input boost slider while it moves and saves where it is let go.
    pub fn boost_slider_moved(&mut self, event: &SliderEvent, cx: &mut Context<Self>) {
        let (value, release) = match event {
            SliderEvent::Change(value) => (value.start(), false),
            SliderEvent::Release(value) => (value.start(), true),
        };
        self.settings.input_boost = snap_input_boost(value);
        if release {
            self.save_settings();
        }
        cx.notify();
    }

    /// Plays the saved test: the same 16 kHz sound Whisper listens to.
    fn play_mic_test(&mut self, cx: &mut Context<Self>) {
        if self.mic_test.recording.is_some() {
            return;
        }
        match self.audio.play(&self.mic_test_path()) {
            Ok(_) => self.info("Je hoort nu wat de computer hoorde."),
            Err(err) => self.error(format!("{err:#}")),
        }
        cx.notify();
    }

    pub fn render_mic_test(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let test = &self.mic_test;
        let recording = test.recording.is_some();
        let can_play = test.result.as_ref().is_some_and(|r| r.saved) && !recording;

        let test_button = Button::new("mic-test")
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
        let play_button = Button::new("mic-test-play")
            .outline()
            .icon(icons::listen())
            .label("Hoor wat de computer hoorde")
            .disabled(!can_play)
            .on_click(cx.listener(|this, _, _, cx| this.play_mic_test(cx)));

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
            .child(self.render_inputs(cx))
            .child(self.render_boost(cx))
            .child(hint(
                format!(
                    "Neem een paar woorden op en luister terug wat de computer hoorde. Zeg bijvoorbeeld: “{TEST_SENTENCE}”"
                ),
                cx,
            ))
            .child(h_flex().gap_2().flex_wrap().child(test_button).child(play_button))
            .children(status)
            .children(test.error.clone().map(|text| hint(text, cx).text_color(theme.danger)))
    }

    /// The input boost: a slider over the offered steps, with the steps marked below it.
    fn render_boost(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let slider = v_flex()
            .gap_1p5()
            .max_w(px(360.))
            .child(
                h_flex()
                    .justify_between()
                    .child(div().font_weight(FontWeight::MEDIUM).child("Versterking"))
                    .child(div().font_weight(FontWeight::SEMIBOLD).child(boost_label(self.settings.input_boost))),
            )
            .child(Slider::new(&self.boost_slider).disabled(self.mic_test.recording.is_some()))
            .child(h_flex().justify_between().children(INPUT_BOOSTS.map(|boost| hint(boost_label(boost), cx))));
        v_flex()
            .gap_1p5()
            .child(slider)
            .child(hint("Maakt de opname harder of zachter voordat de app luistert. Bij 1× blijft hij zoals hij is.", cx))
    }

    /// Which microphone the sound comes from, and which others there are.
    fn render_inputs(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let test = &self.mic_test;
        let Some(inputs) = &test.inputs else {
            let text = if !test.inputs_read {
                "Microfoons opzoeken…".to_string()
            } else if let Some(device) = test.result.as_ref().and_then(|r| r.device.clone()) {
                format!("Geluid komt van: {device}")
            } else {
                "Het systeem zegt niet welke microfoon wordt gebruikt. De app neemt op met de standaardmicrofoon van je systeem.".to_string()
            };
            return hint(text, cx).into_any_element();
        };
        let describe = |input: &Input| {
            let mut parts = vec![input.kind.label().to_string()];
            if let Some(volume) = input.volume {
                parts.push(format!("volume {:.0}%", volume * 100.0));
            }
            if input.muted {
                parts.push("gedempt".into());
            }
            parts.join(", ")
        };

        let source = match inputs.default_input() {
            Some(input) => h_flex()
                .gap_2()
                .flex_wrap()
                .items_baseline()
                .child(hint("Geluid komt van", cx))
                .child(div().font_weight(FontWeight::SEMIBOLD).child(input.label.clone()))
                .child(hint(describe(input), cx).when(input.muted, |d| d.text_color(theme.danger))),
            None => h_flex().child(hint("Er is geen standaardmicrofoon gekozen in je systeeminstellingen.", cx).text_color(theme.danger)),
        };
        let others: Vec<String> = inputs.others().map(|input| format!("{} ({})", input.label, describe(input))).collect();
        let default_muted = inputs.default_input().is_some_and(|input| input.muted);

        v_flex()
            .gap_1()
            .p_3()
            .rounded(px(6.))
            .border_1()
            .border_color(if default_muted { theme.danger } else { theme.border })
            .child(source)
            .when(default_muted, |d| {
                d.child(hint("Deze microfoon staat gedempt. Zet hem aan in je systeeminstellingen.", cx).text_color(theme.danger))
            })
            .when(!others.is_empty(), |d| {
                d.child(hint(format!("Ook aangesloten: {}.", others.join("; ")), cx)).child(hint(
                    "Een andere microfoon kies je in de geluidsinstellingen van je systeem; de app gebruikt de standaardmicrofoon.",
                    cx,
                ))
            })
            .into_any_element()
    }

    fn render_mic_result(&self, result: &MicResult, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let (color, verdict) = match result.loudness {
            Loudness::Good => (theme.success, "Je microfoon werkt."),
            Loudness::Quiet => (theme.warning, "Je klinkt zacht. Zet de versterking hoger of spreek dichter bij de microfoon."),
            Loudness::Clipped => (theme.warning, "Je klinkt te hard en vervormd. Zet de versterking lager."),
            Loudness::Silent => (
                theme.danger,
                "Ik hoorde niets. Kijk hierboven of het de juiste microfoon is en of hij niet gedempt is.",
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
            .children(heard.map(|text| hint(text, cx)))
            .into_any_element()
    }
}

/// An input boost as people read it here: `0,5×`, `1×`, `2,5×`.
fn boost_label(boost: f32) -> String {
    if boost.fract() == 0.0 { format!("{boost:.0}×") } else { format!("{boost:.1}×").replace('.', ",") }
}
