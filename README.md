# Flitskaart

A Dutch-only flashcard app for this laptop. Paste a piece of a conversation you
had with Claude, ChatGPT or Gemini; Claude Code turns it into draft cards; you
check them; the app brings each card back at growing intervals.

Every card is shown in Dutch. The English is stored but stays hidden until you
press `V`.

## Studying

Each card is one panel: the Dutch, two big buttons to test yourself, and the
four scores. Three small helpers sit in the card's top corner. Use any of them
in any order, or none.

The two tests:

- **Spreek** (`Space`) – read the card aloud. Recording stops by itself when you
  finish. Whisper, running on this laptop, writes down what it heard and the app
  grades how understandable you were from 1 to 4; words it missed are underlined
  in the card, and the sound-into-text mark shows what Whisper heard. Press it
  again for another try; the latest try counts. (In Luisteren this button is
  **Toon tekst**.)
- **Schrijf** (`W`) – the Dutch turns into blanks, one short line per letter, and
  you type it. Spaces and punctuation are filled in; case and accents don't
  matter (e for é). Every key fills the next blank: a right letter flashes green
  and turns white, a wrong one shows in red. You may be wrong: go back with
  `Backspace` to fix a red letter (it keeps a faint red mark), or carry on.
  Writing every letter right finishes by itself; otherwise `Enter` or **Klaar**
  finishes and shows the sentence corrected, the letters you missed in red.
  `Esc` stops without a grade. Every key goes to the writing until it is done.
  The grade is the share of letters right first time, on the same 1–4 scale as
  speaking; a fixed letter doesn't count as right. Writing it again doesn't
  change the grade.
- **Overslaan** (`S`) – moves on without saving anything about the card: no
  score, no change to when it comes back. Skipped cards wait until you ask for
  them on the **Alles gedaan** card.

The helpers, top right in the card (they never decide the review):

- **Hoor de zin** (ear, `H`) and **Langzaam** (snail, `L`) – the sentence in
  the Dutch voice chosen in Instellingen.
- **Betekenis** (two cards, `V`) – the English and the note.

Then score:

- **Weet ik niet / Moeilijk / Gaat goed / Makkelijk** (`1`–`4`) – how easily
  the sentence came back. This saves the review and shows the next card.

What you did before scoring decides how far your score is trusted. The panel
doesn't show any of this:

- **Spoken or written, then scored** – counts in full. If the test grade is
  lower than your own score, it is used instead; with both tests, the lower one.
- **Scored only** – the card keeps 70% of the longer pause FSRS proposes
  (`SCORE_ONLY_SHARE` in `crates/core/src/schedule.rs`).
- **Opened the meaning** – counts as Moeilijk at best, and the pause does not
  grow, even if you also read it aloud.
- **Weet ik niet** always counts in full.

A new card has no pause to grow yet, so only the grade rules apply to it. Your
own score, the pronunciation and writing grades, the grade used and the share
are kept with every review.

Pronunciation grades come from the share of the card's letters Whisper
understood: 95%+ is 4 (Heel duidelijk), 85%+ is 3, 65%+ is 2, less is 1.

| Key | What it does |
| --- | --- |
| `Space` | Test the reading (or stop the recording); in Luisteren, show the text |
| `1`–`4` | Score the card and go to the next one |
| `W` | Write the card (`Backspace` fixes, `Enter` finishes, `Esc` stops) |
| `S` | Skip the card for now |
| `R` / `O` | Test again / play your own recording |
| `Esc` | Cancel a recording |
| `V` | Show or hide the meaning |
| `H` / `L` | Hear the sentence / slowly |
| `B` | Edit the card |

Modes: **Lezen** (the panel above) and **Luisteren** (the audio plays first with
the text hidden; there is no reading test, so scores count in full unless you
open the meaning. **Schrijf** there is a dictation: write what you heard).

Scheduling is FSRS, the algorithm modern Anki uses. New cards are limited per
day (10 by default, set in Instellingen). Order: due reviews first, longest due
first; then new cards in random order, so cards from one conversation are
spread out. No card is shown before it is due, so a session ends: once every
card has had its turn, only cards scheduled for later are left and Oefenen shows
**Alles gedaan**, with when the next card is due ("over 4 min"), how many you
skipped and how many new cards the daily limit held back. Its buttons bring the
skipped cards back, or allow more new cards today. A card in its first minutes
(after Weet ik niet or Moeilijk) pops back up by itself when it falls due, if
Oefenen is open.

