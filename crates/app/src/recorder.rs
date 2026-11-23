//! Records the learner reading a card aloud, or testing the microphone.
//!
//! Recording runs on its own thread, straight on cpal: the device's callback
//! sends each burst of sound over a channel, so nothing is dropped however
//! large the bursts are (Bluetooth and PipeWire deliver big ones). It stops by
//! itself after a short silence once speech has started, after 15 seconds, or
//! when asked to, and gives up when the microphone stops delivering sound.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, anyhow, bail};
use rodio::Source as _;
use rodio::cpal::traits::{DeviceTrait as _, HostTrait as _, StreamTrait as _};
use rodio::cpal::{self, FromSample, SampleFormat, SizedSample};

use crate::logging;

/// Speech is judged in blocks of this length.
const BLOCK: Duration = Duration::from_millis(30);
const MAX_LENGTH: Duration = Duration::from_secs(15);
const SILENCE_AFTER_SPEECH: Duration = Duration::from_millis(1300);
/// How long to wait for the learner to start before giving up.
const WAIT_FOR_SPEECH: Duration = Duration::from_secs(8);
/// The background level is learned from the start of the recording.
const LEARN_NOISE: Duration = Duration::from_millis(250);
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

/// Asks the recording thread to stop once dropped, such as when a recording is cancelled.
struct StopOnDrop(Arc<Shared>);

impl Drop for StopOnDrop {
    fn drop(&mut self) {
        self.0.stop.store(true, Ordering::Relaxed);
    }
}

pub struct Recording {
    shared: StopOnDrop,
    thread: JoinHandle<Result<Captured>>,
}

impl Recording {
    /// Starts recording; every sample is multiplied by `boost`.
    pub fn start(boost: f32) -> Result<Recording> {
        let shared = Arc::new(Shared::default());
        let thread = {
            let shared = shared.clone();
            std::thread::Builder::new().name("opname".into()).spawn(move || record(boost, &shared))?
        };
        Ok(Recording { shared: StopOnDrop(shared), thread })
    }

    /// Input loudness, 0.0–1.0, for the level meter.
    pub fn level(&self) -> f32 {
        f32::from_bits(self.shared.0.level.load(Ordering::Relaxed))
    }

    pub fn heard_speech(&self) -> bool {
        self.shared.0.speaking.load(Ordering::Relaxed)
    }

    pub fn is_finished(&self) -> bool {
        self.thread.is_finished()
    }

    pub fn request_stop(&self) {
        self.shared.0.stop.store(true, Ordering::Relaxed);
    }

    /// Waits for the recording thread and returns the audio.
    pub fn finish(self) -> Result<Captured> {
        self.request_stop();
        self.thread.join().map_err(|_| anyhow!("De opname is mislukt."))?
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
    let stream = open_stream(&device, &config, sender, &failed).context("De microfoon kon niet worden geopend")?;
    stream.play().context("De microfoon kon niet worden gestart")?;
    logging::info(format!(
        "Opname gestart: {}, {rate} Hz, {channels} kanaal/kanalen, {:?}, versterking {boost}x",
        name.as_deref().unwrap_or("onbekend apparaat"),
        config.sample_format(),
    ));

    let started = Instant::now();
    let feed = Feed { sound, failed: &failed, boost, stall_limit: STALL_LIMIT };
    let result = listen(feed, rate as usize * usize::from(channels), shared);
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

/// Opens the input stream in the device's own sample format.
fn open_stream(
    device: &cpal::Device,
    config: &cpal::SupportedStreamConfig,
    sender: mpsc::Sender<Vec<f32>>,
    failed: &Arc<AtomicBool>,
) -> Result<cpal::Stream> {
    match config.sample_format() {
        SampleFormat::F32 => open::<f32>(device, config.config(), sender, failed),
        SampleFormat::I16 => open::<i16>(device, config.config(), sender, failed),
        SampleFormat::I32 => open::<i32>(device, config.config(), sender, failed),
        SampleFormat::U16 => open::<u16>(device, config.config(), sender, failed),
        other => bail!("De microfoon levert een onbekend geluidsformaat ({other})."),
    }
}

/// Each burst of sound goes over `sender` as f32 samples.
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

/// Where the sound comes from: the device's bursts, and whether it failed.
struct Feed<'a> {
    sound: Receiver<Vec<f32>>,
    failed: &'a AtomicBool,
    boost: f32,
    stall_limit: Duration,
}

impl Feed<'_> {
    /// Waits briefly for the next burst of sound and adds it, amplified, to
    /// `samples`. Returns whether one came; fails once the microphone has
    /// delivered nothing for too long, or reported an error.
    fn receive(&self, samples: &mut Vec<f32>, last_sound: Instant) -> Result<bool> {
        let burst = self.sound.recv_timeout(Duration::from_millis(100));
        if self.failed.load(Ordering::Relaxed) || burst.is_err() && last_sound.elapsed() > self.stall_limit {
            bail!(STALLED);
        }
        let Ok(burst) = burst else { return Ok(false) };
        samples.extend(burst.into_iter().map(|sample| amplify(sample, self.boost)));
        Ok(true)
    }
}

