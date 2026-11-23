//! The window: a tab strip over five screens, sharing one deck.

use std::time::Duration;

use dutch_flashcards_core::Store;
use dutch_flashcards_core::paths::Paths;
use dutch_flashcards_core::speech::Listener;
use dutch_flashcards_core::store::{Appearance, Settings};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::tab::{Tab, TabBar};
use gpui_kit::component::theme::{Theme, ThemeMode};
use gpui_kit::component::{ActiveTheme as _, Sizable as _, h_flex, v_flex};
use gpui_kit::{
    Context, IntoElement, ParentElement as _, Render, ScrollHandle, SharedString, Styled as _,
    Subscription, Window, div,
};

use crate::add::AddState;
use crate::audio::Sound;
use crate::deck::DeckState;
use crate::drafts::DraftRow;
use crate::mic_test::MicTest;
use crate::study::StudyState;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Screen {
    Study,
    Add,
    Drafts,
    Deck,
    Settings,
}

impl Screen {
    pub const ALL: [Screen; 5] = [Screen::Study, Screen::Add, Screen::Drafts, Screen::Deck, Screen::Settings];
    const LABELS: [&str; 5] = ["Oefenen", "Toevoegen", "Concepten", "Kaarten", "Instellingen"];

    pub fn label(self) -> &'static str {
        Screen::LABELS[self as usize]
    }

    /// The screen with this label, ignoring case.
    pub fn named(name: &str) -> Option<Screen> {
        Screen::ALL.into_iter().find(|screen| screen.label().eq_ignore_ascii_case(name))
    }
}

/// A message across the bottom of the window.
pub struct Notice {
    pub text: SharedString,
    pub is_error: bool,
}

pub struct FlashcardsApp {
    pub store: Store,
    pub listener: Listener,
    /// A WAV file used in place of the microphone (`--recording`), for checks.
    pub fake_recording: Option<std::path::PathBuf>,
    pub settings: Settings,
    pub paths: Paths,
    pub sound: Sound,
    pub screen: Screen,
    pub study: StudyState,
    pub mic_test: MicTest,
    /// Instellingen scrolls; the microphone test brings its result into view.
    pub settings_scroll: ScrollHandle,
    pub add: AddState,
    pub drafts: Vec<DraftRow>,
    pub deck: DeckState,
    pub notice: Option<Notice>,
    _appearance_changed: Subscription,
}

impl FlashcardsApp {
    pub fn new(store: Store, paths: Paths, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let settings = store.settings().unwrap_or_default();
        // Follow the desktop's light/dark switch while the setting says "Systeem".
        let appearance_changed = cx.observe_window_appearance(window, |this, window, cx| {
            if this.settings.appearance == Appearance::System {
                Theme::sync_system_appearance(Some(window), cx);
            }
        });
        let mut app = FlashcardsApp {
            sound: Sound::new(&paths),
            listener: Listener::new(&paths),
            fake_recording: None,
            study: StudyState::new(settings.mode, cx),
            mic_test: MicTest::new(settings.input_boost, cx),
            settings_scroll: ScrollHandle::new(),
            add: AddState::new(window, cx),
            deck: DeckState::new(window, cx),
            drafts: Vec::new(),
            settings,
            store,
            paths,
            screen: Screen::Study,
            notice: None,
            _appearance_changed: appearance_changed,
        };
        app.apply_appearance(window, cx);
        app.reload_drafts(window, cx);
        app.reload_deck();
        app.load_next_card(cx);
        app.study.focus.focus(window, cx);
        app
    }

    pub fn show(&mut self, screen: Screen, window: &mut Window, cx: &mut Context<Self>) {
        self.screen = screen;
        self.notice = None;
        match screen {
            Screen::Study if self.study.card.is_none() => self.load_next_card(cx),
            Screen::Deck => self.reload_deck(),
            Screen::Settings => self.refresh_inputs(cx),
            _ => {}
        }
        match screen {
            Screen::Study => self.study.focus.focus(window, cx),
            Screen::Add => self.add.focus(window, cx),
            _ => {}
        }
        cx.notify();
    }

    /// Shows an error across the bottom of the window, and logs it.
    pub fn error(&mut self, text: impl std::fmt::Display) {
        let text = crate::logging::error(self.screen.label(), text);
        self.notice = Some(Notice { text: text.into(), is_error: true });
    }

