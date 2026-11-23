//! Kaarten: every card in the deck, with search, filters and inline editing.

use dutch_flashcards_core::schedule::describe_due;
use dutch_flashcards_core::{Card, CardId, Status};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{ActiveTheme as _, Colorize as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, AppContext as _, Context, Div, Entity, FontWeight, InteractiveElement as _, IntoElement, KeyDownEvent,
    ParentElement as _, SharedString, StatefulInteractiveElement as _, Styled as _, Subscription, Window, div, px,
};

use crate::app::FlashcardsApp;
use crate::drafts::DraftRow;
use crate::icons;
use crate::ui::{card_box, empty_state, field_label, hint, pill, segment_button, segmented_bar};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DeckFilter {
    #[default]
    All,
    Due,
    New,
    Suspended,
}

impl DeckFilter {
    const ALL: [DeckFilter; 4] = [DeckFilter::All, DeckFilter::Due, DeckFilter::New, DeckFilter::Suspended];

    fn label(self) -> &'static str {
        match self {
            DeckFilter::All => "Alles",
            DeckFilter::Due => "Te herhalen",
            DeckFilter::New => "Nieuw",
            DeckFilter::Suspended => "Gepauzeerd",
        }
    }

    fn id(self) -> &'static str {
        match self {
            DeckFilter::All => "filter-all",
            DeckFilter::Due => "filter-due",
            DeckFilter::New => "filter-new",
            DeckFilter::Suspended => "filter-suspended",
        }
    }

    fn matches(self, card: &Card, now: chrono::DateTime<chrono::Utc>) -> bool {
        match self {
            DeckFilter::All => true,
            DeckFilter::Due => card.status == Status::Active && !card.is_new() && card.due.is_some_and(|due| due <= now),
            DeckFilter::New => card.status == Status::Active && card.is_new(),
            DeckFilter::Suspended => card.status == Status::Suspended,
        }
    }
}

pub struct DeckState {
    pub search: Entity<InputState>,
    pub query: String,
    pub filter: DeckFilter,
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
        DeckState {
            search,
            query: String::new(),
            filter: DeckFilter::All,
            cards: Vec::new(),
            editing: None,
            confirm_delete: None,
            _search_changed: search_changed,
        }
    }
}

#[derive(Clone, Copy)]
enum DeckAction {
    Suspend,
    Resume,
    Reset,
    Delete,
}

