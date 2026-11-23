//! Records the learner reading a card aloud, or testing the microphone.
//!
//! Recording runs on its own thread, straight on cpal: the device's callback
//! sends each burst of sound over a channel, so nothing is dropped however
//! large the bursts are (Bluetooth and PipeWire deliver big ones). It stops by
//! itself after a short silence once speech has started, after 15 seconds, or
//! when asked to, and gives up when the microphone stops delivering sound.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, bail};
use rodio::Source as _;
use rodio::cpal::traits::{DeviceTrait as _, HostTrait as _, StreamTrait as _};
use rodio::cpal::{self, FromSample, SampleFormat, SizedSample};

use crate::logging;

const MAX_LENGTH: Duration = Duration::from_secs(15);
const SILENCE_AFTER_SPEECH: Duration = Duration::from_millis(1300);
/// How long to wait for the learner to start before giving up.
const WAIT_FOR_SPEECH: Duration = Duration::from_secs(8);
/// How long the microphone may go without delivering sound before the
/// recording is given up. Generous, because a Bluetooth headset takes a few
/// seconds to switch to its microphone.
const STALL_LIMIT: Duration = Duration::from_secs(5);

/// Shown when the microphone stops delivering sound in the middle of a recording.
const STALLED: &str =
    "De microfoon geeft geen geluid meer. Gebruik je een Bluetooth-headset, zet die dan uit en weer aan, of kies een andere microfoon.";

pub struct Captured {
    pub samples: Vec<f32>,
    pub rate: u32,
    pub channels: u16,
    /// Whether anything louder than the background was heard.
    pub had_speech: bool,
    /// The input device's name, when the system reports one.
    pub device: Option<String>,
}

/// What the recording thread shares with the window.
#[derive(Default)]
struct Shared {
    stop: AtomicBool,
    /// Input loudness as f32 bits, for the level meter.
    level: AtomicU32,
    speaking: AtomicBool,
}

pub struct Recording {
    shared: Arc<Shared>,
    thread: Option<JoinHandle<Result<Captured>>>,
}

impl Recording {
    /// Starts recording; every sample is multiplied by `boost`.
    pub fn start(boost: f32) -> Recording {
        let shared = Arc::new(Shared::default());
        let thread = {
            let shared = shared.clone();
            std::thread::Builder::new()
                .name("opname".into())
                .spawn(move || record(boost, &shared))
                .expect("could not start the recording thread")
        };
        Recording { shared, thread: Some(thread) }
    }

    /// Input loudness, 0.0–1.0, for the level meter.
    pub fn level(&self) -> f32 {
        f32::from_bits(self.shared.level.load(Ordering::Relaxed))
    }

    pub fn heard_speech(&self) -> bool {
        self.shared.speaking.load(Ordering::Relaxed)
    }

    pub fn is_finished(&self) -> bool {
        self.thread.as_ref().is_none_or(JoinHandle::is_finished)
    }

    pub fn request_stop(&self) {
        self.shared.stop.store(true, Ordering::Relaxed);
    }

    /// Waits for the recording thread and returns the audio.
    pub fn finish(mut self) -> Result<Captured> {
        self.request_stop();
        match self.thread.take().map(JoinHandle::join) {
            Some(Ok(result)) => result,
            _ => bail!("De opname is mislukt."),
        }
    }
}

impl Drop for Recording {
    fn drop(&mut self) {
        self.request_stop();
    }
}

fn record(boost: f32, shared: &Shared) -> Result<Captured> {
    logging::info("Opname: microfoon openen");
    let device = cpal::default_host().default_input_device().context("Geen microfoon gevonden")?;
    let config = device.default_input_config().context("De microfoon kon niet worden ingesteld")?;
    let name = device.description().ok().map(|description| description.name().to_string());
    let (rate, channels) = (config.sample_rate(), config.channels());
    let (sender, sound) = mpsc::channel();
    let failed = Arc::new(AtomicBool::new(false));
    let stream = match config.sample_format() {
        SampleFormat::F32 => open::<f32>(&device, config.config(), sender, &failed),
        SampleFormat::I16 => open::<i16>(&device, config.config(), sender, &failed),
        SampleFormat::I32 => open::<i32>(&device, config.config(), sender, &failed),
        SampleFormat::U16 => open::<u16>(&device, config.config(), sender, &failed),
        SampleFormat::U8 => open::<u8>(&device, config.config(), sender, &failed),
        SampleFormat::I8 => open::<i8>(&device, config.config(), sender, &failed),
        other => bail!("De microfoon levert een onbekend geluidsformaat ({other})."),
    }
    .context("De microfoon kon niet worden geopend")?;
    stream.play().context("De microfoon kon niet worden gestart")?;
    logging::info(format!(
        "Opname gestart: {}, {rate} Hz, {channels} kanaal/kanalen, {:?}, versterking {boost}x",
        name.as_deref().unwrap_or("onbekend apparaat"),
        config.sample_format(),
    ));

    let started = Instant::now();
    let result = listen(&sound, rate, channels, boost, shared, &failed, STALL_LIMIT);
    drop(stream);
    shared.level.store(0, Ordering::Relaxed);
    let (samples, had_speech) = result?;
    logging::info(format!(
        "Opname klaar: {:.1} s geluid in {:.1} s, spraak gehoord: {}",
        samples.len() as f32 / (rate as f32 * f32::from(channels.max(1))),
        started.elapsed().as_secs_f32(),
        if had_speech { "ja" } else { "nee" }
    ));
    Ok(Captured { samples, rate, channels, had_speech, device: name })
}

