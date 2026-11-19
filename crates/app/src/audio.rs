//! Plays the cached WAV files through the default sound device.

use std::path::Path;
use std::time::Duration;

use anyhow::{Context as _, Result};
use rodio::Source as _;

#[derive(Default)]
pub struct Audio {
    sink: Option<rodio::MixerDeviceSink>,
    player: Option<rodio::Player>,
}

impl Audio {
    /// Starts playing `path`, stopping anything already playing.
    /// Returns how long the sound lasts.
    pub fn play(&mut self, path: &Path) -> Result<Duration> {
        if self.sink.is_none() {
            let mut sink = rodio::DeviceSinkBuilder::open_default_sink().context("Geen geluidsapparaat gevonden")?;
            sink.log_on_drop(false);
            self.sink = Some(sink);
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

    pub fn stop(&mut self) {
        if let Some(player) = self.player.take() {
            player.stop();
        }
    }
}
