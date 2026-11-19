# Flitskaart

A Dutch-only flashcard app for this laptop. Paste a piece of a conversation you
had with Claude, ChatGPT or Gemini; Claude Code turns it into draft cards; you
check them; the app brings each card back at growing intervals.

Every card is shown in Dutch. The English is stored but stays hidden until you
press `V`.

## Studying

Each card goes through four steps, and each one waits for you:

1. **Show** – the Dutch appears.
2. **Read** – press `Space` and read it aloud. Recording stops by itself when
   you finish. Whisper, running on this laptop, writes down what it heard and
   the app grades how understandable you were from 1 to 4; words it missed are
   underlined. Press `Enter` instead to skip recording.
3. **Score** – how well did you know it: Weet ik niet / Moeilijk / Gaat goed /
   Makkelijk (`1`–`4`).
4. **Hear** – press `Space` to hear the app say it, `Space` again for the next card.

The card is scheduled on the **lower** of your own grade and your
pronunciation grade, so it comes back sooner if either is weak. Both are kept
with every review.

Pronunciation grades come from the share of the card's letters Whisper
understood: 95%+ is 4 (Heel duidelijk), 85%+ is 3, 65%+ is 2, less is 1.

| Key | What it does |
| --- | --- |
| `Space` | Opnemen (or Stop), then Luister, then Volgende |
| `Enter` | Skip recording; later, next card without hearing it |
| `1`–`4` | Score the card (you can change it until you move on) |
| `R` / `O` | Record again / play your own recording |
| `Esc` | Cancel a recording |
| `V` | Show or hide the English (counted per card) |
| `H` / `L` | Hear it again / slowly (after scoring) |
| `B` | Edit the card |

Modes: **Spreken** (the cycle above), **Stil** (read silently and score, no sound) and
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
scripts/fetch-models.sh      # Piper voices + Whisper, ~830 MB, checksummed
cargo run --release -p dutch-flashcards
```

Card drafting runs `claude -p` with your Claude Code login (no API key). Each
paste is one request and takes 5–15 seconds.

## Where things live

Everything is in `~/.local/share/dutch-flashcards/`: the deck (`cards.db`, a
Turso database), generated speech (`audio/`, one WAV per phrase, voice and
speed), the voices (`voices/`), Whisper (`models/`) and your latest
recording (`recordings/`). Copy the folder to back it up.

| Variable | Default |
| --- | --- |
| `DUTCH_FLASHCARDS_DATA` | `$XDG_DATA_HOME/dutch-flashcards` |
| `DUTCH_FLASHCARDS_VOICES` | `<data>/voices` |
| `DUTCH_FLASHCARDS_MODELS` | `<data>/models` (Whisper large-v3-turbo) |
| `DUTCH_FLASHCARDS_MODEL` | `sonnet` (the Claude model used for drafting) |
| `DUTCH_FLASHCARDS_CLAUDE` / `DUTCH_FLASHCARDS_PIPER` / `DUTCH_FLASHCARDS_WHISPER` | `claude` / `piper` / `whisper-cli` on `PATH` |

## Voices

Generated locally by Piper. Male: Pim, Ronnie and Alex (Dutch from the
Netherlands). Female: Nathalie, who speaks Flemish Dutch, because Piper has no
clear female voice from the Netherlands. Clarity was measured by having Whisper
transcribe a test paragraph from every candidate: these four got 91–93% of the
words right, while the female voices built from audiobook recordings (MLS) got
0–62% and were left out. Try each one under Instellingen.

## Layout

- `crates/core` – cards, FSRS scheduling, the Turso store, Piper voices and
  drafting. No UI code; tested with `cargo test`.
- `crates/app` – the window, built with [GPUI Community Edition](https://gpui-ce.github.io)
  and its component kit.
- `scripts/capture.sh` – runs the app in a headless Weston session and saves a
  screenshot. With `--screen <name>`, `--keys "space 3"` and
  `--appearance donker` it can show any screen and study step; `--recording file.wav`
  stands in for the microphone, and `wacht` in `--keys` waits for Whisper.
- `scripts/fetch-models.sh` – downloads the voices and the speech recognizer.
- `crates/core/examples/` – `draft` (try the drafting prompt on a snippet) and
  `import` (add `Nederlands | English` lines from stdin).
- `docs/` – the original plan and the clickable mockup.
