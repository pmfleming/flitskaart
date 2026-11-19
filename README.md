# Flitskaart

A Dutch-only flashcard app for this laptop. Paste a piece of a conversation you
had with Claude, ChatGPT or Gemini; Claude Code turns it into draft cards; you
check them; the app brings each card back at growing intervals.

Every card is shown in Dutch. The English is stored but stays hidden until you
press `V`.

## Studying

Each card goes through four steps, and each one waits for you:

1. **Show** – the Dutch appears.
2. **Read** – read it aloud, then press `Space` (Gelezen).
3. **Score** – Weet ik niet / Moeilijk / Gaat goed / Makkelijk (`1`–`4`).
4. **Hear** – press `Space` to hear the app say it, `Space` again for the next card.

| Key | What it does |
| --- | --- |
| `Space` | Gelezen, then Luister, then Volgende |
| `1`–`4` | Score the card (you can change it until you move on) |
| `Enter` | Next card without hearing it |
| `V` | Show or hide the English (counted per card) |
| `H` / `L` | Hear it again / slowly (after scoring) |
| `B` | Edit the card |

Modes: **Spreken** (the cycle above), **Stil** (read and score, no sound) and
**Luisteren** (the audio plays first with the text hidden).

Scheduling is FSRS, the algorithm modern Anki uses. New cards are limited per
day (10 by default, set in Instellingen).

## Screens

- **Oefenen** – study.
- **Toevoegen** – paste a snippet. Corrections of your own mistakes become cards
  first, then new phrases and words. An optional line steers it ("alleen de
  correcties", "max 3 kaarten"). Lines typed as `Nederlands | English | uitleg`
  skip Claude.
- **Concepten** – edit, listen to, keep or discard each draft. Duplicates are flagged.
- **Kaarten** – search, edit, pause, reset or delete cards; see misses and
  how often you looked at the English.
- **Instellingen** – male or female voice, light/dark/system colours, new cards per day.

## Running it

```sh
nix develop                  # or `direnv allow` once
scripts/fetch-voices.sh      # Dutch Piper voices, ~330 MB, checksummed
cargo run --release -p dutch-flashcards
```

Card drafting runs `claude -p` with your Claude Code login (no API key). Each
paste is one request and takes 5–15 seconds.

## Where things live

Everything is in `~/.local/share/dutch-flashcards/`: the deck (`cards.db`, a
Turso database), generated speech (`audio/`, one WAV per phrase, voice and
speed) and the voices (`voices/`). Copy the folder to back it up.

| Variable | Default |
| --- | --- |
| `DUTCH_FLASHCARDS_DATA` | `$XDG_DATA_HOME/dutch-flashcards` |
| `DUTCH_FLASHCARDS_VOICES` | `<data>/voices` |
| `DUTCH_FLASHCARDS_MODEL` | `sonnet` (the Claude model used for drafting) |
| `DUTCH_FLASHCARDS_CLAUDE` / `DUTCH_FLASHCARDS_PIPER` | `claude` / `piper` on `PATH` |

## Voices

All voices are Dutch from the Netherlands, generated locally by Piper.
Male: Pim, Ronnie, Alex. Female: three readers from the multi-speaker MLS model
and one single-speaker model in lower quality. Piper's model cards don't record
gender, so it was measured: the male voices speak at about 125–130 Hz and the
female ones at about 200 Hz. Try each one under Instellingen.

## Layout

- `crates/core` – cards, FSRS scheduling, the Turso store, Piper voices and
  drafting. No UI code; tested with `cargo test`.
- `crates/app` – the window, built with [GPUI Community Edition](https://gpui-ce.github.io)
  and its component kit.
- `scripts/capture.sh` – runs the app in a headless Weston session and saves a
  screenshot. With `--screen <name>`, `--keys "space 3"` and
  `--appearance donker` it can show any screen and study step.
- `scripts/fetch-voices.sh` – downloads the voices.
- `crates/core/examples/` – `draft` (try the drafting prompt on a snippet) and
  `import` (add `Nederlands | English` lines from stdin).
- `docs/` – the original plan and the clickable mockup.
