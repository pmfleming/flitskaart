//! Records the learner reading a card aloud, or testing the microphone.
//!
//! The default input uses CPAL. An explicit PipeWire source uses pw-record so
//! capture can target that source without changing the system's routes.

use std::io::ErrorKind;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, anyhow, bail};
use dutch_flashcards_core::inputs::Inputs;
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
/// Wait for the Bluetooth profile switch, then allow speech even with a noise-gated mic.
const WARMUP_LIMIT: Duration = Duration::from_millis(2500);
const LEARN_NOISE: Duration = Duration::from_millis(250);
const MIN_SPEECH_RMS: f32 = 0.001;
/// How long the microphone may go without delivering sound before the
/// recording is given up. Generous, because a Bluetooth headset takes a few
/// seconds to switch to its microphone.
const STALL_LIMIT: Duration = Duration::from_secs(5);

/// A recording whose last sound arrived longer than this before it ended is
/// logged as a microphone that stopped delivering. Sound normally arrives
/// every few tens of milliseconds, a Bluetooth headset's every few hundred.
const STOPPED_EARLY: Duration = Duration::from_secs(1);

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
    /// Samples captured before the prompt, retained for playback and transcription.
    warmup_samples: usize,
}

impl Captured {
    fn has_signal(&self) -> bool {
        self.samples.iter().any(|&sample| sample != 0.0)
    }
}

