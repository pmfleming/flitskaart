//! A one-line field for Dutch that underlines the words the dictionary does
//! not know; right-click one to choose what was probably meant.
//!
//! Only the toolkit's editor draws underlines under parts of the text, so the
//! field is an editor dressed as a plain input: the app's font, one line,
//! `Enter` submits, `Tab` moves on, and quotes are typed as they are.

use std::cell::{Cell, RefCell};
use std::ops::Range;
use std::rc::Rc;

use dutch_flashcards_core::spelling;
use gpui_kit::component::input::{
    Copy as CopyText, Cut, Editor, EditorState, Enter, IndentInline, InputEvent, OutdentInline, Paste, SelectAll, TextDecoration,
    TextDecorationCollection,
};
use gpui_kit::component::native_menu::NativeMenu;
use gpui_kit::component::{ActiveTheme as _, h_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    Action, App, AppContext as _, Entity, EntityInputHandler as _, HighlightStyle, InteractiveElement as _,
    IntoElement, MouseButton, MouseUpEvent, ParentElement as _, SharedString, Styled as _, Subscription, UnderlineStyle, Window, gpui, px,
};

/// Puts `with` in place of the unknown `word` at `start`, from the right-click menu.
#[derive(Action, Clone, PartialEq, Eq, serde::Deserialize)]
#[action(namespace = dutch_field, no_json)]
pub(crate) struct UseSuggestion {
    pub start: usize,
    pub word: String,
    pub with: String,
}

/// An unknown word in the field and what was probably meant, once known.
struct Unknown {
    range: Range<usize>,
    word: String,
    suggestions: Option<Vec<String>>,
}

/// What the field knows about its text, shared with its event handler.
struct Spelling {
    focused: Cell<bool>,
    unknown: RefCell<Vec<Unknown>>,
    /// Where the caret was when the right-click menu opened.
    menu_caret: Cell<usize>,
    underlines: TextDecorationCollection,
}

pub struct DutchField {
    pub state: Entity<EditorState>,
    spelling: Rc<Spelling>,
    _events: Subscription,
}

impl DutchField {
    pub fn new(value: impl Into<SharedString>, placeholder: &'static str, window: &mut Window, cx: &mut App) -> Self {
        let value = value.into();
        let mut underlines = None;
        let state = cx.new(|cx| {
            let mut state = EditorState::new(window, cx)
                .language("plaintext")
                .line_number(false)
                .folding(false)
                .indent_guides(false)
                .auto_close(false)
                .smart_indent(false)
                .searchable(false)
                .soft_wrap(true)
                .submit_on_enter(true)
                .placeholder(placeholder)
                .default_value(value);
            underlines = Some(state.create_decorations_collection(Vec::new(), cx));
            state
        });
        let spelling = Rc::new(Spelling {
            focused: Cell::new(false),
            unknown: RefCell::default(),
            menu_caret: Cell::new(0),
            underlines: underlines.expect("made with the state"),
        });
        let events = cx.subscribe(&state, {
            let spelling = spelling.clone();
            move |state, event: &InputEvent, cx| {
                match event {
                    InputEvent::Focus => spelling.focused.set(true),
                    InputEvent::Blur => spelling.focused.set(false),
                    InputEvent::Change | InputEvent::PressEnter { .. } => {}
                }
                // Typing changes the text; leaving the field finishes the word.
                check(&state, &spelling, matches!(event, InputEvent::Change), cx);
            }
        });
        check(&state, &spelling, false, cx);
        DutchField { state, spelling, _events: events }
    }

    /// The text as typed, on one line.
    pub fn value(&self, cx: &App) -> String {
        self.state.read(cx).value().replace(['\r', '\n'], " ")
    }

    pub fn set_value(&self, value: &str, window: &mut Window, cx: &mut App) {
        self.state.update(cx, |state, cx| state.set_value(value.to_string(), window, cx));
        // Setting the text sends no change, so check it here.
        check(&self.state, &self.spelling, false, cx);
    }

    /// The words underlined now.
    #[cfg(test)]
    pub fn underlined(&self, cx: &App) -> Vec<String> {
        let text = self.state.read(cx).value();
        self.spelling.underlines.get_ranges(cx).into_iter().map(|range| text[range].to_string()).collect()
    }

