//! Instellingen: which logged-in tool and model answers and how much it
//! thinks, and how many tokens the app has used, per model and per day.

use dutch_flashcards_core::llm::{Cli, Effort, ModelChoice, model_name};
use dutch_flashcards_core::usage::{UsageSummary, UsageTotal};
use gpui_kit::component::{ActiveTheme as _, Colorize as _, Icon, h_flex, v_flex};
use gpui_kit::TestSupportExt as _;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, Context, Div, FontWeight, Hsla, IntoElement, InteractiveElement as _, ParentElement as _, SharedString,
    StatefulInteractiveElement as _, Styled as _, div, hsla, px, relative,
};

use crate::app::FlashcardsApp;
use crate::icons;
use crate::ui::{day_chart, day_label, dot, explained, hint, icon_button, section, segment_button, segmented_bar};

/// How many days the chart shows.
const CHART_DAYS: usize = 30;
const CHART_HEIGHT: f32 = 96.;
/// How many days the per-day breakdown lists.
const TABLE_DAYS: usize = 14;

impl FlashcardsApp {
    /// Asks the installed tools which models they offer, in the background.
    pub fn find_models(&mut self, cx: &mut Context<Self>) {
        if self.finding_models {
            return;
        }
        self.finding_models = true;
        cx.notify();
        self.in_background(cx, dutch_flashcards_core::llm::available, |this, choices, _| {
            this.finding_models = false;
            this.llm_choices = Some(choices);
        });
    }

    /// Looks for the tools the first time Instellingen opens.
    pub fn find_models_once(&mut self, cx: &mut Context<Self>) {
        if self.llm_choices.is_some() {
            return;
        }
        self.find_models(cx);
    }

    /// The saved model among the ones on offer, if it is still there.
    fn chosen_model(&self) -> Option<&ModelChoice> {
        self.llm_choices.as_ref()?.iter().find(|choice| choice.is(self.settings.cli, &self.settings.model))
    }

    /// Answers from now on come from `choice`, thinking as near to `effort` as it offers.
    fn choose_model(&mut self, choice: ModelChoice, effort: Effort, cx: &mut Context<Self>) {
        let effort = choice.effort_for(effort);
        (self.settings.cli, self.settings.model, self.settings.effort) = (choice.cli, choice.run_id(effort), effort);
        self.llm.choose(choice.cli, &self.settings.model, effort);
        self.llm_tab = Some(choice.cli);
        self.save_settings();
        cx.notify();
    }

