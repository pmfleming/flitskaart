use std::path::{Path, PathBuf};

/// Where the app keeps its files. Everything lives in one folder so a backup
/// is a single copy.
#[derive(Clone, Debug)]
pub struct Paths {
    root: PathBuf,
}

impl Paths {
    /// `$DUTCH_FLASHCARDS_DATA`, else `$XDG_DATA_HOME/dutch-flashcards`,
    /// else `~/.local/share/dutch-flashcards`.
    pub fn from_env() -> Paths {
        if let Some(dir) = std::env::var_os("DUTCH_FLASHCARDS_DATA") {
            return Paths::at(dir);
        }
        let base = std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .or_else(|| std::env::var_os("HOME").map(|home| Path::new(&home).join(".local/share")))
            .unwrap_or_else(|| PathBuf::from("."));
        Paths::at(base.join("dutch-flashcards"))
    }

    pub fn at(root: impl Into<PathBuf>) -> Paths {
        Paths { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn database(&self) -> PathBuf {
        self.root.join("cards.db")
    }

    /// Generated speech, one WAV per phrase, voice and speed.
    pub fn audio(&self) -> PathBuf {
        self.root.join("audio")
    }

    /// Piper voice models, or `$DUTCH_FLASHCARDS_VOICES`. `scripts/fetch-models.sh` fills it.
    pub fn voices(&self) -> PathBuf {
        self.overridable("DUTCH_FLASHCARDS_VOICES", "voices")
    }

    /// The Whisper speech recognizer, or `$DUTCH_FLASHCARDS_MODELS`. `scripts/fetch-models.sh` fills it.
    pub fn models(&self) -> PathBuf {
        self.overridable("DUTCH_FLASHCARDS_MODELS", "models")
    }

    /// The folder named by `variable`, else `name` inside the data folder.
    fn overridable(&self, variable: &str, name: &str) -> PathBuf {
        std::env::var_os(variable).map_or_else(|| self.root.join(name), PathBuf::from)
    }

    /// The app's log: errors, panics and the recording steps.
    pub fn log_file(&self) -> PathBuf {
        self.root.join("flitskaart.log")
    }

    /// The learner's latest recordings.
    pub fn recordings(&self) -> PathBuf {
        self.root.join("recordings")
    }

    pub fn ensure(&self) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.root)?;
        std::fs::create_dir_all(self.audio())
    }
}

/// The program `$variable` names, else `name` found on the `PATH`.
pub fn program(variable: &str, name: &str) -> PathBuf {
    std::env::var_os(variable).map_or_else(|| PathBuf::from(name), PathBuf::from)
}
