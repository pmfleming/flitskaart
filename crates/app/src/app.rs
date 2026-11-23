//! The window: a tab strip over five screens, sharing one deck.

use std::time::Duration;

use chrono::Utc;
use dutch_flashcards_core::paths::Paths;
use dutch_flashcards_core::schedule::Scheduler;
use dutch_flashcards_core::speech::Listener;
use dutch_flashcards_core::store::{Appearance, Settings};
use dutch_flashcards_core::voice::{self, Speaker, Speed, VoiceOption};
use dutch_flashcards_core::Store;
use gpui_kit::component::tab::{Tab, TabBar};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::theme::{Theme, ThemeMode};
use gpui_kit::component::{ActiveTheme as _, Sizable as _, h_flex, v_flex};
use gpui_kit::*;

use crate::add::AddState;
use crate::audio::Audio;
use crate::deck::DeckState;
use crate::drafts::DraftRow;
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
    const ALL: [Screen; 5] = [Screen::Study, Screen::Add, Screen::Drafts, Screen::Deck, Screen::Settings];

    fn label(self) -> &'static str {
        match self {
            Screen::Study => "Oefenen",
            Screen::Add => "Toevoegen",
            Screen::Drafts => "Concepten",
            Screen::Deck => "Kaarten",
            Screen::Settings => "Instellingen",
        }
    }
}

/// A message across the bottom of the window.
pub struct Notice {
    pub text: SharedString,
    pub is_error: bool,
}

