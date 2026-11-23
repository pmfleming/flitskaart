//! Asks Claude Code on this laptop for an answer in a fixed JSON shape, using
//! the Claude subscription (no API key). Drafting and conversations both go
//! through here.

use std::io::Write as _;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use anyhow::{Context as _, Result, bail};

/// Which Claude Code program to run, and with which model.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Claude {
    pub program: PathBuf,
    pub model: String,
}

impl Claude {
    /// `DUTCH_FLASHCARDS_CLAUDE` (else `claude` on the `PATH`) with
    /// `DUTCH_FLASHCARDS_MODEL` (else `sonnet`).
    pub fn from_env() -> Claude {
        let model = std::env::var("DUTCH_FLASHCARDS_MODEL").unwrap_or_else(|_| "sonnet".into());
        Claude { program: crate::paths::program("DUTCH_FLASHCARDS_CLAUDE", "claude"), model }
    }

    /// Sends `request` with the instructions in `system` and returns the answer,
    /// which follows the JSON `schema`. Blocks for several seconds, so call it
    /// off the UI thread.
    pub fn ask(&self, system: &str, schema: &str, request: &str) -> Result<serde_json::Value> {
        // Run outside any project so no project instructions are picked up.
        let workdir = std::env::temp_dir();
        let mut child = Command::new(&self.program)
            .current_dir(&workdir)
            .args(["-p", "--model", &self.model, "--output-format", "json", "--json-schema", schema])
            .args(["--system-prompt", system])
            .args(["--tools", "", "--strict-mcp-config", "--disable-slash-commands", "--no-session-persistence"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .with_context(|| {
                format!("Claude Code ({}) kon niet starten. Is het geïnstalleerd en ben je ingelogd?", self.program.display())
            })?;
        child.stdin.take().context("Claude Code nam geen tekst aan")?.write_all(request.as_bytes())?;
        let output = child.wait_with_output()?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        if !output.status.success() && stdout.trim().is_empty() {
            bail!("Claude Code gaf een fout: {}", crate::voice::last_line(&output.stderr));
        }
        answer(&stdout)
    }
}

/// Reads the answer out of Claude Code's `--output-format json` envelope.
pub(crate) fn answer(stdout: &str) -> Result<serde_json::Value> {
    let mut envelope: serde_json::Value =
        serde_json::from_str(stdout.trim()).context("Claude Code gaf geen leesbaar antwoord")?;
    let structured = envelope["structured_output"].take();
    let result = envelope["result"].as_str().unwrap_or_default();
    if envelope["is_error"].as_bool() == Some(true) {
        bail!("Claude Code gaf een fout: {}", if result.is_empty() { "onbekende fout" } else { result });
    }
    if !structured.is_null() {
        return Ok(structured);
    }
    // Older Claude Code versions put the answer in the result text instead.
    serde_json::from_str(result).context("Claude Code gaf geen antwoord in de verwachte vorm")
}

#[cfg(test)]
mod tests {
    use super::{Claude, answer};

    #[test]
    fn reads_the_answer_from_either_place_and_reports_errors() {
        let structured = answer(r#"{"is_error":false,"result":"","structured_output":{"a":1}}"#).unwrap();
        assert_eq!(structured["a"], 1);
        assert_eq!(answer(r#"{"is_error":false,"result":"{\"a\":2}"}"#).unwrap()["a"], 2);
        assert!(answer(r#"{"is_error":true,"result":"Not logged in"}"#).unwrap_err().to_string().contains("Not logged in"));
        assert!(answer("not json").is_err());
        assert!(answer(r#"{"is_error":false,"result":"plain words"}"#).is_err());
    }

    #[test]
    fn a_missing_program_says_so() {
        let claude = Claude { program: "/nonexistent/claude".into(), model: "sonnet".into() };
        let err = claude.ask("system", "{}", "hoi").unwrap_err().to_string();
        assert!(err.contains("kon niet starten"), "{err}");
    }
}
