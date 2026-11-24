//! Sound out: the Dutch voice and recordings, through the default sound device.
//!
//! The device is open only while something plays. A Bluetooth headset
//! switches to headset mode for each recording and back afterwards; with the
//! app's output stream still running through those switches, the headset's
//! microphone delivered sound for the first recording and then hardly any for
//! every later one.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

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

    /// Shows "speelt af" until the sound of `ticket`, `length` long, has played.
    fn mark_playing(&mut self, ticket: u64, length: Duration, cx: &mut Context<Self>) {
        self.sound.playing = true;
        self.release_when_played(ticket, length, cx);
    }

    /// Closes the sound device once the sound of `ticket`, `length` long, has played.
    fn release_when_played(&mut self, ticket: u64, length: Duration, cx: &mut Context<Self>) {
        let deadline = Instant::now() + length + OVERRUN;
        self.release_after(ticket, length + Duration::from_millis(150), deadline, cx);
    }

    /// After `wait`, closes the sound device if nothing plays on it, and
    /// clears "speelt af" if `ticket` is still the latest sound. While that
    /// sound still plays it looks again, until `deadline`; a newer sound
    /// closes the device itself once it ends.
    fn release_after(&mut self, ticket: u64, wait: Duration, deadline: Instant, cx: &mut Context<Self>) {
        self.after(wait, cx, move |this, cx| {
            let latest = this.sound.ticket == ticket;
            if this.sound.output.is_playing() {
                if !latest {
                    return;
                }
                if Instant::now() < deadline {
                    return this.release_after(ticket, Duration::from_millis(250), deadline, cx);
                }
                logging::warn("Geluid: het geluid speelt langer dan verwacht; het geluidsapparaat wordt toch gesloten");
            }
            if latest {
                this.sound.playing = false;
            }
            this.sound.output.close("afgespeeld");
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

    /// Stops any sound and closes the sound device; `why` goes in the log.
    pub fn stop_audio(&mut self, why: &str) {
        self.sound.ticket += 1;
        self.sound.playing = false;
        self.sound.output.close(why);
    }

    /// Plays a recording, saying `what` it is.
    pub fn play_file(&mut self, path: &Path, what: &'static str, cx: &mut Context<Self>) {
        self.sound.ticket += 1;
        self.sound.playing = false;
        let ticket = self.sound.ticket;
        match self.sound.output.play(path) {
            Ok(length) => {
                self.info(what, cx);
                self.release_when_played(ticket, length, cx);
            }
            Err(err) => self.error(format!("{err:#}")),
        }
        cx.notify();
    }
}

/// How long past its expected length a sound may still play before the
/// device is closed anyway, such as when a stalled headset stops taking sound.
const OVERRUN: Duration = Duration::from_secs(5);

/// The sound device, open while something plays.
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
            self.close("fout van het geluidsapparaat");
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
        let name = path.file_name().map_or_else(|| path.display().to_string(), |name| name.to_string_lossy().into_owned());
        logging::info(format!("Geluid: speelt {name} ({:.1} s)", length.as_secs_f32()));
        Ok(length)
    }

    /// Whether a sound is still playing.
    fn is_playing(&self) -> bool {
        self.player.as_ref().is_some_and(|player| !player.empty())
    }

    /// Stops any sound and closes the device, logging `why` if it was open.
    fn close(&mut self, why: &str) {
        self.stop();
        let Some(sink) = self.sink.take() else { return };
        let started = Instant::now();
        drop(sink);
        logging::info(format!("Geluid: geluidsapparaat gesloten ({why}, {} ms)", started.elapsed().as_millis()));
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
        logging::info(format!("Geluid: geluidsapparaat geopend ({:?})", sink.config()));
        Ok(sink)
    }

    fn stop(&mut self) {
        if let Some(player) = self.player.take() {
            player.stop();
        }
    }
}
