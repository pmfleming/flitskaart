//! Gesprek: a conversation in Dutch about a topic the learner picks. Claude
//! plays the other person. Every sentence the learner types or says comes back
//! corrected along with the next line, and at the end the lines the learner
//! found difficult become draft cards.

use anyhow::{Context as _, Result, bail};

use crate::claude::Claude;
use crate::model::normalize;

/// The topic Claude picks when the learner leaves it open.
const ANY_TOPIC: &str = "something everyday of your choice";

/// How much of the conversation goes with each request; older lines are left out.
const HISTORY: usize = 30;

const INSTRUCTIONS: &str = r#"You are a friendly conversation partner for someone learning Dutch (around level A2 to B1). You chat with them in Dutch about a topic they chose. You never switch to English with them.

Each turn you do two things.

1. Correct the learner's newest sentence.
- "corrected": that sentence as a native speaker from the Netherlands would naturally say it. Change as little as possible: fix grammar, word order, spelling, wrong words and unnatural phrasing, and keep the learner's meaning.
- If part of it is in English because the learner did not know the Dutch, put that part into Dutch.
- If it was already correct and natural, repeat it exactly.
- If the sentence was spoken and written down by speech recognition, do not correct capitalisation or punctuation, and leave out obvious recognition slips that a listener would not notice.
- "explanation": at most one short sentence in simple Dutch (A2) saying what was wrong and why. An empty string if nothing was wrong.

2. Continue the conversation.
- "reply": your next turn in Dutch. One or two short sentences, at most about 25 words, in simple everyday Dutch. React to what the learner meant, not to their mistakes, and end with a question or something they can easily answer.
- Stay with the topic, but follow the learner if they take it somewhere.
- "reply_english": a natural English translation of "reply".

When the conversation has not started yet, leave "corrected" and "explanation" empty and open it yourself with a short greeting and a first question about the topic.
"#;

const SCHEMA: &str = r#"{
  "type": "object",
  "properties": {
    "corrected": {"type": "string"},
    "explanation": {"type": "string"},
    "reply": {"type": "string"},
    "reply_english": {"type": "string"}
  },
  "required": ["corrected", "explanation", "reply", "reply_english"],
  "additionalProperties": false
}"#;

/// What drafting is told when the conversation becomes cards.
const CARD_STEER: &str = "Maak alleen kaarten van de regels met [MOEILIJK]. \
Bij een regel van Ik met een Correctie: een kaart van de verbeterde zin. \
Bij een regel van Partner: een kaart van die zin, of van de lastige uitdrukking eruit.";

/// The mark on a difficult line in the transcript.
const MARK: &str = " [MOEILIJK]";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Who {
    Partner,
    Learner,
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
    /// The learner opened the meaning of the partner's line.
    pub meaning_opened: bool,
    /// Becomes a card at the end. Corrections and opened meanings start marked.
    pub difficult: bool,
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
            meaning_opened: false,
            difficult: false,
        }
    }

    /// What was meant: the correction if there is one, else what was said.
    pub fn meant(&self) -> &str {
        self.corrected.as_deref().unwrap_or(&self.dutch)
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

    /// Adds the learner's sentence, with its correction, and the partner's reply.
    pub fn add_turn(&mut self, said: Option<Said>, reply: Reply) {
        if let Some(said) = said {
            let mut line = Line::learner(said.text.trim().to_string(), said.spoken);
            let corrected = reply.corrected.trim();
            if !corrected.is_empty() && normalize(corrected) != normalize(&line.dutch) {
                (line.corrected, line.difficult) = (Some(corrected.to_string()), true);
                line.explanation = reply.explanation.trim().to_string();
            }
            self.lines.push(line);
        }
        self.lines.push(Line::partner(reply.reply.trim().to_string(), reply.reply_english.trim().to_string()));
    }

    /// Shows or hides the meaning of a partner's line. Opening it marks the line difficult.
    pub fn toggle_meaning(&mut self, index: usize) {
        if let Some(line) = self.lines.get_mut(index).filter(|line| line.who == Who::Partner) {
            line.meaning_opened = !line.meaning_opened;
            line.difficult |= line.meaning_opened;
        }
    }

    pub fn toggle_difficult(&mut self, index: usize) {
        if let Some(line) = self.lines.get_mut(index) {
            line.difficult = !line.difficult;
        }
    }

    /// How many lines will become cards.
    pub fn difficult(&self) -> usize {
        self.lines.iter().filter(|line| line.difficult).count()
    }

    /// Whether the learner has said anything yet.
    pub fn has_learner_lines(&self) -> bool {
        self.lines.iter().any(|line| line.who == Who::Learner)
    }

    /// The whole conversation as text, difficult lines marked: what drafting
    /// reads, and what Concepten shows as the cards' source.
    pub fn transcript(&self) -> String {
        let topic = if self.topic.is_empty() { "vrij onderwerp" } else { &self.topic };
        let mut out = format!("Gesprek over: {topic}\n");
        for line in &self.lines {
            let mark = if line.difficult { MARK } else { "" };
            let who = if line.who == Who::Partner { "Partner" } else { "Ik" };
            out.push_str(&format!("\n{who}: {}{mark}", line.dutch));
            if let Some(corrected) = &line.corrected {
                out.push_str(&format!("\nCorrectie: {corrected}"));
            }
            if !line.explanation.is_empty() {
                out.push_str(&format!("\nUitleg: {}", line.explanation));
            }
        }
        out.push('\n');
        out
    }

    /// The steering line and card limit for drafting the difficult lines, or
    /// `None` when nothing was marked.
    pub fn card_request(&self) -> Option<(&'static str, usize)> {
        let marked = self.difficult();
        (marked > 0).then(|| (CARD_STEER, (marked + 2).min(20)))
    }
}

