//! The window: a tab strip over six screens, sharing one deck.

use std::time::Duration;

use dutch_flashcards_core::Store;
use dutch_flashcards_core::llm::{Cli, Llm, ModelChoice};
use dutch_flashcards_core::paths::Paths;
use dutch_flashcards_core::speech::Listener;
use dutch_flashcards_core::store::{Appearance, Settings};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::tab::{Tab, TabBar};
use gpui_kit::component::theme::{Theme, ThemeMode};
use gpui_kit::component::{ActiveTheme as _, Colorize as _, Icon, h_flex, v_flex};
use gpui_kit::{
    Context, InteractiveElement as _, IntoElement, KeyDownEvent, ParentElement as _, Render, ScrollHandle, SharedString,
    Styled as _, Subscription, Window, div, px,
};

use crate::add::AddState;
use crate::audio::Sound;
use crate::deck::DeckState;
use crate::drafts::DraftRow;
use crate::mic_test::MicTest;
use crate::study::StudyState;
use crate::talk::TalkState;
use crate::icons;
use crate::ui::{apply_palette, icon_button, surface};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Screen {
    Study,
    Talk,
    Add,
    Drafts,
    Deck,
    Settings,
}

impl Screen {
    pub const ALL: [Screen; 6] = [Screen::Study, Screen::Talk, Screen::Add, Screen::Drafts, Screen::Deck, Screen::Settings];
    const LABELS: [&str; 6] = ["Oefenen", "Gesprek", "Toevoegen", "Concepten", "Kaarten", "Instellingen"];

    pub fn label(self) -> &'static str {
        Screen::LABELS[self as usize]
    }

    pub fn icon(self) -> Icon {
        [icons::practice, icons::talk, icons::paste, icons::drafts, icons::deck, icons::sliders][self as usize]()
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
    /// Claude Code, for drafting cards and for conversations.
    pub llm: Llm,
    /// A WAV file used in place of the microphone (`--recording`), for checks.
    pub fake_recording: Option<std::path::PathBuf>,
    pub settings: Settings,
    pub paths: Paths,
    pub sound: Sound,
    pub screen: Screen,
    pub study: StudyState,
    pub talk: TalkState,
    pub mic_test: MicTest,
    /// Instellingen scrolls; the microphone test brings its result into view.
    pub settings_scroll: ScrollHandle,
    /// Instellingen shows Claude's use per day instead of per model.
    pub usage_by_day: bool,
    /// The models the logged-in tools offer; `None` until they have been asked.
    pub llm_choices: Option<Vec<ModelChoice>>,
    pub finding_models: bool,
    /// The tool whose models Instellingen shows.
    pub llm_tab: Option<Cli>,
    pub add: AddState,
    pub drafts: Vec<DraftRow>,
    pub deck: DeckState,
    pub notice: Option<Notice>,
    notice_ticket: u64,
    _appearance_changed: Subscription,
}

impl FlashcardsApp {
    pub fn new(store: Store, paths: Paths, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let settings = store.settings().unwrap_or_default();
        // Follow the desktop's light/dark switch while the setting says "Systeem".
        let appearance_changed = cx.observe_window_appearance(window, |this, window, cx| {
            if this.settings.appearance == Appearance::System {
                Theme::sync_system_appearance(Some(window), cx);
                apply_palette(cx);
            }
        });
        let mut app = FlashcardsApp {
            sound: Sound::new(&paths),
            listener: Listener::new(&paths),
            llm: Llm::from_env(),
            fake_recording: None,
            study: StudyState::new(settings.mode, cx),
            talk: TalkState::new(window, cx),
            mic_test: MicTest::new(settings.input_boost, cx),
            settings_scroll: ScrollHandle::new(),
            usage_by_day: false,
            llm_choices: None,
            finding_models: false,
            llm_tab: None,
            add: AddState::new(window, cx),
            deck: DeckState::new(window, cx),
            drafts: Vec::new(),
            settings,
            store,
            paths,
            screen: Screen::Study,
            notice: None,
            notice_ticket: 0,
            _appearance_changed: appearance_changed,
        };
        let (cli, model, effort) = (app.settings.cli, app.settings.model.clone(), app.settings.effort);
        app.llm.choose(cli, &model, effort);
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
        self.notice_ticket += 1;
        match screen {
            Screen::Study if self.study.card.is_none() => self.load_next_card(cx),
            Screen::Deck => self.reload_deck(),
            Screen::Settings => {
                self.refresh_inputs(cx);
                self.find_models_once(cx);
            }
            _ => {}
        }
        match screen {
            Screen::Study => self.study.focus.focus(window, cx),
            Screen::Add => self.add.focus(window, cx),
            Screen::Talk => self.talk.focus(window, cx),
            _ => {}
        }
        cx.notify();
    }

    /// Shows an error across the bottom of the window, and logs it.
    pub fn error(&mut self, text: impl std::fmt::Display) {
        let text = crate::logging::error(self.screen.label(), text);
        self.notice_ticket += 1;
        self.notice = Some(Notice { text: text.into(), is_error: true });
    }

    /// Shows a brief confirmation notice that clears itself after a few seconds.
    pub fn info(&mut self, text: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.notice_ticket += 1;
        let ticket = self.notice_ticket;
        self.notice = Some(Notice { text: text.into(), is_error: false });
        self.after(Duration::from_secs(4), cx, move |this, _| {
            if this.notice_ticket == ticket {
                this.notice = None;
            }
        });
    }

