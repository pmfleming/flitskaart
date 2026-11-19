//! Adds cards typed as `Nederlands | English | uitleg`, one per line on stdin.
//! They arrive as drafts; pass `--accept` to put them straight into the deck.
//!
//!     cargo run -p dutch-flashcards-core --example import -- --accept < cards.txt
//!
//! Uses the same data folder as the app (`DUTCH_FLASHCARDS_DATA` to override).
use std::io::Read as _;

use dutch_flashcards_core::{Store, drafting, paths::Paths};

fn main() -> anyhow::Result<()> {
    let accept = std::env::args().any(|a| a == "--accept");
    let mut text = String::new();
    std::io::stdin().read_to_string(&mut text)?;
    let cards = drafting::parse_manual(&text).ok_or_else(|| anyhow::anyhow!("every line needs the form: Nederlands | English"))?;
    let paths = Paths::from_env();
    let store = Store::open(&paths)?;
    let now = chrono::Utc::now();
    let ids = store.add_drafts(None, &cards, now)?;
    if accept {
        for id in &ids {
            store.accept(*id, now)?;
        }
    }
    println!("{} cards {} in {}", ids.len(), if accept { "added to the deck" } else { "added as drafts" }, paths.root().display());
    Ok(())
}
