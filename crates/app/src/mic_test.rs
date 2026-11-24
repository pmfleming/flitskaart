//! Instellingen → Microfoon & uitspraak: Whisper status, which microphone
//! the sound comes from, input boost and a test recording.

use std::path::PathBuf;

use anyhow::Result;
use dutch_flashcards_core::inputs::{Input, InputKind, Inputs};
use dutch_flashcards_core::speech::write_wav_16k;
use dutch_flashcards_core::store::{INPUT_BOOSTS, boost_label, snap_input_boost};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::slider::{Slider, SliderEvent, SliderState};
use gpui_kit::component::{ActiveTheme as _, Colorize as _, Disableable as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{AnyElement, App, AppContext as _, Context, Div, Entity, FontWeight, IntoElement, ParentElement as _, SharedString, Styled as _, Subscription, div, px};

use crate::app::FlashcardsApp;
use crate::capture::Capture;
use crate::icons;
use crate::recorder::{Captured, Loudness};
use crate::ui::{card_box, explained, hint, titled};

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

    pub fn is_recording(&self) -> bool {
        self.capture.is_on()
    }

    /// Shows why the test went wrong, and logs it.
    fn fail(&mut self, err: impl std::fmt::Display) {
        self.error = Some(crate::logging::error("microfoontest", format!("{err:#}")).into());
    }

    fn button_label(&self) -> &'static str {
        match (self.capture.is_on(), self.result.is_some()) {
            (true, _) => "Stop",
            (false, true) => "Opnieuw",
            (false, false) => "Test",
        }
    }

    /// What Whisper made of the test, or that it is still listening.
    fn heard(&self) -> Option<String> {
        if self.transcribing {
            return Some("Even luisteren…".into());
        }
        let heard = self.result.as_ref()?.heard.as_deref()?;
        Some(if heard.is_empty() { "Geen woorden gehoord.".into() } else { format!("“{heard}”") })
    }

    /// Where the sound comes from when PipeWire can't say.
    fn unknown_source(&self) -> String {
        if !self.inputs_read {
            return "Microfoons opzoeken…".into();
        }
        match self.result.as_ref().and_then(|result| result.device.as_deref()) {
            Some(device) => format!("Geluid komt van: {device}"),
            None => "De app neemt op met de standaardmicrofoon van je systeem.".into(),
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
        if saved && matches!(loudness, Loudness::Quiet | Loudness::Good | Loudness::Clipped) && self.listener.is_installed() {
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

    pub fn render_mic_test(&self, cx: &mut Context<Self>) -> Div {
        let theme = cx.theme().clone();
        let test = &self.mic_test;
        let recording = test.capture.is_on();
        let whisper_ready = self.listener.is_installed();
        let whisper_color = if whisper_ready { theme.success } else { theme.danger };
        let whisper_badge = explained("whisper", "Whisper luistert op deze laptop en geeft je uitspraak in Oefenen een cijfer van 1 tot 4.")
            .flex()
            .items_center()
            .gap_1p5()
            .px_2p5()
            .py(px(3.))
            .rounded_full()
            .bg(whisper_color.opacity(0.14))
            .child(div().size(px(7.)).rounded_full().bg(whisper_color))
            .child(
                div()
                    .text_xs()
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(whisper_color)
                    .child(if whisper_ready { "Whisper" } else { "Whisper ontbreekt" }),
            );

        let mut test_button = Button::new("mic-test")
            .outline()
            .icon(if recording { icons::stop() } else { icons::microphone() })
            .label(test.button_label())
            .disabled(test.transcribing || (!recording && self.busy_listening()))
            .on_click(cx.listener(move |this, _, _, cx| {
                if recording { this.mic_test.capture.stop() } else { this.start_mic_test(cx) }
            }));
        if recording {
            test_button = test_button.danger();
        }
        let play_button = Button::new("mic-test-play")
            .outline()
            .icon(icons::playback())
            .tooltip("Hoor wat de computer hoorde")
            .disabled(recording || !test.result.as_ref().is_some_and(|result| result.saved))
            .on_click(cx.listener(|this, _, _, cx| this.play_file(&this.mic_test_path(), "Je hoort nu wat de computer hoorde.", cx)));
        let status = match test.capture.render("Zeg iets.", cx) {
            Some(meter) => Some(meter.into_any_element()),
            None => test.result.as_ref().map(|result| self.render_mic_result(result, cx)),
        };
        let sample = explained("mic-sample", "Zeg bijvoorbeeld")
            .flex()
            .items_center()
            .gap_2()
            .text_sm()
            .text_color(theme.muted_foreground)
            .child(icons::speak().size_4().flex_none())
            .child(div().italic().child(format!("“{TEST_SENTENCE}”")));

        card_box(cx)
            .p_4()
            .gap_3p5()
            .child(
                v_flex()
                    .gap_1()
                    .child(h_flex().justify_between().items_center().flex_wrap().gap_2().child(titled(icons::microphone(), "Microfoon & uitspraak", cx)).child(whisper_badge))
                    .when(!whisper_ready, |d| d.child(hint("Installeer spraakherkenning: scripts/fetch-models.sh", cx).text_color(theme.danger))),
            )
            .child(self.render_inputs(cx))
            .child(self.render_boost(cx))
            .child(
                v_flex()
                    .gap_2p5()
                    .pt_3()
                    .border_t_1()
                    .border_color(theme.border)
                    .child(sample)
                    .child(h_flex().gap_2().flex_wrap().child(test_button).child(play_button))
                    .children(status)
                    .children(test.error.clone().map(|text| hint(text, cx).text_color(theme.danger))),
            )
    }

    /// The input boost: a slider over the offered steps, with equal-width tick labels aligned under each stop.
    fn render_boost(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let current = self.settings.input_boost;
        let ticks = h_flex().w(px(360. + 28.)).ml(px(-14.)).justify_between().children(INPUT_BOOSTS.map(|boost| {
            let active = (boost - current).abs() < 0.01;
            div()
                .w(px(40.))
                .text_center()
                .text_xs()
                .font_weight(if active { FontWeight::SEMIBOLD } else { FontWeight::NORMAL })
                .text_color(if active { theme.primary } else { theme.muted_foreground })
                .child(boost_label(boost))
        }));
        let name = explained("boost", "Maakt de opname harder of zachter voordat de app luistert. Bij 1× blijft hij zoals hij is.")
            .flex()
            .items_center()
            .gap_2()
            .child(icons::boost().size_4().text_color(theme.muted_foreground))
            .child(div().font_weight(FontWeight::MEDIUM).child("Versterking"));
        v_flex()
            .gap_1p5()
            .w(px(360.))
            .child(
                h_flex()
                    .justify_between()
                    .items_center()
                    .child(name)
                    .child(div().font_weight(FontWeight::SEMIBOLD).text_color(theme.primary).child(boost_label(current))),
            )
            .child(Slider::new(&self.mic_test.boost_slider).disabled(self.mic_test.capture.is_on()))
            .child(ticks)
    }

    fn choose_input(&mut self, node: Option<String>, cx: &mut Context<Self>) {
        if self.is_capturing() {
            return;
        }
        self.settings.input_device = node;
        self.save_settings();
        self.refresh_inputs(cx);
        cx.notify();
    }

    fn render_inputs(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let inputs = self.mic_test.inputs.as_ref();
        let selected = self.settings.input_device.as_deref();
        let active = match selected {
            Some(node) => inputs.and_then(|inputs| inputs.0.iter().find(|input| input.node == node)),
            None => inputs.and_then(Inputs::default_input),
        };
        let busy = self.is_capturing();
        let header = h_flex()
            .justify_between()
            .items_center()
            .child(div().text_xs().font_weight(FontWeight::MEDIUM).text_color(theme.muted_foreground).child("Microfoons"))
            .child(
                Button::new("mic-use-default").small().outline().label("Systeemstandaard")
                    .tooltip("Volg de standaardmicrofoon van je systeem")
                    .disabled(busy || selected.is_none())
                    .on_click(cx.listener(|this, _, _, cx| this.choose_input(None, cx))),
            );
        let rows = inputs.into_iter().flat_map(|inputs| &inputs.0).enumerate().map(|(index, input)| {
            let in_use = active.is_some_and(|active| active.node == input.node);
            let mut row = input_row(index, input, in_use, cx);
            if selected != Some(input.node.as_str()) {
                let node = input.node.clone();
                row = row.child(
                    Button::new(("mic-select", index as u64)).small().outline().label("Kies")
                        .tooltip(format!("Gebruik {} voor Flitskaart", input.label))
                        .disabled(busy)
                        .on_click(cx.listener(move |this, _, _, cx| this.choose_input(Some(node.clone()), cx))),
                );
            }
            row
        });
        let note = match (selected, inputs, active) {
            (_, _, Some(input)) if input.muted => Some("De microfoon in gebruik staat gedempt. Zet hem aan in je systeeminstellingen.".into()),
            (Some(_), _, None) if self.mic_test.inputs_read => Some("De gekozen microfoon is niet beschikbaar. Kies een andere of gebruik de systeemstandaard.".into()),
            (None, Some(_), None) => Some("Er is geen standaardmicrofoon gekozen in je systeeminstellingen.".into()),
            (_, None, _) => Some(self.mic_test.unknown_source()),
            _ => None,
        };
        v_flex()
            .gap_1p5()
            .child(header)
            .children(rows)
            .when_some(note, |d, note| d.child(hint(note, cx)))
            .into_any_element()
    }

    fn render_mic_result(&self, result: &MicResult, cx: &App) -> AnyElement {
        let theme = cx.theme();
        let color = match result.loudness {
            Loudness::Good => theme.success,
            Loudness::Quiet | Loudness::Clipped => theme.warning,
            Loudness::NoSignal | Loudness::Silent => theme.danger,
        };
        sub_box(color.opacity(0.45), color.opacity(0.08))
            .gap_2()
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(div().flex_none().size(px(10.)).rounded_full().bg(color))
                    .child(div().font_weight(FontWeight::MEDIUM).child(result.loudness.verdict())),
            )
            .children(self.mic_test.heard().map(|text| {
                explained("mic-heard", "De app hoorde")
                    .flex()
                    .items_center()
                    .gap_1p5()
                    .child(icons::transcript().size_4().flex_none().text_color(theme.muted_foreground))
                    .child(hint(text, cx))
            }))
            .into_any_element()
    }
}

/// A padded, rounded inset box inside a settings card.
fn sub_box(border: gpui_kit::Hsla, bg: gpui_kit::Hsla) -> Div {
    v_flex().p_3().rounded(px(8.)).border_1().border_color(border).bg(bg)
}

/// One microphone: its kind, its name (cut short when long, whole on hover),
/// its volume, whether it is muted, and whether recordings come from it.
fn input_row(index: usize, input: &Input, active: bool, cx: &App) -> Div {
    let theme = cx.theme();
    let id = index as u64;
    let kind = match input.kind {
        InputKind::Bluetooth => icons::headset(),
        InputKind::Usb => icons::usb(),
        InputKind::BuiltIn => icons::laptop(),
        InputKind::Other => icons::microphone(),
    };
    let (fg, muted_fg) = if active { (theme.accent_foreground, theme.accent_foreground.opacity(0.75)) } else { (theme.foreground, theme.muted_foreground) };
    let in_use = h_flex()
        .flex_none()
        .items_center()
        .gap_1()
        .px_2()
        .py(px(1.5))
        .rounded_full()
        .bg(theme.primary.opacity(0.15))
        .text_xs()
        .font_weight(FontWeight::MEDIUM)
        .text_color(theme.primary)
        .child(icons::check().size_3p5())
        .child("In gebruik");
    h_flex()
        .gap_3()
        .items_center()
        .px_3()
        .py_2()
        .rounded(px(8.))
        .border_1()
        .border_color(if active { theme.primary.opacity(0.5) } else { theme.border })
        .when(active, |d| d.bg(theme.accent))
        .child(explained(("mic-kind", id), input.kind.label().to_string()).flex_none().text_color(muted_fg).child(kind.size_5()))
        .child(
            explained(("mic-name", id), input.label.clone())
                .flex_1()
                .min_w_0()
                .truncate()
                .text_color(if active { fg } else { muted_fg })
                .when(active, |d| d.font_weight(FontWeight::SEMIBOLD))
                .child(input.label.clone()),
        )
        .when_some(input.volume, |d, volume| {
            d.child(
                explained(("mic-volume", id), "Volume in je systeem")
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap_1()
                    .text_sm()
                    .text_color(muted_fg)
                    .child(icons::boost().size_4())
                    .child(format!("{:.0}%", volume * 100.0)),
            )
        })
        .when(input.muted, |d| d.child(explained(("mic-muted", id), "Gedempt").flex_none().text_color(theme.danger).child(icons::mic_off().size_4())))
        .when(active, |d| d.child(in_use))
}
