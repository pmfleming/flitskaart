//! Sound out: the Dutch voice and recordings, through the default sound device.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use anyhow::{Context as _, Result};
use dutch_flashcards_core::paths::Paths;
use dutch_flashcards_core::voice::{self, Speaker, Speed, VoiceOption};
use gpui_kit::Context;
use rodio::Source as _;

use crate::app::FlashcardsApp;
use crate::logging;

/// What is playing, and the voice that makes speech.
pub struct Sound {
    pub speaker: Speaker,
    output: Output,
    /// Bumped on every playback so a finished sound only clears its own indicator.
    ticket: u64,
    pub playing: bool,
    /// The voice is generating speech that isn't cached yet.
    pub preparing: bool,
}

impl Sound {
    pub fn new(paths: &Paths) -> Sound {
        Sound { speaker: Speaker::new(paths), output: Output::default(), ticket: 0, playing: false, preparing: false }
    }
}

impl FlashcardsApp {
    /// Speaks `text` with `voice`: generates the audio in the background if it
    /// is not cached yet, then plays it.
    pub fn speak(&mut self, text: String, voice: &'static VoiceOption, speed: Speed, cx: &mut Context<Self>) {
        let speaker = self.sound.speaker.clone();
        self.sound.preparing = !speaker.audio_path(&text, voice, speed).exists();
        self.sound.ticket += 1;
        let ticket = self.sound.ticket;
        self.in_background(cx, move || speaker.speech(&text, voice, speed), move |this, path, cx| {
            this.sound.preparing = false;
            if this.sound.ticket != ticket {
                return;
            }
            match path.and_then(|path| this.sound.output.play(&path)) {
                Ok(length) => this.mark_playing(ticket, length, cx),
                Err(err) => this.error(format!("{err:#}")),
            }
        });
    }

    pub fn current_voice(&self) -> &'static VoiceOption {
        voice::voice(self.settings.voice_id())
    }

    /// Speaks `text` in the voice chosen in the settings.
    pub fn say(&mut self, text: String, cx: &mut Context<Self>) {
        self.speak(text, self.current_voice(), Speed::Normal, cx);
    }

    /// Shows "speelt af" until the sound of `ticket` has played for `length`.
    fn mark_playing(&mut self, ticket: u64, length: Duration, cx: &mut Context<Self>) {
        self.sound.playing = true;
        self.after(length + Duration::from_millis(150), cx, move |this, _| {
            if this.sound.ticket == ticket {
                this.sound.playing = false;
            }
        });
    }

    /// Generates audio ahead of time so playback starts instantly later.
    pub fn prepare_audio(&self, text: String, cx: &mut Context<Self>) {
        let speaker = self.sound.speaker.clone();
        let voice = self.current_voice();
        cx.background_executor()
            .spawn(async move {
                let _ = speaker.speech(&text, voice, Speed::Normal);
            })
            .detach();
    }

    pub fn stop_audio(&mut self) {
        self.sound.ticket += 1;
        self.sound.playing = false;
        self.sound.output.stop();
    }

    /// Plays a recording, saying `what` it is.
    pub fn play_file(&mut self, path: &Path, what: &'static str, cx: &mut Context<Self>) {
        match self.sound.output.play(path) {
            Ok(_) => self.info(what),
            Err(err) => self.error(format!("{err:#}")),
        }
        cx.notify();
    }
}

/// The sound device, opened on first use.
#[derive(Default)]
struct Output {
    sink: Option<rodio::MixerDeviceSink>,
    /// Set when the sound device reports an error, such as a Bluetooth headset
    /// dropping out; the next sound then opens the device afresh.
    failed: Arc<AtomicBool>,
    player: Option<rodio::Player>,
}

impl Output {
    /// Starts playing `path`, stopping anything already playing.
    /// Returns how long the sound lasts.
    fn play(&mut self, path: &Path) -> Result<Duration> {
        if self.failed.swap(false, Ordering::Relaxed) {
            logging::info("Geluid: het geluidsapparaat opnieuw openen na een fout");
            self.sink = None;
        }
        self.stop();
        let file = std::fs::File::open(path).with_context(|| format!("opening {}", path.display()))?;
        let source = rodio::Decoder::try_from(file).context("Het geluid kon niet worden gelezen")?;
        let length = source.total_duration().unwrap_or(Duration::from_secs(3));
        let sink = match self.sink.take() {
            Some(sink) => sink,
            None => self.open_sink()?,
        };
        let player = rodio::Player::connect_new(sink.mixer());
        player.append(source);
        (self.sink, self.player) = (Some(sink), Some(player));
        Ok(length)
    }

    /// The default output device, reporting its errors to the log and to `failed`.
    /// Falls back to rodio's own search when the default device won't open.
    fn open_sink(&self) -> Result<rodio::MixerDeviceSink> {
        let failed = self.failed.clone();
        let mut sink = rodio::DeviceSinkBuilder::from_default_device()
            .map(|builder| {
                builder.with_error_callback(move |err| {
                    logging::error("geluid", format!("Fout van het geluidsapparaat: {err}"));
                    failed.store(true, Ordering::Relaxed);
                })
            })
            .and_then(|builder| builder.open_stream())
            .or_else(|err| {
                logging::error("geluid", format!("Standaard geluidsapparaat ging niet open ({err}); een ander proberen"));
                rodio::DeviceSinkBuilder::open_default_sink()
            })
            .context("Geen geluidsapparaat gevonden")?;
        sink.log_on_drop(false);
        Ok(sink)
    }

    fn stop(&mut self) {
        if let Some(player) = self.player.take() {
            player.stop();
        }
    }
}