    /// The tool, model and effort for conversations and making cards. Like a
    /// model picker: a tab per logged-in tool, its models, then the effort.
    pub fn render_llm(&self, cx: &mut Context<Self>) -> Div {
        let card = section(icons::model(), "Taalmodel", cx);
        let Some(choices) = self.llm_choices.as_ref() else {
            return card.child(hint("Ingelogde programma's zoeken…", cx));
        };
        if choices.is_empty() {
            return card.child(hint("Geen ingelogd programma gevonden. De app werkt met claude, codex, agy (Antigravity) of pi.", cx));
        }
        let tools: Vec<Cli> = Cli::ALL.into_iter().filter(|cli| choices.iter().any(|choice| choice.cli == *cli)).collect();
        let tab = self.llm_tab.filter(|cli| tools.contains(cli)).or(Some(self.settings.cli).filter(|cli| tools.contains(cli))).unwrap_or(tools[0]);
        let tabs = segmented_bar(cx).children(tools.iter().map(|cli| {
            let cli = *cli;
            segment_button(SharedString::from(format!("llm-tab-{}", cli.id())), cli.name(), tab == cli, cx)
                .when(cli == self.settings.cli, |button| button.icon(icons::check()))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.llm_tab = Some(cli);
                    cx.notify();
                }))
        }));
        let models: Vec<_> = choices.iter().filter(|choice| choice.cli == tab).map(|choice| self.model_row(choice, cx)).collect();
        let chosen = self.chosen_model();
        let efforts = match chosen {
            Some(choice) if !choice.efforts.is_empty() => {
                let choice = choice.clone();
                segmented_bar(cx)
                    .children(choice.efforts.clone().into_iter().map(|effort| {
                        let choice = choice.clone();
                        segment_button(SharedString::from(format!("effort-{}", effort.as_str())), effort.label(), self.settings.effort == effort, cx)
                            .when(effort == choice.default_effort, |button| button.tooltip("Standaard"))
                            .on_click(cx.listener(move |this, _, _, cx| this.choose_model(choice.clone(), effort, cx)))
                    }))
                    .into_any_element()
            }
            Some(_) => hint("Dit model heeft een vast denkwerk.", cx).into_any_element(),
            None => hint("Kies eerst een model.", cx).into_any_element(),
        };
        let missing = chosen.is_none().then(|| {
            hint(format!("Het gekozen model ({} via {}) is niet meer beschikbaar.", model_name(&self.settings.model), self.settings.cli.name()), cx)
                .text_color(cx.theme().warning)
        });
        let refresh = icon_button("llm-refresh", icons::retry(), "Opnieuw zoeken")
            .loading(self.finding_models)
            .on_click(cx.listener(|this, _, _, cx| this.find_models(cx)));
        let tools = h_flex().items_center().gap_1().child(tabs).child(refresh);
        card.child(setting_row("llm-tool", icons::model(), "Programma", "Een programma waarmee je op deze laptop bent ingelogd", tools, cx))
            .child(div().id("llm-models").grid().grid_cols(2).gap_1p5().max_h(px(260.)).overflow_y_scroll().children(models))
            .children(missing)
            .child(setting_row("llm-effort", icons::effort(), "Denkwerk", "Meer denkwerk geeft betere antwoorden, maar duurt langer", efforts, cx))
    }

    /// One model to choose, ticked when it is the one in use.
    fn model_row(&self, choice: &ModelChoice, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let theme = cx.theme().clone();
        let chosen = choice.is(self.settings.cli, &self.settings.model);
        let pick = choice.clone();
        h_flex()
            .id(SharedString::from(format!("model-{}-{}", choice.cli.id(), choice.id)))
            .test_support()
            .gap_2()
            .px_3()
            .py_2()
            .items_center()
            .rounded(px(8.))
            .border_1()
            .border_color(if chosen { theme.primary.opacity(0.5) } else { theme.border })
            .when(chosen, |row| row.bg(theme.accent).text_color(theme.accent_foreground))
            .when(!chosen, |row| row.cursor_pointer().hover(move |style| style.bg(theme.muted)))
            .on_click(cx.listener(move |this, _, _, cx| this.choose_model(pick.clone(), this.settings.effort, cx)))
            .child(div().flex_1().min_w_0().truncate().when(chosen, |name| name.font_weight(FontWeight::SEMIBOLD)).child(choice.name.clone()))
            .when(chosen, |row| row.child(icons::check().size_4().flex_none()))
            .into_any_element()
    }

    /// How many tokens the language models used for this app: in all, per
    /// day, per kind and per model or day.
    pub fn render_usage(&self, cx: &mut Context<Self>) -> Div {
        let summary = self.store.usage_summary(Self::now(), CHART_DAYS).unwrap_or_default();
        let card = section(icons::tokens(), "Verbruik", cx);
        if summary.total.calls == 0 {
            return card.child(hint("Nog niets gebruikt. Gesprekken en kaarten maken tellen hier mee.", cx));
        }
        let total = &summary.total;
        let headline = h_flex()
            .items_baseline()
            .gap_2()
            .child(div().text_size(px(30.)).font_weight(FontWeight::SEMIBOLD).child(compact(total.tokens())))
            .child(hint(format!("tokens · {} antwoorden", total.calls), cx));
        card.child(headline)
            .child(self.render_chart(&summary, cx))
            .child(render_kinds(total, cx))
            .child(self.render_breakdown(&summary, cx))
    }

    /// Tokens per day for the last days, each bar split by model.
    fn render_chart(&self, summary: &UsageSummary, cx: &App) -> Div {
        let colors: Vec<Hsla> = (0..summary.by_model.len()).map(|index| color_at(index, cx)).collect();
        let legend = h_flex().flex_wrap().gap_3().children(summary.by_model.iter().zip(&colors).map(|((model, _), color)| {
            h_flex().items_center().gap_1p5().child(dot(*color)).child(hint(model_name(model), cx).text_xs())
        }));
        let tip = |day, tokens: &[u64]| format!("{}: {} tokens", day_label(day), compact(tokens.iter().sum()));
        day_chart("chart-day", &summary.daily, &colors, CHART_HEIGHT, tip, cx).child(legend)
    }

    /// Per model or per day: answers, share of the tokens and tokens.
    fn render_breakdown(&self, summary: &UsageSummary, cx: &mut Context<Self>) -> Div {
        let theme = cx.theme().clone();
        let by_day = self.usage_by_day;
        let toggle = segmented_bar(cx).children([(false, "Model"), (true, "Dag")].map(|(day, label)| {
            segment_button(if day { "usage-per-day" } else { "usage-per-model" }, label, by_day == day, cx).on_click(cx.listener(move |this, _, _, cx| {
                this.usage_by_day = day;
                cx.notify();
            }))
        }));
        let rows: Vec<(String, Hsla, &UsageTotal)> = if by_day {
            summary.by_day.iter().take(TABLE_DAYS).map(|(day, total)| (day_label(*day), theme.primary, total)).collect()
        } else {
            summary.by_model.iter().enumerate().map(|(index, (model, total))| (model_name(model), color_at(index, cx), total)).collect()
        };
        let all = summary.total.tokens().max(1) as f32;
        let number = |text: String, width: f32| div().w(px(width)).flex_none().text_right().child(text);
        let header = h_flex()
            .gap_3()
            .pb_1p5()
            .border_b_1()
            .border_color(theme.border)
            .text_xs()
            .font_weight(FontWeight::MEDIUM)
            .text_color(theme.muted_foreground)
            .child(div().flex_1().child(if by_day { "Dag" } else { "Model" }))
            .child(number("Antwoorden".into(), 84.))
            .child(number("Aandeel".into(), 60.))
            .child(number("Tokens".into(), 64.));
        let body = rows.into_iter().map(|(name, color, total)| {
            let share = total.tokens() as f32 / all;
            h_flex()
                .gap_3()
                .py_2()
                .items_center()
                .border_b_1()
                .border_color(theme.border.opacity(0.6))
                .text_sm()
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .gap_1()
                        .child(div().font_weight(FontWeight::MEDIUM).child(name))
                        .child(div().h(px(2.)).w(relative(share.max(0.01))).rounded_full().bg(color)),
                )
                .child(number(total.calls.to_string(), 84.).text_color(theme.muted_foreground))
                .child(number(format!("{:.1}%", share * 100.0).replace('.', ","), 60.).text_color(theme.muted_foreground))
                .child(number(compact(total.tokens()), 64.))
        });
        v_flex()
            .gap_2()
            .pt_2()
            .child(h_flex().justify_between().items_center().child(div().font_weight(FontWeight::MEDIUM).child("Overzicht")).child(toggle))
            .child(header)
            .children(body)
    }
}

