//! The Dutch voice: Piper, run locally, with every phrase cached as a WAV file.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context as _, Result, anyhow, bail};
use sha2::{Digest, Sha256};

use crate::paths::Paths;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Gender {
    Male,
    Female,
}

impl Gender {
    pub fn as_str(self) -> &'static str {
        match self {
            Gender::Male => "man",
            Gender::Female => "vrouw",
        }
    }

    pub fn parse(value: &str) -> Gender {
        if value == "vrouw" { Gender::Female } else { Gender::Male }
    }

    pub fn label(self) -> &'static str {
        match self {
            Gender::Male => "Man",
            Gender::Female => "Vrouw",
        }
    }
}

#[derive(Debug)]
pub struct VoiceOption {
    pub id: &'static str,
    pub label: &'static str,
    pub gender: Gender,
    /// File stem of the Piper model in the voices folder.
    pub model: &'static str,
    /// Shown next to the name in the settings, such as the accent.
    pub note: &'static str,
}

/// The voices on offer: Piper's clearest Dutch voices.
///
/// Clarity was measured by having Whisper transcribe a test paragraph from each
/// candidate: these four got 91–93% of the words right. The female voices built
/// from MLS audiobook recordings got 0–62% and were dropped, which leaves
/// Nathalie, who speaks Flemish Dutch, as the only clear female voice.
pub const VOICES: &[VoiceOption] = &[
    VoiceOption { id: "pim", label: "Pim", gender: Gender::Male, model: "nl_NL-pim-medium", note: "" },
    VoiceOption { id: "ronnie", label: "Ronnie", gender: Gender::Male, model: "nl_NL-ronnie-medium", note: "" },
    VoiceOption { id: "alex", label: "Alex", gender: Gender::Male, model: "nl_NL-alex-medium", note: "" },
    VoiceOption {
        id: "nathalie",
        label: "Nathalie",
        gender: Gender::Female,
        model: "nl_BE-nathalie-medium",
        note: "Vlaams (België)",
    },
];

pub fn voice(id: &str) -> &'static VoiceOption {
    VOICES.iter().find(|v| v.id == id).unwrap_or(&VOICES[0])
}

pub fn voices_for(gender: Gender) -> impl Iterator<Item = &'static VoiceOption> {
    VOICES.iter().filter(move |v| v.gender == gender)
}

/// A sentence for trying out a voice.
pub const SAMPLE: &str = "Hallo! Zo klink ik. Ik lees je kaarten graag voor.";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Speed {
    Normal,
    Slow,
}

impl Speed {
    fn length_scale(self) -> &'static str {
        match self {
            Speed::Normal => "1.0",
            Speed::Slow => "1.45",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Speaker {
    voices: PathBuf,
    audio: PathBuf,
    piper: PathBuf,
}

impl Speaker {
    pub fn new(paths: &Paths) -> Speaker {
        let piper = std::env::var_os("DUTCH_FLASHCARDS_PIPER").map_or_else(|| PathBuf::from("piper"), PathBuf::from);
        Speaker { voices: paths.voices(), audio: paths.audio(), piper }
    }

    /// Where the audio for this phrase lives once generated.
    pub fn audio_path(&self, text: &str, voice: &VoiceOption, speed: Speed) -> PathBuf {
        let mut hash = Sha256::new();
        for part in [voice.model, speed.length_scale(), text.trim()] {
            hash.update(part.as_bytes());
            hash.update([0]);
        }
        let digest = hash.finalize();
        let name: String = digest.iter().take(16).map(|b| format!("{b:02x}")).collect();
        self.audio.join(format!("{name}.wav"))
    }

    pub fn model_path(&self, voice: &VoiceOption) -> PathBuf {
        self.voices.join(format!("{}.onnx", voice.model))
    }

    pub fn is_installed(&self, voice: &VoiceOption) -> bool {
        has_file(&self.model_path(voice)) && has_file(&self.model_path(voice).with_extension("onnx.json"))
    }

    /// Returns the cached audio, generating it with Piper first if needed.
    /// Blocks for about a second when generating, so call it off the UI thread.
    pub fn speech(&self, text: &str, voice: &VoiceOption, speed: Speed) -> Result<PathBuf> {
        let text = text.trim();
        if text.is_empty() {
            bail!("Er is geen tekst om voor te lezen.");
        }
        let target = self.audio_path(text, voice, speed);
        if has_file(&target) {
            return Ok(target);
        }
        if !self.is_installed(voice) {
            return Err(anyhow!(
                "De stem {} is niet geïnstalleerd. Voer scripts/fetch-models.sh uit.",
                voice.label
            ));
        }
        std::fs::create_dir_all(&self.audio)?;
        let partial = target.with_extension("partial.wav");
        let mut command = Command::new(&self.piper);
        command
            .arg("--model")
            .arg(self.model_path(voice))
            .arg("--length-scale")
            .arg(speed.length_scale())
            .arg("--output-file")
            .arg(&partial)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        let mut child = command.spawn().with_context(|| {
            format!("Piper kon niet starten ({}). Start de app vanuit `nix develop`.", self.piper.display())
        })?;
        child.stdin.take().unwrap().write_all(text.as_bytes())?;
        let output = child.wait_with_output()?;
        if !output.status.success() || !has_file(&partial) {
            let _ = std::fs::remove_file(&partial);
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!("Piper gaf een fout: {}", stderr.lines().last().unwrap_or("onbekend"));
        }
        std::fs::rename(&partial, &target)?;
        Ok(target)
    }
}

fn has_file(path: &Path) -> bool {
    path.metadata().is_ok_and(|m| m.is_file() && m.len() > 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_gender_has_a_voice_and_ids_are_unique() {
        assert!(voices_for(Gender::Male).count() >= 1);
        assert!(voices_for(Gender::Female).count() >= 1);
        let mut ids: Vec<_> = VOICES.iter().map(|v| v.id).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), VOICES.len());
    }

    #[test]
    fn audio_cache_key_depends_on_text_voice_and_speed() {
        let speaker = Speaker::new(&Paths::at("/tmp/x"));
        let a = speaker.audio_path("Hallo.", voice("pim"), Speed::Normal);
        assert_eq!(a, speaker.audio_path(" Hallo. ", voice("pim"), Speed::Normal));
        assert_ne!(a, speaker.audio_path("Hallo!", voice("pim"), Speed::Normal));
        assert_ne!(a, speaker.audio_path("Hallo.", voice("ronnie"), Speed::Normal));
        assert_ne!(a, speaker.audio_path("Hallo.", voice("pim"), Speed::Slow));
        assert_ne!(a, speaker.audio_path("Hallo.", voice("nathalie"), Speed::Normal));
    }
    #[test]
    fn unknown_voice_ids_fall_back() {
        assert_eq!(voice("gone").id, VOICES[0].id);
    }
}
