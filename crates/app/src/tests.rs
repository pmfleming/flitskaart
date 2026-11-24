//! The whole app in a headless window: real clicks and keys through each screen.

use dutch_flashcards_core::Store;
use dutch_flashcards_core::llm::{Cli, Effort, ModelChoice};
use dutch_flashcards_core::paths::Paths;
use dutch_flashcards_core::speech::write_wav_16k;
use dutch_flashcards_core::store::{Appearance, Mode};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AnyWindowHandle, App, AppContext as _, Bounds, Context, ElementId, Entity, Point, TestAppContext, Window, WindowBounds,
    WindowOptions, gpui, px, size,
};

use crate::app::{FlashcardsApp, Screen};

/// The app on an empty deck in a temporary folder, without voices or Whisper.
struct Harness {
    window: AnyWindowHandle,
    app: Entity<FlashcardsApp>,
    dir: tempfile::TempDir,
}

impl Harness {
    fn open(cx: &mut TestAppContext) -> Harness {
        let dir = tempfile::tempdir().unwrap();
        let (paths, bounds) = (Paths::at(dir.path()), Bounds { origin: Point::default(), size: size(px(1000.), px(2000.)) });
        let store = Store::open(&paths).unwrap();
        cx.update(gpui_kit::init);
        let options = WindowOptions { window_bounds: Some(WindowBounds::Windowed(bounds)), ..Default::default() };
        let build = |window: &mut Window, cx: &mut App| cx.new(|cx| FlashcardsApp::new(store, paths, window, cx));
        let (window, app) = cx.update(|cx| gpui_kit::open_window(options, cx, build)).unwrap();
        Harness { window, app, dir }
    }

    /// Runs `f` on the app, then lets background work finish and draws a frame.
    fn act<R>(&self, cx: &mut TestAppContext, f: impl FnOnce(&mut FlashcardsApp, &mut Window, &mut Context<FlashcardsApp>) -> R) -> R {
        let result = cx.update_window(self.window, |_, window, cx| self.app.update(cx, |app, cx| f(app, window, cx))).unwrap();
        self.input(cx, |window, cx| window.render_frame(cx));
        result
    }

    /// Sends input to the window, as a person would, then lets the app settle.
    fn input(&self, cx: &mut TestAppContext, f: impl FnOnce(&mut Window, &mut App)) {
        cx.update_window(self.window, |_, window, cx| f(window, cx)).unwrap();
        cx.run_until_parked();
        cx.update_window(self.window, |_, window, cx| window.render_frame(cx)).unwrap();
    }

    fn show(&self, cx: &mut TestAppContext, screen: Screen) {
        self.act(cx, |app, window, cx| app.show(screen, window, cx));
    }

    /// Clicks each element in turn.
    fn click<const N: usize>(&self, cx: &mut TestAppContext, ids: [ElementId; N]) {
        for id in ids {
            self.input(cx, |window, cx| window.click(id, cx));
        }
    }

    fn press(&self, cx: &mut TestAppContext, keys: &str) {
        for key in keys.split_whitespace() {
            self.input(cx, |window, cx| window.press(key, cx));
        }
    }

    fn read<R>(&self, cx: &mut TestAppContext, f: impl FnOnce(&FlashcardsApp) -> R) -> R {
        cx.read(|cx| f(self.app.read(cx)))
    }
}

/// A row's button, such as `("accept", 3)`.
fn row(name: &'static str, id: u64) -> ElementId {
    (name, id).into()
}