#[cfg(test)]
mod tests {
    use super::{Conversation, Reply, Said, Who, next_turn};
    use crate::claude::Claude;

    fn reply(corrected: &str, explanation: &str, next: &str) -> Reply {
        Reply { corrected: corrected.into(), explanation: explanation.into(), reply: next.into(), reply_english: "English".into() }
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
    fn corrections_are_kept_and_marked_but_punctuation_alone_is_not_a_correction() {
        let mut conversation = started();
        conversation.add_turn(Some(typed("Ik heb naar de markt gegaan")), reply("Ik ben naar de markt gegaan.", "Bij gaan hoort zijn.", "Wat heb je gekocht?"));
        conversation.add_turn(Some(typed("ik heb kaas gekocht")), reply("Ik heb kaas gekocht.", "Hoofdletter.", "Lekker!"));
        let whos: Vec<Who> = conversation.lines.iter().map(|line| line.who).collect();
        assert_eq!(whos, [Who::Partner, Who::Learner, Who::Partner, Who::Learner, Who::Partner]);
        let (fixed, fine) = (&conversation.lines[1], &conversation.lines[3]);
        assert_eq!((fixed.corrected.as_deref(), fixed.difficult), (Some("Ik ben naar de markt gegaan."), true));
        assert_eq!(fixed.explanation, "Bij gaan hoort zijn.");
        assert_eq!((fine.corrected.as_deref(), fine.difficult, fine.explanation.as_str()), (None, false, ""));
        assert_eq!(conversation.difficult(), 1);
    }

    #[test]
    fn opening_a_meaning_or_marking_a_line_makes_it_a_card() {
        let mut conversation = started();
        assert_eq!(conversation.card_request(), None, "nothing difficult yet");
        conversation.toggle_meaning(0);
        conversation.toggle_meaning(0);
        assert!(conversation.lines[0].difficult && !conversation.lines[0].meaning_opened, "closing it again keeps the mark");
        conversation.add_turn(Some(typed("Ik ben thuis gebleven.")), reply("Ik ben thuis gebleven.", "", "Gezellig."));
        conversation.toggle_meaning(1);
        assert!(!conversation.lines[1].difficult, "the learner's own lines have no meaning to open");
        conversation.toggle_difficult(1);
        conversation.toggle_difficult(0);
        assert_eq!(conversation.difficult(), 1);
        assert!(conversation.card_request().is_some_and(|(steer, limit)| steer.contains("[MOEILIJK]") && limit == 3));
    }

    #[test]
    fn the_transcript_marks_difficult_lines_with_their_corrections() {
        let mut conversation = started();
        assert!(!conversation.has_learner_lines());
        conversation.add_turn(Some(typed("Ik heb naar de markt gegaan.")), reply("Ik ben naar de markt gegaan.", "Bij gaan hoort zijn.", "Leuk!"));
        assert!(conversation.has_learner_lines());
        let transcript = conversation.transcript();
        assert_eq!(
            transcript,
            "Gesprek over: het weekend\n\nPartner: Hoi! Wat heb je dit weekend gedaan?\n\
             Ik: Ik heb naar de markt gegaan. [MOEILIJK]\nCorrectie: Ik ben naar de markt gegaan.\n\
             Uitleg: Bij gaan hoort zijn.\nPartner: Leuk!\n"
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
