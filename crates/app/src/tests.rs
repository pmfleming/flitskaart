//! The whole app in a headless window: real clicks and keys through each screen.

use dutch_flashcards_core::Store;
use dutch_flashcards_core::conversation::DUTCH_NEWS_TOPIC;
use dutch_flashcards_core::llm::{Cli, Effort, ModelChoice};
use dutch_flashcards_core::paths::Paths;
use dutch_flashcards_core::speech::{Listener, write_wav_16k};
use dutch_flashcards_core::store::{Appearance, Mode};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AnyWindowHandle, App, AppContext as _, Bounds, Context, ElementId, Entity, Point, TestAppContext, Window, WindowBounds,
    WindowOptions, gpui, px, size,
};

use crate::app::{FlashcardsApp, Screen};

/// The app on an empty deck in a temporary folder, without voices.
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

    fn script(&self, name: &str, script: &str) -> std::path::PathBuf {
        use std::os::unix::fs::PermissionsExt as _;
        let path = self.dir.path().join(name);
        std::fs::write(&path, script).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    fn recording(&self, cx: &mut TestAppContext, whisper_script: &str) {
        let whisper = self.script("whisper", whisper_script);
        let wav = self.dir.path().join("test.wav");
        let speech: Vec<f32> = (0..16_000).map(|i| 0.3 * (i as f32 * 0.17).sin()).collect();
        write_wav_16k(&wav, &speech, 16_000, 1).unwrap();
        self.act(cx, |app, _, _| {
            // A nonempty file is enough for the stand-in program's model.
            app.listener = Listener::with_program(whisper, wav.clone());
            app.fake_recording = Some(wav);
        });
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
    assert_eq!(Screen::named("voortgang"), Some(Screen::Progress));
    app.show(cx, Screen::Progress);
    assert!(app.read(cx, |app| app.progress.as_ref().is_some_and(|p| !p.started)), "nothing practised yet");
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
    app.recording(cx, "#!/bin/sh\nprintf 'Hallo   daar.\\n'\n");
    app.press(cx, "r");
    assert!(app.read(cx, |app| {
        let attempt = &app.study.attempt;
        !attempt.judging && attempt.has_recording && attempt.speech.as_ref().is_some_and(|speech| speech.heard == "Hallo daar.")
    }));
    assert!(app.dir.path().join("recordings/laatste.wav").exists());
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
    assert_eq!(app.read(cx, |app| app.study.streak.days), 1, "Alles gedaan shows the streak");
    // Voortgang: one card scored and timed, the other skipped and still waiting.
    app.show(cx, Screen::Progress);
    let progress = app.read(cx, |app| app.progress.clone().unwrap());
    assert_eq!((progress.streak.days, progress.today.done, progress.today.waiting, progress.learned), (1, 1, 1, 1));
    assert!(progress.started && progress.reviews == 1 && progress.writing.is_some() && progress.seconds >= 0.0);
    app.show(cx, Screen::Study);
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
    app.recording(cx, "#!/bin/sh\necho 'testfout' >&2\nexit 1\n");
    // The logged-in tools as Instellingen would find them, so none are run here.
    let model = |cli, id: &str, efforts: Vec<Effort>, default_effort| ModelChoice { cli, id: id.into(), name: id.into(), efforts, default_effort };
    let choices = vec![
        model(Cli::ClaudeCode, "opus", Effort::ALL.to_vec(), Effort::Medium),
        model(Cli::ClaudeCode, "sonnet", Effort::ALL.to_vec(), Effort::Medium),
        model(Cli::Antigravity, "gemini-x", vec![Effort::Low, Effort::High], Effort::Low),
    ];
    app.act(cx, |app, _, _| app.llm_settings.choices = Some(choices));
    app.show(cx, Screen::Settings);
    app.click(cx, ["vrouw".into(), "licht".into(), "more".into(), "mic-test".into()]);
    let settings = app.read(cx, |app| app.settings.clone());
    assert_eq!((settings.voice_id(), settings.appearance, settings.new_per_day), ("nathalie", Appearance::Light, 11));
    assert!(app.dir.path().join("recordings/microfoontest.wav").exists(), "the stand-in recording is saved for playback");
    assert!(app.read(cx, |app| !app.mic_test.is_busy()));
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

#[gpui_kit::test]
fn dutch_news_offers_todays_articles_before_starting_the_conversation(cx: &mut TestAppContext) {
    let app = Harness::open(cx);
    let program = app.script("claude", r#"#!/bin/sh
request=$(cat)
case "$request" in
  *'"dutch_news":true'*)
    today=${request##*'"today":"'}
    today=${today%%'"'*}
    printf '{"modelUsage":{"sonnet":{"webSearchRequests":1}},"structured_output":{"sources":[{"title":"Oud bericht","url":"https://example.org/oud","date":"2000-01-01","facts":"Dit verscheen eerder."},{"title":"De dijken","url":"https://example.org/dijken","date":"%s","facts":"Er wordt gewerkt aan de dijken."}]}}\n' "$today" ;;
  *'"angle":"De dijken"'*)
    echo '{"structured_output":{"corrected":"","explanation":"","reply":"Wat vind je van de plannen voor de dijken?","reply_english":"What do you think of the plans for the dikes?"}}' ;;
  *) exit 1 ;;
esac
"#);
    app.act(cx, |app, _, _| app.llm.program = program);
    app.show(cx, Screen::Talk);
    app.click(cx, [format!("topic-{DUTCH_NEWS_TOPIC}").into()]);
    assert!(app.read(cx, |app| {
        matches!(app.talk.options.as_slice(), [article] if article.title == "De dijken")
            && app.talk.current.as_ref().is_some_and(|conversation| conversation.lines.is_empty())
    }));

    app.click(cx, ["talk-more-options".into(), row("talk-option", 0)]);
    assert!(app.read(cx, |app| app.talk.current.as_ref().is_some_and(|conversation| {
        conversation.angle.as_deref() == Some("De dijken")
            && conversation.sources.len() == 1
            && conversation.lines.first().is_some_and(|line| line.dutch.contains("dijken"))
    })));
}

/// Gesprek: a failed start, then a conversation whose English, unclear words,
/// opened meaning and correction become drafts when it ends; every piece plays.

#[gpui_kit::test]
fn a_conversation_becomes_cards(cx: &mut TestAppContext) {
    let app = Harness::open(cx);
    let program = app.script("claude", FAKE_CLAUDE);
    app.recording(cx, "#!/bin/sh\necho 'Ik heb naar de market gegaan'\n");
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
    // The sources fold out under the header, newest first.
    app.click(cx, ["talk-all-sources".into(), row("talk-source", 0), "talk-all-sources".into()]);
    app.click(cx, ["talk-speak".into()]);
    assert!(app.read(cx, |app| {
        let line = &app.talk.current.as_ref().unwrap().lines[1];
        line.spoken && line.corrected.as_deref() == Some("Ik ben naar de markt gegaan.") && app.talk.pending.is_none()
    }), "spoken, transcribed and answered");
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
    app.act(cx, |app, _, _| app.llm_settings.choices = Some(Vec::new()));
    app.show(cx, Screen::Settings);
    app.click(cx, ["usage-per-day".into()]);
    assert!(app.read(cx, |app| app.llm_settings.usage_by_day), "the breakdown turns to days");
    app.show(cx, Screen::Talk);
    app.click(cx, ["topic-Het weekend".into(), row("talk-option", 0)]);
    app.act(cx, |app, window, cx| app.talk.message.set_value("Hoe vraag ik beleefd om hulp?", window, cx));
    app.click(cx, ["talk-send".into()]);
    assert!(app.read(cx, |app| app.talk.current.as_ref().unwrap().transcript(None).contains("Leervraag: Beleefd om hulp vragen")));
    app.act(cx, |app, window, cx| app.talk.message.set_value("Tot ziens!", window, cx));
    app.click(cx, ["talk-send".into()]);
    assert!(app.read(cx, |app| app.talk.current.is_none() && app.drafts.len() == 2), "goodbye closes and reviews the transcript");
    assert_eq!(app.read(cx, |app| app.store.drafts().unwrap()[1].dutch.clone()), "Zou u mij kunnen helpen?");

    app.show(cx, Screen::Talk);
    app.click(cx, ["talk-recap".into()]);
    app.click(cx, ["topic-Het weekend".into(), row("talk-option", 2)]);
    app.act(cx, |app, window, cx| app.talk.message.set_value("Hoe vraag ik beleefd om hulp?", window, cx));
    app.click(cx, ["talk-send".into()]);
    app.act(cx, |app, _, _| app.llm.program = missing);
    app.click(cx, ["talk-finish".into()]);
    assert!(app.read(cx, |app| app.talk.current.is_none() && app.talk.error.is_some() && app.drafts.len() == 2));
    // Concepten says it failed, and tries again from there.
    assert!(app.read(cx, |app| app.screen == Screen::Drafts && app.making.is_none() && app.making_failed.is_some()));
    app.act(cx, |app, _, _| app.llm.program = program);
    app.click(cx, ["making-retry".into()]);
    assert_eq!(app.read(cx, |app| (app.drafts.len(), app.making_failed.is_none(), app.making.is_none())), (3, true, true));
    // The runs that made cards were timed, with their model and length, for the next estimate.
    app.read(cx, |app| {
        let runs = app.store.drafting_runs().unwrap();
        assert_eq!(runs.len(), 3, "the three that made cards, not the one that failed");
        assert!(runs.iter().all(|run| run.model == app.settings.model && run.effort == app.settings.effort.as_str() && run.chars > 20));
    });
    // Gesprek no longer shows the failure either.
    app.show(cx, Screen::Talk);
    assert!(app.read(cx, |app| app.talk.error.is_none()));

    // Each conversation counts towards Voortgang from its first answered line.
    app.show(cx, Screen::Progress);
    let progress = app.read(cx, |app| app.progress.clone().unwrap());
    assert_eq!(progress.talking.map(|t| (t.talks, t.lines)), Some((3, 4)));
    assert_eq!((progress.started, progress.streak.days, progress.reviews), (true, 1, 0), "a conversation keeps the streak");
}

/// A bad export of the window's icon fails here, not quietly at start-up.
#[test]
fn the_window_icon_loads() {
    let icon = crate::app_icon().expect("the icon decodes");
    assert_eq!((icon.width(), icon.height()), (256, 256));
}

/// Gesprek with the partner's opening line, waiting for the learner's answer.
fn open_conversation(app: &Harness, cx: &mut TestAppContext) {
    use dutch_flashcards_core::conversation::{Conversation, Reply};
    let mut conversation = Conversation::new("");
    conversation.add_turn(None, Reply { reply: "Hallo! Hoe gaat het?".into(), ..Default::default() });
    app.act(cx, |app, _, _| app.talk.current = Some(conversation));
    app.show(cx, Screen::Talk);
}

/// Dutch the dictionary does not know is underlined in the answer, but not
/// the word still being typed.
#[gpui::test]
fn unknown_dutch_in_an_answer_is_underlined(cx: &mut TestAppContext) {
    let app = Harness::open(cx);
    open_conversation(&app, cx);
    let underlined = |cx: &mut TestAppContext| cx.read(|cx| app.app.read(cx).talk.message.underlined(cx));
    app.act(cx, |app, window, cx| app.talk.message.set_value("Wij hebben het al gezeggd.", window, cx));
    assert_eq!(underlined(cx), ["gezeggd"]);
    app.act(cx, |app, window, cx| app.talk.message.set_value("", window, cx));

    app.act(cx, |app, window, cx| app.talk.focus(window, cx));
    app.input(cx, |window, cx| window.input("Ik heb het gezeggd", cx));
    assert!(underlined(cx).is_empty(), "still typing it");
    app.input(cx, |window, cx| window.input(" en", cx));
    assert_eq!(underlined(cx), ["gezeggd"]);
    app.press(cx, "shift-enter");
    let raw = cx.read(|cx| app.app.read(cx).talk.message.state.read(cx).value().to_string());
    assert_eq!(raw, "Ik heb het gezeggd en", "one line only");
}

/// Right-clicking an underlined word offers what was meant, and choosing it fixes the word.
#[gpui::test]
fn the_right_click_menu_fixes_a_word(cx: &mut TestAppContext) {
    let app = Harness::open(cx);
    open_conversation(&app, cx);
    // Long enough that the middle of the field, where the click lands, is on a word.
    let answer = ["gezeggd"; 12].join(" ");
    app.act(cx, |app, window, cx| app.talk.message.set_value(&answer, window, cx));
    let field = app.read(cx, |app| app.talk.message.state.entity_id());
    app.input(cx, |window, cx| window.right_click(("input", field), cx));
    app.press(cx, "down enter");
    let answer = cx.read(|cx| app.app.read(cx).talk.message.value(cx));
    assert_eq!(answer.matches("gezegd ").count(), 1, "{answer}");
}

/// The conversation shows which model answers, and a click opens the choice.
#[gpui::test]
fn the_conversation_shows_its_model_and_opens_the_choice(cx: &mut TestAppContext) {
    let app = Harness::open(cx);
    open_conversation(&app, cx);
    app.act(cx, |app, _, _| (app.settings.cli, app.settings.model, app.settings.effort) = (Cli::ClaudeCode, "opus".into(), Effort::High));
    app.click(cx, ["talk-llm".into()]);
    assert_eq!(app.read(cx, |app| app.screen), Screen::Settings);
}

/// Making cards from a pasted text moves to Concepten; when the model fails,
/// Concepten says so and leads back to the text, still in Toevoegen.
#[gpui::test]
fn a_failed_paste_leads_back_to_its_text(cx: &mut TestAppContext) {
    let app = Harness::open(cx);
    let missing = app.dir.path().join("no-such-program");
    app.act(cx, |app, window, cx| {
        app.llm.program = missing;
        app.add.snippet.update(cx, |snippet, cx| snippet.set_value("Ik heb gisteren lang gewerkt.", window, cx));
    });
    app.show(cx, Screen::Add);
    app.click(cx, ["make".into()]);
    assert!(app.read(cx, |app| app.screen == Screen::Drafts && app.making.is_none() && app.making_failed.is_some()));
    app.click(cx, ["making-back".into()]);
    assert_eq!(app.read(cx, |app| app.screen), Screen::Add);
    let text = cx.read(|cx| app.app.read(cx).add.snippet.read(cx).value().to_string());
    assert_eq!(text, "Ik heb gisteren lang gewerkt.");
}
