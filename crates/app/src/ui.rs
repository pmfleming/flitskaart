//! Shared visual primitives and the app's Delft-blue light/dark palette.

use chrono::{Datelike as _, NaiveDate};
use gpui_kit::component::button::{Button, ButtonCustomVariant, ButtonVariants as _};
use gpui_kit::component::theme::Theme;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::component::{ActiveTheme as _, Colorize as _, Icon, Sizable as _, h_flex, v_flex};
use gpui_kit::{
    App, Div, ElementId, FontWeight, Hsla, InteractiveElement as _, ParentElement as _, SharedString, Stateful,
    StatefulInteractiveElement as _, Styled as _, div, hsla, px, relative,
};

fn hex(color: u32) -> Hsla {
    let r = ((color >> 16) & 0xff) as f32 / 255.0;
    let g = ((color >> 8) & 0xff) as f32 / 255.0;
    let b = (color & 0xff) as f32 / 255.0;
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let l = (max + min) * 0.5;
    let d = max - min;
    if d == 0.0 {
        return hsla(0.0, 0.0, l, 1.0);
    }
    let s = d / (1.0 - (2.0 * l - 1.0).abs());
    let h = if max == r {
        ((g - b) / d).rem_euclid(6.0)
    } else if max == g {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    } / 6.0;
    hsla(h, s, l, 1.0)
}

/// Applies the app's calm Delft-blue palette on top of the active light or dark mode.
pub fn apply_palette(cx: &mut App) {
    Theme::update(cx, |theme| {
        let dark = theme.is_dark();
        let c = &mut theme.colors;
        let shade = |light, night| hex(if dark { night } else { light });
        c.background = shade(0xf4f6fb, 0x0f1420);
        c.tab_bar = shade(0xffffff, 0x151b2b);
        c.group_box = shade(0xffffff, 0x171f30);
        c.popover = c.group_box;
        c.secondary = shade(0xe9eef7, 0x1f283d);
        c.secondary_hover = shade(0xdde5f2, 0x28344f);
        c.secondary_active = shade(0xd0dbed, 0x313f5e);
        c.secondary_foreground = shade(0x1e2a42, 0xd7e0f2);
        c.muted = shade(0xeef2f8, 0x1b2336);
        c.muted_foreground = shade(0x55617a, 0x97a5c2);
        c.foreground = shade(0x121a2c, 0xe9edf7);
        c.border = shade(0xd4dce9, 0x2a354d);
        c.input = shade(0xc8d3e3, 0x32405c);
        c.primary = shade(0x1d3f9e, 0x7ba0f4);
        c.primary_hover = shade(0x183587, 0x8fb0f7);
        c.primary_active = shade(0x142c72, 0x6790f0);
        c.primary_foreground = shade(0xffffff, 0x0d1320);
        c.button = shade(0xffffff, 0x1f283d);
        c.button_hover = shade(0xf1f4fa, 0x28344f);
        c.button_active = shade(0xe5ebf5, 0x313f5e);
        c.button_foreground = c.foreground;
        c.button_primary = c.primary;
        c.button_primary_hover = c.primary_hover;
        c.button_primary_active = c.primary_active;
        c.button_primary_foreground = c.primary_foreground;
        c.button_secondary = c.secondary;
        c.button_secondary_hover = c.secondary_hover;
        c.button_secondary_active = c.secondary_active;
        c.button_secondary_foreground = c.secondary_foreground;
        c.accent = shade(0xe2e9fa, 0x1d2b4d);
        c.accent_foreground = shade(0x16327e, 0xc5d7fb);
        c.ring = c.primary;
        c.slider_bar = c.primary;
        c.slider_thumb = shade(0xffffff, 0xe9edf7);
        c.tab_foreground = c.muted_foreground;
        c.tab_active_foreground = c.foreground;
        c.danger = shade(0xb8322c, 0xef7d75);
        c.danger_hover = shade(0xa12924, 0xf3918a);
        c.danger_active = shade(0x8c221d, 0xe46860);
        c.danger_foreground = shade(0xffffff, 0x190909);
        c.button_danger = c.danger;
        c.button_danger_hover = c.danger_hover;
        c.button_danger_active = c.danger_active;
        c.button_danger_foreground = c.danger_foreground;
        c.warning = shade(0xa6630c, 0xe4b24e);
        c.warning_hover = shade(0x915609, 0xebbf68);
        c.warning_active = shade(0x7c4908, 0xd5a13b);
        c.warning_foreground = shade(0xffffff, 0x1b1305);
        c.blue = c.primary;
        c.success = shade(0x1f7a4d, 0x62c994);
        c.success_hover = shade(0x196740, 0x77d4a4);
        c.success_active = shade(0x145534, 0x4db981);
        c.success_foreground = shade(0xffffff, 0x071a10);
        theme.radius = px(7.);
        theme.radius_lg = px(12.);
    });
}

