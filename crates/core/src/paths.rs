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

    /// Piper voice models. `scripts/fetch-voices.sh` fills it.
    pub fn voices(&self) -> PathBuf {
        if let Some(dir) = std::env::var_os("DUTCH_FLASHCARDS_VOICES") {
            return PathBuf::from(dir);
        }
        self.root.join("voices")
    }

    pub fn ensure(&self) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.root)?;
        std::fs::create_dir_all(self.audio())
    }
}
