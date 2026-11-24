//! Judging how understandable the learner is when reading a card aloud.
//!
//! The recording is transcribed locally by Whisper (whisper.cpp). The closer
//! the transcript is to the card, the easier the learner was to understand.

use std::ops::Range;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context as _, Result, bail};

use crate::model::Grade;
use crate::paths::{Paths, program};

const MODEL_FILE: &str = "ggml-large-v3-turbo-q5_0.bin";

/// Runs Whisper on recordings.
#[derive(Clone, Debug)]
pub struct Listener {
    whisper: PathBuf,
    model: PathBuf,
}

impl Listener {
    pub fn new(paths: &Paths) -> Listener {
        Listener::with_program(program("DUTCH_FLASHCARDS_WHISPER", "whisper-cli"), paths.models().join(MODEL_FILE))
    }

    pub fn with_program(whisper: PathBuf, model: PathBuf) -> Listener {
        Listener { whisper, model }
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
            bail!("Whisper gaf een fout: {}", crate::voice::last_line(&output.stderr));
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
    let mut bytes = wav_header_16k_mono((out.len() * 2) as u32);
    for sample in out {
        bytes.extend_from_slice(&((sample.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes());
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, bytes).with_context(|| format!("writing {}", path.display()))
}

/// The 44-byte header of a 16 kHz mono 16-bit PCM WAV holding `data_len` bytes.
fn wav_header_16k_mono(data_len: u32) -> Vec<u8> {
    // Format chunk: PCM, mono, 16 kHz, byte rate, block align, bits per sample.
    let fields: [&[u8]; 12] = [
        b"RIFF", &(36 + data_len).to_le_bytes(), b"WAVEfmt ", &16u32.to_le_bytes(),
        &1u16.to_le_bytes(), &1u16.to_le_bytes(), &16_000u32.to_le_bytes(), &32_000u32.to_le_bytes(),
        &2u16.to_le_bytes(), &16u16.to_le_bytes(), b"data", &data_len.to_le_bytes(),
    ];
    fields.concat()
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

/// The pronunciation grade's wording, by [`Grade::index`].
const SPEECH_LABELS: [&str; 4] = ["Onverstaanbaar", "Moeilijk te verstaan", "Goed te verstaan", "Heel duidelijk"];

/// The lowest share of letters understood for each grade above Onverstaanbaar.
const GRADE_FLOORS: [(f32, Grade); 3] = [(0.95, Grade::Easy), (0.85, Grade::Comfortable), (0.65, Grade::Hard)];

/// The grade for a share of letters right (0.0–1.0), when reading aloud or writing.
pub fn grade_for(accuracy: f32) -> Grade {
    GRADE_FLOORS.iter().find(|(floor, _)| accuracy >= *floor).map_or(Grade::DontKnow, |(_, grade)| *grade)
}

/// The pronunciation grade's wording.
pub fn speech_label(grade: Grade) -> &'static str {
    SPEECH_LABELS[grade.index()]
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
    let grade = grade_for(accuracy);
    let matched = aligned_matches(&card_words, &heard_words);
    let words = display_words(card).into_iter().zip(matched).map(|(word, heard)| WordMark { word, heard }).collect();
    Assessment { heard: heard.trim().to_string(), accuracy, grade, words }
}

/// Where in `card` the words the recognizer missed are, as byte ranges, so the
/// card text itself can mark them. Each range covers the word's letters
/// without surrounding punctuation. `words` comes from [`assess`] on the same text.
pub fn missed_ranges(card: &str, words: &[WordMark]) -> Vec<Range<usize>> {
    word_tokens(card)
        .zip(words)
        .filter(|(_, mark)| !mark.heard)
        .filter_map(|(token, _)| {
            let offset = token.as_ptr() as usize - card.as_ptr() as usize;
            let start = token.find(char::is_alphanumeric)?;
            let (last, letter) = token.char_indices().rfind(|(_, c)| c.is_alphanumeric())?;
            Some(offset + start..offset + last + letter.len_utf8())
        })
        .collect()
}

/// The card's words as written, punctuation attached; hyphens split words.
fn word_tokens(text: &str) -> impl Iterator<Item = &str> {
    text.split(|c: char| c.is_whitespace() || c == '-').filter(|w| w.chars().any(char::is_alphanumeric))
}

/// Lower-case words without punctuation, split as [`word_tokens`].
fn words(text: &str) -> Vec<String> {
    word_tokens(text)
        .map(|w| w.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect())
        .collect()
}

/// The card's words as written, split as [`word_tokens`].
fn display_words(text: &str) -> Vec<String> {
    word_tokens(text).map(str::to_string).collect()
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

/// The word-level edit-distance table between `card` and `heard`.
fn alignment_costs(card: &[String], heard: &[String]) -> Vec<Vec<usize>> {
    let mut cost: Vec<Vec<usize>> = (0..=card.len()).map(|i| (i..=i + heard.len()).collect()).collect();
    for i in 1..=card.len() {
        for j in 1..=heard.len() {
            let substitute = cost[i - 1][j - 1] + usize::from(card[i - 1] != heard[j - 1]);
            cost[i][j] = substitute.min(cost[i - 1][j] + 1).min(cost[i][j - 1] + 1);
        }
    }
    cost
}

/// For each card word, whether the alignment matched it to an identical heard word.
fn aligned_matches(card: &[String], heard: &[String]) -> Vec<bool> {
    let cost = alignment_costs(card, heard);
    let mut matched = vec![false; card.len()];
    let (mut i, mut j) = (card.len(), heard.len());
    while i > 0 && j > 0 {
        let same = card[i - 1] == heard[j - 1];
        if cost[i][j] == cost[i - 1][j - 1] + usize::from(!same) {
            matched[i - 1] = same;
            (i, j) = (i - 1, j - 1);
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
    use super::{Grade, assess, missed_ranges, write_wav_16k};

    const CARD: &str = "Vandaag ben ik een Linux-programma aan het schrijven.";

    /// What was heard, the grade it earns, and the card's words marked as missed.
    /// Marks cover a word's letters: punctuation stays outside, letters beyond ASCII whole.
    #[test]
    fn readings_are_graded_and_missed_words_marked_in_the_card() {
        let cases: [(&str, &str, Grade, &[&str]); 4] = [
            (CARD, "Vandaag ben ik een Linux programma aan het schrijven.", Grade::Easy, &[]),
            (CARD, "Vandaag ben ik een programma aan het schrijven.", Grade::Comfortable, &["Linux"]),
            (CARD, "Vandaag ben ik een programma het schrijven.", Grade::Hard, &["Linux", "aan"]),
            ("Hé, één café?", "hé", Grade::DontKnow, &["één", "café"]),
        ];
        for (card, heard, grade, missed) in cases {
            let result = assess(card, heard);
            let marked: Vec<_> = missed_ranges(card, &result.words).into_iter().map(|range| &card[range]).collect();
            assert_eq!((result.grade, marked.as_slice()), (grade, missed), "{heard}");
        }
        for mumble in ["", "Ondertiteling door de redactie", "Vandaag ben ik"] {
            assert_eq!(assess(CARD, mumble).grade, Grade::DontKnow, "{mumble}");
        }
        let vowel = assess("Ik heb kaas gekocht.", "Ik heb kees gekocht.");
        assert!(vowel.accuracy > assess("Ik heb kaas gekocht.", "Ik heb gekocht.").accuracy, "a vowel costs less than a word");
    }

    #[test]
    fn writes_a_16k_mono_wav() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("x.wav");
        // One second of stereo at 48 kHz.
        write_wav_16k(&path, &vec![0.25f32; 48_000 * 2], 48_000, 2).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(&bytes[0..4], b"RIFF");
        assert_eq!(u32::from_le_bytes(bytes[24..28].try_into().unwrap()), 16_000);
        assert_eq!(bytes.len(), 44 + 16_000 * 2);
    }
}
