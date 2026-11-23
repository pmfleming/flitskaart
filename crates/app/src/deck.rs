//! Kaarten: every card in the deck, with search and editing.

use dutch_flashcards_core::schedule::describe_due;
use dutch_flashcards_core::{Card, CardId, Status};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{ActiveTheme as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, AppContext as _, Context, Div, Entity, InteractiveElement as _, IntoElement, ParentElement as _, StatefulInteractiveElement as _,
    Styled as _, Subscription, Window, div, px,
};

use crate::app::FlashcardsApp;
use crate::drafts::DraftRow;
use crate::ui::{empty_state, hint};

pub struct DeckState {
    pub search: Entity<InputState>,
    pub query: String,
    pub cards: Vec<Card>,
    /// The card being edited, reusing the draft row's fields.
    pub editing: Option<DraftRow>,
    pub confirm_delete: Option<CardId>,
    _search_changed: Subscription,
}

impl DeckState {
    pub fn new(window: &mut Window, cx: &mut Context<FlashcardsApp>) -> Self {
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Zoek een woord, zin of onderwerp"));
        let search_changed = cx.subscribe_in(&search, window, |this: &mut FlashcardsApp, state, event: &InputEvent, _, cx| {
            if matches!(event, InputEvent::Change) {
                this.deck.query = state.read(cx).value().to_string();
                this.reload_deck();
                cx.notify();
            }
        });
        DeckState { search, query: String::new(), cards: Vec::new(), editing: None, confirm_delete: None, _search_changed: search_changed }
    }
}

#[derive(Clone, Copy)]
enum DeckAction {
    Suspend,
    Resume,
    Reset,
    Delete,
}

/// The column widths after the Dutch, with their headings.
const COLUMNS: [(f32, &str); 5] = [(110., "Volgende keer"), (56., "Fout"), (70., "Vertaling"), (76., "Uitspraak"), (250., "")];

/// A cell of column `i`: the "next time" column reads left to right, numbers line up on the right.
fn cell(i: usize, width: f32) -> Div {
    div().w(px(width)).when(i > 0, |d| d.text_right())
}

impl FlashcardsApp {
    pub fn reload_deck(&mut self) {
        match self.store.deck(&self.deck.query) {
            Ok(cards) => self.deck.cards = cards,
            Err(err) => self.error(format!("{err:#}")),
        }
    }

    pub fn start_editing(&mut self, id: CardId, window: &mut Window, cx: &mut Context<Self>) {
        let Ok(Some(card)) = self.store.card(id) else { return };
        self.deck.editing = Some(DraftRow::new(&card, false, window, cx));
        self.deck.confirm_delete = None;
        cx.notify();
    }

    fn save_edit(&mut self, cx: &mut Context<Self>) {
        let Some(row) = self.deck.editing.take() else { return };
        let [dutch, english, note, tags] = row.values(cx);
        if let Err(err) = self.store.update_text(row.id, &dutch, &english, &note, &tags) {
            self.deck.editing = Some(row);
            return self.error(format!("{err:#}"));
        }
        if self.study.card.as_ref().is_some_and(|card| card.id == row.id) {
            self.study.card = self.store.card(row.id).ok().flatten();
        }
        self.prepare_audio(dutch, cx);
        self.reload_deck();
        self.info("Kaart bewaard.");
    }

    fn deck_action(&mut self, id: CardId, action: DeckAction, cx: &mut Context<Self>) {
        let result = match action {
            DeckAction::Suspend => self.store.set_suspended(id, true),
            DeckAction::Resume => self.store.set_suspended(id, false),
            DeckAction::Reset => self.store.reset(id, Self::now()),
            DeckAction::Delete => self.store.delete(id),
        };
        if let Err(err) = result {
            self.error(format!("{err:#}"));
        }
        self.deck.confirm_delete = None;
        if self.study.card.as_ref().is_some_and(|card| card.id == id) {
            self.study.card = None;
        }
        self.after_deck_change(cx);
    }

    pub fn render_deck(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let body = if self.deck.cards.is_empty() {
            let (title, detail) = if self.deck.query.trim().is_empty() {
                ("Nog geen kaarten", "Kaarten die je bij Concepten bewaart, verschijnen hier.")
            } else {
                ("Niets gevonden", "Probeer een ander woord.")
            };
            empty_state(title, detail, cx).into_any_element()
        } else {
            let rows = self.deck.cards.iter().map(|card| match &self.deck.editing {
                Some(row) if row.id == card.id => self.render_edit_row(row, cx).into_any_element(),
                _ => self.render_deck_row(card, cx).into_any_element(),
            });
            v_flex().id("deck-list").flex_1().min_h_0().overflow_y_scroll().children(rows.collect::<Vec<_>>()).into_any_element()
        };
        let headings = COLUMNS.iter().enumerate().map(|(i, (width, heading))| cell(i, *width).child(*heading));
        v_flex()
            .size_full()
            .p_5()
            .gap_3()
            .child(
                h_flex()
                    .gap_3()
                    .child(div().flex_1().child(Input::new(&self.deck.search)))
                    .child(hint(format!("{} kaarten", self.deck.cards.len()), cx)),
            )
            .child(
                h_flex()
                    .gap_3()
                    .px_2()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(div().flex_1().child("Nederlands"))
                    .children(headings),
            )
            .child(body)
    }

