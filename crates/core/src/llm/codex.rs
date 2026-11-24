//! Codex: `codex exec` with the schema in a file. OpenAI holds the answer to
//! the schema strictly, so the schema is first made strict: every property
//! required, the optional ones allowed to be `null` (dropped again after).

use anyhow::{Context as _, Result, bail};
use serde_json::{Value, json};

use super::{Llm, Usage, json_in, without_nulls};

pub(super) fn ask(llm: &Llm, system: &str, schema: &str, request: &str) -> Result<Value> {
    let schema: Value = serde_json::from_str(schema).context("het JSON-schema is ongeldig")?;
    let file = tempfile::NamedTempFile::new().context("het schema kon niet worden aangemaakt")?;
    serde_json::to_writer(file.as_file(), &strict(schema)).context("het schema kon niet worden weggeschreven")?;
    let path = file.path().to_str().context("het pad naar het schema is geen UTF-8")?;
    let effort = format!("model_reasoning_effort=\"{}\"", llm.effort.as_str());
    let mut args = vec!["exec", "--json", "--skip-git-repo-check", "--ephemeral", "--sandbox", "read-only", "--color", "never"];
    args.extend(["--output-schema", path, "-c", &effort]);
    if !llm.model.is_empty() {
        args.extend(["--model", &llm.model]);
    }
    args.push("-");
    let input = format!("{system}\n\nAntwoord direct, zonder gereedschap te gebruiken.\n\n{request}");
    let ran = llm.run(&args, &input)?;
    llm.note(usage_in(llm, &ran.stdout));
    ran.reply(llm.cli, reply)
}

/// The last message's JSON, or the error Codex reported.
fn reply(stdout: &str) -> Result<Value> {
    let mut answer = None;
    for event in stdout.lines().filter_map(|line| serde_json::from_str::<Value>(line).ok()) {
        match event["type"].as_str() {
            Some("item.completed") if event["item"]["type"] == "agent_message" => answer = event["item"]["text"].as_str().map(String::from),
            Some("turn.failed") => bail!("Codex gaf een fout: {}", message(&event["error"]["message"])),
            Some("error") => bail!("Codex gaf een fout: {}", message(&event["message"])),
            _ => {}
        }
    }
    Ok(without_nulls(json_in(&answer.context("Codex gaf geen antwoord")?)?))
}

/// An error message, which may itself be an API error in JSON.
fn message(value: &Value) -> String {
    let text = value.as_str().unwrap_or("onbekende fout");
    serde_json::from_str::<Value>(text).ok().and_then(|inner| inner["error"]["message"].as_str().map(String::from)).unwrap_or_else(|| text.into())
}

fn usage_in(llm: &Llm, stdout: &str) -> Vec<Usage> {
    let events = stdout.lines().filter_map(|line| serde_json::from_str::<Value>(line).ok());
    let turns = events.filter(|event| event["type"] == "turn.completed");
    turns
        .map(|event| {
            let count = |key: &str| event["usage"][key].as_u64().unwrap_or(0);
            // Codex counts the cached input within the input.
            Usage {
                input: count("input_tokens").saturating_sub(count("cached_input_tokens")),
                cache_read: count("cached_input_tokens"),
                cache_write: count("cache_write_input_tokens"),
                output: count("output_tokens"),
                ..llm.usage_of(if llm.model.is_empty() { "codex" } else { &llm.model })
            }
        })
        .filter(|usage| usage.tokens() > 0)
        .collect()
}

/// The schema as OpenAI's strict mode wants it.
fn strict(mut schema: Value) -> Value {
    let Some(object) = schema.as_object_mut() else { return schema };
    let mut required = object.remove("required");
    if let Some(fields) = object.get_mut("properties").and_then(Value::as_object_mut) {
        let keys: Vec<_> = fields.keys().map(|key| json!(key)).collect();
        let was_required = required.as_ref().and_then(Value::as_array).map_or(&[][..], Vec::as_slice);
        for ((_, field), key) in fields.iter_mut().zip(&keys) {
            let value = strict(field.take());
            *field = if was_required.contains(key) { value } else { nullable(value) };
        }
        required = Some(Value::Array(keys));
        object.insert("additionalProperties".into(), Value::Bool(false));
    }
    if let Some(required) = required {
        object.insert("required".into(), required);
    }
    if let Some(items) = object.get_mut("items") {
        *items = strict(items.take());
    }
    schema
}

/// Lets an optional property be `null`.
fn nullable(mut schema: Value) -> Value {
    let Some(kind) = schema.get_mut("type") else { return schema };
    match kind {
        Value::String(_) => *kind = json!([kind.take(), "null"]),
        Value::Array(kinds) if !kinds.contains(&json!("null")) => kinds.push(json!("null")),
        _ => {}
    }
    schema
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{reply, strict, usage_in};
    use crate::llm::{Cli, Llm};

    #[test]
    fn makes_a_schema_strict_and_optional_fields_nullable() {
        let schema = json!({"type": "object", "properties": {"a": {"type": "string"}, "b": {"type": "array", "items": {"type": "object", "properties": {"c": {"type": "integer"}}}}}, "required": ["a"]});
        let strict = strict(schema);
        assert_eq!(strict["required"], json!(["a", "b"]));
        assert_eq!(strict["additionalProperties"], json!(false));
        assert_eq!(strict["properties"]["b"]["type"], json!(["array", "null"]));
        assert_eq!(strict["properties"]["b"]["items"]["required"], json!(["c"]));
        assert_eq!(strict["properties"]["b"]["items"]["properties"]["c"]["type"], json!(["integer", "null"]));
    }

    #[test]
    fn reads_the_answer_errors_and_usage() {
        let stdout = r#"{"type":"turn.started"}
{"type":"item.completed","item":{"id":"item_0","type":"agent_message","text":"{\"word\":\"hallo\",\"note\":null}"}}
{"type":"turn.completed","usage":{"input_tokens":14848,"cached_input_tokens":848,"cache_write_input_tokens":0,"output_tokens":16}}"#;
        assert_eq!(reply(stdout).unwrap(), json!({"word": "hallo"}));
        let usage = usage_in(&Llm::new(Cli::Codex, "gpt-6.2-sol"), stdout);
        assert_eq!((usage[0].input, usage[0].cache_read, usage[0].output, usage[0].model.as_str()), (14000, 848, 16, "gpt-6.2-sol"));
        let failed = r#"{"type":"turn.failed","error":{"message":"{\"error\":{\"message\":\"Invalid schema\"}}"}}"#;
        assert!(reply(failed).unwrap_err().to_string().contains("Invalid schema"));
    }
}
