//! Dutch flashcards: a desktop window built with GPUI Community Edition.

mod add;
mod app;
mod audio;
mod deck;
mod drafts;
mod settings;
mod study;
mod ui;

use dutch_flashcards_core::store::Appearance;
use dutch_flashcards_core::{Store, paths::Paths};
use gpui_kit::*;

/// Developer flags for screenshots and checks:
/// `--screen <oefenen|toevoegen|concepten|kaarten|instellingen>` opens that screen,
/// `--keys "space 3"` replays study shortcuts after start-up,
/// `--appearance <systeem|licht|donker>` overrides the colours for this run only.
#[derive(Default)]
struct DevArgs {
    screen: Option<String>,
    keys: Vec<String>,
    appearance: Option<Appearance>,
}

fn dev_args() -> DevArgs {
    let mut out = DevArgs::default();
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--screen" => out.screen = args.next(),
            "--keys" => out.keys = args.next().unwrap_or_default().split_whitespace().map(String::from).collect(),
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
    let store = match Store::open(&paths) {
        Ok(store) => store,
        Err(err) => {
            eprintln!("Kan de kaarten niet openen in {}: {err:#}", paths.root().display());
            std::process::exit(1);
        }
    };

    gpui_kit::application()
        .with_assets(gpui_kit::assets::Assets)
        .with_quit_mode(QuitMode::LastWindowClosed)
        .run(move |cx| {
            gpui_kit::init(cx);
            let bounds = Bounds::centered(None, size(px(1000.), px(740.)), cx);
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(TitlebarOptions {
                    title: Some("Nederlandse kaarten".into()),
                    ..Default::default()
                }),
                app_id: Some("dutch-flashcards".into()),
                window_min_size: Some(size(px(560.), px(480.))),
                ..Default::default()
            };
            gpui_kit::open_window(options, cx, |window, cx| {
                cx.new(|cx| {
                    let mut app = app::FlashcardsApp::new(store, paths, window, cx);
                    if let Some(appearance) = dev.appearance {
                        app.settings.appearance = appearance;
                        app.apply_appearance(window, cx);
                    }
                    app.replay(dev.screen.as_deref(), &dev.keys, window, cx);
                    app
                })
            })
            .expect("could not open the window");
            cx.activate(true);
        });
}