/// Opens the input stream; each burst of sound goes over `sender` as f32 samples.
fn open<T>(device: &cpal::Device, config: cpal::StreamConfig, sender: mpsc::Sender<Vec<f32>>, failed: &Arc<AtomicBool>) -> Result<cpal::Stream>
where
    T: SizedSample,
    f32: FromSample<T>,
{
    let failed = failed.clone();
    let stream = device.build_input_stream::<T, _, _>(
        config,
        move |data: &[T], _| {
            let _ = sender.send(data.iter().map(|&sample| f32::from_sample_(sample)).collect());
        },
        move |err| {
            logging::error("microfoon", format!("Fout van de microfoon: {err}"));
            failed.store(true, Ordering::Relaxed);
        },
        None,
    )?;
    Ok(stream)
}

/// Reads sound until the learner is done, and judges speech in 30 ms blocks.
/// Returns the samples and whether speech was heard.
fn listen(
    sound: &Receiver<Vec<f32>>,
    rate: u32,
    channels: u16,
    boost: f32,
    shared: &Shared,
    failed: &AtomicBool,
    stall_limit: Duration,
) -> Result<(Vec<f32>, bool)> {
    let block = (rate as usize * usize::from(channels) * 30 / 1000).max(1);
    let block_time = Duration::from_millis(30);
    let mut samples: Vec<f32> = Vec::with_capacity(rate as usize * usize::from(channels) * 6);
    let mut judged = 0; // How many samples have been judged.
    let mut noise_floor: Option<f32> = None;
    let (mut elapsed, mut quiet_for) = (Duration::ZERO, Duration::ZERO);
    let mut had_speech = false;
    let mut last_sound = Instant::now();

    while !shared.stop.load(Ordering::Relaxed) && elapsed < MAX_LENGTH {
        if samples.len() - judged < block {
            match sound.recv_timeout(Duration::from_millis(100)) {
                Ok(burst) => {
                    if samples.is_empty() && !burst.is_empty() {
                        logging::info(format!("Opname: eerste geluid na {:.1} s", last_sound.elapsed().as_secs_f32()));
                    }
                    samples.extend(burst.into_iter().map(|sample| amplify(sample, boost)));
                    last_sound = Instant::now();
                }
                Err(RecvTimeoutError::Timeout) if last_sound.elapsed() > stall_limit => bail!(STALLED),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => bail!(STALLED),
            }
            if failed.load(Ordering::Relaxed) {
                bail!(STALLED);
            }
            continue;
        }
        let chunk = &samples[judged..judged + block];
        judged += block;
        elapsed += block_time;

        let rms = (chunk.iter().map(|s| s * s).sum::<f32>() / chunk.len() as f32).sqrt();
        shared.level.store((rms * 6.0).min(1.0).to_bits(), Ordering::Relaxed);
        // Learn the background level from the first quarter second.
        let floor = *noise_floor.get_or_insert(rms);
        if elapsed < Duration::from_millis(250) {
            noise_floor = Some(floor.max(rms));
            continue;
        }
        let threshold = (floor * 3.0).max(0.012);
        if rms > threshold {
            had_speech = true;
            shared.speaking.store(true, Ordering::Relaxed);
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
    samples.truncate(judged);
    Ok((samples, had_speech))
}

/// Reads a WAV file as if it had been recorded, for checks without a microphone.
pub fn from_file(path: &std::path::Path, boost: f32) -> Result<Captured> {
    let file = std::fs::File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let decoder = rodio::Decoder::try_from(file)?;
    let rate = decoder.sample_rate().get();
    let channels = decoder.channels().get();
    let device = path.file_name().map(|name| format!("{} (testbestand)", name.to_string_lossy()));
    Ok(Captured { samples: decoder.map(|s| amplify(s, boost)).collect(), rate, channels, had_speech: true, device })
}

/// Applies the input boost; a sample pushed past full scale is cut off there.
fn amplify(sample: f32, boost: f32) -> f32 {
    (sample * boost).clamp(-1.0, 1.0)
}

/// How loud a recording was, for the microphone test.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Loudness {
    /// Nothing above the background noise.
    Silent,
    /// Speech was heard, but faintly.
    Quiet,
    Good,
    /// The signal hit the ceiling, so it is distorted.
    Clipped,
}

/// Judges a recording by its loudest 30 ms and by how many samples hit the ceiling.
pub fn loudness(captured: &Captured) -> Loudness {
    if !captured.had_speech || captured.samples.is_empty() {
        return Loudness::Silent;
    }
    let block = (captured.rate as usize * usize::from(captured.channels.max(1)) * 30 / 1000).max(1);
    let loudest = captured
        .samples
        .chunks(block)
        .map(|chunk| (chunk.iter().map(|s| s * s).sum::<f32>() / chunk.len() as f32).sqrt())
        .fold(0.0_f32, f32::max);
    // A few touches of the ceiling are harmless; a run of them is audible.
    let clipped = captured.samples.iter().filter(|s| s.abs() >= 0.99).count();
    if clipped > captured.rate as usize / 200 {
        Loudness::Clipped
    } else if loudest < 0.04 {
        Loudness::Quiet
    } else {
        Loudness::Good
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(amplitude: f32, seconds: f32) -> Captured {
        let rate = 16_000;
        let samples = (0..(rate as f32 * seconds) as usize)
            .map(|i| amplitude * (i as f32 * 440.0 * std::f32::consts::TAU / rate as f32).sin())
            .collect();
        Captured { samples, rate, channels: 1, had_speech: true, device: None }
    }

    #[test]
    fn the_boost_scales_samples_and_cuts_them_off_at_full_scale() {
        assert_eq!(amplify(0.2, 0.5), 0.1);
        assert_eq!(amplify(0.2, 2.0), 0.4);
        assert_eq!(amplify(0.6, 2.5), 1.0);
        assert_eq!(amplify(-0.6, 2.5), -1.0);
    }

    /// One second of a 440 Hz tone at 16 kHz mono, or silence.
    fn second(amplitude: f32) -> Vec<f32> {
        tone(amplitude, 1.0).samples
    }

    #[test]
    fn large_bursts_of_sound_are_kept_whole() {
        let (sender, sound) = mpsc::channel();
        // Quiet, then speech, then silence, in bursts far larger than one block.
        let mut all = second(0.0);
        all.extend(second(0.3));
        all.extend(second(0.0));
        all.extend(second(0.0));
        for burst in all.chunks(8_000) {
            sender.send(burst.to_vec()).unwrap();
        }
        let shared = Shared::default();
        let (samples, had_speech) =
            listen(&sound, 16_000, 1, 1.0, &shared, &AtomicBool::new(false), Duration::from_secs(5)).unwrap();
        assert!(had_speech);
        assert_eq!(samples, all[..samples.len()], "nothing dropped or reordered");
        assert!(samples.len() > 32_000, "listened until the silence after the speech");
    }

    #[test]
    fn a_microphone_that_stops_delivering_ends_the_recording() {
        let (sender, sound) = mpsc::channel();
        sender.send(vec![0.0; 1_600]).unwrap();
        let shared = Shared::default();
        let started = Instant::now();
        let result = listen(&sound, 16_000, 1, 1.0, &shared, &AtomicBool::new(false), Duration::from_millis(300));
        assert!(result.unwrap_err().to_string().contains("geen geluid meer"));
        assert!(started.elapsed() < Duration::from_secs(2));
        drop(sender);
    }

    #[test]
    fn loudness_tells_faint_good_and_distorted_apart() {
        assert_eq!(loudness(&tone(0.03, 1.0)), Loudness::Quiet);
        assert_eq!(loudness(&tone(0.3, 1.0)), Loudness::Good);
        assert_eq!(loudness(&tone(1.0, 1.0)), Loudness::Clipped);
        let mut silent = tone(0.3, 1.0);
        silent.had_speech = false;
        assert_eq!(loudness(&silent), Loudness::Silent);
    }
}