    /// The field in the frame of a medium-sized input.
    pub fn render(&self, disabled: bool, cx: &App) -> impl IntoElement {
        let theme = cx.theme();
        let border = if self.spelling.focused.get() && !disabled { theme.ring } else { theme.input };
        let state = self.state.clone();
        let menu = {
            let spelling = self.spelling.clone();
            move |menu: NativeMenu, _: &mut Window, _: &mut App| context_menu(menu, &spelling)
        };
        // The editor builds its menu on the button's release, in the middle of
        // an update, so note the caret (moved on the press) just before.
        let note_caret = {
            let (state, spelling) = (self.state.clone(), self.spelling.clone());
            move |event: &MouseUpEvent, _: &mut Window, cx: &mut App| {
                if event.button == MouseButton::Right {
                    spelling.menu_caret.set(state.read(cx).cursor());
                }
            }
        };
        h_flex()
            .id(("dutch-field", self.state.entity_id()))
            .w_full()
            .items_center()
            .rounded(theme.radius)
            .border_1()
            .border_color(border)
            .bg(theme.input_background())
            .when(disabled, |d| d.opacity(0.5))
            .capture_any_mouse_up(note_caret)
            // An editor indents on Tab; here Tab moves on, as in any input.
            .capture_action(|_: &IndentInline, window, cx| {
                window.focus_next(cx);
                cx.stop_propagation();
            })
            .capture_action(|_: &OutdentInline, window, cx| {
                window.focus_prev(cx);
                cx.stop_propagation();
            })
            // One line only: Shift+Enter would start a second.
            .capture_action(|enter: &Enter, _, cx| {
                if enter.shift {
                    cx.stop_propagation();
                }
            })
            .on_action(move |action: &UseSuggestion, window, cx| {
                state.update(cx, |state, cx| {
                    let end = action.start + action.word.len();
                    if state.value().get(action.start..end) != Some(action.word.as_str()) {
                        return;
                    }
                    let text = state.value();
                    let utf16 = |byte: usize| text[..byte].encode_utf16().count();
                    let range = utf16(action.start)..utf16(end);
                    state.replace_text_in_range(Some(range), &action.with, window, cx);
                });
            })
            .child(
                Editor::new(&self.state)
                    .appearance(false)
                    .disabled(disabled)
                    .context_menu(menu)
                    .flex_1()
                    .min_w_0()
                    .font_family(theme.font_family.clone())
                    .text_size(px(14.))
                    .line_height(px(20.))
                    // The editor pads its line by 8px above and below; an input's 32px needs 5.
                    .my(px(-3.)),
            )
    }
}

/// The suggestions for the unknown word under the caret, then the usual
/// cut, copy and paste.
fn context_menu(mut menu: NativeMenu, spelling: &Spelling) -> NativeMenu {
    let cursor = spelling.menu_caret.get();
    let unknown = spelling.unknown.borrow();
    if let Some(found) = unknown.iter().find(|u| u.range.contains(&cursor) || u.range.end == cursor) {
        // Normally looked up long before anyone right-clicks.
        let suggestions = found.suggestions.clone().unwrap_or_else(|| spelling::suggestions(&found.word));
        if suggestions.is_empty() {
            menu = menu.menu_with_disabled("Geen suggesties", true, Box::new(SelectAll));
        }
        for with in suggestions {
            let action = UseSuggestion { start: found.range.start, word: found.word.clone(), with: with.clone() };
            menu = menu.menu(with, Box::new(action));
        }
        menu = menu.separator();
    }
    menu.menu("Knippen", Box::new(Cut))
        .menu("Kopiëren", Box::new(CopyText))
        .menu("Plakken", Box::new(Paste))
        .separator()
        .menu("Alles selecteren", Box::new(SelectAll))
}

/// Underlines the unknown words straight away, then looks up what was meant
/// in the background. While `typing`, the word ending at the caret is left
/// alone: it may not be finished.
fn check(state: &Entity<EditorState>, spelling: &Rc<Spelling>, typing: bool, cx: &mut App) {
    if !spelling::is_ready() {
        // The dictionary is still being read; look again once it is.
        let (state, spelling) = (state.downgrade(), spelling.clone());
        let ready = cx.background_executor().spawn(async { spelling::warm_up() });
        cx.spawn(async move |cx| {
            ready.await;
            if let Some(state) = state.upgrade() {
                cx.update(|cx| check(&state, &spelling, typing, cx));
            }
        })
        .detach();
        return;
    }
    let (text, cursor) = state.read_with(cx, |state, _| (state.value().to_string(), state.cursor()));
    let unknown: Vec<Unknown> = spelling::unknown_words(&text)
        .into_iter()
        .filter(|range| !(typing && range.end == cursor))
        .map(|range| {
            let word = text[range.clone()].to_string();
            Unknown { suggestions: spelling::known_suggestions(&word), range, word }
        })
        .collect();
    let missing: Vec<String> = unknown.iter().filter(|u| u.suggestions.is_none()).map(|u| u.word.clone()).collect();
    let underline = HighlightStyle {
        underline: Some(UnderlineStyle { color: Some(cx.theme().danger), thickness: px(1.), wavy: true }),
        ..Default::default()
    };
    let underlines = unknown.iter().map(|u| TextDecoration::new(u.range.clone(), underline)).collect();
    spelling.underlines.set(underlines, cx);
    *spelling.unknown.borrow_mut() = unknown;
    if missing.is_empty() {
        return;
    }
    let looked_up = cx.background_executor().spawn(async move {
        for word in missing {
            spelling::suggestions(&word);
        }
    });
    let spelling = Rc::downgrade(spelling);
    cx.spawn(async move |_| {
        looked_up.await;
        if let Some(spelling) = spelling.upgrade() {
            for unknown in spelling.unknown.borrow_mut().iter_mut() {
                unknown.suggestions = unknown.suggestions.take().or_else(|| spelling::known_suggestions(&unknown.word));
            }
        }
    })
    .detach();
}