/// From an empty deck: paste cards, check the drafts, study them in both
/// modes, then edit, pause, delete and search them in Kaarten.
#[gpui_kit::test]
fn a_deck_from_paste_to_practice(cx: &mut TestAppContext) {
    let app = Harness::open(cx);
    app.show(cx, Screen::Add);
    app.click(cx, ["make".into()]);
    assert!(app.read(cx, |app| app.add.error.is_some()), "nothing pasted yet");
    let pasted = "Hallo daar. | Hello there | een groet\nDank je wel. | Thank you\nHallo daar! | Hi";
    app.act(cx, |app, window, cx| app.add.snippet.update(cx, |snippet, cx| snippet.set_value(pasted, window, cx)));
    app.click(cx, ["make".into()]);
    let ids: Vec<u64> = app.read(cx, |app| app.drafts.iter().map(|row| row.id as u64).collect());
    assert_eq!(app.read(cx, |app| (app.screen, ids.len(), app.study.card.is_none())), (Screen::Drafts, 3, true), "drafts aren't studied");
    assert!(app.read(cx, |app| app.drafts[2].duplicate), "the second greeting is flagged");
    app.click(cx, [row("english", ids[0]), row("source", ids[0]), row("discard", ids[2]), row("accept", ids[0]), "accept-all".into()]);
    assert_eq!(app.read(cx, |app| (app.drafts.len(), app.deck.cards.len())), (0, 2));

    // Lezen: write the card with one wrong key, fixed with Backspace; the
    // writing then grades the review. Every key goes to the writing meanwhile.
    app.show(cx, Screen::Study);
    // Skipping moves on and saves nothing; the rest screen offers it again.
    let skipped = app.read(cx, |app| app.study.card.as_ref().unwrap().id);
    app.click(cx, ["skip".into()]);
    assert!(app.read(cx, |app| app.study.card.as_ref().is_some_and(|card| card.id != skipped) && app.study.reviewed_today == 0));
    assert_eq!(app.read(cx, |app| app.store.card(skipped).unwrap().unwrap().reviews), 0);
    let dutch = app.read(cx, |app| app.study.card.as_ref().unwrap().dutch.clone());
    let letters: Vec<String> = dutch.chars().filter(|c| c.is_alphanumeric()).map(|c| c.to_lowercase().to_string()).collect();
    app.press(cx, "w q");
    assert!(app.read(cx, |app| app.study.attempt.writing.as_ref().is_some_and(|w| w.cursor() == Some(1))), "a wrong letter fills its blank");
    app.press(cx, &format!("backspace {}", letters.join(" ")));
    let written = app.read(cx, |app| app.study.attempt.written.expect("written"));
    assert!(written.accuracy < 1.0 && written.accuracy > 0.8, "{written:?}");
    // Writing again and finishing early shows the rest; the first grade stays.
    app.click(cx, ["write".into()]);
    app.press(cx, "enter");
    assert!(app.read(cx, |app| app.study.attempt.writing.as_ref().is_some_and(|w| w.is_done()) && app.study.attempt.written == Some(written)));

    // The meaning counts against the review; then Luisteren, veiled until Space.
    app.press(cx, "v h 3");
    assert!(app.read(cx, |app| app.study.reviewed_today == 1 && app.notice.as_ref().is_some_and(|n| n.is_error)), "no voice installed");
    // Everything has had its turn: the scored card waits its 10 minutes, so
    // the session is done, with the skipped card on offer.
    assert!(app.read(cx, |app| app.study.card.is_none()), "all done, nothing shown early");
    app.click(cx, ["rest-skipped".into()]);
    assert_eq!(app.read(cx, |app| app.study.card.as_ref().map(|card| card.id)), Some(skipped));
    app.click(cx, ["mode-Luisteren".into()]);
    app.press(cx, "4");
    assert_eq!(app.read(cx, |app| (app.settings.mode, app.study.reviewed_today)), (Mode::Listen, 1), "no score before the text shows");
    app.press(cx, "space b");
    assert_eq!(app.read(cx, |app| (app.screen, app.deck.editing.is_some())), (Screen::Deck, true), "B edits the card");
    app.click(cx, ["edit-save".into()]);
    app.show(cx, Screen::Study);
    app.press(cx, "1");
    assert!(app.read(cx, |app| app.study.reviewed_today == 2 && app.study.card.is_none()), "every card had its turn: all done");

    app.show(cx, Screen::Deck);
    let first = app.read(cx, |app| app.deck.cards[0].id as u64);
    app.click(cx, [row("pause", first), row("edit", first), "edit-cancel".into()]);
    app.click(cx, [row("delete", first), row("delete-no", first), row("delete", first), row("delete-yes", first)]);
    app.act(cx, |app, _, _| {
        app.deck.query = "geen treffer".into();
        app.reload_deck();
    });
    assert_eq!(app.read(cx, |app| (app.store.deck("").unwrap().len(), app.deck.cards.len())), (1, 0));
}

