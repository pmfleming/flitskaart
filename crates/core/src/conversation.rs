//! Gesprek: a conversation in Dutch about a topic the learner picks. Claude
//! plays the other person. Every sentence the learner types or says comes back
//! corrected along with the next line.
//!
//! Sentences aren't rated. What becomes a card follows from what happened:
//! Dutch the learner didn't know and said in English, partner lines they
//! didn't understand (they said so, or opened the meaning), and their
//! corrected mistakes.

use anyhow::{Context as _, Result, bail};

use crate::claude::Claude;
use crate::model::normalize;

/// The topic Claude picks when the learner leaves it open.
const ANY_TOPIC: &str = "something everyday of your choice";

/// How much of the conversation goes with each request; older lines are left out.
const HISTORY: usize = 30;

const INSTRUCTIONS: &str = r#"You are a friendly conversation partner for someone learning Dutch (around level A2 to B1). You chat with them in Dutch about a topic they chose. You never switch to English with them.

Each turn you do three things.

1. Correct the learner's newest sentence.
- "corrected": that sentence as a native speaker from the Netherlands would naturally say it. Change as little as possible: fix grammar, word order, spelling, wrong words and unnatural phrasing, and keep the learner's meaning.
- If part of it is in English (or another language) because the learner did not know the Dutch, put that part into Dutch.
- If it was already correct and natural, repeat it exactly.
- If the sentence was spoken and written down by speech recognition, do not correct capitalisation or punctuation, and leave out obvious recognition slips that a listener would not notice.
- "explanation": at most one short sentence in simple Dutch (A2) saying what was wrong and why. An empty string if nothing was wrong.

2. Notice what the learner did not know.
- "english_used": every part of the newest sentence the learner said in English (or another language) instead of Dutch, each with "english" (exactly as they said it) and "dutch" (the natural Dutch for it, nouns with de or het). An empty list if they only used Dutch. Ordinary loanwords that Dutch people also use don't count.
- "not_understood": if the newest sentence says or shows that the learner did not understand your previous line (for example "Ik begrijp het niet", "Wat betekent ontbijt?", "Sorry?", or the same in English), the Dutch words or phrases from your previous line that they did not understand, written as in that line. If it is unclear which part, give your whole previous line. An empty list otherwise.

3. Continue the conversation.
- "reply": your next turn in Dutch. One or two short sentences, at most about 25 words, in simple everyday Dutch. React to what the learner meant, not to their mistakes, and end with a question or something they can easily answer.
- If the learner did not understand you, first say it again more simply in Dutch, then go on.
- Stay with the topic, but follow the learner if they take it somewhere.
- "reply_english": a natural English translation of "reply".

When the conversation has not started yet, leave "corrected" and "explanation" empty and both lists empty, and open the conversation yourself with a short greeting and a first question about the topic.
"#;

const SCHEMA: &str = r#"{
  "type": "object",
  "properties": {
    "corrected": {"type": "string"},
    "explanation": {"type": "string"},
    "english_used": {
      "type": "array",
      "items": {
        "type": "object",
        "properties": {"english": {"type": "string"}, "dutch": {"type": "string"}},
        "required": ["english", "dutch"],
        "additionalProperties": false
      }
    },
    "not_understood": {"type": "array", "items": {"type": "string"}},
    "reply": {"type": "string"},
    "reply_english": {"type": "string"}
  },
  "required": ["corrected", "explanation", "english_used", "not_understood", "reply", "reply_english"],
  "additionalProperties": false
}"#;

/// What drafting is told when the conversation becomes cards.
const CARD_STEER: &str = "Maak alleen kaarten van wat de leerling niet wist. \
Bij 'In het Engels gezegd': een kaart van het Nederlands, in een korte zin uit het gesprek (liefst de verbeterde zin). \
Bij 'Niet begrepen': een kaart van die woorden of uitdrukking, in de zin uit het gesprek. \
Bij 'Correctie': een kaart van de verbeterde zin, als die niet al een kaart is. \
Maak geen kaarten van de rest van het gesprek.";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Who {
    Partner,
    Learner,
}

/// Something the learner said in English, with the Dutch for it.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Deserialize)]
pub struct Translation {
    pub english: String,
    pub dutch: String,
}

