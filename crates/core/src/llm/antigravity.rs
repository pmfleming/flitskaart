//! Antigravity (`agy`), Google's tool for the Gemini models. It holds the
//! answer to the schema itself, but takes the prompt only as an argument and
//! has no separate system prompt, so the instructions go in front.

use anyhow::{Result, bail};
use serde_json::Value;

use super::{Effort, Llm, Usage, without_nulls};

pub(super) fn ask(llm: &Llm, system: &str, schema: &str, request: &str) -> Result<Value> {
    let prompt = format!("{system}\n\nAntwoord direct, zonder gereedschap te gebruiken.\n\n{request}");
    let mut args = vec!["--output-format", "json", "--json-schema", schema, "--disable-slash-commands"];
    if !llm.model.is_empty() {
        args.extend(["--model", &llm.model]);
    }
    // Most models carry their effort in their name; the others take it apart.
    if !has_effort(&llm.model) {
        args.extend(["--effort", llm.effort.as_str()]);
    }
    args.extend(["--print", &prompt]);
    let ran = llm.run(&args, "")?;
    llm.note(usage_in(llm, &ran.stdout));
    ran.reply(llm.cli, reply)
}

/// Whether a model's name ends in an effort, such as `gemini-4-neon-high`.
pub(super) fn has_effort(model: &str) -> bool {
    split_effort(model).1.is_some()
}

/// `gemini-4-neon-high` → (`gemini-4-neon`, High).
pub(super) fn split_effort(model: &str) -> (&str, Option<Effort>) {
    model.rsplit_once('-').and_then(|(base, suffix)| Effort::parse(suffix).map(|effort| (base, Some(effort)))).unwrap_or((model, None))
}

fn reply(stdout: &str) -> Result<Value> {
    let mut envelope: Value = serde_json::from_str(stdout.trim()).map_err(|_| anyhow::anyhow!("Antigravity gaf geen leesbaar antwoord"))?;
    if envelope["status"] != "SUCCESS" {
        bail!("Antigravity gaf een fout: {}", envelope["error"].as_str().unwrap_or("onbekende fout"));
    }
    match envelope["structured_output"].take() {
        Value::Null => super::json_in(envelope["response"].as_str().unwrap_or_default()).map(without_nulls),
        structured => Ok(without_nulls(structured)),
    }
}

fn usage_in(llm: &Llm, stdout: &str) -> Vec<Usage> {
    let Ok(envelope) = serde_json::from_str::<Value>(stdout.trim()) else { return Vec::new() };
    let count = |key: &str| envelope["usage"][key].as_u64().unwrap_or(0);
    // The cached input is counted within the input; the output includes the thinking.
    let usage = Usage {
        input: count("input_tokens").saturating_sub(count("cache_read_tokens")),
        cache_read: count("cache_read_tokens"),
        output: count("output_tokens"),
        ..llm.usage_of(if llm.model.is_empty() { "antigravity" } else { &llm.model })
    };
    if usage.tokens() > 0 { vec![usage] } else { Vec::new() }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{reply, split_effort, usage_in};
    use crate::llm::{Cli, Effort, Llm};

    #[test]
    fn reads_the_answer_errors_usage_and_efforts() {
        let stdout = r#"{"status":"SUCCESS","response":"{\"word\":\"hallo\"}\n","structured_output":{"word":"hallo"},"usage":{"input_tokens":10121,"output_tokens":127,"thinking_tokens":110,"cache_read_tokens":0,"total_tokens":10248}}"#;
        assert_eq!(reply(stdout).unwrap(), json!({"word": "hallo"}));
        let usage = usage_in(&Llm::new(Cli::Antigravity, "gemini-3.8-flash-low"), stdout);
        assert_eq!((usage[0].tokens(), usage[0].cli.as_str()), (10248, "agy"));
        assert!(reply(r#"{"status":"ERROR","error":"Error: empty prompt"}"#).unwrap_err().to_string().contains("empty prompt"));
        assert_eq!(split_effort("gemini-4-neon-xhigh"), ("gemini-4-neon", Some(Effort::XHigh)));
        assert_eq!(split_effort("gemini-pro-orchestrator"), ("gemini-pro-orchestrator", None));
    }
}
