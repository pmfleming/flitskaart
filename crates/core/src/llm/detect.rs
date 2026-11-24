//! Which tools are installed and logged in on this laptop, and which models
//! each offers, asked of the tools themselves.

use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

use serde_json::Value;

use super::{Cli, Effort, MODELS};
use super::antigravity::split_effort;

/// How long a tool may take to say what it offers.
const PATIENCE: Duration = Duration::from_secs(30);

/// A model one of the tools can use.
#[derive(Clone, Debug, PartialEq)]
pub struct ModelChoice {
    pub cli: Cli,
    /// As the tool knows it, without an effort in the name.
    pub id: String,
    pub name: String,
    /// The efforts it offers; empty when it has no choice.
    pub efforts: Vec<Effort>,
    pub default_effort: Effort,
}

impl ModelChoice {
    fn new(cli: Cli, id: impl Into<String>, name: impl Into<String>, efforts: Vec<Effort>, default_effort: Effort) -> ModelChoice {
        ModelChoice { cli, id: id.into(), name: name.into(), efforts, default_effort }
    }

    /// The offered effort closest to `wanted`.
    pub fn effort_for(&self, wanted: Effort) -> Effort {
        let closest = self.efforts.iter().min_by_key(|effort| (**effort as i32 - wanted as i32).abs());
        closest.copied().unwrap_or(wanted)
    }

    /// The model as the tool is told it: Antigravity names carry the effort.
    pub fn run_id(&self, effort: Effort) -> String {
        match self.cli {
            Cli::Antigravity if !self.efforts.is_empty() => format!("{}-{}", self.id, self.effort_for(effort).as_str()),
            _ => self.id.clone(),
        }
    }

    /// Whether this is the model saved as `model` for `cli`.
    pub fn is(&self, cli: Cli, model: &str) -> bool {
        self.cli == cli && (self.id == model || (cli == Cli::Antigravity && split_effort(model).0 == self.id))
    }
}

/// Every logged-in tool's models. Runs the tools, which takes a few seconds,
/// so call it off the UI thread.
pub fn available() -> Vec<ModelChoice> {
    let lookups: Vec<_> = Cli::ALL.into_iter().map(|cli| std::thread::spawn(move || models_of(cli))).collect();
    lookups.into_iter().flat_map(|lookup| lookup.join().unwrap_or_default()).collect()
}

fn models_of(cli: Cli) -> Vec<ModelChoice> {
    let program = cli.program();
    let ask = |args: &[&str]| quiet(&program, args);
    match cli {
        Cli::ClaudeCode => {
            let logged_in = ask(&["auth", "status"]).and_then(|out| serde_json::from_str::<Value>(&out).ok()).is_some_and(|status| status["loggedIn"] == true);
            if !logged_in {
                return Vec::new();
            }
            MODELS.iter().map(|(alias, name)| ModelChoice::new(cli, *alias, *name, Effort::ALL.to_vec(), Effort::default())).collect()
        }
        Cli::Codex => match ask(&["login", "status"]) {
            Some(status) if status.contains("Logged in") => ask(&["debug", "models"]).map(|json| codex_models(&json)).unwrap_or_default(),
            _ => Vec::new(),
        },
        Cli::Antigravity => ask(&["models"]).map(|list| antigravity_models(&list)).unwrap_or_default(),
        Cli::Pi => ask(&["--list-models", "--offline"]).map(|table| pi_models(&table)).unwrap_or_default(),
    }
}

/// `codex debug models`: the models it lists, with their efforts.
pub(super) fn codex_models(json: &str) -> Vec<ModelChoice> {
    let Ok(list) = serde_json::from_str::<Value>(json) else { return Vec::new() };
    let models = list["models"].as_array().into_iter().flatten().filter(|model| model["visibility"] == "list");
    models
        .filter_map(|model| {
            let efforts = model["supported_reasoning_levels"].as_array().into_iter().flatten();
            let efforts: Vec<Effort> = efforts.filter_map(|level| level["effort"].as_str().and_then(Effort::parse)).collect();
            let default = model["default_reasoning_level"].as_str().and_then(Effort::parse).unwrap_or_default();
            let id = model["slug"].as_str()?;
            Some(ModelChoice::new(Cli::Codex, id, model["display_name"].as_str().unwrap_or(id), efforts, default))
        })
        .collect()
}

