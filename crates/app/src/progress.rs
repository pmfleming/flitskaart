//! Voortgang: the streak, today's queue, how strong the cards are and how
//! each way of practising is going. The numbers come from
//! [`dutch_flashcards_core::stats`] and are worked out each time it opens.

use dutch_flashcards_core::stats::{Progress, STRONG_FROM_DAYS, SHAKY_BELOW_DAYS, Skill, Streak, Today, WINDOW_DAYS, weekday_short};
use gpui_kit::component::{ActiveTheme as _, Colorize as _, Icon, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, Context, Div, FontWeight, Hsla, InteractiveElement as _, IntoElement, ParentElement as _, StatefulInteractiveElement as _,
    Styled as _, div, px, relative,
};

use crate::app::{FlashcardsApp, Screen, go_buttons};
use crate::icons;
use crate::ui::{card_box, day_chart, day_label, distribution_bar, dot, empty_state, explained, hint, surface, titled};

const CHART_HEIGHT: f32 = 100.;
/// The width of a skill's name, so the meters line up.
const SKILL_LABEL: f32 = 84.;

impl FlashcardsApp {
    /// Works out Voortgang's numbers again.
    pub fn reload_progress(&mut self) {
        match self.store.progress(Self::now(), self.new_limit()) {
            Ok(progress) => self.progress = Some(progress),
            Err(err) => {
                self.progress = None;
                self.error(format!("Voortgang niet geladen: {err:#}"));
            }
        }
    }

    /// In a `narrow` window the four totals go two by two.
    pub fn render_progress(&self, narrow: bool, cx: &mut Context<Self>) -> impl IntoElement {
        let screen = v_flex().id("progress").size_full().overflow_y_scroll().p_5().pb_10().gap_4();
        let Some(progress) = &self.progress else { return screen };
        if !progress.started {
            let deck = progress.strength.total() + progress.strength.paused;
            let ways = if deck == 0 { vec![Screen::Talk, Screen::Add] } else { vec![Screen::Study, Screen::Talk] };
            let detail = "Oefen een kaart of voer een gesprek. Je reeks en je voortgang verschijnen hier.";
            return screen.child(
                card_box(cx)
                    .w_full()
                    .max_w(px(520.))
                    .mx_auto()
                    .px_6()
                    .pb_6()
                    .gap_4()
                    .items_center()
                    .child(empty_state(icons::progress(), "Nog niets geoefend", detail, cx))
                    .child(h_flex().flex_wrap().justify_center().gap_2().children(go_buttons(ways, cx))),
            );
        }
        let rows = [
            h_flex()
                .flex_wrap()
                .items_stretch()
                .gap_4()
                .child(render_streak(progress.streak, &progress.week, cx))
                .child(self.render_today(progress.today, cx)),
            render_tiles(progress, if narrow { 2 } else { 4 }, cx),
            render_activity(progress, cx),
            h_flex().flex_wrap().items_stretch().gap_4().child(render_strength(progress, cx)).child(render_skills(progress, cx)),
        ];
        screen.children(rows.map(|row| row.w_full().max_w(px(760.)).mx_auto()))
    }

    /// How much of today's queue has had its turn, with a way to the rest.
    fn render_today(&self, today: Today, cx: &mut Context<Self>) -> Div {
        let theme = cx.theme().clone();
        let total = today.done + today.waiting;
        let share = if total == 0 { 1.0 } else { today.done as f32 / total as f32 };
        let status = match (total, today.waiting) {
            (0, _) => "Niets te doen vandaag".to_string(),
            (_, 0) => "Alles gedaan voor nu".to_string(),
            (_, 1) => "Nog 1 kaart wacht".to_string(),
            (_, waiting) => format!("Nog {waiting} kaarten wachten"),
        };
        card_box(cx)
            .flex_1()
            .min_w(px(220.))
            .p_4()
            .gap_3()
            .child(titled(icons::practice(), "Vandaag", cx))
            .child(
                h_flex()
                    .items_baseline()
                    .gap_2()
                    .child(big(today.done.to_string()))
                    .child(hint(format!("van {total} {}", if total == 1 { "kaart" } else { "kaarten" }), cx)),
            )
            .child(meter(share, 8., theme.primary, cx))
            .child(
                h_flex()
                    .justify_between()
                    .items_center()
                    .gap_2()
                    .flex_wrap()
                    .child(hint(status, cx))
                    .when(today.waiting > 0, |row| row.children(go_buttons(vec![Screen::Study], cx))),
            )
    }
}

