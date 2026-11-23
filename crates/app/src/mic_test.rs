//! Instellingen → Microfoon: which microphone the sound comes from, and a test
//! that records a few words, says how loud they came in, plays back exactly
//! what the computer heard and, when Whisper is installed, what it understood.

use std::path::PathBuf;

use anyhow::Result;
use dutch_flashcards_core::inputs::Inputs;
use dutch_flashcards_core::speech::write_wav_16k;
use dutch_flashcards_core::store::{INPUT_BOOSTS, boost_label, snap_input_boost};
use gpui_kit::component::button::Button;
use gpui_kit::component::slider::{Slider, SliderEvent, SliderState};
use gpui_kit::component::{ActiveTheme as _, Disableable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{AnyElement, App, AppContext as _, Context, Div, Entity, Subscription, FontWeight, IntoElement, ParentElement as _, SharedString, Styled as _, div, px};

use crate::app::FlashcardsApp;
use crate::capture::Capture;
use crate::icons;
use crate::recorder::{Captured, Loudness};
use crate::ui::{heading, hint};

/// A sentence to say during the test.
const TEST_SENTENCE: &str = "Goedemorgen, hoe gaat het met je?";

pub struct MicTest {
    capture: Capture,
    /// The input boost slider.
    boost_slider: Entity<SliderState>,
    _boost_changed: Subscription,
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
    pub fn new(boost: f32, cx: &mut Context<FlashcardsApp>) -> MicTest {
        let steps = SliderState::new().min(INPUT_BOOSTS[0]).max(INPUT_BOOSTS[INPUT_BOOSTS.len() - 1]).step(0.5);
        let boost_slider = cx.new(|_| steps.default_value(boost));
        let boost_changed = cx.subscribe(&boost_slider, |this, _, event: &SliderEvent, cx| this.boost_slider_moved(event, cx));
        MicTest {
            capture: Capture::default(),
            boost_slider,
            _boost_changed: boost_changed,
            result: None,
            transcribing: false,
            error: None,
            inputs: None,
            inputs_read: false,
        }
    }

    pub fn is_busy(&self) -> bool {
        self.capture.is_on() || self.transcribing
    }

    /// Shows why the test went wrong, and logs it.
    fn fail(&mut self, err: impl std::fmt::Display) {
        self.error = Some(crate::logging::error("microfoontest", format!("{err:#}")).into());
    }

    fn button_label(&self) -> &'static str {
        match (self.capture.is_on(), self.result.is_some()) {
            (true, _) => "Stop",
            (false, true) => "Opnieuw testen",
            (false, false) => "Test de microfoon",
        }
    }

    /// What Whisper made of the test, or that it is still listening.
    fn heard(&self) -> Option<String> {
        if self.transcribing {
            return Some("Even luisteren…".into());
        }
        let heard = self.result.as_ref()?.heard.as_deref()?;
        Some(if heard.is_empty() { "De app hoorde geen woorden.".into() } else { format!("De app hoorde: {heard}") })
    }

    /// Where the sound comes from when PipeWire can't say.
    fn unknown_source(&self) -> String {
        if !self.inputs_read {
            return "Microfoons opzoeken…".into();
        }
        match self.result.as_ref().and_then(|result| result.device.as_deref()) {
            Some(device) => format!("Geluid komt van: {device}"),
            None => "Het systeem zegt niet welke microfoon wordt gebruikt. De app neemt op met de standaardmicrofoon van je systeem.".into(),
        }
    }
}

impl FlashcardsApp {
    fn mic_test_path(&self) -> PathBuf {
        self.paths.recordings().join("microfoontest.wav")
    }

    /// Reads the system's microphones again in the background.
    pub fn refresh_inputs(&mut self, cx: &mut Context<Self>) {
        self.in_background(cx, Inputs::current, |this, inputs, _| {
            (this.mic_test.inputs, this.mic_test.inputs_read) = (inputs, true);
        });
    }

    pub fn start_mic_test(&mut self, cx: &mut Context<Self>) {
        if !self.busy() {
            self.refresh_inputs(cx);
            (self.mic_test.result, self.mic_test.error) = (None, None);
            self.start_capture(|this| &mut this.mic_test.capture, Self::finish_mic_test, cx);
        }
    }

    /// Saves the test for playback, judges how loud it was, and lets Whisper
    /// say what it understood.
    fn finish_mic_test(&mut self, captured: Result<Captured>, cx: &mut Context<Self>) {
        match captured {
            Ok(captured) => self.judge_mic_test(captured, cx),
            Err(err) => self.mic_test.fail(err),
        }
    }

    fn judge_mic_test(&mut self, captured: Captured, cx: &mut Context<Self>) {
        let loudness = Loudness::of(&captured);
        crate::logging::info(format!("Microfoontest: {loudness:?}"));
        let path = self.mic_test_path();
        let saved = !captured.samples.is_empty()
            && write_wav_16k(&path, &captured.samples, captured.rate, captured.channels).inspect_err(|err| self.mic_test.fail(err)).is_ok();
        self.mic_test.result = Some(MicResult { loudness, device: captured.device, saved, heard: None });
        self.show_mic_test_result();
        if saved && loudness != Loudness::Silent && self.listener.is_installed() {
            self.transcribe_mic_test(path, cx);
        }
    }