/// `agy models`: one line per model and effort; one choice per model.
pub(super) fn antigravity_models(list: &str) -> Vec<ModelChoice> {
    let mut choices: Vec<ModelChoice> = Vec::new();
    for (id, name) in list.lines().filter_map(|line| line.split_once('\t')) {
        let (base, effort) = split_effort(id.trim());
        let name = name.trim().split(" (").next().unwrap_or(name).to_string();
        match choices.iter_mut().find(|choice| choice.id == base) {
            Some(choice) => choice.efforts.extend(effort),
            None => choices.push(ModelChoice::new(Cli::Antigravity, base, name, effort.into_iter().collect(), Effort::default())),
        }
    }
    for choice in &mut choices {
        choice.efforts.sort();
        choice.default_effort = choice.effort_for(Effort::default());
    }
    choices
}

/// `pi --list-models`: a table of provider and model.
pub(super) fn pi_models(table: &str) -> Vec<ModelChoice> {
    let rows = table.lines().map(str::split_whitespace).filter_map(|mut cells| Some((cells.next()?, cells.next()?)));
    let models = rows.filter(|(provider, _)| *provider != "provider");
    models.map(|(provider, model)| ModelChoice::new(Cli::Pi, format!("{provider}/{model}"), format!("{model} · {provider}"), Effort::ALL.to_vec(), Effort::default())).collect()
}

/// What a tool prints (stdout and stderr), if it runs and succeeds in time.
fn quiet(program: &std::path::Path, args: &[&str]) -> Option<String> {
    let mut command = Command::new(program);
    command.args(args).current_dir(std::env::temp_dir()).stdin(Stdio::null());
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = sender.send(command.output());
    });
    let output = receiver.recv_timeout(PATIENCE).ok()?.ok().filter(|output| output.status.success())?;
    Some(format!("{}{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr)))
}

#[cfg(test)]
mod tests {
    use super::{antigravity_models, codex_models, pi_models};
    use crate::llm::{Cli, Effort};

    #[test]
    fn reads_each_tools_models() {
        let codex = r#"{"models":[{"slug":"gpt-6.2-sol","display_name":"GPT-6.2-Sol","default_reasoning_level":"low","supported_reasoning_levels":[{"effort":"low"},{"effort":"high"},{"effort":"ultra"}],"visibility":"list"},{"slug":"gpt-reserve","visibility":"hide"}]}"#;
        let codex = codex_models(codex);
        assert_eq!((codex.len(), codex[0].name.as_str(), codex[0].efforts.clone(), codex[0].default_effort), (1, "GPT-6.2-Sol", vec![Effort::Low, Effort::High], Effort::Low));

        let agy = antigravity_models("Fetching available models...\ngemini-4-neon-high\tGemini 4 Neon (High)\ngemini-4-neon-low\tGemini 4 Neon (Low)\ngemini-pro-orchestrator\tOrchestrator (Carbon)\n");
        assert_eq!(agy.len(), 2);
        assert_eq!((agy[0].id.as_str(), agy[0].name.as_str(), agy[0].efforts.clone()), ("gemini-4-neon", "Gemini 4 Neon", vec![Effort::Low, Effort::High]));
        assert_eq!((agy[0].run_id(Effort::Medium), agy[0].run_id(Effort::Max)), ("gemini-4-neon-low".into(), "gemini-4-neon-high".into()));
        assert_eq!(agy[1].run_id(Effort::High), "gemini-pro-orchestrator", "no effort in the name to choose");
        assert!(agy[0].is(Cli::Antigravity, "gemini-4-neon-high"));

        let pi = pi_models("provider      model          context\nopenai-codex  gpt-6.2-sol    500K\n");
        assert_eq!((pi.len(), pi[0].id.as_str(), pi[0].name.as_str()), (1, "openai-codex/gpt-6.2-sol", "gpt-6.2-sol · openai-codex"));
    }
}
