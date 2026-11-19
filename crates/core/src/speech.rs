//! Judging how understandable the learner is when reading a card aloud.
//!
//! The recording is transcribed locally by Whisper (whisper.cpp). The closer
//! the transcript is to the card, the easier the learner was to understand.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context as _, Result, bail};

use crate::model::Grade;
use crate::paths::Paths;

pub const MODEL_FILE: &str = "ggml-large-v3-turbo-q5_0.bin";

/// Runs Whisper on recordings.
#[derive(Clone, Debug)]
pub struct Listener {
    whisper: PathBuf,
    model: PathBuf,
}

impl Listener {
    pub fn new(paths: &Paths) -> Listener {
        let whisper =
            std::env::var_os("DUTCH_FLASHCARDS_WHISPER").map_or_else(|| PathBuf::from("whisper-cli"), PathBuf::from);
        Listener { whisper, model: paths.models().join(MODEL_FILE) }
    }

    pub fn is_installed(&self) -> bool {
        self.model.metadata().is_ok_and(|m| m.len() > 0)
    }

    /// What Whisper hears in a 16 kHz mono WAV file. Takes about a second on
    /// the GPU, so call it off the UI thread.
    pub fn transcribe(&self, wav: &Path) -> Result<String> {
        if !self.is_installed() {
            bail!("De spraakherkenning is niet geïnstalleerd. Voer scripts/fetch-models.sh uit.");
        }
        let output = Command::new(&self.whisper)
            .arg("--model")
            .arg(&self.model)
            .args(["--language", "nl", "--threads", "8", "--no-timestamps", "--no-prints", "--file"])
            .arg(wav)
            .output()
            .with_context(|| {
                format!("Whisper kon niet starten ({}). Start de app vanuit `nix develop`.", self.whisper.display())
            })?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!("Whisper gaf een fout: {}", stderr.lines().last().unwrap_or("onbekend"));
        }
        let text = String::from_utf8_lossy(&output.stdout);
        Ok(text.split_whitespace().collect::<Vec<_>>().join(" "))
    }
}

/// Writes audio as a 16 kHz mono 16-bit WAV, the format Whisper reads.
pub fn write_wav_16k(path: &Path, samples: &[f32], rate: u32, channels: u16) -> Result<()> {
    let mono: Vec<f32> = samples
        .chunks(usize::from(channels.max(1)))
        .map(|frame| frame.iter().sum::<f32>() / frame.len() as f32)
        .collect();
    let out = resample(&mono, rate, 16_000);
    let data_len = (out.len() * 2) as u32;
    let mut bytes = Vec::with_capacity(44 + out.len() * 2);
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + data_len).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16u32.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes()); // PCM
    bytes.extend_from_slice(&1u16.to_le_bytes()); // mono
    bytes.extend_from_slice(&16_000u32.to_le_bytes());
    bytes.extend_from_slice(&32_000u32.to_le_bytes()); // byte rate
    bytes.extend_from_slice(&2u16.to_le_bytes()); // block align
    bytes.extend_from_slice(&16u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data_len.to_le_bytes());
    for sample in out {
        bytes.extend_from_slice(&((sample.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes());
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, bytes).with_context(|| format!("writing {}", path.display()))
}

/// Linear-interpolation resampling; plenty for speech recognition.
fn resample(input: &[f32], from: u32, to: u32) -> Vec<f32> {
    if from == to || input.is_empty() {
        return input.to_vec();
    }
    let ratio = f64::from(from) / f64::from(to);
    let len = (input.len() as f64 / ratio) as usize;
    (0..len)
        .map(|i| {
            let pos = i as f64 * ratio;
            let left = pos.floor() as usize;
            let right = (left + 1).min(input.len() - 1);
            let t = (pos - left as f64) as f32;
            input[left] * (1.0 - t) + input[right] * t
        })
        .collect()
}

/// How one word of the card came across.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WordMark {
    pub word: String,
    pub heard: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Assessment {
    /// What the speech recognizer understood.
    pub heard: String,
    /// Share of the card's letters it understood, 0.0–1.0.
    pub accuracy: f32,
    pub grade: Grade,
    /// The card's words, each marked as understood or not.
    pub words: Vec<WordMark>,
}

/// The pronunciation grade's wording.
pub fn speech_label(grade: Grade) -> &'static str {
    match grade {
        Grade::DontKnow => "Onverstaanbaar",
        Grade::Hard => "Moeilijk te verstaan",
        Grade::Comfortable => "Goed te verstaan",
        Grade::Easy => "Heel duidelijk",
    }
}

/// Compares what was heard with the card.
///
/// The grade comes from letter-level similarity, so spelling choices such as
/// "Linux-programma" versus "Linux programma" don't count against the learner.
/// The word marks come from a word-level alignment, to show what was missed.
pub fn assess(card: &str, heard: &str) -> Assessment {
    let card_words = words(card);
    let heard_words = words(heard);
    let card_letters: Vec<char> = card_words.concat().chars().collect();
    let heard_letters: Vec<char> = heard_words.concat().chars().collect();
    let accuracy = if card_letters.is_empty() {
        0.0
    } else {
        let errors = edit_distance(&card_letters, &heard_letters) as f32;
        (1.0 - errors / card_letters.len() as f32).max(0.0)
    };
    let grade = if accuracy >= 0.95 {
        Grade::Easy
    } else if accuracy >= 0.85 {
        Grade::Comfortable
    } else if accuracy >= 0.65 {
        Grade::Hard
    } else {
        Grade::DontKnow
    };
    let matched = aligned_matches(&card_words, &heard_words);
    let words = display_words(card).into_iter().zip(matched).map(|(word, heard)| WordMark { word, heard }).collect();
    Assessment { heard: heard.trim().to_string(), accuracy, grade, words }
}