    fn transcribe_mic_test(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        self.mic_test.transcribing = true;
        let listener = self.listener.clone();
        self.in_background(cx, move || listener.transcribe(&path), |this, heard, _| {
            let test = &mut this.mic_test;
            test.transcribing = false;
            match (heard, test.result.as_mut()) {
                (Ok(heard), Some(result)) => result.heard = Some(heard),
                (Err(err), _) => test.fail(err),
                (Ok(_), None) => {}
            }
            this.show_mic_test_result();
        });
    }

    /// Scrolls Instellingen so the whole microphone section is in view.
    fn show_mic_test_result(&self) {
        self.settings_scroll.scroll_to_item(crate::settings::MICROPHONE_SECTION);
    }

    /// Follows the input boost slider while it moves and saves where it is let go.
    fn boost_slider_moved(&mut self, event: &SliderEvent, cx: &mut Context<Self>) {
        let (SliderEvent::Change(value) | SliderEvent::Release(value)) = event;
        self.settings.input_boost = snap_input_boost(value.start());
        if matches!(event, SliderEvent::Release(_)) {
            self.save_settings();
        }
        cx.notify();
    }

    pub fn render_mic_test(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let test = &self.mic_test;
        let recording = test.capture.is_on();
        let test_button = Button::new("mic-test")
            .outline()
            .icon(icons::speak())
            .label(test.button_label())
            .disabled(test.transcribing || (!recording && self.busy_listening()))
            .on_click(cx.listener(move |this, _, _, cx| {
                if recording { this.mic_test.capture.stop() } else { this.start_mic_test(cx) }
            }));
        let play_button = Button::new("mic-test-play")
            .outline()
            .icon(icons::listen())
            .label("Hoor wat de computer hoorde")
            .disabled(recording || !test.result.as_ref().is_some_and(|result| result.saved))
            .on_click(cx.listener(|this, _, _, cx| this.play_file(&this.mic_test_path(), "Je hoort nu wat de computer hoorde.", cx)));
        let status = match test.capture.render("Zeg iets.", cx) {
            Some(meter) => Some(meter.into_any_element()),
            None => test.result.as_ref().map(|result| self.render_mic_result(result, cx)),
        };
        let explanation = format!("Neem een paar woorden op en luister terug wat de computer hoorde. Zeg bijvoorbeeld: “{TEST_SENTENCE}”");
        v_flex()
            .gap_3()
            .max_w(px(640.))
            .child(heading("Microfoon"))
            .child(self.render_inputs(cx))
            .child(self.render_boost(cx))
            .child(hint(explanation, cx))
            .child(h_flex().gap_2().flex_wrap().child(test_button).child(play_button))
            .children(status)
            .children(test.error.clone().map(|text| hint(text, cx).text_color(cx.theme().danger)))
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
            .child(Slider::new(&self.mic_test.boost_slider).disabled(self.mic_test.capture.is_on()))
            .child(h_flex().justify_between().children(INPUT_BOOSTS.map(|boost| hint(boost_label(boost), cx))));
        v_flex()
            .gap_1p5()
            .child(slider)
            .child(hint("Maakt de opname harder of zachter voordat de app luistert. Bij 1× blijft hij zoals hij is.", cx))
    }

    /// Which microphone the sound comes from, and which others there are.
    fn render_inputs(&self, cx: &App) -> AnyElement {
        let theme = cx.theme();
        let Some(inputs) = &self.mic_test.inputs else { return hint(self.mic_test.unknown_source(), cx).into_any_element() };
        let muted = inputs.default_input().is_some_and(|input| input.muted);
        let source = match inputs.default_input() {
            Some(input) => h_flex()
                .gap_2()
                .flex_wrap()
                .items_baseline()
                .child(hint("Geluid komt van", cx))
                .child(div().font_weight(FontWeight::SEMIBOLD).child(input.label.clone()))
                .child(hint(input.describe(), cx).when(muted, |d| d.text_color(theme.danger))),
            None => h_flex().child(hint("Er is geen standaardmicrofoon gekozen in je systeeminstellingen.", cx).text_color(theme.danger)),
        };
        let others: Vec<String> = inputs.others().map(|input| format!("{} ({})", input.label, input.describe())).collect();
        box_with_border(if muted { theme.danger } else { theme.border })
            .gap_1()
            .child(source)
            .when(muted, |d| d.child(hint("Deze microfoon staat gedempt. Zet hem aan in je systeeminstellingen.", cx).text_color(theme.danger)))
            .when(!others.is_empty(), |d| {
                d.child(hint(format!("Ook aangesloten: {}.", others.join("; ")), cx)).child(hint(
                    "Een andere microfoon kies je in de geluidsinstellingen van je systeem; de app gebruikt de standaardmicrofoon.",
                    cx,
                ))
            })
            .into_any_element()
    }

    fn render_mic_result(&self, result: &MicResult, cx: &App) -> AnyElement {
        let theme = cx.theme();
        let color = match result.loudness {
            Loudness::Good => theme.success,
            Loudness::Quiet | Loudness::Clipped => theme.warning,
            Loudness::Silent => theme.danger,
        };
        box_with_border(theme.border)
            .gap_2()
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(div().flex_none().size(px(10.)).rounded_full().bg(color))
                    .child(div().font_weight(FontWeight::MEDIUM).child(result.loudness.verdict())),
            )
            .children(self.mic_test.heard().map(|text| hint(text, cx)))
            .into_any_element()
    }
}

/// A padded, rounded box with a coloured border.
fn box_with_border(color: gpui_kit::Hsla) -> Div {
    v_flex().p_3().rounded(px(6.)).border_1().border_color(color)
}
