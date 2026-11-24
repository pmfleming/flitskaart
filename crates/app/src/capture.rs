//! A recording in progress, shared by Oefenen's reading test and the
//! microphone test: the level meter while it runs, and its result when done.

use std::time::Duration;

use anyhow::Result;
use gpui_kit::{App, Context, Div, ParentElement as _, Task};

use crate::app::FlashcardsApp;
use crate::recorder::{self, Captured, Recording};
use crate::ui::{hint, level_meter};

#[derive(Default)]
pub struct Capture {
    recording: Option<Recording>,
    poller: Option<Task<()>>,
    level: f32,
}

/// Which capture a recording fills.
type Slot = fn(&mut FlashcardsApp) -> &mut Capture;
/// What to do with what was captured.
type Done = fn(&mut FlashcardsApp, Result<Captured>, &mut Context<FlashcardsApp>);

impl Capture {
    pub fn is_on(&self) -> bool {
        self.recording.is_some()
    }

    /// Ends the recording early; what was heard so far is still judged.
    pub fn stop(&self) {
        if let Some(recording) = &self.recording {
            recording.request_stop();
        }
    }

    /// Drops the recording without judging it.
    pub fn cancel(&mut self) {
        *self = Capture::default();
    }

    /// The level meter, with `prompt` until speech is heard. `None` when not recording.
    pub fn render(&self, prompt: &'static str, cx: &App) -> Option<Div> {
        let recording = self.recording.as_ref()?;
        let text = if !recording.is_ready() {
            "Microfoon wordt klaargemaakt…"
        } else if recording.heard_speech() {
            "Opname… stopt vanzelf als je klaar bent."
        } else {
            prompt
        };
        Some(level_meter(self.level, cx).child(hint(text, cx)))
    }
}

impl FlashcardsApp {
    pub fn is_capturing(&self) -> bool {
        self.study.attempt.capture.is_on() || self.mic_test.is_recording() || self.talk.capture.is_on()
    }

    /// Records into `slot` and hands the result to `done`. With `--recording`,
    /// the file stands in for the microphone and `done` runs straight away.
    pub fn start_capture(&mut self, slot: Slot, done: Done, cx: &mut Context<Self>) {
        if self.is_capturing() {
            return;
        }
        // A Bluetooth headset is switched to headset mode for the recording;
        // the app's own sound must not be running through that switch.
        self.stop_audio("opname begint");
        let boost = self.settings.input_boost;
        if let Some(captured) = self.fake_recording.as_deref().map(|path| recorder::from_file(path, boost)) {
            return done(self, captured, cx);
        }
        match Recording::start(boost, self.settings.input_device.clone()) {
            Ok(recording) => slot(self).recording = Some(recording),
            Err(err) => return done(self, Err(err), cx),
        }
        // Dropping the slot cancels its poller as well as its recording.
        slot(self).poller = Some(cx.spawn(async move |this, cx| {
            while this.update(cx, |this, cx| this.poll_capture(slot, done, cx)).unwrap_or(false) {
                cx.background_executor().timer(Duration::from_millis(50)).await;
            }
        }));
    }

    /// Refreshes the level meter, or hands a finished recording to `done`.
    /// Returns whether the recording goes on.
    fn poll_capture(&mut self, slot: Slot, done: Done, cx: &mut Context<Self>) -> bool {
        cx.notify();
        let capture = slot(self);
        let Some(finished) = capture.recording.take_if(|recording| recording.is_finished()) else {
            capture.level = capture.recording.as_ref().map_or(0.0, Recording::level);
            return capture.recording.is_some();
        };
        capture.level = 0.0;
        done(self, finished.finish(), cx);
        false
    }
}