/// Column widths after Nederlands: Volgende keer, Fout, Vertaling, Uitspraak, Actions.
const COL_DUE: f32 = 104.;
const COL_MISSES: f32 = 46.;
const COL_PEEKS: f32 = 64.;
const COL_SPEECH: f32 = 68.;
const COL_ACTIONS: f32 = 248.;
const COL_ACTIONS_COMPACT: f32 = 224.;

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
        self.info("Kaart bewaard.", cx);
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

    fn on_edit_key(&mut self, event: &KeyDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
        if event.keystroke.key == "escape" && self.deck.editing.is_some() {
            self.deck.editing = None;
            cx.notify();
            cx.stop_propagation();
        }
    }

    pub fn render_deck(&mut self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let now = Self::now();
        let compact = window.viewport_size().width < px(780.);
        let visible: Vec<&Card> = self.deck.cards.iter().filter(|card| self.deck.filter.matches(card, now)).collect();
        let theme = cx.theme().clone();

        let count_text = if visible.len() == self.deck.cards.len() {
            format!("{} kaarten", self.deck.cards.len())
        } else {
            format!("{} van {} kaarten", visible.len(), self.deck.cards.len())
        };

        let filters = segmented_bar(cx).children(DeckFilter::ALL.map(|filter| {
            segment_button(filter.id(), filter.label(), self.deck.filter == filter, cx).on_click(cx.listener(move |this, _, _, cx| {
                this.deck.filter = filter;
                cx.notify();
            }))
        }));

        let controls = h_flex()
            .flex_wrap()
            .items_center()
            .gap_3()
            .child(div().flex_1().min_w(px(220.)).child(Input::new(&self.deck.search)))
            .child(filters)
            .child(hint(count_text, cx));

        let header = h_flex()
            .gap_3()
            .px_3()
            .py_2()
            .bg(theme.muted)
            .border_b_1()
            .border_color(theme.border)
            .text_xs()
            .font_weight(FontWeight::MEDIUM)
            .text_color(theme.muted_foreground)
            .child(div().flex_1().min_w_0().child("Nederlands"))
            .child(div().w(px(COL_DUE)).child("Volgende keer"))
            .when(!compact, |d| {
                d.child(div().w(px(COL_MISSES)).text_right().child("Fout"))
                    .child(div().w(px(COL_PEEKS)).text_right().child("Vertaling"))
                    .child(div().w(px(COL_SPEECH)).text_right().child("Uitspraak"))
            })
            .child(div().w(px(if compact { COL_ACTIONS_COMPACT } else { COL_ACTIONS })));

        let body = if visible.is_empty() {
            let (title, detail) = if self.deck.cards.is_empty() && self.deck.query.trim().is_empty() {
                ("Nog geen kaarten", "Kaarten die je bij Concepten bewaart, verschijnen hier.")
            } else if self.deck.filter != DeckFilter::All && self.deck.query.trim().is_empty() {
                ("Geen kaarten in dit filter", "Kies een ander filter om je kaarten te zien.")
            } else {
                ("Niets gevonden", "Probeer een ander woord of filter.")
            };
            v_flex().flex_1().items_center().justify_center().child(empty_state(title, detail, cx)).into_any_element()
        } else {
            let rows: Vec<_> = visible
                .into_iter()
                .map(|card| match &self.deck.editing {
                    Some(row) if row.id == card.id => self.render_edit_row(row, cx).into_any_element(),
                    _ => self.render_deck_row(card, now, compact, cx).into_any_element(),
                })
                .collect();
            v_flex().id("deck-list").flex_1().min_h_0().overflow_y_scroll().children(rows).into_any_element()
        };

        v_flex()
            .size_full()
            .p_5()
            .gap_3()
            .child(controls)
            .child(card_box(cx).flex_1().min_h_0().overflow_hidden().child(header).child(body))
    }

    fn render_deck_row(&self, card: &Card, now: chrono::DateTime<chrono::Utc>, compact: bool, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let suspended = card.status == Status::Suspended;
        let due_el = self.render_due_cell(card, now, cx);

        let dim = theme.muted_foreground.opacity(0.55);
        let misses_color = if card.misses >= 3 {
            theme.danger
        } else if card.misses > 0 {
            theme.warning
        } else {
            dim
        };
        let peeks_color = if card.peeks > 0 { theme.foreground } else { dim };
        let (acc_text, acc_color) = match card.speech_accuracy {
            None => ("–".to_string(), dim),
            Some(a) if a < 0.75 => (format!("{:.0}%", a * 100.0), theme.warning),
            Some(a) if a >= 0.90 => (format!("{:.0}%", a * 100.0), theme.success),
            Some(a) => (format!("{:.0}%", a * 100.0), theme.foreground),
        };

        let hover_bg = theme.muted;
        h_flex()
            .id(SharedString::from(format!("deck-row-{}", card.id)))
            .gap_3()
            .px_3()
            .py_2p5()
            .items_center()
            .border_b_1()
            .border_color(theme.border)
            .hover(move |s| s.bg(hover_bg))
            .when(suspended, |d| d.text_color(theme.muted_foreground))
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_0p5()
                    .child(div().font_weight(FontWeight::MEDIUM).child(card.dutch.clone()))
                    .when(!card.tags.is_empty(), |d| d.child(div().text_xs().text_color(theme.muted_foreground).child(card.tags.clone()))),
            )
            .child(div().w(px(COL_DUE)).child(due_el))
            .when(!compact, |d| {
                d.child(div().w(px(COL_MISSES)).text_right().text_sm().text_color(misses_color).child(card.misses.to_string()))
                    .child(div().w(px(COL_PEEKS)).text_right().text_sm().text_color(peeks_color).child(card.peeks.to_string()))
                    .child(div().w(px(COL_SPEECH)).text_right().text_sm().text_color(acc_color).child(acc_text))
            })
            .child(
                h_flex()
                    .w(px(if compact { COL_ACTIONS_COMPACT } else { COL_ACTIONS }))
                    .justify_end()
                    .child(self.render_row_actions(card, cx)),
            )
    }

    fn render_due_cell(&self, card: &Card, now: chrono::DateTime<chrono::Utc>, cx: &App) -> Div {
        let theme = cx.theme();
        if card.status == Status::Suspended {
            return h_flex().child(pill("gepauzeerd", theme.secondary, theme.muted_foreground));
        }
        if card.is_new() {
            return h_flex().child(pill("nieuw", theme.accent, theme.accent_foreground));
        }
        let text = card.due.map(|due| describe_due(due, now)).unwrap_or_default();
        if text == "nu" {
            h_flex().child(pill("nu", theme.primary.opacity(0.16), theme.primary))
        } else {
            div().text_sm().text_color(theme.muted_foreground).child(text)
        }
    }

    /// Quick listen, edit, pause, reset and delete; deleting asks first.
    fn render_row_actions(&self, card: &Card, cx: &mut Context<Self>) -> impl IntoElement {
        let id = card.id;
        let dutch = card.dutch.clone();
        let action = |name: &'static str, label: &'static str| Button::new((name, id as u64)).ghost().xsmall().label(label);
        if self.deck.confirm_delete == Some(id) {
            return h_flex()
                .gap_1()
                .items_center()
                .child(div().text_xs().font_weight(FontWeight::MEDIUM).text_color(cx.theme().danger).child("Verwijderen?"))
                .child(Button::new(("delete-yes", id as u64)).danger().xsmall().label("Ja").on_click(Self::run_action(id, DeckAction::Delete, cx)))
                .child(action("delete-no", "Nee").on_click(cx.listener(|this, _, _, cx| this.confirm_delete(None, cx))));
        }
        let suspended = card.status == Status::Suspended;
        let (pause_label, pause) = if suspended { ("Hervat", DeckAction::Resume) } else { ("Pauzeer", DeckAction::Suspend) };
        h_flex()
            .gap_0p5()
            .items_center()
            .child(
                Button::new(("deck-listen", id as u64))
                    .ghost()
                    .xsmall()
                    .icon(icons::listen())
                    .on_click(cx.listener(move |this, _, _, cx| this.say(dutch.clone(), cx))),
            )
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
            .m_2()
            .p_4()
            .gap_3()
            .rounded(px(10.))
            .border_1()
            .border_color(theme.primary)
            .bg(theme.muted)
            .on_key_down(cx.listener(Self::on_edit_key))
            .child(row.fields(true, cx))
            .child(v_flex().gap_1().w(px(280.)).child(field_label("Onderwerp", cx)).child(Input::new(&row.tags).small()))
            .child(
                h_flex()
                    .justify_between()
                    .items_center()
                    .gap_2()
                    .child(row.listen_button("edit-listen", cx))
                    .child(
                        h_flex()
                            .gap_2()
                            .child(Button::new("edit-cancel").outline().small().label("Annuleren").on_click(cx.listener(|this, _, _, cx| {
                                this.deck.editing = None;
                                cx.notify();
                            })))
                            .child(Button::new("edit-save").primary().small().label("Opslaan").on_click(cx.listener(|this, _, _, cx| this.save_edit(cx)))),
                    ),
            )
    }
}
