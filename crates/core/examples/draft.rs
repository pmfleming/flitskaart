//! Drafts cards from a snippet on stdin: `cargo run -p dutch-flashcards-core --example draft < snippet.txt`
use std::io::Read as _;

fn main() -> anyhow::Result<()> {
    let mut snippet = String::new();
    std::io::stdin().read_to_string(&mut snippet)?;
    let steer = std::env::args().nth(1).unwrap_or_default();
    let started = std::time::Instant::now();
    let cards = dutch_flashcards_core::drafting::draft_cards(&snippet, &steer, dutch_flashcards_core::drafting::DEFAULT_LIMIT)?;
    eprintln!("{} cards in {:.1}s", cards.len(), started.elapsed().as_secs_f32());
    println!("{}", serde_json::to_string_pretty(&cards)?);
    Ok(())
}