/// Instellingen: voice, colours, the daily limit, and the microphone test
/// with a recording standing in for the microphone.
#[gpui_kit::test]
fn settings_and_the_microphone_test(cx: &mut TestAppContext) {
    let app = Harness::open(cx);
    let wav = app.dir.path().join("test.wav");
    let speech: Vec<f32> = (0..32_000).map(|i| if i > 4_000 { 0.3 * (i as f32 * 0.17).sin() } else { 0.0 }).collect();
    write_wav_16k(&wav, &speech, 16_000, 1).unwrap();
    // The logged-in tools as Instellingen would find them, so none are run here.
    let model = |cli, id: &str, efforts: Vec<Effort>, default_effort| ModelChoice { cli, id: id.into(), name: id.into(), efforts, default_effort };
    let choices = vec![
        model(Cli::ClaudeCode, "opus", Effort::ALL.to_vec(), Effort::Medium),
        model(Cli::ClaudeCode, "sonnet", Effort::ALL.to_vec(), Effort::Medium),
        model(Cli::Antigravity, "gemini-x", vec![Effort::Low, Effort::High], Effort::Low),
    ];
    app.act(cx, |app, _, _| (app.fake_recording, app.llm_choices) = (Some(wav), Some(choices)));
    app.show(cx, Screen::Settings);
    app.click(cx, ["vrouw".into(), "licht".into(), "more".into(), "mic-test".into()]);
    let settings = app.read(cx, |app| app.settings.clone());
    assert_eq!((settings.voice_id(), settings.appearance, settings.new_per_day), ("nathalie", Appearance::Light, 11));
    assert!(app.dir.path().join("recordings/microfoontest.wav").exists(), "the stand-in recording is saved for playback");
    app.click(cx, ["fewer".into()]);
    assert_eq!(app.read(cx, |app| app.store.settings().unwrap().new_per_day), 10, "saved as it changes");
    // The tool, model and effort go to every answer, and are saved.
    app.click(cx, ["model-claude-opus".into(), "effort-high".into()]);
    assert_eq!(app.read(cx, |app| (app.llm.cli, app.llm.model.clone(), app.llm.effort)), (Cli::ClaudeCode, "opus".to_string(), Effort::High));
    // Antigravity names the effort in the model.
    app.click(cx, ["llm-tab-agy".into(), "model-agy-gemini-x".into()]);
    assert_eq!(app.read(cx, |app| (app.llm.cli, app.llm.model.clone())), (Cli::Antigravity, "gemini-x-high".to_string()));
    app.click(cx, ["effort-low".into()]);
    let saved = app.read(cx, |app| app.store.settings().unwrap());
    assert_eq!((saved.cli, saved.model.as_str(), saved.effort), (Cli::Antigravity, "gemini-x-low", Effort::Low));
}

