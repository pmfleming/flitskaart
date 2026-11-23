# RustQualityLens review and refactor

## Scope and evidence

Measured with the local `../rust-quality-lens/target/release/rqlens` and this
project's `rqlens.toml`. The producers used were `hotspots`, `clones`,
`escape-hatches`, `locality`, `leverage`, `test-quality`, `type-health`,
`api-health` and `coverage`. Coverage uses cargo-llvm-cov with LLVM 22 tools,
matching rustc's LLVM.

The baseline is a copy of the working tree before the refactor, measured with
the same commands as the result. Its evidence, the final evidence and the
scripts are in `target/refactor-review/`:

- `measure.sh`
- `compare.sh`
- `comparison.txt`

These are local, ignored files. No thresholds or source exclusions were set.

Two caveats:

- `type-health` and `api-health` are partial. Their syntax extractor can't
  resolve impl blocks for `FlashcardsApp` that are spread over several
  modules. Those numbers aren't used below.
- The baseline's coverage run first failed on a flaky test (see Findings,
  "Tests"). It was re-run until it passed, so both coverage figures come from
  a complete run.

## Results

| Metric | Before | After |
|---|---:|---:|
| Cognitive complexity, sum | 517 | 338 |
| Cognitive complexity, max | 27 | 9 |
| Functions at cognitive ≥ 10 | 16 | 0 |
| Cyclomatic complexity, sum | 828 | 802 |
| Cyclomatic complexity, max | 18 | 13 |
| Halstead effort, functions | 3,674,088 | 2,991,912 |
| Halstead effort, largest function | 209,169 | 124,492 |
| Highest function hotspot score | 157.1 | 50.7 |
| Locality, mean / weakest module | 97.24 / 76 | 97.33 / 82 |
| Leverage, mean | 35.2 | 36.3 |
| Duplicated lines (clone groups) | 153 (12) | 0 (0) |
| Escape hatches | 24 | 1 |
| `.clone()` calls | 48 | 34 |
| `unwrap`/`expect` in production code | 13 | 0 |
| Production lines | 4,564 | 4,359 |
| Line / region / function coverage | 36.9% / 38.7% / 42.1% | 77.7% / 77.7% / 77.1% |
| Tests / assertions | 52 / 146 | 30 / 108 |
| Test lines | 675 | 635 |

Some metrics moved less than others:

- **Cyclomatic complexity** fell only 3%. Most branches are real decisions:
  card states, keys, sample formats.
- **Leverage and locality** moved only a little. `app.rs` still wires every
  screen together.
- **Line counts:** "production" is everything outside `#[cfg(test)]` modules
  and `tests.rs`, in both trees.

Line coverage per file:

| Crate | File | Before | After |
|---|---|---:|---:|
| app | `study.rs` | 0% | 88% |
| app | `deck.rs` | 0% | 91% |
| app | `drafts.rs` | 0% | 95% |
| app | `settings.rs` | 0% | 89% |
| app | `mic_test.rs` | 0% | 78% |
| app | `app.rs` | 0% | 85% |
| app | `add.rs` | 14% | 77% |
| core | `schedule.rs` | 94% | 100% |
| core | `model.rs` | 76% | 100% |
| core | `inputs.rs` | 84% | 99% |
| core | `voice.rs` | 49% | 74% |

## Findings addressed

### Complexity

**`recorder::listen`** was the worst function: cognitive 27, cyclomatic 18.
It is now three parts:

- `Feed` receives sound bursts and fails when the microphone stalls.
- `SpeechGate` decides, 30 ms at a time, whether the learner has spoken and
  stopped.
- A short loop joins the two.

No function in the project is now above cognitive 9.

**Study panel:**

- What's done with the current card is now one `Attempt` value, so starting a
  card resets it in one line instead of nine.
- What the empty study screen says is a pure `Rest::new` instead of three
  nested branches.
- The phrase and the meaning are separate render functions.
- Shortcut keys fall through to grade parsing instead of matching four digit
  arms.

**Settings:**

- The 130-line `render_settings` (highest Halstead effort, 209k) is now one
  function per section, using a shared `ui::section`.
- The microphone description moved to `Input::describe` in core, where it is
  unit-tested.
- The loudness verdict moved to `Loudness::verdict`.

**Core:**

- `count` uses `Grade`'s order (`min`) instead of comparing numbers.
- Grade, speech and input labels are lookup tables.
- `assess` finds its grade from a table of thresholds.
- `aligned_matches` is split from building its cost table.
- The WAV header is one list of fields.
- `inputs::parse` is a small `Dump` type with one method per question.

### Leverage and locality