/// The flame and the days in a row, over the last seven days.
fn render_streak(streak: Streak, week: &[(chrono::NaiveDate, bool)], cx: &App) -> Div {
    let theme = cx.theme();
    let Streak { days, longest, today } = streak;
    let unit = if days == 1 { "dag op rij" } else { "dagen op rij" };
    let note = match (days, today) {
        (0, _) => "Begin vandaag een nieuwe reeks.".to_string(),
        (_, false) => "Oefen vandaag om je reeks te houden.".to_string(),
        _ if longest > days => format!("Langste reeks: {longest} dagen."),
        _ => "Je langste reeks tot nu toe.".to_string(),
    };
    let last = week.len().saturating_sub(1);
    let week_row = h_flex().justify_between().gap_1().children(
        week.iter().enumerate().map(|(index, &(day, practised))| streak_day(index, index == last, day, practised, cx))
    );
    card_box(cx)
        .flex_1()
        .min_w(px(280.))
        .p_4()
        .gap_4()
        .child(
            h_flex()
                .items_center()
                .gap_3()
                .child(
                    explained("streak-flame", if today { "Vandaag geoefend" } else { "Vandaag nog niet geoefend" })
                        .text_color(if today { theme.warning } else { theme.muted_foreground })
                        .child(icons::flame().size(px(44.))),
                )
                .child(
                    v_flex()
                        .gap_0p5()
                        .child(h_flex().items_baseline().gap_2().child(big(days.to_string())).child(div().font_weight(FontWeight::MEDIUM).child(unit)))
                        .child(hint(note, cx)),
                ),
        )
        .child(week_row)
}

fn streak_day(index: usize, is_today: bool, day: chrono::NaiveDate, practised: bool, cx: &App) -> impl IntoElement {
    let theme = cx.theme();
    let circle = div().size(px(26.)).rounded_full().flex().items_center().justify_center();
    let circle = if practised {
        circle.bg(theme.primary).text_color(theme.primary_foreground).child(icons::check().size(px(14.)))
    } else {
        circle.border_1().border_color(if is_today { theme.primary } else { theme.border }).bg(theme.secondary)
    };
    let tip = format!("{}{}", day_label(day), if practised { ": geoefend" } else { "" });
    explained(("streak-day", index as u64), tip)
        .flex()
        .flex_col()
        .items_center()
        .gap_1()
        .text_xs()
        .text_color(if is_today { theme.foreground } else { theme.muted_foreground })
        .when(is_today, |d| d.font_weight(FontWeight::SEMIBOLD))
        .child(circle)
        .child(weekday_short(day))
}

/// Four totals in `columns` columns, each explained on hover.
fn render_tiles(progress: &Progress, columns: u16, cx: &App) -> Div {
    let minutes = (progress.seconds / 60.0).round() as u64;
    let window = format!("in {WINDOW_DAYS} dagen");
    let time = if minutes > 0 {
        (minutes.to_string(), format!("minuten {window}"), "Tijd met een kaart voor je en met antwoorden in Gesprek")
    } else {
        (progress.reviews.to_string(), format!("beurten {window}"), "Kaarten beoordeeld")
    };
    let tiles = [
        (progress.learned.to_string(), "kaarten geleerd".to_string(), "Kaarten in je stapel die je minstens één keer hebt geoefend"),
        (progress.strength.strong.to_string(), "zitten er sterk in".to_string(), "Kaarten die je naar verwachting over een maand nog weet"),
        (
            progress.recall.map_or("–".to_string(), percent),
            "onthouden".to_string(),
            "Hoe vaak je een kaart nog wist als die na een dag of langer terugkwam, de laatste 30 dagen",
        ),
        time,
    ];
    let theme = cx.theme();
    div().grid().grid_cols(columns).gap_4().children(tiles.into_iter().enumerate().map(|(index, (value, label, tip))| {
        explained(("progress-tile", index as u64), tip)
            .min_w_0()
            .flex()
            .flex_col()
            .gap_0p5()
            .px_4()
            .py_3()
            .rounded(px(12.))
            .border_1()
            .border_color(theme.border)
            .bg(surface(cx))
            .child(div().text_size(px(24.)).font_weight(FontWeight::SEMIBOLD).child(value))
            .child(hint(label, cx))
    }))
}

