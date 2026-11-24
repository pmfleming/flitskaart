//! Tries the conversation prompt: `cargo run -p dutch-flashcards-core --example talk -- "het weekend" < answers.txt`
//! Each line on stdin is one answer from the learner. Prints every turn, then
//! the transcript. Pass `--cards` after the topic to print its draft cards too.
use std::io::BufRead as _;

use dutch_flashcards_core::llm::Llm;
use dutch_flashcards_core::conversation::{Conversation, Said, Search, continue_turn, look_up};
use dutch_flashcards_core::drafting;

fn main() -> anyhow::Result<()> {
    let llm = Llm::from_env();
    let mut args = std::env::args().skip(1);
    let mut conversation = Conversation::new(&args.next().unwrap_or_default());
    let make_cards = args.any(|arg| arg == "--cards");
    if !conversation.topic.is_empty() {
        match look_up(&llm, &Search::about(&conversation.topic)) {
            Ok(sources) => conversation.add_sources(sources),
            Err(err) => eprintln!("Geen bronnen: {err:#}"),
        }
    }
    let mut said = None;
    let mut answers = std::io::stdin().lock().lines();
    loop {
        let started = std::time::Instant::now();
        let (reply, sources) = continue_turn(&llm, &conversation.request(said.as_ref()))?;
        eprintln!("({:.1}s) {reply:?}", started.elapsed().as_secs_f32());
        let finished = reply.finished;
        conversation.add_sources(sources);
        conversation.add_turn(said.take(), reply);
        if finished {
            break;
        }
        let Some(answer) = answers.next().transpose()? else { break };
        said = Some(Said { text: answer, spoken: false });
    }
    let transcript = conversation.transcript(None);
    println!("{transcript}");
    if make_cards && let Some((steer, limit)) = conversation.card_request(None) {
        let (cards, recap) = drafting::review_conversation(&llm, &transcript, steer, limit)?;
        eprintln!("{recap}");
        println!("{}", serde_json::to_string_pretty(&cards)?);
    }
    Ok(())
}