/// A labelled setting: icon and name on the left (explained on hover), the
/// choice on the right.
fn setting_row(id: &'static str, icon: Icon, label: &'static str, tip: &'static str, choice: impl IntoElement, cx: &App) -> Div {
    h_flex()
        .justify_between()
        .items_center()
        .flex_wrap()
        .gap_2()
        .child(
            explained(id, tip)
                .flex()
                .items_center()
                .gap_2()
                .child(icon.size_4().text_color(cx.theme().muted_foreground))
                .child(div().font_weight(FontWeight::MEDIUM).child(label)),
        )
        .child(choice)
}

/// One bar split into new input, cache read, cache written and output, with a legend.
fn render_kinds(total: &UsageTotal, cx: &App) -> Div {
    let theme = cx.theme();
    let kinds = [
        ("Invoer", "Nieuwe tekst naar het model", total.input, theme.primary),
        ("Cache gelezen", "Tekst die het model al had, goedkoop hergebruikt", total.cache_read, theme.primary.opacity(0.35)),
        ("Cache geschreven", "Tekst bewaard om later te hergebruiken", total.cache_write, theme.warning),
        ("Uitvoer", "Wat het model schreef", total.output, theme.success),
    ];
    let all = total.tokens().max(1) as f32;
    let bar = h_flex().h(px(8.)).w_full().rounded_full().overflow_hidden().bg(theme.secondary).children(
        kinds.iter().filter(|(_, _, tokens, _)| *tokens > 0).map(|(_, _, tokens, color)| div().h_full().w(relative(*tokens as f32 / all)).bg(*color)),
    );
    let legend = h_flex().flex_wrap().gap_x_4().gap_y_1().children(kinds.iter().enumerate().map(|(index, (label, tip, tokens, color))| {
        explained(("usage-kind", index as u64), *tip)
            .flex()
            .items_center()
            .gap_1p5()
            .text_xs()
            .child(dot(*color))
            .child(div().text_color(theme.muted_foreground).child(*label))
            .child(div().font_weight(FontWeight::MEDIUM).child(compact(*tokens)))
    }));
    v_flex().gap_2().child(bar).child(legend)
}

/// The colour of the `index`th model in the chart, the bars and the legend.
fn color_at(index: usize, cx: &App) -> Hsla {
    let theme = cx.theme();
    let colors = [theme.primary, theme.success, theme.warning, hsla(0.76, 0.45, 0.62, 1.0), theme.danger, hsla(0.5, 0.5, 0.45, 1.0)];
    colors.get(index).copied().unwrap_or(theme.muted_foreground)
}

/// 950, "12,3k", "4,56 mln", "1,20 mld": three figures at most, Dutch style.
pub fn compact(count: u64) -> String {
    let (value, unit) = match count {
        n if n >= 1_000_000_000 => (n as f64 / 1e9, " mld"),
        n if n >= 1_000_000 => (n as f64 / 1e6, " mln"),
        n if n >= 1_000 => (n as f64 / 1e3, "k"),
        n => return n.to_string(),
    };
    let decimals = if value < 10.0 { 2 } else if value < 100.0 { 1 } else { 0 };
    format!("{value:.decimals$}{unit}").replace('.', ",")
}

#[cfg(test)]
mod tests {
    use super::compact;

    #[test]
    fn numbers_read_the_dutch_way() {
        assert_eq!([compact(950), compact(12_345), compact(4_560_000), compact(6_390_000_000)], ["950", "12,3k", "4,56 mln", "6,39 mld"]);
    }
}
