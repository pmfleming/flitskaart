//! Plays the cached WAV files through the default sound device.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use anyhow::{Context as _, Result};
use rodio::Source as _;

use crate::logging;

#[derive(Default)]
pub struct Audio {
    sink: Option<rodio::MixerDeviceSink>,
    /// Set when the sound device reports an error, such as a Bluetooth headset
    /// dropping out; the next sound then opens the device afresh.
    failed: Arc<AtomicBool>,
    player: Option<rodio::Player>,
}

impl Audio {
    /// Starts playing `path`, stopping anything already playing.
    /// Returns how long the sound lasts.
    pub fn play(&mut self, path: &Path) -> Result<Duration> {
        if self.failed.swap(false, Ordering::Relaxed) {
            logging::info("Geluid: het geluidsapparaat opnieuw openen na een fout");
            self.player = None;
            self.sink = None;
        }
        if self.sink.is_none() {
            self.sink = Some(self.open_sink()?);
        }
        self.stop();
        let file = std::fs::File::open(path).with_context(|| format!("opening {}", path.display()))?;
        let source = rodio::Decoder::try_from(file).context("Het geluid kon niet worden gelezen")?;
        let length = source.total_duration().unwrap_or(Duration::from_secs(3));
        let player = rodio::Player::connect_new(self.sink.as_ref().unwrap().mixer());
        player.append(source);
        self.player = Some(player);
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

    pub fn stop(&mut self) {
        if let Some(player) = self.player.take() {
            player.stop();
        }
    }
}