/// Reviews per day, new cards on top of the cards seen before.
fn render_activity(progress: &Progress, cx: &App) -> Div {
    let theme = cx.theme();
    let colors = [theme.primary, theme.success];
    let days: Vec<_> = progress.activity.iter().map(|day| (day.date, vec![day.again as u64, day.new as u64])).collect();
    let tip = |day, parts: &[u64]| match parts.iter().sum::<u64>() {
        0 => format!("{}: niet geoefend", day_label(day)),
        _ => format!("{}: {} herhaald, {} nieuw", day_label(day), parts[0], parts[1]),
    };
    let legend = h_flex()
        .flex_wrap()
        .gap_4()
        .children([("herhaald", colors[0]), ("nieuw geleerd", colors[1])].map(|(label, color)| legend_item(label, None, color, cx)));
    card_box(cx)
        .p_4()
        .gap_3()
        .child(header(icons::progress(), "Activiteit", format!("laatste {WINDOW_DAYS} dagen · {} beurten", progress.reviews), cx))
        .child(day_chart("activity-day", &days, &colors, CHART_HEIGHT, tip, cx))
        .child(legend)
}

/// The cards by how long they are expected to stay remembered.
fn render_strength(progress: &Progress, cx: &App) -> Div {
    let theme = cx.theme();
    let s = progress.strength;
    let shaky = format!("Blijft minder dan {SHAKY_BELOW_DAYS} dagen hangen");
    let growing = format!("Blijft {SHAKY_BELOW_DAYS} tot {STRONG_FROM_DAYS} dagen hangen");
    let strong = format!("Blijft {STRONG_FROM_DAYS} dagen of langer hangen");
    let levels = [
        ("Nieuw", "Nog nooit geoefend".to_string(), s.new, theme.muted_foreground.opacity(0.35)),
        ("Wankel", shaky, s.shaky, theme.warning),
        ("Groeit", growing, s.growing, theme.primary),
        ("Sterk", strong, s.strong, theme.success),
    ];
    let bar = distribution_bar(10., s.total() as u64, levels.iter().map(|level| (level.2 as u64, level.3)), cx);
    let legend = h_flex().flex_wrap().gap_x_4().gap_y_1().children(levels.into_iter().enumerate().map(|(index, (label, tip, count, color))| {
        explained(("strength", index as u64), tip).child(legend_item(label, Some(count), color, cx))
    }));
    let cards = if s.total() == 1 { "1 kaart".to_string() } else { format!("{} kaarten", s.total()) };
    card_box(cx)
        .flex_1()
        .min_w(px(280.))
        .p_4()
        .gap_3()
        .child(header(icons::deck(), "Hoe sterk zitten je kaarten", cards, cx))
        .child(bar)
        .child(legend)
        .when(s.paused > 0, |d| d.child(hint(format!("{} gepauzeerd, niet meegeteld", s.paused), cx).text_xs()))
}

