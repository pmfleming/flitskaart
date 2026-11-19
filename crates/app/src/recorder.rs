//! Records the learner reading a card aloud.
//!
//! Recording runs on its own thread. It stops by itself after a short silence
//! once speech has started, after 15 seconds, or when asked to.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::thread::JoinHandle;
use std::time::Duration;

use anyhow::{Context as _, Result};
use rodio::Source as _;

const MAX_LENGTH: Duration = Duration::from_secs(15);
const SILENCE_AFTER_SPEECH: Duration = Duration::from_millis(1300);
/// How long to wait for the learner to start before giving up.
const WAIT_FOR_SPEECH: Duration = Duration::from_secs(8);

pub struct Captured {
    pub samples: Vec<f32>,
    pub rate: u32,
    pub channels: u16,
    /// Whether anything louder than the background was heard.
    pub had_speech: bool,
}

pub struct Recording {
    stop: Arc<AtomicBool>,
    level: Arc<AtomicU32>,
    speaking: Arc<AtomicBool>,
    thread: Option<JoinHandle<Result<Captured>>>,
}

impl Recording {
    pub fn start() -> Recording {
        let stop = Arc::new(AtomicBool::new(false));
        let level = Arc::new(AtomicU32::new(0));
        let speaking = Arc::new(AtomicBool::new(false));
        let thread = {
            let (stop, level, speaking) = (stop.clone(), level.clone(), speaking.clone());
            std::thread::spawn(move || record(&stop, &level, &speaking))
        };
        Recording { stop, level, speaking, thread: Some(thread) }
    }

    /// Input loudness, 0.0–1.0, for the level meter.
    pub fn level(&self) -> f32 {
        f32::from_bits(self.level.load(Ordering::Relaxed))
    }

    pub fn heard_speech(&self) -> bool {
        self.speaking.load(Ordering::Relaxed)
    }

    pub fn is_finished(&self) -> bool {
        self.thread.as_ref().is_none_or(JoinHandle::is_finished)
    }

    pub fn request_stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }

    /// Waits for the recording thread and returns the audio.
    pub fn finish(mut self) -> Result<Captured> {
        self.request_stop();
        match self.thread.take().map(JoinHandle::join) {
            Some(Ok(result)) => result,
            _ => anyhow::bail!("De opname is mislukt."),
        }
    }
}

impl Drop for Recording {
    fn drop(&mut self) {
        self.request_stop();
    }
}

fn record(stop: &AtomicBool, level: &AtomicU32, speaking: &AtomicBool) -> Result<Captured> {
    let mut mic = rodio::microphone::MicrophoneBuilder::new()
        .default_device()
        .context("Geen microfoon gevonden")?
        .default_config()
        .context("De microfoon kon niet worden ingesteld")?
        .open_stream()
        .context("De microfoon kon niet worden geopend")?;
    let rate = mic.sample_rate().get();
    let channels = mic.channels().get();
    // Measure loudness in 30 ms blocks.
    let block = (rate as usize * usize::from(channels) * 30 / 1000).max(1);
    let block_time = Duration::from_millis(30);
    let mut samples = Vec::with_capacity(rate as usize * usize::from(channels) * 6);
    let mut noise_floor: Option<f32> = None;
    let (mut elapsed, mut quiet_for) = (Duration::ZERO, Duration::ZERO);
    let mut had_speech = false;
    let mut chunk = Vec::with_capacity(block);

    while !stop.load(Ordering::Relaxed) && elapsed < MAX_LENGTH {
        chunk.clear();
        chunk.extend(mic.by_ref().take(block));
        if chunk.len() < block {
            break; // The device stopped.
        }
        samples.extend_from_slice(&chunk);
        elapsed += block_time;

        let rms = (chunk.iter().map(|s| s * s).sum::<f32>() / chunk.len() as f32).sqrt();
        level.store((rms * 6.0).min(1.0).to_bits(), Ordering::Relaxed);
        // Learn the background level from the first quarter second.
        let floor = *noise_floor.get_or_insert(rms);
        if elapsed < Duration::from_millis(250) {
            noise_floor = Some(floor.max(rms));
            continue;
        }
        let threshold = (floor * 3.0).max(0.012);
        if rms > threshold {
            had_speech = true;
            speaking.store(true, Ordering::Relaxed);
            quiet_for = Duration::ZERO;
        } else {
            quiet_for += block_time;
        }
        if had_speech && quiet_for >= SILENCE_AFTER_SPEECH {
            break;
        }
        if !had_speech && elapsed >= WAIT_FOR_SPEECH {
            break;
        }
    }
    level.store(0, Ordering::Relaxed);
    Ok(Captured { samples, rate, channels, had_speech })
}

/// Reads a WAV file as if it had been recorded, for checks without a microphone.
pub fn from_file(path: &std::path::Path) -> Result<Captured> {
    let file = std::fs::File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let decoder = rodio::Decoder::try_from(file)?;
    let rate = decoder.sample_rate().get();
    let channels = decoder.channels().get();
    Ok(Captured { samples: decoder.collect(), rate, channels, had_speech: true })
}