pub struct FlashcardsApp {
    pub store: Store,
    pub scheduler: Scheduler,
    pub speaker: Speaker,
    pub listener: Listener,
    /// A WAV file used in place of the microphone (`--recording`), for checks.
    pub fake_recording: Option<std::path::PathBuf>,
    pub settings: Settings,
    pub paths: Paths,
    pub audio: Audio,
    pub screen: Screen,
    pub study: StudyState,
    pub mic_test: crate::mic_test::MicTest,
    pub add: AddState,
    pub drafts: Vec<DraftRow>,
    pub deck: DeckState,
    pub deck_query: String,
    pub notice: Option<Notice>,
    /// Bumped on every playback so a finished sound only clears its own indicator.
    playback: u64,
    pub playing: bool,
    pub preparing_audio: bool,
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
            _appearance_changed: appearance_changed,
            scheduler: Scheduler::default(),
            speaker: Speaker::new(&paths),
            listener: Listener::new(&paths),
            fake_recording: None,
            study: StudyState::new(settings.mode, cx),
            mic_test: Default::default(),
            add: AddState::new(window, cx),
            deck: DeckState::new(window, cx),
            deck_query: String::new(),
            drafts: Vec::new(),
            settings,
            store,
            paths,
            audio: Audio::default(),
            screen: Screen::Study,
            notice: None,
            playback: 0,
            playing: false,
            preparing_audio: false,
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
            Screen::Study => {
                if self.study.card.is_none() {
                    self.load_next_card(cx);
                }
                self.study.focus.focus(window, cx);
            }
            Screen::Deck => self.reload_deck(),
            Screen::Add => self.add.snippet.focus_handle(cx).focus(window, cx),
            _ => {}
        }
        cx.notify();
    }

    pub fn error(&mut self, text: impl std::fmt::Display) {
        self.notice = Some(Notice { text: text.to_string().into(), is_error: true });
    }

    pub fn info(&mut self, text: impl Into<SharedString>) {
        self.notice = Some(Notice { text: text.into(), is_error: false });
    }

    pub fn current_voice(&self) -> &'static VoiceOption {
        voice::voice(self.settings.voice_id())
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

    /// Speaks `text` with `voice`: generates the audio in the background if it
    /// is not cached yet, then plays it.
    pub fn speak(&mut self, text: String, voice: &'static VoiceOption, speed: Speed, cx: &mut Context<Self>) {
        let speaker = self.speaker.clone();
        self.preparing_audio = !speaker.audio_path(&text, voice, speed).exists();
        self.playback += 1;
        let ticket = self.playback;
        cx.notify();
        let job = cx.background_executor().spawn(async move { speaker.speech(&text, voice, speed) });
        cx.spawn(async move |this, cx| {
            let result = job.await;
            let finished = this.update(cx, |this, cx| {
                this.preparing_audio = false;
                if this.playback != ticket {
                    return None;
                }
                let played = result.and_then(|path| this.audio.play(&path));
                let length = match played {
                    Ok(length) => length,
                    Err(err) => {
                        this.error(format!("{err:#}"));
                        cx.notify();
                        return None;
                    }
                };
                this.playing = true;
                cx.notify();
                Some(cx.background_executor().timer(length + Duration::from_millis(150)))
            });
            if let Ok(Some(timer)) = finished {
                timer.await;
                let _ = this.update(cx, |this, cx| {
                    if this.playback == ticket {
                        this.playing = false;
                        cx.notify();
                    }
                });
            }
        })
        .detach();
    }

    /// Generates audio ahead of time so playback starts instantly later.
    pub fn prepare_audio(&self, text: String, cx: &mut Context<Self>) {
        let speaker = self.speaker.clone();
        let voice = self.current_voice();
        cx.background_executor()
            .spawn(async move {
                let _ = speaker.speech(&text, voice, Speed::Normal);
            })
            .detach();
    }

    pub fn stop_audio(&mut self) {
        self.playback += 1;
        self.playing = false;
        self.audio.stop();
    }

    fn render_tabs(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let selected = Screen::ALL.iter().position(|s| *s == self.screen).unwrap_or(0);
        let draft_count = self.drafts.len();
        TabBar::new("screens")
            .underline()
            .selected_index(selected)
            .on_click(cx.listener(|this, index: &usize, window, cx| {
                this.show(Screen::ALL[*index], window, cx);
            }))
            .children(Screen::ALL.map(|screen| {
                let tab = Tab::new().label(screen.label());
                if screen == Screen::Drafts && draft_count > 0 {
                    tab.suffix(
                        div()
                            .ml_1()
                            .px_1p5()
                            .rounded_full()
                            .bg(cx.theme().primary)
                            .text_color(cx.theme().primary_foreground)
                            .text_xs()
                            .child(draft_count.to_string()),
                    )
                } else {
                    tab
                }
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
                .child(
                    Button::new("dismiss-notice")
                        .label("Sluiten")
                        .small()
                        .ghost()
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.notice = None;
                            cx.notify();
                        })),
                ),
        )
    }

    pub fn now() -> chrono::DateTime<Utc> {
        Utc::now()
    }

    /// Opens a screen and replays study keys, for screenshots and checks.
    pub fn replay(&mut self, screen: Option<&str>, keys: &[String], window: &mut Window, cx: &mut Context<Self>) {
        if let Some(name) = screen {
            match Screen::ALL.iter().find(|s| s.label().eq_ignore_ascii_case(name)) {
                Some(screen) => self.show(*screen, window, cx),
                None => self.error(format!("Onbekend scherm: {name}")),
            }
        }
        self.replay_keys(keys.to_vec(), window, cx);
    }

    /// Replays study keys (on Instellingen, `microfoon` runs the microphone test);
    /// `wacht` waits until recording and listening are done.
    fn replay_keys(&mut self, mut keys: Vec<String>, window: &mut Window, cx: &mut Context<Self>) {
        while !keys.is_empty() {
            let key = keys.remove(0);
            if key == "wacht" {
                cx.spawn_in(window, async move |this, cx| {
                    loop {
                        cx.background_executor().timer(Duration::from_millis(100)).await;
                        let busy = this.update(cx, |this, _| this.busy_listening() || this.mic_test.is_busy()).unwrap_or(false);
                        if !busy {
                            break;
                        }
                    }
                    let _ = this.update_in(cx, |this, window, cx| this.replay_keys(keys, window, cx));
                })
                .detach();
                return;
            }
            if self.screen == Screen::Study {
                self.study_key(&key, window, cx);
            } else if self.screen == Screen::Settings && key == "microfoon" {
                self.start_mic_test(cx);
            }
        }
    }
}

impl Render for FlashcardsApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let body = match self.screen {
            Screen::Study => self.render_study(window, cx).into_any_element(),
            Screen::Add => self.render_add(window, cx).into_any_element(),
            Screen::Drafts => self.render_drafts(window, cx).into_any_element(),
            Screen::Deck => self.render_deck(window, cx).into_any_element(),
            Screen::Settings => self.render_settings(window, cx).into_any_element(),
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