/// Elevated card/panel surface distinct from the app canvas.
pub fn surface(cx: &App) -> Hsla {
    cx.theme().group_box
}

/// An elevated surface card with a crisp border.
pub fn card_box(cx: &App) -> Div {
    let theme = cx.theme();
    v_flex().rounded(px(12.)).border_1().border_color(theme.border).bg(surface(cx))
}

/// A compact semantic badge/pill.
pub fn pill(text: impl Into<SharedString>, bg: Hsla, fg: Hsla) -> Div {
    div()
        .px_2()
        .py(px(1.5))
        .rounded_full()
        .bg(bg)
        .text_xs()
        .font_weight(FontWeight::MEDIUM)
        .text_color(fg)
        .child(text.into())
}

/// Container for a connected segmented control.
pub fn segmented_bar(cx: &App) -> Div {
    let theme = cx.theme();
    let track = if theme.is_dark() { theme.background } else { theme.secondary };
    h_flex()
        .p(px(2.))
        .gap(px(2.))
        .rounded(px(8.))
        .border_1()
        .border_color(theme.border)
        .bg(track)
}

/// One clickable option inside [`segmented_bar`].
pub fn segment_button(id: impl Into<ElementId>, label: impl Into<SharedString>, active: bool, cx: &App) -> Button {
    let theme = cx.theme();
    if active {
        let fill = if theme.is_dark() { theme.accent } else { surface(cx) };
        let border = if theme.is_dark() { theme.primary.opacity(0.45) } else { theme.border };
        let fg = if theme.is_dark() { theme.accent_foreground } else { theme.primary };
        let style = ButtonCustomVariant::new(cx)
            .color(fill)
            .foreground(fg)
            .hover(fill)
            .active(fill);
        Button::new(id)
            .small()
            .custom(style)
            .bg(fill)
            .border_1()
            .border_color(border)
            .font_weight(FontWeight::SEMIBOLD)
            .label(label)
    } else {
        Button::new(id)
            .small()
            .ghost()
            .text_color(theme.muted_foreground)
            .label(label)
    }
}

/// A key hint such as `Spatie` or `V`.
pub fn keycap(label: &str, cx: &App) -> Div {
    let theme = cx.theme();
    div()
        .px_1p5()
        .py(px(1.))
        .rounded(px(4.))
        .border_1()
        .border_b_2()
        .border_color(theme.border)
        .bg(surface(cx))
        .text_xs()
        .text_color(theme.muted_foreground)
        .child(label.to_string())
}

/// A key hint rendered inside a primary filled button.
pub fn keycap_primary(label: &str, cx: &App) -> Div {
    let fg = cx.theme().primary_foreground;
    div()
        .px_1p5()
        .py(px(1.))
        .rounded(px(4.))
        .border_1()
        .border_b_2()
        .border_color(fg.opacity(0.32))
        .bg(fg.opacity(0.14))
        .text_xs()
        .text_color(fg)
        .child(label.to_string())
}

/// Muted helper text.
pub fn hint(text: impl Into<SharedString>, cx: &App) -> Div {
    div().text_sm().text_color(cx.theme().muted_foreground).child(text.into())
}

/// A recording indicator and loudness-zone bar (0.0–1.0): muted when quiet,
/// green in the clear range, red near clipping.
pub fn level_meter(level: f32, cx: &App) -> Div {
    let theme = cx.theme();
    let clamped = level.clamp(0.0, 1.0);
    let fill = if clamped >= 0.98 {
        theme.danger
    } else if clamped >= 0.53 {
        theme.success
    } else {
        theme.muted_foreground
    };
    h_flex()
        .gap_3()
        .items_center()
        .child(
            div()
                .flex()
                .items_center()
                .justify_center()
                .size(px(16.))
                .rounded_full()
                .bg(theme.danger.opacity(0.18))
                .child(div().size(px(8.)).rounded_full().bg(theme.danger)),
        )
        .child(
            div()
                .w(px(176.))
                .h(px(8.))
                .rounded_full()
                .border_1()
                .border_color(theme.border)
                .bg(theme.secondary)
                .overflow_hidden()
                .child(div().h_full().rounded_full().bg(fill).w(px(176. * clamped))),
        )
}

/// A heading for a screen.
pub fn heading(text: impl Into<SharedString>) -> Div {
    div().text_lg().font_weight(FontWeight::SEMIBOLD).child(text.into())
}

/// A heading with its icon in front.
pub fn titled(icon: Icon, text: impl Into<SharedString>, cx: &App) -> Div {
    h_flex().items_center().gap_2().child(icon.size_5().text_color(cx.theme().primary)).child(heading(text))
}