/// One line of the conversation.
#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    pub who: Who,
    /// The partner's Dutch, or the learner's sentence as typed or heard.
    pub dutch: String,
    /// The partner's line in English, hidden until the learner asks.
    pub english: String,
    /// The learner's sentence as a native speaker would say it, when that differs.
    pub corrected: Option<String>,
    /// Why it was corrected, in simple Dutch. May be empty.
    pub explanation: String,
    /// The learner spoke the sentence, and Whisper wrote it down.
    pub spoken: bool,
    /// What the learner said in English in this sentence, with the Dutch.
    pub english_used: Vec<Translation>,
    /// The words of this partner line the learner said they didn't understand.
    pub not_understood: Vec<String>,
    /// The English of this partner line is showing.
    pub meaning_shown: bool,
    /// The learner opened the English of this partner line at some point.
    pub looked_up: bool,
}

impl Line {
    fn partner(dutch: String, english: String) -> Line {
        Line { who: Who::Partner, dutch, english, ..Line::learner(String::new(), false) }
    }

    fn learner(dutch: String, spoken: bool) -> Line {
        Line {
            who: Who::Learner,
            dutch,
            english: String::new(),
            corrected: None,
            explanation: String::new(),
            spoken,
            english_used: Vec::new(),
            not_understood: Vec::new(),
            meaning_shown: false,
            looked_up: false,
        }
    }

    /// What was meant: the correction if there is one, else what was said.
    pub fn meant(&self) -> &str {
        self.corrected.as_deref().unwrap_or(&self.dutch)
    }

    /// The learner wasn't understood here: they didn't follow this partner line.
    pub fn not_followed(&self) -> bool {
        self.looked_up || !self.not_understood.is_empty()
    }

    /// How many things in this line the learner didn't know, each worth a card.
    pub fn gaps(&self) -> usize {
        match self.who {
            Who::Partner => usize::from(self.not_followed()),
            // A correction that only puts the English into Dutch adds nothing.
            Who::Learner if self.english_used.is_empty() => usize::from(self.corrected.is_some()),
            Who::Learner => self.english_used.len(),
        }
    }
}

/// A sentence from the learner, typed or spoken.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Said {
    pub text: String,
    pub spoken: bool,
}

/// Claude's answer to one turn.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Deserialize)]
pub struct Reply {
    pub corrected: String,
    pub explanation: String,
    #[serde(default)]
    pub english_used: Vec<Translation>,
    #[serde(default)]
    pub not_understood: Vec<String>,
    pub reply: String,
    pub reply_english: String,
}

