//! Asks a language model for an answer in a fixed JSON shape, through a
//! command-line tool that is logged in on this laptop: Claude Code, Codex,
//! Antigravity (Gemini) or Pi. The subscription behind the tool pays; no API
//! key is needed. Every answer's token use is noted, for Instellingen.

mod antigravity;
mod claude_code;
mod codex;
mod detect;
mod pi;

use std::io::Write as _;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};

use anyhow::{Context as _, Result};

pub use detect::{ModelChoice, available};
#[cfg(test)]
pub(crate) use claude_code::answer;

/// Claude Code's models: the alias it knows, and the name people see.
pub const MODELS: [(&str, &str); 4] = [("opus", "Opus 5.5"), ("sonnet", "Sonnet 5.5"), ("fable", "Fable 5.2"), ("haiku", "Haiku 5.5")];
pub const DEFAULT_MODEL: &str = "sonnet";

/// The name people see for a model: Claude's by alias or full name (such as
/// `claude-opus-5-5`), any other as it is.
pub fn model_name(model: &str) -> String {
    let name = MODELS.iter().find(|(alias, _)| model == *alias || model.contains(&format!("-{alias}-")));
    name.map_or_else(|| model.to_string(), |(_, name)| name.to_string())
}

/// A command-line tool that answers.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Cli {
    #[default]
    ClaudeCode,
    Codex,
    /// Google's Antigravity, for the Gemini models.
    Antigravity,
    /// Pi, which reaches whichever providers it is logged in to.
    Pi,
}

impl Cli {
    pub const ALL: [Cli; 4] = [Cli::ClaudeCode, Cli::Codex, Cli::Antigravity, Cli::Pi];
    const IDS: [&str; 4] = ["claude", "codex", "agy", "pi"];
    const NAMES: [&str; 4] = ["Claude Code", "Codex", "Antigravity", "Pi"];
    const VARIABLES: [&str; 4] = ["DUTCH_FLASHCARDS_CLAUDE", "DUTCH_FLASHCARDS_CODEX", "DUTCH_FLASHCARDS_AGY", "DUTCH_FLASHCARDS_PI"];

    /// The program's name, also how the setting is saved.
    pub fn id(self) -> &'static str {
        Cli::IDS[self as usize]
    }

    pub fn name(self) -> &'static str {
        Cli::NAMES[self as usize]
    }

    pub fn parse(id: &str) -> Option<Cli> {
        Cli::ALL.into_iter().find(|cli| cli.id() == id)
    }

    /// The program to run: from its `DUTCH_FLASHCARDS_…` variable, else by name on the `PATH`.
    pub fn program(self) -> PathBuf {
        crate::paths::program(Cli::VARIABLES[self as usize], self.id())
    }
}

/// How much the model thinks before it answers.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum Effort {
    Low,
    #[default]
    Medium,
    High,
    XHigh,
    Max,
}

impl Effort {
    pub const ALL: [Effort; 5] = [Effort::Low, Effort::Medium, Effort::High, Effort::XHigh, Effort::Max];
    const NAMES: [&str; 5] = ["low", "medium", "high", "xhigh", "max"];
    const LABELS: [&str; 5] = ["Laag", "Gemiddeld", "Hoog", "Extra hoog", "Max"];

    pub fn as_str(self) -> &'static str {
        Effort::NAMES[self as usize]
    }

    pub fn label(self) -> &'static str {
        Effort::LABELS[self as usize]
    }

    pub fn parse(name: &str) -> Option<Effort> {
        Effort::ALL.into_iter().find(|effort| effort.as_str() == name)
    }
}

/// The tokens one model used for one answer.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Usage {
    /// Which tool answered, by [`Cli::id`].
    pub cli: String,
    /// The model's full name, such as `claude-sonnet-5-5`.
    pub model: String,
    pub effort: String,
    /// New input, not read from the cache.
    pub input: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    /// Output, thinking included.
    pub output: u64,
}

impl Usage {
    pub fn tokens(&self) -> u64 {
        self.input + self.cache_read + self.cache_write + self.output
    }
}

/// Answers' token use waiting to be saved. Clones share it, so the copies
/// handed to background work all report to the same place.
#[derive(Clone, Debug, Default)]
pub struct UsageLog(Arc<Mutex<Vec<Usage>>>);

impl UsageLog {
    fn add(&self, usage: Vec<Usage>) {
        if let Ok(mut waiting) = self.0.lock() {
            waiting.extend(usage);
        }
    }

    /// Everything noted since the last call.
    pub fn take(&self) -> Vec<Usage> {
        self.0.lock().map(|mut waiting| std::mem::take(&mut *waiting)).unwrap_or_default()
    }
}

/// Which tool to run, with which model and effort.
#[derive(Clone, Debug)]
pub struct Llm {
    pub cli: Cli,
    pub program: PathBuf,
    /// As the tool knows it; for Antigravity it includes the effort, such as `gemini-4-neon-high`.
    pub model: String,
    pub effort: Effort,
    pub usage: UsageLog,
}

impl Llm {
    pub fn new(cli: Cli, model: &str) -> Llm {
        Llm { cli, program: cli.program(), model: model.into(), effort: Effort::default(), usage: UsageLog::default() }
    }