    fn render_deck_row(&self, card: &Card, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let suspended = card.status == Status::Suspended;
        let due = if suspended {
            "gepauzeerd".to_string()
        } else if card.is_new() {
            "nieuw".to_string()
        } else {
            card.due.map(|due| describe_due(due, Self::now())).unwrap_or_default()
        };
        let accuracy = card.speech_accuracy.map_or("–".to_string(), |a| format!("{:.0}%", a * 100.0));
        let cells = [due, card.misses.to_string(), card.peeks.to_string(), accuracy];
        h_flex()
            .gap_3()
            .px_2()
            .py_2()
            .border_b_1()
            .border_color(theme.border)
            .when(suspended, |d| d.text_color(theme.muted_foreground))
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .child(div().child(card.dutch.clone()))
                    .when(!card.tags.is_empty(), |d| d.child(div().text_xs().text_color(theme.muted_foreground).child(card.tags.clone()))),
            )
            .children(cells.into_iter().zip(COLUMNS).enumerate().map(|(i, (text, (width, _)))| cell(i, width).text_sm().child(text)))
            .child(h_flex().w(px(COLUMNS[4].0)).justify_end().child(self.render_row_actions(card, cx)))
    }

    /// Edit, pause, reset and delete; deleting asks first.
    fn render_row_actions(&self, card: &Card, cx: &mut Context<Self>) -> impl IntoElement {
        let id = card.id;
        let action = |name: &'static str, label: &'static str| Button::new((name, id as u64)).ghost().xsmall().label(label);
        if self.deck.confirm_delete == Some(id) {
            return h_flex()
                .gap_1()
                .child(div().text_sm().text_color(cx.theme().danger).child("Verwijderen?"))
                .child(Button::new(("delete-yes", id as u64)).danger().xsmall().label("Ja").on_click(Self::run_action(id, DeckAction::Delete, cx)))
                .child(action("delete-no", "Nee").on_click(cx.listener(|this, _, _, cx| this.confirm_delete(None, cx))));
        }
        let suspended = card.status == Status::Suspended;
        let (pause_label, pause) = if suspended { ("Hervat", DeckAction::Resume) } else { ("Pauzeer", DeckAction::Suspend) };
        h_flex()
            .gap_1()
            .child(action("edit", "Bewerken").on_click(cx.listener(move |this, _, window, cx| this.start_editing(id, window, cx))))
            .child(action("pause", pause_label).on_click(Self::run_action(id, pause, cx)))
            .when(!card.is_new(), |d| d.child(action("reset", "Opnieuw").on_click(Self::run_action(id, DeckAction::Reset, cx))))
            .child(action("delete", "Verwijder").on_click(cx.listener(move |this, _, _, cx| this.confirm_delete(Some(id), cx))))
    }

    fn run_action<E>(id: CardId, action: DeckAction, cx: &mut Context<Self>) -> impl Fn(&E, &mut Window, &mut App) + 'static {
        cx.listener(move |this, _: &E, _, cx| this.deck_action(id, action, cx))
    }

    fn confirm_delete(&mut self, id: Option<CardId>, cx: &mut Context<Self>) {
        self.deck.confirm_delete = id;
        cx.notify();
    }

    fn render_edit_row(&self, row: &DraftRow, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        v_flex()
            .my_1()
            .p_3()
            .gap_2()
            .rounded(px(8.))
            .border_1()
            .border_color(theme.primary)
            .bg(theme.secondary)
            .child(row.fields(true))
            .child(Input::new(&row.tags).small())
            .child(
                h_flex()
                    .gap_2()
                    .child(Button::new("edit-save").primary().small().label("Opslaan").on_click(cx.listener(|this, _, _, cx| this.save_edit(cx))))
                    .child(Button::new("edit-cancel").outline().small().label("Annuleren").on_click(cx.listener(|this, _, _, cx| {
                        this.deck.editing = None;
                        cx.notify();
                    })))
                    .child(row.listen_button("edit-listen", cx)),
            )
    }
}