    /// Whether a recording, Whisper or Claude is busy, in Oefenen, Gesprek or the microphone test.
    pub fn busy(&self) -> bool {
        self.busy_listening() || self.talk.busy() || self.mic_test.is_busy()
    }

    /// Applies the light/dark setting and the app's palette to the window.
    pub fn apply_appearance(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.settings.appearance {
            Appearance::System => Theme::sync_system_appearance(Some(window), cx),
            Appearance::Light => Theme::change(ThemeMode::Light, Some(window), cx),
            Appearance::Dark => Theme::change(ThemeMode::Dark, Some(window), cx),
        }
        apply_palette(cx);
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

    /// Global shortcuts: Ctrl+1..6 (or Alt+1..6) switches directly to a screen.
    fn on_global_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let m = &event.keystroke.modifiers;
        let nav_mod = (m.control || m.alt || m.platform) && !m.shift && !event.is_held;
        if !nav_mod {
            return;
        }
        if let Ok(n) = event.keystroke.key.parse::<usize>()
            && let Some(&screen) = n.checked_sub(1).and_then(|i| Screen::ALL.get(i))
        {
            self.show(screen, window, cx);
            cx.stop_propagation();
        }
    }

    fn render_tabs(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let selected = Screen::ALL.iter().position(|s| *s == self.screen).unwrap_or(0);
        let drafts = self.drafts.len();
        let waiting = self.study.counts.due + self.study.counts.new;
        let on_study = self.screen == Screen::Study;
        let theme = cx.theme().clone();
        TabBar::new("screens")
            .underline()
            .selected_index(selected)
            .on_click(cx.listener(|this, index: &usize, window, cx| this.show(Screen::ALL[*index], window, cx)))
            .children(Screen::ALL.map(|screen| {
                let tab = Tab::new().label(screen.label()).prefix(screen.icon().size_4().mr_1p5());
                match screen {
                    Screen::Drafts if drafts > 0 => {
                        let (bg, fg) = if on_study {
                            (theme.secondary, theme.muted_foreground)
                        } else {
                            (theme.accent, theme.accent_foreground)
                        };
                        let badge = div().ml_1p5().px_1p5().rounded_full().bg(bg).text_color(fg).text_xs().child(drafts.to_string());
                        tab.suffix(badge)
                    }
                    Screen::Study if !on_study && waiting > 0 => {
                        let badge = div()
                            .ml_1p5()
                            .px_1p5()
                            .rounded_full()
                            .bg(theme.secondary)
                            .text_color(theme.muted_foreground)
                            .text_xs()
                            .child(waiting.to_string());
                        tab.suffix(badge)
                    }
                    _ => tab,
                }
            }))
    }

    fn render_notice(&mut self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        let notice = self.notice.as_ref()?;
        let theme = cx.theme();
        let (bg, border, dot, text) = if notice.is_error {
            (theme.danger.opacity(0.10), theme.danger.opacity(0.35), theme.danger, theme.danger)
        } else {
            (surface(cx), theme.border, theme.primary, theme.foreground)
        };
        Some(
            h_flex()
                .px_4()
                .py_2()
                .gap_2p5()
                .items_center()
                .border_t_1()
                .border_color(border)
                .bg(bg)
                .child(div().flex_none().size(px(8.)).rounded_full().bg(dot))
                .child(div().flex_1().text_sm().text_color(text).child(notice.text.clone()))
                .child(icon_button("dismiss-notice", icons::close(), "Sluiten").on_click(cx.listener(|this, _, _, cx| {
                    this.notice = None;
                    this.notice_ticket += 1;
                    cx.notify();
                }))),
        )
    }
}

impl Render for FlashcardsApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Claude's answers note their token use; keep it with the deck.
        let used = self.llm.usage.take();
        if !used.is_empty() && let Err(err) = self.store.add_usage(&used, Self::now()) {
            crate::logging::error("verbruik", format!("{err:#}"));
        }
        let body = match self.screen {
            Screen::Study => self.render_study(cx).into_any_element(),
            Screen::Talk => self.render_talk(cx).into_any_element(),
            Screen::Add => self.render_add(cx).into_any_element(),
            Screen::Drafts => self.render_drafts(cx).into_any_element(),
            Screen::Deck => self.render_deck(window, cx).into_any_element(),
            Screen::Settings => self.render_settings(cx).into_any_element(),
        };
        let theme = cx.theme();
        v_flex()
            .size_full()
            .bg(theme.background)
            .text_color(theme.foreground)
            .on_key_down(cx.listener(Self::on_global_key))
            .child(div().px_4().bg(theme.tab_bar).border_b_1().border_color(theme.border).child(self.render_tabs(cx)))
            .child(div().flex_1().min_h_0().child(body))
            .children(self.render_notice(cx))
    }
}

/// Buttons that open other screens, each with that screen's icon; the first is the main one.
pub fn go_buttons(screens: Vec<Screen>, cx: &mut Context<FlashcardsApp>) -> Vec<Button> {
    let buttons = screens.into_iter().enumerate().map(|(i, screen)| {
        let button = Button::new(SharedString::from(format!("go-{}", screen.label()))).icon(screen.icon()).label(screen.label());
        let button = if i == 0 { button.primary() } else { button.outline() };
        button.on_click(cx.listener(move |this, _, window, cx| this.show(screen, window, cx)))
    });
    buttons.collect()
}