/// What the recording thread shares with the window.
#[derive(Default)]
struct Shared {
    stop: AtomicBool,
    /// Input loudness as f32 bits, for the level meter.
    level: AtomicU32,
    ready: AtomicBool,
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
    pub fn start(boost: f32, input: Option<String>) -> Result<Recording> {
        let shared = Arc::new(Shared::default());
        let thread = {
            let shared = shared.clone();
            std::thread::Builder::new().name("opname".into()).spawn(move || match input {
                Some(node) => record_pipewire(&node, boost, &shared),
                None => record_default(boost, &shared),
            })?
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

    pub fn is_ready(&self) -> bool {
        self.shared.0.ready.load(Ordering::Relaxed)
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

fn record_default(boost: f32, shared: &Shared) -> Result<Captured> {
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
    log_default_input();

    let captured = collect(Feed::new(sound, &failed, boost, STALL_LIMIT), rate, channels, name, shared);
    drop(stream);
    captured
}

fn record_pipewire(node: &str, boost: f32, shared: &Shared) -> Result<Captured> {
    const RATE: u32 = 48_000;
    logging::info(format!("Opname: PipeWire-microfoon {node}, {RATE} Hz, mono, versterking {boost}x"));
    let mut command = Command::new("pw-record");
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::process::CommandExt as _;
        let app_pid = std::process::id() as libc::pid_t;
        // The worker normally reaps the child. Stop it even if the app exits
        // before the worker gets to clean up a stalled capture.
        unsafe {
            command.pre_exec(move || {
                if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL) == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                if libc::getppid() != app_pid {
                    return Err(std::io::Error::from_raw_os_error(libc::ESRCH));
                }
                Ok(())
            });
        }
    }
    let child = command
        .arg(format!("--target={node}"))
        .arg(format!("--rate={RATE}"))
        .args([
            "--raw", "--channels=1", "--channel-map=mono", "--format=f32", "--latency=20ms",
            r#"--properties={"node.dont-fallback":true,"node.dont-reconnect":true,"node.dont-move":true}"#,
            "-",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .spawn()
        .context("De gekozen microfoon kon niet worden geopend via PipeWire (pw-record)")?;
    std::thread::scope(|scope| {
        let mut child = CaptureProcess(child);
        let mut stdout = child.0.stdout.take().context("Geen audiokanaal van pw-record")?;
        let (sender, sound) = mpsc::channel();
        let failed = AtomicBool::new(false);
        scope.spawn(move || read_pipewire(&mut stdout, sender));
        let captured = collect(Feed::new(sound, &failed, boost, STALL_LIMIT), RATE, 1, Some(node.into()), shared);
        drop(child);
        captured.with_context(|| format!("Opname met de gekozen microfoon ({node}) mislukt"))
    })
}

struct CaptureProcess(Child);

impl Drop for CaptureProcess {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn read_pipewire(output: &mut impl std::io::Read, sender: Sender<Vec<f32>>) {
    let mut buffer = [0; 480 * size_of::<f32>()];
    loop {
        if let Err(err) = output.read_exact(&mut buffer) {
            if err.kind() != ErrorKind::UnexpectedEof {
                logging::warn(format!("Opname: kon geen PipeWire-audio lezen: {err}"));
            }
            break;
        }
        let samples = buffer.chunks_exact(4).map(|bytes| f32::from_ne_bytes(bytes.try_into().unwrap())).collect();
        if sender.send(samples).is_err() {
            break;
        }
    }
}

fn collect(feed: Feed<'_>, rate: u32, channels: u16, device: Option<String>, shared: &Shared) -> Result<Captured> {
    let started = Instant::now();
    let result = listen(feed, rate, channels, shared);
    shared.level.store(0, Ordering::Relaxed);
    let mut captured = result?;
    captured.device = device;
    logging::info(format!(
        "Opname klaar: {:.1} s geluid in {:.1} s, spraak gehoord: {}",
        captured.samples.len() as f32 / (rate as f32 * f32::from(channels.max(1))),
        started.elapsed().as_secs_f32(),
        if captured.had_speech { "ja" } else { "nee" }
    ));
    Ok(captured)
}

/// Logs which microphone PipeWire records from, since cpal only knows it as
/// "Default Audio Device". `pw-dump` takes a moment, so it runs on its own thread.
fn log_default_input() {
    let spawned = std::thread::Builder::new().name("microfoon-info".into()).spawn(|| {
        let text = match Inputs::current() {
            Some(inputs) => match inputs.default_input() {
                Some(input) => format!("{} ({})", input.label, input.describe()),
                None => "geen standaardmicrofoon ingesteld".into(),
            },
            None => "onbekend, pw-dump gaf geen antwoord".into(),
        };
        logging::info(format!("Opname: standaardmicrofoon volgens PipeWire: {text}"));
    });
    if let Err(err) = spawned {
        logging::warn(format!("Opname: kon de standaardmicrofoon niet opvragen: {err}"));
    }
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
    delivery: Delivery,
}

impl<'a> Feed<'a> {
    fn new(sound: Receiver<Vec<f32>>, failed: &'a AtomicBool, boost: f32, stall_limit: Duration) -> Self {
        Feed { sound, failed, boost, stall_limit, delivery: Delivery::new() }
    }

    /// Waits briefly for the next burst of sound and adds it, amplified, to
    /// `samples`. Returns whether one came; fails once the microphone has
    /// delivered nothing for too long, or reported an error.
    fn receive(&mut self, samples: &mut Vec<f32>, last_sound: Instant) -> Result<bool> {
        let burst = self.sound.recv_timeout(Duration::from_millis(100));
        if self.failed.load(Ordering::Relaxed)
            || matches!(burst, Err(RecvTimeoutError::Disconnected))
            || burst.is_err() && last_sound.elapsed() > self.stall_limit
        {
            bail!(STALLED);
        }
        let Ok(burst) = burst else { return Ok(false) };
        self.delivery.arrived(&burst);
        samples.extend(burst.into_iter().map(|sample| amplify(sample, self.boost)));
        Ok(true)
    }
}

/// When the microphone's sound arrived, for the log. A microphone that stops
/// delivering partway shows up here even when the learner ends the recording
/// before the stall limit, which the recording itself can't tell from silence.
struct Delivery {
    started: Instant,
    bursts: usize,
    samples: usize,
    first: Option<Duration>,
    last: Option<Duration>,
    longest_gap: Duration,
    first_signal: Option<usize>,
    max_rms: f32,
}

impl Delivery {
    fn new() -> Self {
        Delivery {
            started: Instant::now(), bursts: 0, samples: 0, first: None, last: None,
            longest_gap: Duration::ZERO, first_signal: None, max_rms: 0.0,
        }
    }

    /// Notes a burst of `samples` arriving now.
    fn arrived(&mut self, samples: &[f32]) {
        let at = self.started.elapsed();
        if let Some(last) = self.last {
            self.longest_gap = self.longest_gap.max(at - last);
        }
        self.first.get_or_insert(at);
        self.last = Some(at);
        self.bursts += 1;
        if self.first_signal.is_none() {
            self.first_signal = samples.iter().position(|&sample| sample != 0.0).map(|offset| self.samples + offset);
        }
        self.samples += samples.len();
    }

    /// Logs how the sound arrived, and warns when it stopped well before the end.
    fn report(&self, per_second: usize) {
        let ended = self.started.elapsed();
        let (Some(first), Some(last)) = (self.first, self.last) else {
            return logging::warn(format!("Opname: de microfoon leverde in {:.1} s helemaal geen geluid", ended.as_secs_f32()));
        };
        logging::info(format!(
            "Opname: {} stukken geluid ({} samples), eerste na {:.2} s, laatste na {:.2} s, langste pauze {:.2} s, einde na {:.2} s",
            self.bursts,
            self.samples,
            first.as_secs_f32(),
            last.as_secs_f32(),
            self.longest_gap.as_secs_f32(),
            ended.as_secs_f32(),
        ));
        match self.first_signal {
            Some(sample) => logging::info(format!(
                "Opname: eerste niet-nulsample op {:.3} s in de audio; hoogste RMS na versterking {:.5}",
                sample as f32 / per_second as f32, self.max_rms,
            )),
            None => logging::warn("Opname: de ontvangen samples waren allemaal nul"),
        }
        if ended - last > STOPPED_EARLY {
            logging::warn(format!(
                "Opname: de microfoon leverde na {:.2} s geen geluid meer; de laatste {:.1} s van de opname kwam er niets binnen",
                last.as_secs_f32(),
                (ended - last).as_secs_f32(),
            ));
        }
    }
}

/// Decides, a block at a time, whether the learner has spoken and finished.
#[derive(Default)]
struct SpeechGate {
    noise_floor: Option<f32>,
    warmed_for: Duration,
    calibrated_for: Duration,
    ready: bool,
    elapsed: Duration,
    quiet_for: Duration,
    had_speech: bool,
}

impl SpeechGate {
    /// Judges one block of `loudness` (RMS). Returns whether the recording is over.
    fn hear(&mut self, loudness: f32) -> bool {
        if !self.ready {
            self.warmed_for += BLOCK;
            // Digital silence during a profile switch carries no information about background noise.
            if loudness > 0.0 {
                self.calibrated_for += BLOCK;
                // A startup pop should not set the background level.
                self.noise_floor = Some(self.noise_floor.map_or(loudness, |floor| floor.min(loudness)));
            }
            self.ready = self.warmed_for >= WARMUP_LIMIT || self.calibrated_for >= LEARN_NOISE;
            return false;
        }
        self.elapsed += BLOCK;
        if loudness > (self.noise_floor.unwrap_or_default() * 3.0).max(MIN_SPEECH_RMS) {
            self.had_speech = true;
            self.quiet_for = Duration::ZERO;
        } else {
            self.quiet_for += BLOCK;
        }
        let waited = if self.had_speech { self.quiet_for >= SILENCE_AFTER_SPEECH } else { self.elapsed >= WAIT_FOR_SPEECH };
        waited || self.elapsed >= MAX_LENGTH
    }
}

/// Reads sound until the learner is done and logs how the microphone delivered it.
fn listen(mut feed: Feed<'_>, rate: u32, channels: u16, shared: &Shared) -> Result<Captured> {
    let heard = listen_until_done(&mut feed, rate, channels, shared);
    feed.delivery.report(rate as usize * usize::from(channels.max(1)));
    heard
}

fn listen_until_done(feed: &mut Feed<'_>, rate: u32, channels: u16, shared: &Shared) -> Result<Captured> {
    let channel_count = usize::from(channels.max(1));
    let block = frames_per_block(rate) * channel_count;
    let mut samples: Vec<f32> = Vec::with_capacity(rate as usize * channel_count * 6);
    let mut gate = SpeechGate::default();
    let started = Instant::now();
    let (mut judged, mut last_sound) = (0, started);
    let mut warmup_samples = 0;
    while !shared.stop.load(Ordering::Relaxed) && started.elapsed() < MAX_LENGTH + WARMUP_LIMIT {
        if samples.len() - judged < block {
            if feed.receive(&mut samples, last_sound)? {
                last_sound = Instant::now();
            }
            continue;
        }
        let loudness = rms(&samples[judged..judged + block], channel_count);
        judged += block;
        feed.delivery.max_rms = feed.delivery.max_rms.max(loudness);
        if gate.ready {
            shared.level.store(meter_level(loudness).to_bits(), Ordering::Relaxed);
        } else {
            warmup_samples = judged;
        }
        let done = gate.hear(loudness);
        if gate.ready {
            shared.ready.store(true, Ordering::Relaxed);
        }
        shared.speaking.store(gate.had_speech, Ordering::Relaxed);
        if done {
            break;
        }
    }
    samples.truncate(judged);
    Ok(Captured { samples, rate, channels, had_speech: gate.had_speech, device: None, warmup_samples })
}

/// A 60 dB display range keeps quiet speech visible on the meter.
fn meter_level(loudness: f32) -> f32 {
    (1.0 + loudness.log10() / 3.0).clamp(0.0, 1.0)
}

fn frames_per_block(rate: u32) -> usize {
    (rate as usize * BLOCK.as_millis() as usize / 1000).max(1)
}

/// Measure sound independently of any constant offset in the microphone signal.
fn rms(block: &[f32], channels: usize) -> f32 {
    let frames = (block.len() / channels).max(1) as f64;
    let power = (0..channels)
        .map(|channel| {
            let (sum, squares) = block.iter().skip(channel).step_by(channels).fold((0.0, 0.0), |(sum, squares), &sample| {
                let sample = f64::from(sample);
                (sum + sample, squares + sample * sample)
            });
            (squares / frames - (sum / frames).powi(2)).max(0.0)
        })
        .sum::<f64>();
    (power / channels as f64).sqrt() as f32
}

/// Reads a WAV file as if it had been recorded, for checks without a microphone.
pub fn from_file(path: &std::path::Path, boost: f32) -> Result<Captured> {
    let file = std::fs::File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let decoder = rodio::Decoder::try_from(file)?;
    let (rate, channels) = (decoder.sample_rate().get(), decoder.channels().get());
    let device = path.file_name().map(|name| format!("{} (testbestand)", name.to_string_lossy()));
    Ok(Captured { samples: decoder.map(|s| amplify(s, boost)).collect(), rate, channels, had_speech: true, device, warmup_samples: 0 })
}

/// Applies the input boost; a sample pushed past full scale is cut off there.
fn amplify(sample: f32, boost: f32) -> f32 {
    (sample * boost).clamp(-1.0, 1.0)
}

/// How loud a recording was, for the microphone test.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Loudness {
    NoSignal,
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
        if !captured.has_signal() {
            return Loudness::NoSignal;
        }
        if !captured.had_speech {
            return Loudness::Silent;
        }
        let channels = usize::from(captured.channels.max(1));
        let block = frames_per_block(captured.rate) * channels;
        let active = &captured.samples[captured.warmup_samples..];
        let loudest = active.chunks(block).map(|block| rms(block, channels)).fold(0.0_f32, f32::max);
        // A few touches of the ceiling are harmless; a run of them is audible.
        let clipped = active.iter().filter(|s| s.abs() >= 0.99).count();
        if clipped > captured.rate as usize * channels / 200 {
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
            Loudness::NoSignal => "Er kwam geen hoorbaar signaal binnen. Controleer of de microfoon gedempt is of kies een andere.",
            Loudness::Good => "Je microfoon werkt.",
            Loudness::Quiet => "Je klinkt zacht. Zet de versterking hoger of spreek dichter bij de microfoon.",
            Loudness::Clipped => "Je klinkt te hard en vervormd. Zet de versterking lager.",
            Loudness::Silent => "Er kwam geluid binnen, maar ik herkende geen spraak. Spreek dichter bij de microfoon.",
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicBool;
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    use super::{BLOCK, Captured, Feed, LEARN_NOISE, Loudness, Shared, SpeechGate, WARMUP_LIMIT, listen, meter_level, rms};

    /// `seconds` of a 440 Hz tone at 16 kHz mono; amplitude 0 is silence.
    fn tone(amplitude: f32, seconds: f32) -> Captured {
        let samples = (0..(16_000.0 * seconds) as usize)
            .map(|i| amplitude * (i as f32 * 440.0 * std::f32::consts::TAU / 16_000.0).sin())
            .collect();
        Captured { samples, rate: 16_000, channels: 1, had_speech: true, device: None, warmup_samples: 0 }
    }

    /// Listens to `sound` delivered in large bursts by a device that stays open.
    fn listen_to(sound: &[f32], boost: f32, stall_limit: Duration) -> anyhow::Result<Captured> {
        let (sender, receiver) = mpsc::channel();
        sound.chunks(8_000).for_each(|burst| sender.send(burst.to_vec()).unwrap());
        let failed = AtomicBool::new(false);
        let result = listen(Feed::new(receiver, &failed, boost, stall_limit), 16_000, 1, &Shared::default());
        drop(sender);
        result
    }

    /// A second of background, a second of speech at `amplitude`, then two of quiet.
    fn utterance(amplitude: f32) -> Vec<f32> {
        [0.0001, amplitude, 0.0, 0.0].iter().flat_map(|&amplitude| tone(amplitude, 1.0).samples).collect()
    }

    fn peak(samples: &[f32]) -> f32 {
        samples.iter().fold(0.0, |peak, s| peak.max(s.abs()))
    }

    #[test]
    fn large_bursts_are_kept_whole_boosted_and_cut_off_at_full_scale() {
        let all = utterance(0.3);
        let captured = listen_to(&all, 1.0, Duration::from_secs(5)).unwrap();
        assert!(captured.had_speech && captured.samples.len() > 32_000, "listened until the silence after the speech");
        assert_eq!(captured.samples, all[..captured.samples.len()], "nothing dropped or reordered");
        let boosted = [2.5, 4.0].map(|boost| peak(&listen_to(&all, boost, Duration::from_secs(5)).unwrap().samples));
        assert!((0.74..=0.76).contains(&boosted[0]) && boosted[1] == 1.0, "0.3 × 2.5, and cut off at 1: {boosted:?}");
    }

    #[test]
    fn a_microphone_that_stops_delivering_ends_the_recording() {
        let started = Instant::now();
        let result = listen_to(&[0.0; 1_600], 1.0, Duration::from_millis(300));
        assert!(matches!(result, Err(err) if err.to_string().contains("geen geluid meer")));
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn startup_zeros_do_not_set_the_noise_floor_and_quiet_speech_is_detected() {
        let mut gate = SpeechGate::default();
        for _ in 0..45 {
            assert!(!gate.hear(0.0));
            assert!(!gate.ready);
        }
        for _ in 0..LEARN_NOISE.as_nanos().div_ceil(BLOCK.as_nanos()) {
            assert!(!gate.hear(0.0001));
        }
        assert!(gate.ready);
        assert_eq!(gate.noise_floor, Some(0.0001));
        assert!(!gate.hear(0.00356));
        assert!(gate.had_speech);
        assert!(meter_level(0.00356) > 0.15);

        let audio: Vec<_> = vec![0.0; 21_600].into_iter().chain(utterance(0.005)).collect();
        let captured = listen_to(&audio, 1.0, Duration::from_secs(5)).unwrap();
        assert_eq!(Loudness::of(&captured), Loudness::Quiet);
    }

    #[test]
    fn digital_silence_times_out_separately_from_steady_background() {
        let mut gate = SpeechGate::default();
        for _ in 0..WARMUP_LIMIT.as_nanos().div_ceil(BLOCK.as_nanos()) {
            assert!(!gate.hear(0.0));
        }
        assert!(gate.ready);
        assert!(!gate.hear(0.0));
        assert!(!gate.had_speech);
        assert_eq!(Loudness::of(&tone(0.0, 1.0)), Loudness::NoSignal);

        let captured = listen_to(&tone(0.004, 9.0).samples, 1.0, Duration::from_secs(5)).unwrap();
        assert_eq!(Loudness::of(&captured), Loudness::Silent);
    }

    #[test]
    fn input_bias_or_a_startup_pop_does_not_mask_quiet_speech() {
        let mut audio: Vec<_> = utterance(0.01).into_iter().map(|sample| sample - 0.26).collect();
        audio[..150].fill(1.0);
        let captured = listen_to(&audio, 1.0, Duration::from_secs(5)).unwrap();
        assert!(captured.had_speech);
        assert_eq!(Loudness::of(&captured), Loudness::Quiet);
        assert!(rms(&[-0.26, 0.3, -0.26, 0.3], 2) < 1e-6);
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
