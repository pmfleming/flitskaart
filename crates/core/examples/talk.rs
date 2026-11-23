//! Tries the conversation prompt: `cargo run -p dutch-flashcards-core --example talk -- "het weekend" < answers.txt`
//! Each line on stdin is one answer from the learner. Prints every turn, then
//! the transcript that drafting would read.
use std::io::BufRead as _;

use dutch_flashcards_core::claude::Claude;
use dutch_flashcards_core::conversation::{Conversation, Said, next_turn};

fn main() -> anyhow::Result<()> {
    let claude = Claude::from_env();
    let mut conversation = Conversation::new(&std::env::args().nth(1).unwrap_or_default());
    let mut said = None;
    let mut answers = std::io::stdin().lock().lines();
    loop {
        let started = std::time::Instant::now();
        let reply = next_turn(&claude, &conversation.request(said.as_ref()))?;
        eprintln!("({:.1}s) {reply:?}", started.elapsed().as_secs_f32());
        conversation.add_turn(said.take(), reply);
        let Some(answer) = answers.next().transpose()? else { break };
        said = Some(Said { text: answer, spoken: false });
    }
    println!("{}", conversation.transcript());
    Ok(())
}