## Gesprek

Practise speaking about a topic. Pick one of the topic tiles, type your own, or
leave it open. For a chosen topic, Claude looks up a little background and shows
the sources above the conversation. A fictional partner opens with a point of
view and helps you say what you mean in Dutch. The Dutch voice reads the partner’s
lines out loud. If research finds no sources, you can still talk or retry the
lookup before replying.

Answer each line by typing (`Enter` or the send button) or with the
speech-bubble button. Recording stops by itself when you finish, and Whisper
writes down what you said. Either way your sentence comes back corrected with
the next line. A correction shows what you said struck through, then how a
Dutch speaker would say it, with the changed words underlined and a short
explanation by the light bulb. When you spoke, punctuation and capitals aren't
corrected.

Don't know a word? Say it in English: the correction puts it into Dutch, and a
chip marked EN under your sentence shows the Dutch word. Didn't understand
Claude? Say so ("Wat betekent dat?", "Sorry?") and Claude says it again more
simply; the words you missed get a chip with a question mark. Or open
**Betekenis** (the two cards) for the English. Whisper listens for Dutch, so an
English word you *speak* may be heard wrongly; typed English always works.

Every piece of Dutch plays: click a sentence (or the ear before it), the snail
to hear it slowly, or a chip to hear that word. For your own lines, the
corrected sentence is what plays.

Choose **Afronden** any time or say goodbye in the conversation. The app saves
the exchange first, then drafts cards for the Dutch you asked for or needed.
That includes useful corrections, phrases you wanted to practise and words you
asked how to say. Review the cards in **Concepten**. **Bron** shows the transcript
and source links. Gesprek also offers a short **Terugblik** for the latest
reviewed conversation. If drafting fails, retry it from Gesprek. A conversation
in progress is lost if you close the app.

## Screens

Every screen keeps its text short: tabs, buttons, column headings and counts
carry the app's own icons, and hovering one shows what it means.

- **Oefenen** – study.
- **Gesprek** – a conversation with Claude that becomes cards (above).
- **Toevoegen** – paste a snippet. Corrections of your own mistakes become cards
  first, then new phrases and words. An optional line steers it ("alleen de
  correcties", "max 3 kaarten"). Lines typed as `Nederlands | English | uitleg`
  skip Claude.
- **Concepten** – edit, listen to, keep or discard each draft. Each field is
  marked by an icon (NL, bulb for the note, two cards for the English, label for
  the topic). Duplicates are flagged.
- **Kaarten** – search, edit, pause, reset or delete cards; see misses and
  how often you looked at the English. The filters show how many cards each holds.
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

Card drafting and conversations run `claude -p` with your Claude Code login (no
API key). Each paste is one request and takes 5–15 seconds; each conversation
turn takes about 3 seconds.

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
| `DUTCH_FLASHCARDS_MODEL` | `sonnet` (the Claude model for drafting and conversations) |
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
  and its component kit. Its icons are drawn for this app, in `crates/app/assets/icons`.
  `src/tests.rs` drives the whole app in GPUI's headless test window (an empty
  deck, no voices or Whisper), so `cargo test` covers the screens too.
- `scripts/capture.sh` – runs the app in a headless Weston session and saves a
  screenshot. With `--screen <name>`, `--keys "space v"` and
  `--appearance donker` it can show any screen and study state; `--recording file.wav`
  stands in for the microphone, and `wacht` in `--keys` waits for Whisper and
  Claude. On `--screen gesprek`, `--keys "begin wacht spreek wacht"` starts a
  conversation and answers it with the recording. On
  `--screen instellingen`, `--keys microfoon` runs the microphone test.
- `scripts/fetch-models.sh` – downloads the voices and the speech recognizer.
- `crates/core/examples/` – `draft` (try the drafting prompt on a snippet),
  `talk` (try the conversation prompt: one answer per line on stdin) and
  `import` (add `Nederlands | English` lines from stdin).
- `docs/` – the original plan, the clickable mockup, the study-panel plan and
  the RustQualityLens review (`rustqualitylens-review.md`).
- `rqlens.toml` – configuration for [rust-quality-lens](../rust-quality-lens):
  `rqlens measure hotspots` and the other producers write their evidence to
  `target/analysis`.
