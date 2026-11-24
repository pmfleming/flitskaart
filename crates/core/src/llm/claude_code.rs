//! Claude Code: `claude -p` with a JSON schema it enforces itself.

use anyhow::{Context as _, Result, bail};

use super::{Llm, Usage};

pub(super) fn ask(llm: &Llm, system: &str, schema: &str, request: &str) -> Result<serde_json::Value> {
    answer(&run(llm, system, schema, request, &["--tools", ""])?)
}

/// Research only has access to public web search and fetch.
pub(super) fn research(llm: &Llm, system: &str, schema: &str, request: &str) -> Result<serde_json::Value> {
    let tools = "WebSearch,WebFetch";
    let stdout = run(llm, system, schema, request, &["--tools", tools, "--allowed-tools", tools, "--permission-mode", "dontAsk", "--max-turns", "6"])?;
    let result = answer(&stdout)?;
    let envelope: serde_json::Value = serde_json::from_str(&stdout)?;
    let searched = envelope["modelUsage"]
        .as_object()
        .is_some_and(|models| models.values().any(|usage| usage["webSearchRequests"].as_u64().unwrap_or(0) > 0))
        || envelope["usage"]["server_tool_use"]["web_search_requests"].as_u64().unwrap_or(0) > 0;
    if !searched {
        bail!("Claude Code heeft geen webzoekopdracht uitgevoerd.");
    }
    Ok(result)
}

fn run(llm: &Llm, system: &str, schema: &str, request: &str, tool_args: &[&str]) -> Result<String> {
    let mut args = vec!["-p", "--model", &llm.model, "--effort", llm.effort.as_str(), "--output-format", "json", "--json-schema", schema];
    args.extend(["--system-prompt", system]);
    args.extend(tool_args);
    args.extend(["--strict-mcp-config", "--disable-slash-commands", "--no-session-persistence"]);
    let ran = llm.run(&args, request)?;
    llm.note(usage_in(llm, &ran.stdout));
    if !ran.ok && ran.stdout.trim().is_empty() {
        bail!("Claude Code gaf een fout: {}", ran.problem);
    }
    Ok(ran.stdout)
}

/// The tokens each model used, from the envelope's `modelUsage`; nothing if
/// the output can't be read.
pub(super) fn usage_in(llm: &Llm, stdout: &str) -> Vec<Usage> {
    let Ok(envelope) = serde_json::from_str::<serde_json::Value>(stdout.trim()) else { return Vec::new() };
    let Some(models) = envelope["modelUsage"].as_object() else { return Vec::new() };
    let count = |usage: &serde_json::Value, key: &str| usage[key].as_u64().unwrap_or(0);
    models
        .iter()
        .map(|(model, usage)| Usage {
            input: count(usage, "inputTokens"),
            cache_read: count(usage, "cacheReadInputTokens"),
            cache_write: count(usage, "cacheCreationInputTokens"),
            output: count(usage, "outputTokens"),
            ..llm.usage_of(model.clone())
        })
        .filter(|usage| usage.tokens() > 0)
        .collect()
}

/// Reads the answer out of Claude Code's `--output-format json` envelope.
pub(crate) fn answer(stdout: &str) -> Result<serde_json::Value> {
    let mut envelope: serde_json::Value = serde_json::from_str(stdout.trim()).context("Claude Code gaf geen leesbaar antwoord")?;
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
    use super::{answer, usage_in};
    use crate::llm::{Cli, Effort, Llm};

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
    fn notes_each_models_tokens() {
        let llm = Llm { effort: Effort::High, ..Llm::new(Cli::ClaudeCode, "haiku") };
        let stdout = r#"{"is_error":false,"result":"x","modelUsage":{"claude-haiku-5-5":{"inputTokens":1177,"outputTokens":20,"cacheReadInputTokens":649,"cacheCreationInputTokens":3276,"costUSD":0.00079},"claude-opus-5-5":{"inputTokens":0,"outputTokens":0}}}"#;
        let usage = usage_in(&llm, stdout);
        assert_eq!(usage.len(), 1, "a model that used nothing is left out");
        assert_eq!((usage[0].tokens(), usage[0].effort.as_str(), usage[0].cli.as_str()), (1177 + 20 + 649 + 3276, "high", "claude"));
        assert!(usage_in(&llm, "not json").is_empty());
    }
}