    pub fn info(&mut self, text: impl Into<SharedString>) {
        self.notice = Some(Notice { text: text.into(), is_error: false });
    }

    /// Whether a recording or Whisper is busy, in Oefenen or the microphone test.
    pub fn busy(&self) -> bool {
        self.busy_listening() || self.mic_test.is_busy()
    }

    /// Applies the light/dark setting to the window.
    pub fn apply_appearance(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.settings.appearance {
            Appearance::System => Theme::sync_system_appearance(Some(window), cx),
            Appearance::Light => Theme::change(ThemeMode::Light, Some(window), cx),
            Appearance::Dark => Theme::change(ThemeMode::Dark, Some(window), cx),
        }
    }

    pub fn save_settings(&mut self) {
        if let Err(err) = self.store.save_settings(&self.settings) {
            self.error(format!("Instellingen niet bewaard: {err:#}"));
        }
    }

    /// Runs `job` off the UI thread, then hands its result to `then` back on it.
    pub fn in_background<T: Send + 'static>(
        &self,
        cx: &mut Context<Self>,
        job: impl FnOnce() -> T + Send + 'static,
        then: impl FnOnce(&mut Self, T, &mut Context<Self>) + 'static,
    ) {
        let task = cx.background_executor().spawn(async move { job() });
        cx.spawn(async move |this, cx| {
            let value = task.await;
            let _ = this.update(cx, |this, cx| {
                then(this, value, cx);
                cx.notify();
            });
        })
        .detach();
    }

    /// Runs `then` after `delay`.
    pub fn after(&self, delay: Duration, cx: &mut Context<Self>, then: impl FnOnce(&mut Self, &mut Context<Self>) + 'static) {
        let timer = cx.background_executor().timer(delay);
        cx.spawn(async move |this, cx| {
            timer.await;
            let _ = this.update(cx, |this, cx| {
                then(this, cx);
                cx.notify();
            });
        })
        .detach();
    }

    pub fn now() -> chrono::DateTime<chrono::Utc> {
        chrono::Utc::now()
    }

    fn render_tabs(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let selected = Screen::ALL.iter().position(|s| *s == self.screen).unwrap_or(0);
        let drafts = self.drafts.len();
        let theme = cx.theme().clone();
        TabBar::new("screens")
            .underline()
            .selected_index(selected)
            .on_click(cx.listener(|this, index: &usize, window, cx| this.show(Screen::ALL[*index], window, cx)))
            .children(Screen::ALL.map(|screen| {
                let tab = Tab::new().label(screen.label());
                if screen != Screen::Drafts || drafts == 0 {
                    return tab;
                }
                let badge = div().ml_1().px_1p5().rounded_full().bg(theme.primary).text_color(theme.primary_foreground);
                tab.suffix(badge.text_xs().child(drafts.to_string()))
            }))
    }

    fn render_notice(&mut self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        let notice = self.notice.as_ref()?;
        let theme = cx.theme();
        let color = if notice.is_error { theme.danger } else { theme.foreground };
        Some(
            h_flex()
                .px_4()
                .py_2()
                .gap_3()
                .border_t_1()
                .border_color(theme.border)
                .bg(theme.secondary)
                .child(div().flex_1().text_sm().text_color(color).child(notice.text.clone()))
                .child(Button::new("dismiss-notice").label("Sluiten").small().ghost().on_click(cx.listener(|this, _, _, cx| {
                    this.notice = None;
                    cx.notify();
                }))),
        )
    }
}

impl Render for FlashcardsApp {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let body = match self.screen {
            Screen::Study => self.render_study(cx).into_any_element(),
            Screen::Add => self.render_add(cx).into_any_element(),
            Screen::Drafts => self.render_drafts(cx).into_any_element(),
            Screen::Deck => self.render_deck(cx).into_any_element(),
            Screen::Settings => self.render_settings(cx).into_any_element(),
        };
        let theme = cx.theme();
        v_flex()
            .size_full()
            .bg(theme.background)
            .text_color(theme.foreground)
            .child(div().px_3().bg(theme.tab_bar).border_b_1().border_color(theme.border).child(self.render_tabs(cx)))
            .child(div().flex_1().min_h_0().child(body))
            .children(self.render_notice(cx))
    }
}
