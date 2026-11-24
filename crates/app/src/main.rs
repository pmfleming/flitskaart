//! Dutch flashcards: a desktop window built with GPUI Community Edition.

mod add;
mod app;
mod audio;
mod capture;
mod deck;
mod drafts;
mod icons;
mod logging;
mod mic_test;
mod recorder;
mod settings;
mod speaking;
mod study;
mod talk;
#[cfg(test)]
mod tests;
mod ui;
mod writing;

use std::collections::VecDeque;
use std::time::Duration;

use dutch_flashcards_core::store::Appearance;
use dutch_flashcards_core::{Store, paths::Paths};
use gpui_kit::{
    AppContext as _, Bounds, Context, QuitMode, TitlebarOptions, Window, WindowBounds, WindowOptions, px, size,
};

use crate::app::{FlashcardsApp, Screen};

/// Developer flags for screenshots and checks:
/// `--screen <oefenen|gesprek|toevoegen|concepten|kaarten|instellingen>` opens that screen,
/// `--keys "space 3"` replays study shortcuts after start-up,
/// `--appearance <systeem|licht|donker>` overrides the colours for this run only,
/// `--recording <file.wav>` uses a WAV file instead of the microphone.
#[derive(Default)]
struct DevArgs {
    screen: Option<String>,
    keys: VecDeque<String>,
    appearance: Option<Appearance>,
    recording: Option<std::path::PathBuf>,
}

fn dev_args() -> DevArgs {
    let mut out = DevArgs::default();
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--screen" => out.screen = args.next(),
            "--keys" => out.keys = args.next().unwrap_or_default().split_whitespace().map(String::from).collect(),
            "--recording" => out.recording = args.next().map(Into::into),
            "--appearance" => out.appearance = args.next().as_deref().and_then(Appearance::parse),
            other => {
                eprintln!("Onbekende optie: {other}");
                std::process::exit(2);
            }
        }
    }
    out
}

fn main() {
    let dev = dev_args();
    let paths = Paths::from_env();
    logging::init(&paths.log_file());
    let store = match Store::open(&paths) {
        Ok(store) => store,
        Err(err) => {
            logging::error("start", format!("Kan de kaarten niet openen in {}: {err:#}", paths.root().display()));
            std::process::exit(1);
        }
    };

    gpui_kit::application().with_assets(gpui_kit::assets::Assets).with_quit_mode(QuitMode::LastWindowClosed).run(move |cx| {
        gpui_kit::init(cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::centered(None, size(px(1000.), px(740.)), cx))),
            titlebar: Some(TitlebarOptions { title: Some("Nederlandse kaarten".into()), ..Default::default() }),
            app_id: Some("dutch-flashcards".into()),
            window_min_size: Some(size(px(560.), px(480.))),
            ..Default::default()
        };
        let opened = gpui_kit::open_window(options, cx, |window, cx| {
            cx.new(|cx| {
                let mut app = FlashcardsApp::new(store, paths, window, cx);
                app.replay(dev, window, cx);
                app
            })
        });
        if let Err(err) = opened {
            logging::error("start", format!("Het venster ging niet open: {err:#}"));
            cx.quit();
        }
        cx.activate(true);
    });
}

impl FlashcardsApp {
    /// Applies the developer flags: a stand-in recording, colours, a screen and keys.
    fn replay(&mut self, dev: DevArgs, window: &mut Window, cx: &mut Context<Self>) {
        self.fake_recording = dev.recording;
        if let Some(appearance) = dev.appearance {
            self.settings.appearance = appearance;
            self.apply_appearance(window, cx);
        }
        match dev.screen.as_deref().map(|name| (name, Screen::named(name))) {
            Some((_, Some(screen))) => self.show(screen, window, cx),
            Some((name, None)) => self.error(format!("Onbekend scherm: {name}")),
            None => {}
        }
        self.replay_keys(dev.keys, window, cx);
    }

    /// Replays study keys; on Instellingen, `microfoon` runs the microphone
    /// test; on Gesprek, `begin` starts a conversation and `spreek` answers
    /// by voice. `wacht` waits until recording, listening and Claude are done.
    fn replay_keys(&mut self, mut keys: VecDeque<String>, window: &mut Window, cx: &mut Context<Self>) {
        while let Some(key) = keys.pop_front() {
            match (self.screen, key.as_str()) {
                (_, "wacht") => return self.replay_when_idle(keys, window, cx),
                (Screen::Study, key) => _ = self.study_key(key, window, cx),
                (Screen::Settings, "microfoon") => self.start_mic_test(cx),
                (Screen::Talk, "begin") => self.start_conversation(window, cx),
                (Screen::Talk, "spreek") => self.speak_answer(cx),
                _ => {}
            }
        }
    }

    fn replay_when_idle(&mut self, keys: VecDeque<String>, window: &mut Window, cx: &mut Context<Self>) {
        cx.spawn_in(window, async move |this, cx| {
            while this.update(cx, |this, _| this.busy()).unwrap_or(false) {
                cx.background_executor().timer(Duration::from_millis(100)).await;
            }
            let _ = this.update_in(cx, |this, window, cx| this.replay_keys(keys, window, cx));
        })
        .detach();
    }
}