/// The grade that schedules the card: the lower of how well the learner knew
/// it and how understandable they were, so either weakness brings it back sooner.
pub fn combine(own: Grade, speech: Option<Grade>) -> Grade {
    match speech {
        Some(speech) if speech.number() < own.number() => speech,
        _ => own,
    }
}

/// Lower-case words without punctuation; hyphens split words.
fn words(text: &str) -> Vec<String> {
    text.split(|c: char| c.is_whitespace() || c == '-')
        .map(|w| w.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect::<String>())
        .filter(|w| !w.is_empty())
        .collect()
}

/// The card's words as written, split the same way as [`words`].
fn display_words(text: &str) -> Vec<String> {
    text.split(|c: char| c.is_whitespace() || c == '-')
        .filter(|w| w.chars().any(char::is_alphanumeric))
        .map(str::to_string)
        .collect()
}

fn edit_distance<T: PartialEq>(a: &[T], b: &[T]) -> usize {
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, x) in a.iter().enumerate() {
        let mut diagonal = row[0];
        row[0] = i + 1;
        for (j, y) in b.iter().enumerate() {
            let next = (row[j + 1] + 1).min(row[j] + 1).min(diagonal + usize::from(x != y));
            diagonal = row[j + 1];
            row[j + 1] = next;
        }
    }
    row[b.len()]
}

/// For each card word, whether the alignment matched it to an identical heard word.
fn aligned_matches(card: &[String], heard: &[String]) -> Vec<bool> {
    let (n, m) = (card.len(), heard.len());
    let mut cost = vec![vec![0usize; m + 1]; n + 1];
    for (i, row) in cost.iter_mut().enumerate() {
        row[0] = i;
    }
    for (j, cell) in cost[0].iter_mut().enumerate() {
        *cell = j;
    }
    for i in 1..=n {
        for j in 1..=m {
            let substitute = cost[i - 1][j - 1] + usize::from(card[i - 1] != heard[j - 1]);
            cost[i][j] = substitute.min(cost[i - 1][j] + 1).min(cost[i][j - 1] + 1);
        }
    }
    let mut matched = vec![false; n];
    let (mut i, mut j) = (n, m);
    while i > 0 && j > 0 {
        let same = card[i - 1] == heard[j - 1];
        if cost[i][j] == cost[i - 1][j - 1] + usize::from(!same) {
            matched[i - 1] = same;
            i -= 1;
            j -= 1;
        } else if cost[i][j] == cost[i - 1][j] + 1 {
            i -= 1;
        } else {
            j -= 1;
        }
    }
    matched
}

#[cfg(test)]
mod tests {
    use super::*;

    const CARD: &str = "Vandaag ben ik een Linux-programma aan het schrijven.";

    #[test]
    fn a_perfect_reading_is_very_clear() {
        let result = assess(CARD, "Vandaag ben ik een Linux programma aan het schrijven.");
        assert_eq!(result.grade, Grade::Easy);
        assert!((result.accuracy - 1.0).abs() < f32::EPSILON);
        assert!(result.words.iter().all(|w| w.heard));
        assert_eq!(result.words.len(), 9);
        assert_eq!(result.words[4].word, "Linux");
    }

    #[test]
    fn a_missed_word_lowers_the_grade_and_is_marked() {
        let result = assess(CARD, "Vandaag ben ik een programma aan het schrijven.");
        assert_eq!(result.grade, Grade::Comfortable, "{}", result.accuracy);
        let missed: Vec<_> = result.words.iter().filter(|w| !w.heard).map(|w| w.word.as_str()).collect();
        assert_eq!(missed, ["Linux"]);
    }

    #[test]
    fn misheard_vowels_cost_less_than_missing_words() {
        let vowel = assess("Ik heb kaas gekocht.", "Ik heb kees gekocht.");
        let missing = assess("Ik heb kaas gekocht.", "Ik heb gekocht.");
        assert!(vowel.accuracy > missing.accuracy);
        assert_eq!(vowel.words.iter().filter(|w| !w.heard).count(), 1);
    }

    #[test]
    fn mumbling_or_silence_is_not_understandable() {
        assert_eq!(assess(CARD, "").grade, Grade::DontKnow);
        assert_eq!(assess(CARD, "Ondertiteling door de redactie").grade, Grade::DontKnow);
        assert_eq!(assess(CARD, "Vandaag ben ik").grade, Grade::DontKnow);
    }

    #[test]
    fn the_lower_grade_schedules_the_card() {
        assert_eq!(combine(Grade::Easy, Some(Grade::Hard)), Grade::Hard);
        assert_eq!(combine(Grade::Hard, Some(Grade::Easy)), Grade::Hard);
        assert_eq!(combine(Grade::Comfortable, None), Grade::Comfortable);
    }

    #[test]
    fn writes_a_16k_mono_wav() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("x.wav");
        // One second of stereo at 48 kHz.
        let samples = vec![0.25f32; 48_000 * 2];
        write_wav_16k(&path, &samples, 48_000, 2).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(&bytes[0..4], b"RIFF");
        assert_eq!(u32::from_le_bytes(bytes[24..28].try_into().unwrap()), 16_000);
        assert_eq!(bytes.len(), 44 + 16_000 * 2);
    }
}
