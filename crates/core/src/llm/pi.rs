//! Pi (`pi -p`), which reaches the providers it is logged in to. It does not
//! hold answers to a schema, so the schema goes into the instructions and the
//! JSON is read out of the answer.

use anyhow::{Context as _, Result, bail};
use serde_json::Value;

use super::{Llm, Usage, json_in, without_nulls};

pub(super) fn ask(llm: &Llm, system: &str, schema: &str, request: &str) -> Result<Value> {
    let system = format!(
        "{system}\n\nAntwoord uitsluitend met één JSON-object dat aan dit JSON-schema voldoet, zonder uitleg en zonder codeblok:\n{schema}"
    );
    let mut args = vec!["-p", "--mode", "json", "--no-session", "--no-tools", "--no-extensions", "--no-skills", "--no-prompt-templates"];
    args.extend(["--no-themes", "--no-context-files", "--no-mcp", "--offline", "--thinking", llm.effort.as_str(), "--system-prompt", &system]);
    if !llm.model.is_empty() {
        args.extend(["--model", &llm.model]);
    }
    let ran = llm.run(&args, request)?;
    llm.note(usage_in(llm, &ran.stdout));
    match reply(&ran.stdout) {
        Err(_) if !ran.ok && !ran.problem.is_empty() => bail!("Pi gaf een fout: {}", ran.problem),
        result => result,
    }
}

/// The assistant's message at the end of the last turn.
fn last_turn(stdout: &str) -> Option<Value> {
    let mut events = stdout.lines().filter_map(|line| serde_json::from_str::<Value>(line).ok());
    events.rfind(|event| event["type"] == "turn_end").map(|mut event| event["message"].take())
}

fn reply(stdout: &str) -> Result<Value> {
    let message = last_turn(stdout).context("Pi gaf geen antwoord")?;
    if message["stopReason"] == "error" {
        bail!("Pi gaf een fout: {}", message["errorMessage"].as_str().unwrap_or("onbekende fout"));
    }
    let parts = message["content"].as_array().into_iter().flatten();
    let text: String = parts.filter(|part| part["type"] == "text").filter_map(|part| part["text"].as_str()).collect();
    Ok(without_nulls(json_in(&text)?))
}

fn usage_in(llm: &Llm, stdout: &str) -> Vec<Usage> {
    let Some(message) = last_turn(stdout) else { return Vec::new() };
    let usage = &message["usage"];
    let count = |key: &str| usage[key].as_u64().unwrap_or(0);
    let model = match (message["provider"].as_str(), message["model"].as_str()) {
        (Some(provider), Some(model)) => format!("{provider}/{model}"),
        _ => llm.model.clone(),
    };
    let used = Usage {
        input: count("input"),
        cache_read: count("cacheRead"),
        cache_write: count("cacheWrite"),
        output: count("output"),
        ..llm.usage_of(model)
    };
    if used.tokens() > 0 { vec![used] } else { Vec::new() }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{reply, usage_in};
    use crate::llm::{Cli, Llm};

    #[test]
    fn reads_the_answer_errors_and_usage() {
        let stdout = r#"{"type":"turn_start"}
{"type":"turn_end","message":{"role":"assistant","content":[{"type":"text","text":"{\"word\":\"hallo\"}"}],"provider":"openai-codex","model":"gpt-6.2-sol","usage":{"input":36,"output":10,"cacheRead":4,"cacheWrite":0,"cost":{"total":0.000172}},"stopReason":"stop"}}"#;
        assert_eq!(reply(stdout).unwrap(), json!({"word": "hallo"}));
        let usage = usage_in(&Llm::new(Cli::Pi, ""), stdout);
        assert_eq!((usage[0].tokens(), usage[0].model.as_str()), (50, "openai-codex/gpt-6.2-sol"));
        let failed = r#"{"type":"turn_end","message":{"content":[],"stopReason":"error","errorMessage":"No API key"}}"#;
        assert!(reply(failed).unwrap_err().to_string().contains("No API key"));
    }
}
