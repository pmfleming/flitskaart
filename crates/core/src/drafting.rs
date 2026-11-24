//! Turns a pasted piece of conversation into draft cards, using Claude Code
//! on this laptop (the Claude subscription, no API key).

use anyhow::{Context as _, Result, bail};

use crate::claude::Claude;
use crate::model::NewCard;

pub const DEFAULT_LIMIT: usize = 6;

const INSTRUCTIONS: &str = r#"You turn a snippet of a Dutch-learning conversation into flashcards.
The learner pastes part of a chat they had with an AI tutor (Claude, ChatGPT or Gemini) while practising Dutch.
The learner studies Dutch only: the card face is always Dutch, the English is hidden and shown only on request.

What to look for, in this order of priority:
1. Corrections of the learner's own mistakes. Make a card of the CORRECTED Dutch sentence, never the mistake.
2. Useful new phrases or sentences that appear in the conversation.
3. Single new words. Put each one in a short, natural example sentence, ideally the one from the conversation.

Rules for every card:
- "dutch": one natural phrase or sentence a person would actually say, correctly spelled and punctuated. Keep it short (at most about 15 words).
- Nouns that stand alone must carry their article (de or het).
- "english": a natural English translation of the Dutch.
- "note": optional. At most one short sentence in SIMPLE Dutch (A2 level) explaining the point, for example why the correction is right. Use an empty string when nothing useful can be said.
- "tag": one short lowercase Dutch topic word, such as "restaurant", "werk", "grammatica". Empty string if unclear.
- Only use Dutch that appears in, or follows directly from, the snippet. Do not invent unrelated material.
- Skip greetings and filler unless they are genuinely useful. Never make two cards with the same Dutch.
- Return at most the requested number of cards, best first. Return an empty list if the snippet has nothing worth learning.
"#;

const SCHEMA: &str = r#"{
  "type": "object",
  "properties": {
    "cards": {
      "type": "array",
      "items": {
        "type": "object",
        "properties": {
          "dutch": {"type": "string"},
          "english": {"type": "string"},
          "note": {"type": "string"},
          "tag": {"type": "string"}
        },
        "required": ["dutch", "english", "note", "tag"],
        "additionalProperties": false
      }
    },
    "recap": {"type": "string"}
  },
  "required": ["cards"],
  "additionalProperties": false
}"#;

#[derive(serde::Deserialize)]
struct Reply {
    cards: Vec<NewCard>,
}

/// Lines typed as `Nederlands | English | uitleg` skip Claude entirely.
/// Returns `None` unless every non-empty line has that shape.
pub fn parse_manual(text: &str) -> Option<Vec<NewCard>> {
    let lines: Vec<&str> = text.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    if lines.is_empty() || !lines.iter().all(|l| l.contains('|')) {
        return None;
    }
    let cards: Vec<NewCard> = lines
        .iter()
        .map(|line| {
            let mut parts = line.split('|').map(str::trim);
            NewCard {
                dutch: parts.next().unwrap_or_default().to_string(),
                english: parts.next().unwrap_or_default().to_string(),
                note: parts.next().unwrap_or_default().to_string(),
                tag: String::new(),
            }
        })
        .filter(|card| !card.dutch.is_empty())
        .collect();
    (!cards.is_empty()).then_some(cards)
}

/// The text sent to Claude for one snippet.
fn request_text(snippet: &str, steer: &str, limit: usize) -> String {
    let steer = steer.trim();
    let mut out = format!("Make at most {limit} cards.\n");
    if !steer.is_empty() {
        out.push_str(&format!("The learner adds this instruction (follow it): {steer}\n"));
    }
    out.push_str("\n<snippet>\n");
    out.push_str(snippet.trim());
    out.push_str("\n</snippet>\n");
    out
}

/// Asks Claude Code for draft cards. Blocks for several seconds, so call it off
/// the UI thread.
pub fn draft_cards(claude: &Claude, snippet: &str, steer: &str, limit: usize) -> Result<Vec<NewCard>> {
    if snippet.trim().is_empty() {
        bail!("Plak eerst een stuk van een gesprek.");
    }
    cards_from(claude.ask(INSTRUCTIONS, SCHEMA, &request_text(snippet, steer, limit))?, limit)
}