/// Decides, a block at a time, whether the learner has spoken and finished.
#[derive(Default)]
struct SpeechGate {
    noise_floor: Option<f32>,
    elapsed: Duration,
    quiet_for: Duration,
    had_speech: bool,
}

impl SpeechGate {
    /// Judges one block of `loudness` (RMS). Returns whether the recording is over.
    fn hear(&mut self, loudness: f32) -> bool {
        self.elapsed += BLOCK;
        let floor = *self.noise_floor.get_or_insert(loudness);
        if self.elapsed < LEARN_NOISE {
            self.noise_floor = Some(floor.max(loudness));
            return false;
        }
        if loudness > (floor * 3.0).max(0.012) {
            self.had_speech = true;
            self.quiet_for = Duration::ZERO;
        } else {
            self.quiet_for += BLOCK;
        }
        let waited = if self.had_speech { self.quiet_for >= SILENCE_AFTER_SPEECH } else { self.elapsed >= WAIT_FOR_SPEECH };
        waited || self.elapsed >= MAX_LENGTH
    }
}

/// Reads sound until the learner is done. `per_second` counts samples across
/// all channels. Returns the samples and whether speech was heard.
fn listen(feed: Feed, per_second: usize, shared: &Shared) -> Result<(Vec<f32>, bool)> {
    let block = (per_second * BLOCK.as_millis() as usize / 1000).max(1);
    let mut samples: Vec<f32> = Vec::with_capacity(per_second * 6);
    let mut gate = SpeechGate::default();
    let (mut judged, mut last_sound) = (0, Instant::now());
    while !shared.stop.load(Ordering::Relaxed) {
        if samples.len() - judged < block {
            if feed.receive(&mut samples, last_sound)? {
                last_sound = Instant::now();
            }
            continue;
        }
        let loudness = rms(&samples[judged..judged + block]);
        judged += block;
        shared.level.store((loudness * 6.0).min(1.0).to_bits(), Ordering::Relaxed);
        let done = gate.hear(loudness);
        shared.speaking.store(gate.had_speech, Ordering::Relaxed);
        if done {
            break;
        }
    }
    samples.truncate(judged);
    Ok((samples, gate.had_speech))
}

fn rms(block: &[f32]) -> f32 {
    (block.iter().map(|s| s * s).sum::<f32>() / block.len().max(1) as f32).sqrt()
}