/// Stands in for Claude Code: always corrects "heb … gegaan", notes "market" as
/// English and "dit weekend" as not understood, and asks what was bought;
/// asked for cards, makes one of the correction. Researching a topic finds three
/// angles; any other search finds one known source and one new one.
const FAKE_CLAUDE: &str = r#"#!/bin/sh
request=$(cat)
case "$*" in
  *'"sources"'*) case "$request" in
  *'"options":true'*) echo '{"is_error":false,"modelUsage":{"sonnet":{"webSearchRequests":1}},"structured_output":{"sources":[{"title":"De markt","url":"https://example.org/markt","date":"2026-11-24","facts":"Er is een markt op zaterdag."},{"title":"Fietsen","url":"https://example.org/fiets","date":"","facts":"Veel mensen fietsen in het weekend."},{"title":"Uitslapen","url":"https://example.org/slapen","date":"2026-10-01","facts":"Uitslapen is populair."}]}}' ;;
  *) echo '{"is_error":false,"modelUsage":{"sonnet":{"webSearchRequests":1}},"structured_output":{"sources":[{"title":"Fietsen","url":"https://example.org/fiets","date":"","facts":"Al bekend."},{"title":"Nieuwe fietspaden","url":"https://example.org/nieuw","date":"2026-11-20","facts":"Er komen nieuwe fietspaden."}]}}' ;;
  esac ;;
  *'"cards"'*) case "$request" in
  *'Leervraag: Beleefd om hulp vragen'*) echo '{"is_error":false,"structured_output":{"recap":"Je vroeg hoe je beleefd om hulp vraagt.","cards":[{"dutch":"Zou u mij kunnen helpen?","english":"Could you help me?","note":"Dit is beleefd.","tag":"hulp"}]}}' ;;
  *) echo '{"is_error":false,"modelUsage":{"claude-sonnet-5-5":{"inputTokens":100,"outputTokens":40,"cacheReadInputTokens":10,"costUSD":0.002}},"structured_output":{"cards":[{"dutch":"Ik ben naar de markt gegaan.","english":"I went to the market.","note":"Bij gaan hoort zijn.","tag":"grammatica"}]}}' ;;
  esac ;;
  *) case "$request" in
  *'"opening":true'*) echo '{"is_error":false,"structured_output":{"corrected":"","explanation":"","reply":"Ik ga graag naar de markt. Wat heb je dit weekend gedaan?","reply_english":"I like going to the market.","partner":{"name":"Noor","role":"marktliefhebber","opinion":"Ik ga graag naar de markt.","reason":"Ik winkel het liefst buiten."}}}' ;;
  *'"newest":{'*'"text":"Hoe vraag ik beleefd om hulp?"'*) echo '{"is_error":false,"structured_output":{"corrected":"","explanation":"","reply":"Zou u mij kunnen helpen?","reply_english":"Could you help me?","learning_need":"Beleefd om hulp vragen: Zou u mij kunnen helpen?"}}' ;;
  *'"newest":{'*'"text":"Tot ziens!"'*) echo '{"is_error":false,"structured_output":{"corrected":"","explanation":"","reply":"Tot ziens!","reply_english":"Goodbye!","finished":true}}' ;;
  *) echo '{"is_error":false,"result":"","structured_output":{"corrected":"Ik ben naar de markt gegaan.","explanation":"Bij gaan hoort zijn.","english_used":[{"english":"market","dutch":"de markt"}],"not_understood":["dit weekend"],"reply":"Leuk! Wat heb je gekocht?","reply_english":"Nice! What did you buy?"}}' ;;
  esac ;;
esac
"#;

/// Gesprek: a failed start, then a conversation whose English, unclear words,
/// opened meaning and correction become drafts when it ends; every piece plays.