/// Each way of practising: a score out of 100, how often, and the change.
fn render_skills(progress: &Progress, cx: &App) -> Div {
    let theme = cx.theme();
    let detail = |skill: Option<Skill>, what: &str| {
        skill.map(|s| format!("{what} · {} keer{}", s.count, change(s.change)))
    };
    let talk = progress.talking.map(|t| {
        let talks = if t.talks == 1 { "1 gesprek".to_string() } else { format!("{} gesprekken", t.talks) };
        (t.spoken, format!("gesproken · {} zinnen in {talks}", t.lines))
    });
    let rows = [
        ("Spreken", icons::speak(), progress.speaking.map(|s| s.score), detail(progress.speaking, "verstaanbaar")),
        ("Schrijven", icons::write(), progress.writing.map(|s| s.score), detail(progress.writing, "letters goed")),
        ("Luisteren", icons::listen(), progress.listening.map(|s| s.score), detail(progress.listening, "geweten")),
        ("Getest", icons::check(), progress.tested.map(|s| s.score), progress.tested.map(|s| format!("van je beurten gesproken of geschreven{}", change(s.change)))),
        ("Gesprek", icons::talk(), talk.as_ref().map(|t| t.0), talk.map(|t| t.1)),
    ];
    card_box(cx)
        .flex_1()
        .min_w(px(300.))
        .p_4()
        .gap_3()
        .child(header(icons::speak(), "Vaardigheden", format!("laatste {WINDOW_DAYS} dagen"), cx))
        .children(rows.into_iter().map(|(label, icon, score, detail): (&str, Icon, Option<f32>, Option<String>)| {
            v_flex()
                .gap_0p5()
                .child(
                    h_flex()
                        .items_center()
                        .gap_2()
                        .text_sm()
                        .child(
                            h_flex()
                                .w(px(SKILL_LABEL))
                                .flex_none()
                                .items_center()
                                .gap_1p5()
                                .child(icon.size_4().text_color(theme.muted_foreground))
                                .child(label),
                        )
                        .child(div().flex_1().child(meter(score.unwrap_or(0.0), 6., theme.primary, cx)))
                        .child(div().w(px(44.)).flex_none().text_right().font_weight(FontWeight::SEMIBOLD).child(score.map_or("–".to_string(), percent))),
                )
                .child(hint(detail.unwrap_or_else(|| format!("Niet geoefend in de laatste {WINDOW_DAYS} dagen")), cx).text_xs().pl(px(SKILL_LABEL + 8.)))
        }))
}

/// A section's icon and title, with a note on the right.
fn header(icon: Icon, title: &str, note: String, cx: &App) -> Div {
    h_flex().justify_between().items_center().flex_wrap().gap_2().child(titled(icon, title.to_string(), cx)).child(hint(note, cx).text_xs())
}

fn legend_item(label: &str, count: Option<usize>, color: Hsla, cx: &App) -> AnyElement {
    let theme = cx.theme();
    h_flex()
        .items_center()
        .gap_1p5()
        .text_xs()
        .child(dot(color))
        .child(div().text_color(theme.muted_foreground).child(label.to_string()))
        .children(count.map(|count| div().font_weight(FontWeight::MEDIUM).child(count.to_string())))
        .into_any_element()
}

/// A thin bar filled to `share`, 0.0–1.0.
fn meter(share: f32, height: f32, color: Hsla, cx: &App) -> Div {
    let fill = share.clamp(0.0, 1.0);
    div()
        .w_full()
        .h(px(height))
        .rounded_full()
        .overflow_hidden()
        .bg(cx.theme().secondary)
        .when(fill > 0.0, |bar| bar.child(div().h_full().rounded_full().w(relative(fill)).bg(color)))
}

fn big(text: String) -> Div {
    div().text_size(px(34.)).font_weight(FontWeight::BOLD).child(text)
}

/// "88%".
fn percent(share: f32) -> String {
    format!("{:.0}%", share * 100.0)
}

/// " · 3 punten beter", or nothing for a change under half a point or none.
fn change(points: Option<f32>) -> String {
    match points {
        Some(p) if p.abs() >= 0.5 => {
            let n = p.abs().round() as u32;
            format!(" · {n} {} {}", if n == 1 { "punt" } else { "punten" }, if p > 0.0 { "beter" } else { "lager" })
        }
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::{change, percent};

    #[test]
    fn changes_read_as_points() {
        assert_eq!([change(Some(3.2)), change(Some(-1.0)), change(Some(0.3)), change(None)], [" · 3 punten beter", " · 1 punt lager", "", ""]);
        assert_eq!(percent(0.876), "88%");
    }
}