/// Reads a WAV file as if it had been recorded, for checks without a microphone.
pub fn from_file(path: &std::path::Path, boost: f32) -> Result<Captured> {
    let file = std::fs::File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let decoder = rodio::Decoder::try_from(file)?;
    let (rate, channels) = (decoder.sample_rate().get(), decoder.channels().get());
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

impl Loudness {
    /// Judges a recording by its loudest block and by how many samples hit the ceiling.
    pub fn of(captured: &Captured) -> Loudness {
        if !captured.had_speech || captured.samples.is_empty() {
            return Loudness::Silent;
        }
        let block = (captured.rate as usize * usize::from(captured.channels.max(1)) * 30 / 1000).max(1);
        let loudest = captured.samples.chunks(block).map(rms).fold(0.0_f32, f32::max);
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

    /// What the microphone test says about it.
    pub fn verdict(self) -> &'static str {
        match self {
            Loudness::Good => "Je microfoon werkt.",
            Loudness::Quiet => "Je klinkt zacht. Zet de versterking hoger of spreek dichter bij de microfoon.",
            Loudness::Clipped => "Je klinkt te hard en vervormd. Zet de versterking lager.",
            Loudness::Silent => "Ik hoorde niets. Kijk hierboven of het de juiste microfoon is en of hij niet gedempt is.",
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicBool;
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    use super::{Captured, Feed, Loudness, Shared, listen};

    /// `seconds` of a 440 Hz tone at 16 kHz mono; amplitude 0 is silence.
    fn tone(amplitude: f32, seconds: f32) -> Captured {
        let samples = (0..(16_000.0 * seconds) as usize)
            .map(|i| amplitude * (i as f32 * 440.0 * std::f32::consts::TAU / 16_000.0).sin())
            .collect();
        Captured { samples, rate: 16_000, channels: 1, had_speech: true, device: None }
    }

    /// Listens to `sound` delivered in large bursts by a device that stays open.
    fn listen_to(sound: &[f32], boost: f32, stall_limit: Duration) -> anyhow::Result<(Vec<f32>, bool)> {
        let (sender, receiver) = mpsc::channel();
        sound.chunks(8_000).for_each(|burst| sender.send(burst.to_vec()).unwrap());
        let failed = AtomicBool::new(false);
        let result = listen(Feed { sound: receiver, failed: &failed, boost, stall_limit }, 16_000, &Shared::default());
        drop(sender);
        result
    }

    /// A second of quiet, a second of speech at `amplitude`, then two of quiet.
    fn utterance(amplitude: f32) -> Vec<f32> {
        [0.0, amplitude, 0.0, 0.0].iter().flat_map(|&amplitude| tone(amplitude, 1.0).samples).collect()
    }

    fn peak(samples: &[f32]) -> f32 {
        samples.iter().fold(0.0, |peak, s| peak.max(s.abs()))
    }

    #[test]
    fn large_bursts_are_kept_whole_boosted_and_cut_off_at_full_scale() {
        let all = utterance(0.3);
        let (samples, had_speech) = listen_to(&all, 1.0, Duration::from_secs(5)).unwrap();
        assert!(had_speech && samples.len() > 32_000, "listened until the silence after the speech");
        assert_eq!(samples, all[..samples.len()], "nothing dropped or reordered");
        let boosted = [2.5, 4.0].map(|boost| peak(&listen_to(&all, boost, Duration::from_secs(5)).unwrap().0));
        assert!((0.74..=0.76).contains(&boosted[0]) && boosted[1] == 1.0, "0.3 × 2.5, and cut off at 1: {boosted:?}");
    }

    #[test]
    fn a_microphone_that_stops_delivering_ends_the_recording() {
        let started = Instant::now();
        let result = listen_to(&[0.0; 1_600], 1.0, Duration::from_millis(300));
        assert!(result.unwrap_err().to_string().contains("geen geluid meer"));
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn loudness_tells_faint_good_distorted_and_silent_apart() {
        let mut silent = tone(0.3, 1.0);
        silent.had_speech = false;
        let judged = [tone(0.03, 1.0), tone(0.3, 1.0), tone(1.0, 1.0), silent].map(|captured| Loudness::of(&captured));
        assert_eq!(judged, [Loudness::Quiet, Loudness::Good, Loudness::Clipped, Loudness::Silent]);
        assert!(judged.iter().all(|loudness| !loudness.verdict().is_empty()));
    }
}