#[gpui_kit::test]
fn a_conversation_becomes_cards(cx: &mut TestAppContext) {
    use std::os::unix::fs::PermissionsExt as _;
    let app = Harness::open(cx);
    let program = app.dir.path().join("claude");
    std::fs::write(&program, FAKE_CLAUDE).unwrap();
    std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
    let missing = app.dir.path().join("geen-claude");
    app.act(cx, |app, _, _| app.llm.program = missing.clone());
    app.show(cx, Screen::Talk);
    app.click(cx, ["topic-Het weekend".into()]);
    assert!(app.read(cx, |app| app.talk.current.is_none() && app.talk.error.is_some()), "back to the topics when Claude can't start");

    app.act(cx, |app, _, _| app.llm.program = program.clone());
    app.click(cx, ["topic-Het weekend".into()]);
    assert_eq!(app.read(cx, |app| (app.talk.options.len(), app.talk.current.as_ref().map(|c| c.lines.len()))), (3, Some(0)), "three angles to choose from");
    app.click(cx, ["talk-more-options".into()]);
    assert_eq!(app.read(cx, |app| app.talk.options.len()), 3, "nothing new found, so the same angles stay");
    app.click(cx, [row("talk-option", 1)]);
    assert_eq!(app.read(cx, |app| app.talk.current.as_ref().map(|c| (c.topic.clone(), c.lines.len()))), Some(("Het weekend".into(), 1)));
    assert!(app.read(cx, |app| app.talk.options.is_empty() && app.talk.current.as_ref().is_some_and(|c| {
        c.angle.as_deref() == Some("Fietsen") && c.sources.len() == 1 && c.partner.as_ref().is_some_and(|p| p.name == "Noor")
    })));
    // Bijwerken adds only what the partner didn't know yet.
    app.click(cx, ["talk-refresh".into()]);
    let urls = app.read(cx, |app| app.talk.current.as_ref().unwrap().sources.iter().map(|s| s.url.clone()).collect::<Vec<_>>());
    assert_eq!(urls, ["https://example.org/fiets", "https://example.org/nieuw"]);
    assert!(app.read(cx, |app| !app.talk.refreshing && app.notice.as_ref().is_some_and(|n| n.text.contains("1 nieuwe bron"))));
    app.act(cx, |app, window, cx| app.talk.message.update(cx, |field, cx| field.set_value("Ik heb naar de market gegaan", window, cx)));
    app.click(cx, ["talk-send".into()]);
    let corrected = app.read(cx, |app| app.talk.current.as_ref().unwrap().lines[1].corrected.clone());
    assert_eq!(corrected.as_deref(), Some("Ik ben naar de markt gegaan."));
    let field = app.read(cx, |app| app.talk.message.clone());
    assert!(cx.read(|cx| field.read(cx).value().is_empty()) && app.read(cx, |app| app.talk.pending.is_none()), "sent and answered");
    app.click(cx, ["line-0".into(), "line-1-play".into(), "english-1-0".into(), "unclear-0-0".into(), row("line-slow", 2)]);
    assert!(app.read(cx, |app| app.notice.as_ref().is_some_and(|n| n.is_error)), "each tries to play; no voice is installed here");
    assert_eq!(app.read(cx, |app| app.talk.current.as_ref().unwrap().gaps()), 2, "the English and the unclear words");
    app.click(cx, [row("line-meaning", 2), row("line-meaning", 2)]);
    assert_eq!(app.read(cx, |app| app.talk.current.as_ref().unwrap().gaps()), 3, "and the meaning, even closed again");

    app.click(cx, ["talk-finish".into()]);
    assert_eq!(app.read(cx, |app| (app.screen, app.drafts.len(), app.talk.current.is_none())), (Screen::Drafts, 1, true));
    let source = app.read(cx, |app| app.drafts[0].snippet_id.and_then(|id| app.store.snippet(id).unwrap()).unwrap());
    assert!(source.starts_with("Gesprek over: Het weekend") && source.contains("In het Engels gezegd: market = de markt"), "{source}");
    // The answer's token use is kept with the deck, for Instellingen.
    let used = app.read(cx, |app| app.store.usage_summary(chrono::Utc::now(), 7).unwrap());
    assert_eq!((used.total.calls, used.total.tokens(), used.by_model[0].0.as_str()), (1, 150, "claude-sonnet-5-5"));
    app.act(cx, |app, _, _| app.llm_choices = Some(Vec::new()));
    app.show(cx, Screen::Settings);
    app.click(cx, ["usage-per-day".into()]);
    assert!(app.read(cx, |app| app.usage_by_day), "the breakdown turns to days");
    app.show(cx, Screen::Talk);

    app.show(cx, Screen::Talk);
    app.click(cx, ["topic-Het weekend".into(), row("talk-option", 0)]);
    app.act(cx, |app, window, cx| app.talk.message.update(cx, |field, cx| field.set_value("Hoe vraag ik beleefd om hulp?", window, cx)));
    app.click(cx, ["talk-send".into()]);
    assert!(app.read(cx, |app| app.talk.current.as_ref().unwrap().transcript().contains("Leervraag: Beleefd om hulp vragen")));
    app.act(cx, |app, window, cx| app.talk.message.update(cx, |field, cx| field.set_value("Tot ziens!", window, cx)));
    app.click(cx, ["talk-send".into()]);
    assert!(app.read(cx, |app| app.talk.current.is_none() && app.drafts.len() == 2), "goodbye closes and reviews the transcript");
    assert_eq!(app.read(cx, |app| app.store.drafts().unwrap()[1].dutch.clone()), "Zou u mij kunnen helpen?");

    app.show(cx, Screen::Talk);
    app.click(cx, ["talk-recap".into()]);
    app.click(cx, ["topic-Het weekend".into(), row("talk-option", 2)]);
    app.act(cx, |app, window, cx| app.talk.message.update(cx, |field, cx| field.set_value("Hoe vraag ik beleefd om hulp?", window, cx)));
    app.click(cx, ["talk-send".into()]);
    app.act(cx, |app, _, _| app.llm.program = missing);
    app.click(cx, ["talk-finish".into()]);
    assert!(app.read(cx, |app| app.talk.current.is_none() && app.talk.error.is_some() && app.drafts.len() == 2));
    app.act(cx, |app, _, _| app.llm.program = program);
    app.click(cx, ["talk-retry-cards".into()]);
    assert_eq!(app.read(cx, |app| app.drafts.len()), 3);
}