- **Shared recording session.** Oefenen's reading test and the microphone
  test each had their own copy of the start/poll/finish recording loop and the
  level meter. They now share `capture.rs`: `start_capture` takes the slot to
  record into and what to do with the result.
- **Background tasks.** Six spots hand-wrote "spawn on the background
  executor, then update the view". They now use `FlashcardsApp::in_background`
  and `after`.
- **Sound.** Playback state, `speak`, `say`, `play_file` and the current voice
  moved from `app.rs` to `audio.rs`. The two copies of "play this recording and
  say so" became `play_file`.
- **Ownership moved to where it's used:**
  - The scheduler belongs to the study state.
  - The boost slider and its subscription belong to the microphone test.
  - The deck search text belongs to `DeckState`.
  - The screenshot replay (`--screen`, `--keys`) lives in `main.rs` beside the
    flags that drive it.

  `app.rs` (locality 76 → still the lowest) lost its dependencies on
  `schedule`, `voice` and the slider. The next-weakest module now scores 82.
- **Shared tool paths.** `paths::program` handles the `$DUTCH_FLASHCARDS_*`
  overrides for Whisper, Piper and Claude Code, and `Paths::overridable` does
  the same for the voice and model folders.

### Dead code, cloning, escape hatches and size

- **Dead public items:**
  - Removed `Status::as_str`, `Card.created_at` (along with its column in
    `CARD_COLUMNS`) and `Input.id`.
  - Made private: `drafting::request_text`, `drafting::parse_reply`,
    `Store::open_file`, `Speaker::model_path` and `MODEL_FILE`.
- **Escape hatches:**
  - Replaced all 19 glob imports with explicit imports.
  - The clippy `too_many_arguments` suppression on `record_review` is gone:
    the review's inputs are one `store::Review`.
  - `logging.rs` went from four `unsafe` blocks to one, by using `io::pipe`
    and `try_clone_to_owned`. Pointing stderr at the pipe still needs one
    `dup2`.
- **`unwrap`/`expect` in production, 13 → 0:**
  - Piper's and Claude Code's stdin is handled with `context`.
  - Recording-thread setup and joining return errors.
  - The window opening is logged on failure.
  - The default voices and the start of the local day no longer `unwrap`.
- **Cloning, 48 → 34 `.clone()` calls:**
  - Claude Code's reply is taken from the envelope rather than cloned.
  - The pasted text travels to the background job and back instead of being
    copied.
  - Settings are saved from borrowed strings.
  - Editing reloads the card from the store instead of copying four fields
    back.
  - Deck rows no longer clone their card for the edit button.
- **Duplication** (12 clone groups, 3.39% of lines) is gone. It was mostly:
  - the two recording loops;
  - the deck and draft field editors (now `DraftRow::fields` and
    `listen_button`);
  - toggles and confirmations (`toggle_draft`, `confirm_delete`,
    `run_action`);
  - repeated store-test setup.

### Tests

**UI tests.** `crates/app/src/tests.rs` runs the real app in GPUI's headless
test window: an empty deck in a temporary folder, and no voices or Whisper.

- **First test:** pastes cards, checks drafts (duplicate flagged, source,
  discard, accept), studies them in Lezen and Luisteren by key, edits from the
  study screen, then pauses, edits, deletes and searches in Kaarten.
- **Second test:** changes voice, colours and the daily limit, and runs the
  microphone test with a WAV file standing in for the microphone.

Together they take most of the app crate from 0% to 77–95%.

**Fewer, broader tests.** Many single-purpose unit tests are now
table-driven:

- how a review counts (twelve cases);
- grading and missed-word marking;
- describing gaps and due dates (`describe_due` had no test before);
- loudness.

Tests that repeated others were merged or removed. The recorder tests now
cover the input boost through `listen` instead of a separate `amplify` test.

**A pre-existing flaky test is fixed.**
`opening_the_meaning_does_not_lengthen_the_pause` used the scheduler with
fuzz. About one run in twenty, fuzz stretched a 13-day pause to 17 days, past
its tolerance; that failure broke the baseline's coverage run. What it checked
is covered without fuzz in `schedule::trust_decides_how_far_a_known_card_moves`
and through the store in `each_review_saves_how_it_counted`. The store tests
now use `Scheduler::exact()`.

## Not changed

- **`app.rs`** remains the hub that builds every screen's state.
- **Speech-path coverage:**
  - The reading test's Whisper path (`speaking.rs`, 17%) and the live
    microphone thread (`recorder::record`) need Whisper and a real microphone.
    They're covered by the headless screenshot checks in `scripts/capture.sh`,
    not by tests.
  - Audio output is not exercised, so tests don't make sound on the machine
    running them.
- **Not checked by hand:** the slider's drag behaviour and a real microphone
  recording.