    /// Claude Code with `DUTCH_FLASHCARDS_MODEL` (else `sonnet`).
    pub fn from_env() -> Llm {
        Llm::new(Cli::ClaudeCode, &std::env::var("DUTCH_FLASHCARDS_MODEL").unwrap_or_else(|_| DEFAULT_MODEL.into()))
    }

    /// Switches to another tool and model.
    pub fn choose(&mut self, cli: Cli, model: &str, effort: Effort) {
        if cli != self.cli {
            self.program = cli.program();
        }
        (self.cli, self.model, self.effort) = (cli, model.into(), effort);
    }

    /// Sends `request` with the instructions in `system` and returns the answer,
    /// which follows the JSON `schema`. Blocks for several seconds, so call it
    /// off the UI thread.
    pub fn ask(&self, system: &str, schema: &str, request: &str) -> Result<serde_json::Value> {
        match self.cli {
            Cli::ClaudeCode => claude_code::ask(self, system, schema, request),
            Cli::Codex => codex::ask(self, system, schema, request),
            Cli::Antigravity => antigravity::ask(self, system, schema, request),
            Cli::Pi => pi::ask(self, system, schema, request),
        }
    }

    /// Research searches the web, which only Claude Code is set up to do here;
    /// with another tool chosen, Claude Code's default model does it.
    pub(crate) fn research(&self, system: &str, schema: &str, request: &str) -> Result<serde_json::Value> {
        if self.cli == Cli::ClaudeCode {
            return claude_code::research(self, system, schema, request);
        }
        let claude = Llm { usage: self.usage.clone(), effort: self.effort, ..Llm::new(Cli::ClaudeCode, DEFAULT_MODEL) };
        claude_code::research(&claude, system, schema, request)
    }

    /// Runs the tool in a scratch folder (so no project instructions are
    /// picked up) with `args`, writing `input` to it.
    fn run(&self, args: &[&str], input: &str) -> Result<Ran> {
        let name = self.cli.name();
        let mut child = Command::new(&self.program)
            .current_dir(std::env::temp_dir())
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .with_context(|| format!("{name} ({}) kon niet starten. Is het geïnstalleerd en ben je ingelogd?", self.program.display()))?;
        child.stdin.take().with_context(|| format!("{name} nam geen tekst aan"))?.write_all(input.as_bytes())?;
        let output = child.wait_with_output()?;
        Ok(Ran {
            ok: output.status.success(),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            problem: crate::voice::last_line(&output.stderr),
        })
    }

    fn note(&self, usage: Vec<Usage>) {
        self.usage.add(usage);
    }

    /// A usage record for this tool and effort.
    fn usage_of(&self, model: impl Into<String>) -> Usage {
        Usage { cli: self.cli.id().into(), model: model.into(), effort: self.effort.as_str().into(), ..Usage::default() }
    }
}

/// What a tool printed.
struct Ran {
    ok: bool,
    stdout: String,
    /// The last line it wrote to stderr.
    problem: String,
}

/// The JSON object in a model's text answer, without a code fence around it.
fn json_in(text: &str) -> Result<serde_json::Value> {
    let start = text.find('{').context("geen JSON in het antwoord")?;
    let end = text.rfind('}').context("geen JSON in het antwoord")?;
    serde_json::from_str(&text[start..=end]).context("het antwoord was geen geldige JSON")
}

/// Drops `null` fields, so missing answers fall back to their defaults.
fn without_nulls(value: serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(map) => {
            serde_json::Value::Object(map.into_iter().filter(|(_, v)| !v.is_null()).map(|(k, v)| (k, without_nulls(v))).collect())
        }
        serde_json::Value::Array(items) => serde_json::Value::Array(items.into_iter().map(without_nulls).collect()),
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::{Cli, Effort, Llm, json_in, model_name, without_nulls};

    #[test]
    fn names_tools_models_and_efforts() {
        assert_eq!((model_name("claude-haiku-5-5"), model_name("opus"), model_name("gpt-6.2-sol")), ("Haiku 5.5".into(), "Opus 5.5".into(), "gpt-6.2-sol".into()));
        assert_eq!((Cli::parse("agy"), Cli::Antigravity.name()), (Some(Cli::Antigravity), "Antigravity"));
        assert_eq!(Effort::parse("xhigh"), Some(Effort::XHigh));
    }

    #[test]
    fn finds_the_json_in_a_text_answer() {
        assert_eq!(json_in("```json\n{\"a\": 1}\n```").unwrap()["a"], 1);
        assert!(json_in("geen json").is_err());
        assert_eq!(without_nulls(serde_json::json!({"a": null, "b": [{"c": null, "d": 1}]})), serde_json::json!({"b": [{"d": 1}]}));
    }

    #[test]
    fn a_missing_program_says_so() {
        for cli in Cli::ALL {
            let llm = Llm { program: "/nonexistent/tool".into(), ..Llm::new(cli, "x") };
            let err = llm.ask("system", "{}", "hoi").unwrap_err().to_string();
            assert!(err.contains("kon niet starten") && err.contains(cli.name()), "{err}");
        }
    }
}