/// A section card under its icon and title; add the controls as children.
pub fn section(icon: Icon, title: &str, cx: &App) -> Div {
    card_box(cx).p_4().gap_3().child(titled(icon, title.to_string(), cx))
}

/// A small icon-only button that names itself on hover.
pub fn icon_button(id: impl Into<ElementId>, icon: Icon, tip: impl Into<SharedString>) -> Button {
    // A custom size draws the icon at three quarters of it.
    Button::new(id).ghost().with_size(px(26.)).tab_stop(false).icon(icon).tooltip(tip)
}

/// Something that says what it is on hover, such as an icon or a count.
pub fn explained(id: impl Into<ElementId>, tip: impl Into<SharedString>) -> Stateful<Div> {
    let tip = tip.into();
    div().id(id).tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
}

/// A muted icon and a number, with what it counts on hover.
pub fn stat(id: impl Into<ElementId>, icon: Icon, value: impl Into<SharedString>, tip: impl Into<SharedString>, cx: &App) -> Stateful<Div> {
    explained(id, tip)
        .flex()
        .items_center()
        .gap_1()
        .text_sm()
        .text_color(cx.theme().muted_foreground)
        .child(icon.size_4())
        .child(value.into())
}

/// "24 nov".
pub fn day_label(day: NaiveDate) -> String {
    const MONTHS: [&str; 12] = ["jan", "feb", "mrt", "apr", "mei", "jun", "jul", "aug", "sep", "okt", "nov", "dec"];
    format!("{} {}", day.day(), MONTHS[day.month0() as usize])
}

/// A small square of colour, for a legend.
pub fn dot(color: Hsla) -> Div {
    div().flex_none().size(px(8.)).rounded(px(2.)).bg(color)
}

/// A horizontal bar with proportional segments, omitting zero-width parts.
pub fn distribution_bar(height: f32, total: u64, parts: impl IntoIterator<Item = (u64, Hsla)>, cx: &App) -> Div {
    let total = total.max(1) as f32;
    h_flex().h(px(height)).w_full().rounded_full().overflow_hidden().bg(cx.theme().secondary).children(
        parts.into_iter().filter(|(count, _)| *count > 0).map(|(count, color)| div().h_full().w(relative(count as f32 / total)).bg(color)),
    )
}

/// A bar per day, each split into parts stacked from the bottom in the order
/// of `colors`, with the first and last day under it. `tip` says what a day's
/// bar holds; `id` keeps the bars apart from any other chart.
pub fn day_chart(
    id: &'static str,
    days: &[(NaiveDate, Vec<u64>)],
    colors: &[Hsla],
    height: f32,
    tip: impl Fn(NaiveDate, &[u64]) -> String,
    cx: &App,
) -> Div {
    let theme = cx.theme();
    let highest = days.iter().map(|(_, parts)| parts.iter().sum::<u64>()).max().unwrap_or(0).max(1) as f32;
    let bars = days.iter().enumerate().map(|(index, (day, parts))| {
        let sum: u64 = parts.iter().sum();
        let stacked = parts.iter().zip(colors).rev().filter(|(part, _)| **part > 0);
        explained((id, index as u64), tip(*day, parts))
            .flex_1()
            .h_full()
            .flex()
            .flex_col()
            .justify_end()
            .rounded_t(px(2.))
            .overflow_hidden()
            .hover(|style| style.opacity(0.75))
            .children(stacked.map(|(part, color)| div().w_full().h(px(height * *part as f32 / highest)).bg(*color)))
            .when(sum == 0, |bar| bar.child(div().w_full().h(px(1.)).bg(theme.border)))
    });
    let (first, last) = (days.first().map(|(day, _)| *day), days.last().map(|(day, _)| *day));
    v_flex()
        .gap_1p5()
        .child(h_flex().h(px(height)).items_end().gap(px(3.)).children(bars))
        .child(h_flex().justify_between().text_xs().text_color(theme.muted_foreground).children(first.map(day_label)).children(last.map(day_label)))
}

/// A centred picture and message for empty screens, with optional detail.
pub fn empty_state(icon: Icon, title: &str, detail: &str, cx: &App) -> Div {
    let theme = cx.theme();
    v_flex()
        .items_center()
        .gap_2()
        .pt_8()
        .pb_4()
        .child(
            div()
                .flex()
                .items_center()
                .justify_center()
                .size(px(64.))
                .mb_2()
                .rounded_full()
                .bg(theme.accent)
                .text_color(theme.accent_foreground)
                .child(icon.size(px(32.))),
        )
        .child(div().text_xl().font_weight(FontWeight::SEMIBOLD).child(title.to_string()))
        .when(!detail.is_empty(), |d| d.child(hint(detail.to_string(), cx).max_w(px(420.)).text_center()))
}