/// Reviews a finished Gesprek in the same call that extracts its cards.
pub fn review_conversation(claude: &Claude, transcript: &str, steer: &str, limit: usize) -> Result<(Vec<NewCard>, String)> {
    let request = request_text(transcript, steer, limit);
    let request = format!(
        "This is a finished Gesprek session. Also write \"recap\" in simple Dutch, directly addressing the learner as \"je\". Give one short sentence about what they discussed and, if there was a clear learning need, one phrase worth remembering. Base it on the learner's own lines.\n{request}"
    );
    let answer = claude.ask(INSTRUCTIONS, SCHEMA, &request)?;
    let recap = answer["recap"].as_str().unwrap_or_default().trim().chars().take(400).collect();
    Ok((cards_from(answer, limit)?, recap))
}

/// The usable cards in Claude's answer: no blanks or repeats, at most `limit`.
fn cards_from(answer: serde_json::Value, limit: usize) -> Result<Vec<NewCard>> {
    let reply: Reply = serde_json::from_value(answer).context("Claude Code gaf kaarten in een onverwachte vorm")?;
    let mut seen = std::collections::HashSet::new();
    Ok(reply
        .cards
        .into_iter()
        .filter(|card| !card.dutch.trim().is_empty())
        .filter(|card| seen.insert(crate::model::normalize(&card.dutch)))
        .take(limit)
        .collect())
}

#[cfg(test)]
mod tests {
    use super::{NewCard, cards_from, parse_manual, request_text};
    use crate::claude::answer;

    fn parse_reply(stdout: &str, limit: usize) -> anyhow::Result<Vec<NewCard>> {
        cards_from(answer(stdout)?, limit)
    }

    #[test]
    fn manual_lines_become_cards() {
        let cards = parse_manual("Ik heb er zin in. | I'm looking forward to it\n\nDag! | Bye | informeel").unwrap();
        assert_eq!(cards.len(), 2);
        assert_eq!(cards[0].dutch, "Ik heb er zin in.");
        assert_eq!(cards[0].english, "I'm looking forward to it");
        assert_eq!(cards[1].note, "informeel");
    }

    #[test]
    fn conversation_text_is_not_manual() {
        assert!(parse_manual("Ik: Ik woon voor drie jaar in Utrecht.\nClaude: Bijna!").is_none());
        assert!(parse_manual("Ik woon hier | I live here\nClaude: goed zo").is_none());
        assert!(parse_manual("   ").is_none());
    }

    #[test]
    fn reads_claude_codes_reply() {
        let stdout = r#"{"type":"result","is_error":false,"result":"","structured_output":{"cards":[
            {"dutch":"Ik woon al drie jaar in Utrecht.","english":"I have lived in Utrecht for three years.","note":"Met al: tegenwoordige tijd.","tag":"grammatica"},
            {"dutch":"ik woon al drie jaar in utrecht","english":"dup","note":"","tag":""},
            {"dutch":"  ","english":"blank","note":"","tag":""}]}}"#;
        let cards = parse_reply(stdout, 6).unwrap();
        assert_eq!(cards.len(), 1);
        assert_eq!(cards[0].tag, "grammatica");
        // The cards may come in the result text instead; the limit holds.
        let stdout = r#"{"is_error":false,"result":"{\"cards\":[{\"dutch\":\"een\",\"english\":\"one\",\"note\":\"\",\"tag\":\"\"},{\"dutch\":\"twee\",\"english\":\"two\",\"note\":\"\",\"tag\":\"\"}]}"}"#;
        assert_eq!(parse_reply(stdout, 1).unwrap().len(), 1);
        assert!(parse_reply(r#"{"is_error":true,"result":"Not logged in"}"#, 6).unwrap_err().to_string().contains("Not logged in"));
        assert!(parse_reply("not json", 6).is_err());
    }

    #[test]
    fn request_includes_limit_and_steering() {
        let text = request_text("Ik: hoi", "alleen de correcties", 3);
        assert!(text.contains("at most 3"));
        assert!(text.contains("alleen de correcties"));
        assert!(text.contains("<snippet>\nIk: hoi\n</snippet>"));
    }
}
