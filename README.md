# Flitskaart

A Dutch-only flashcard app for this laptop. Paste a piece of a conversation you
had with Claude, ChatGPT or Gemini; Claude Code turns it into draft cards; you
check them; the app brings each card back at growing intervals.

Every card is shown in Dutch. The English is stored but stays hidden until you
press `V`.

## Studying

Each card is one panel: the Dutch, three buttons, and the four scores. Use the
buttons in any order, or none of them.

- **Test mijn uitspraak** (`Space`) – read the card aloud. Recording stops by
  itself when you finish. Whisper, running on this laptop, writes down what it
  heard and the app grades how understandable you were from 1 to 4; words it
  missed are underlined in the card. The button then reads **Opnieuw** for
  another try; the latest try counts.
- **Hoor de zin** (`H`) – the sentence in the Dutch voice chosen in
  Instellingen. Listening never changes how the review counts.
- **Betekenis** (`V`) – the English and the note.
- **Weet ik niet / Moeilijk / Gaat goed / Makkelijk** (`1`–`4`) – how easily
  the sentence came back. This saves the review and shows the next card.

What you did before scoring decides how far your score is trusted. The panel
doesn't show any of this:

- **Read aloud, then scored** – counts in full. If your pronunciation grade is
  lower than your own score, it is used instead.
- **Scored only** – the card keeps 70% of the longer pause FSRS proposes
  (`SCORE_ONLY_SHARE` in `crates/core/src/schedule.rs`).
- **Opened the meaning** – counts as Moeilijk at best, and the pause does not
  grow, even if you also read it aloud.
- **Weet ik niet** always counts in full.

A new card has no pause to grow yet, so only the grade rules apply to it. Your
own score, the pronunciation grade, the grade used and the share are kept with
every review.

Pronunciation grades come from the share of the card's letters Whisper
understood: 95%+ is 4 (Heel duidelijk), 85%+ is 3, 65%+ is 2, less is 1.

| Key | What it does |
| --- | --- |
| `Space` | Test the reading (or stop the recording); in Luisteren, show the text |
| `1`–`4` | Score the card and go to the next one |
| `R` / `O` | Test again / play your own recording |
| `Esc` | Cancel a recording |
| `V` | Show or hide the meaning |
| `H` / `L` | Hear the sentence / slowly |
| `B` | Edit the card |

Modes: **Lezen** (the panel above) and **Luisteren** (the audio plays first with
the text hidden; there is no reading test, so scores count in full unless you
open the meaning).

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
- **Instellingen** – male or female voice, a microphone test, light/dark/system
  colours, new cards per day. The Microfoon section names the microphone the
  sound comes from (read from PipeWire with `pw-dump`: Bluetooth, USB or the
  laptop, its volume and whether it is muted) and lists the others. The test
  records a few words, says whether they came in too quiet, too loud or fine,
  plays back exactly what the computer heard and shows what Whisper made of it.
  The app always records from the system's default microphone. **Versterking**
  (0,5×–2,5×, a slider) amplifies every recording, in study and in the test,
  before the app listens to it; samples pushed past full scale are cut off.

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

`flitskaart.log` in the same folder is the app's log: every error it shows,
panics with a backtrace, each recording's device, format and timing, and
whatever audio libraries print to stderr. Lines also go to the terminal. Past
1 MB it moves to `flitskaart.log.1` and a new log starts. When something goes
wrong, the end of this file says what happened. A crash in native code
(a segfault) can't be logged, but the last line shows what the app was doing.

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
  and its component kit. The study panel's own icons are in `crates/app/assets/icons`.
  `src/tests.rs` drives the whole app in GPUI's headless test window (an empty
  deck, no voices or Whisper), so `cargo test` covers the screens too.
- `scripts/capture.sh` – runs the app in a headless Weston session and saves a
  screenshot. With `--screen <name>`, `--keys "space v"` and
  `--appearance donker` it can show any screen and study state; `--recording file.wav`
  stands in for the microphone, and `wacht` in `--keys` waits for Whisper. On
  `--screen instellingen`, `--keys microfoon` runs the microphone test.
- `scripts/fetch-models.sh` – downloads the voices and the speech recognizer.
- `crates/core/examples/` – `draft` (try the drafting prompt on a snippet) and
  `import` (add `Nederlands | English` lines from stdin).
- `docs/` – the original plan, the clickable mockup, the study-panel plan and
  the RustQualityLens review (`rustqualitylens-review.md`).
- `rqlens.toml` – configuration for [rust-quality-lens](../rust-quality-lens):
  `rqlens measure hotspots` and the other producers write their evidence to
  `target/analysis`.