/// Asks Claude for the next turn. Blocks for several seconds, so call it off
/// the UI thread. `request` comes from [`Conversation::request`].
pub fn next_turn(claude: &Claude, request: &str) -> Result<Reply> {
    let reply: Reply = serde_json::from_value(claude.ask(INSTRUCTIONS, SCHEMA, request)?)
        .context("Claude Code antwoordde in een onverwachte vorm")?;
    if reply.reply.trim().is_empty() {
        bail!("Claude Code gaf geen antwoord in het gesprek.");
    }
    Ok(reply)
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Conversation {
    pub topic: String,
    pub lines: Vec<Line>,
}

impl Conversation {
    pub fn new(topic: &str) -> Conversation {
        Conversation { topic: topic.trim().to_string(), lines: Vec::new() }
    }

    /// The text sent to Claude for the next turn: the topic, the latest part of
    /// the conversation and the learner's new sentence (none for the opening).
    pub fn request(&self, said: Option<&Said>) -> String {
        let topic = if self.topic.is_empty() { ANY_TOPIC } else { &self.topic };
        let mut out = format!("Topic: {topic}\n\n");
        let Some(said) = said else {
            out.push_str("The conversation has not started yet. Open it.\n");
            return out;
        };
        out.push_str("<conversation>\n");
        for line in &self.lines[self.lines.len().saturating_sub(HISTORY)..] {
            let who = if line.who == Who::Partner { "You" } else { "Learner" };
            out.push_str(&format!("{who}: {}\n", line.meant()));
        }
        out.push_str("</conversation>\n\n");
        let how = if said.spoken { "spoken, written down by speech recognition" } else { "typed" };
        out.push_str(&format!("The learner's newest sentence ({how}):\n<learner>\n{}\n</learner>\n", said.text.trim()));
        out
    }

    /// Adds the learner's sentence, with its correction and the English they
    /// used, and the partner's reply. Words they didn't understand go with the
    /// partner line before it.
    pub fn add_turn(&mut self, said: Option<Said>, reply: Reply) {
        if let Some(said) = said {
            let unclear: Vec<String> = reply.not_understood.iter().map(|w| w.trim().to_string()).filter(|w| !w.is_empty()).collect();
            if let Some(asked) = self.lines.iter_mut().rev().find(|line| line.who == Who::Partner) {
                asked.not_understood.extend(unclear);
            }
            let mut line = Line::learner(said.text.trim().to_string(), said.spoken);
            let corrected = reply.corrected.trim();
            if !corrected.is_empty() && normalize(corrected) != normalize(&line.dutch) {
                line.corrected = Some(corrected.to_string());
                line.explanation = reply.explanation.trim().to_string();
            }
            line.english_used = reply
                .english_used
                .into_iter()
                .map(|t| Translation { english: t.english.trim().to_string(), dutch: t.dutch.trim().to_string() })
                .filter(|t| !t.english.is_empty() && !t.dutch.is_empty())
                .collect();
            self.lines.push(line);
        }
        self.lines.push(Line::partner(reply.reply.trim().to_string(), reply.reply_english.trim().to_string()));
    }

    /// Shows or hides the English of a partner's line. Once opened, the line
    /// counts as not understood.
    pub fn toggle_meaning(&mut self, index: usize) {
        if let Some(line) = self.lines.get_mut(index).filter(|line| line.who == Who::Partner) {
            line.meaning_shown = !line.meaning_shown;
            line.looked_up |= line.meaning_shown;
        }
    }

    /// How many things the learner didn't know, each worth a card.
    pub fn gaps(&self) -> usize {
        self.lines.iter().map(Line::gaps).sum()
    }

    /// Whether the learner has said anything yet.
    pub fn has_learner_lines(&self) -> bool {
        self.lines.iter().any(|line| line.who == Who::Learner)
    }

    /// The whole conversation as text, with what the learner didn't know under
    /// each line: what drafting reads, and what Concepten shows as the source.
    pub fn transcript(&self) -> String {
        let topic = if self.topic.is_empty() { "vrij onderwerp" } else { &self.topic };
        let mut out = format!("Gesprek over: {topic}\n");
        for line in &self.lines {
            let who = if line.who == Who::Partner { "Partner" } else { "Ik" };
            out.push_str(&format!("\n{who}: {}", line.dutch));
            if let Some(corrected) = &line.corrected {
                out.push_str(&format!("\n  Correctie: {corrected}"));
            }
            if !line.explanation.is_empty() {
                out.push_str(&format!("\n  Uitleg: {}", line.explanation));
            }
            for used in &line.english_used {
                out.push_str(&format!("\n  In het Engels gezegd: {} = {}", used.english, used.dutch));
            }
            if !line.not_understood.is_empty() {
                out.push_str(&format!("\n  Niet begrepen: {}", line.not_understood.join(", ")));
            } else if line.looked_up {
                out.push_str("\n  Niet begrepen: de hele zin (vertaling bekeken)");
            }
        }
        out.push('\n');
        out
    }

    /// The steering line and card limit for drafting what the learner didn't
    /// know, or `None` when there was nothing.
    pub fn card_request(&self) -> Option<(&'static str, usize)> {
        let gaps = self.gaps();
        (gaps > 0).then(|| (CARD_STEER, (gaps + 2).min(20)))
    }
}

#[cfg(test)]
mod tests {
    use super::{Conversation, Reply, Said, Translation, Who, next_turn};
    use crate::claude::Claude;

    fn reply(corrected: &str, explanation: &str, next: &str) -> Reply {
        Reply { corrected: corrected.into(), explanation: explanation.into(), reply: next.into(), reply_english: "English".into(), ..Reply::default() }
    }

    fn typed(text: &str) -> Said {
        Said { text: text.into(), spoken: false }
    }

    fn started() -> Conversation {
        let mut conversation = Conversation::new(" het weekend ");
        conversation.add_turn(None, reply("", "", "Hoi! Wat heb je dit weekend gedaan?"));
        conversation
    }

    #[test]
    fn the_opening_and_each_turn_are_asked_for_with_the_conversation_so_far() {
        let open = Conversation::new("").request(None);
        assert!(open.contains("of your choice") && open.contains("not started"), "{open}");
        let mut conversation = started();
        conversation.add_turn(Some(typed("Ik heb naar de markt gegaan.")), reply("Ik ben naar de markt gegaan.", "Gaan: zijn.", "Leuk!"));
        let request = conversation.request(Some(&Said { text: " ik koop kaas ".into(), spoken: true }));
        assert!(request.starts_with("Topic: het weekend\n"));
        assert!(request.contains("You: Hoi!") && request.contains("Learner: Ik ben naar de markt gegaan."), "what was meant: {request}");
        assert!(request.contains("(spoken, written down by speech recognition):\n<learner>\nik koop kaas\n</learner>"), "{request}");
    }

    #[test]
    fn corrections_count_but_punctuation_alone_is_not_a_correction() {
        let mut conversation = started();
        conversation.add_turn(Some(typed("Ik heb naar de markt gegaan")), reply("Ik ben naar de markt gegaan.", "Bij gaan hoort zijn.", "Wat heb je gekocht?"));
        conversation.add_turn(Some(typed("ik heb kaas gekocht")), reply("Ik heb kaas gekocht.", "Hoofdletter.", "Lekker!"));
        let whos: Vec<Who> = conversation.lines.iter().map(|line| line.who).collect();
        assert_eq!(whos, [Who::Partner, Who::Learner, Who::Partner, Who::Learner, Who::Partner]);
        let (fixed, fine) = (&conversation.lines[1], &conversation.lines[3]);
        assert_eq!((fixed.corrected.as_deref(), fixed.explanation.as_str()), (Some("Ik ben naar de markt gegaan."), "Bij gaan hoort zijn."));
        assert_eq!((fine.corrected.as_deref(), fine.explanation.as_str()), (None, ""));
        assert_eq!(conversation.gaps(), 1);
    }

    #[test]
    fn english_used_and_lines_not_understood_are_what_the_learner_did_not_know() {
        let mut conversation = started();
        assert_eq!(conversation.card_request(), None, "nothing yet");
        let mut answer = reply("Wat betekent gedaan? Ik heb een fiets gekocht.", "", "Gedaan: wat je deed. Wat voor fiets?");
        answer.english_used = vec![Translation { english: " bike ".into(), dutch: "de fiets".into() }, Translation::default()];
        answer.not_understood = vec!["gedaan".into(), " ".into()];
        conversation.add_turn(Some(typed("Wat betekent gedaan? Ik heb een bike gekocht.")), answer);
        let (asked, said) = (&conversation.lines[0], &conversation.lines[1]);
        assert_eq!(asked.not_understood, ["gedaan"], "kept with the partner line it was about");
        assert_eq!(said.english_used, [Translation { english: "bike".into(), dutch: "de fiets".into() }], "blanks dropped");
        assert_eq!(conversation.gaps(), 2, "the correction only put the English into Dutch");

        conversation.toggle_meaning(2);
        conversation.toggle_meaning(2);
        assert!(conversation.lines[2].looked_up && !conversation.lines[2].meaning_shown, "closing the meaning again still counts");
        conversation.toggle_meaning(1);
        assert!(!conversation.lines[1].looked_up, "the learner's own lines have no meaning to open");
        assert!(conversation.card_request().is_some_and(|(steer, limit)| steer.contains("In het Engels gezegd") && limit == 5));
    }

    #[test]
    fn the_transcript_notes_what_the_learner_did_not_know() {
        let mut conversation = started();
        assert!(!conversation.has_learner_lines());
        let mut answer = reply("Ik ben naar de markt gegaan.", "Bij gaan hoort zijn.", "Leuk!");
        answer.english_used = vec![Translation { english: "market".into(), dutch: "de markt".into() }];
        conversation.add_turn(Some(typed("Ik heb naar de market gegaan.")), answer);
        conversation.toggle_meaning(2);
        assert!(conversation.has_learner_lines());
        assert_eq!(
            conversation.transcript(),
            "Gesprek over: het weekend\n\nPartner: Hoi! Wat heb je dit weekend gedaan?\n\
             Ik: Ik heb naar de market gegaan.\n  Correctie: Ik ben naar de markt gegaan.\n  Uitleg: Bij gaan hoort zijn.\n\
             \x20 In het Engels gezegd: market = de markt\n\
             Partner: Leuk!\n  Niet begrepen: de hele zin (vertaling bekeken)\n"
        );
    }

    #[test]
    fn an_empty_reply_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let program = dir.path().join("claude");
        let envelope = r#"{"is_error":false,"result":"","structured_output":{"corrected":"","explanation":"","reply":" ","reply_english":""}}"#;
        std::fs::write(&program, format!("#!/bin/sh\ncat > /dev/null\necho '{envelope}'\n")).unwrap();
        std::fs::set_permissions(&program, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
        let claude = Claude { program, model: "sonnet".into() };
        assert!(next_turn(&claude, "Topic: x").unwrap_err().to_string().contains("geen antwoord"));
    }
}
