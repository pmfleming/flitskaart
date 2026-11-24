//! Cards being made in the background, from a conversation or a pasted text.
//! The app moves to Concepten, which shows it happening until the drafts
//! arrive, or what went wrong.
//!
//! The model gives no sign of how far along it is, so the progress bar is an
//! estimate: how long earlier runs took for this much text with this model and
//! effort (see [`drafting_time`]), slowing down as it nears the end so it never
//! claims to be done before it is.

use std::time::{Duration, Instant};

use dutch_flashcards_core::drafting_time::{self, Estimate, Run};
use dutch_flashcards_core::llm::model_name;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::progress::Progress;
use gpui_kit::component::skeleton::Skeleton;
use gpui_kit::component::{ActiveTheme as _, Colorize as _, h_flex, v_flex};
use gpui_kit::{
    Animation, AnimationExt as _, AnyElement, Context, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _, Styled as _, Task, div,
    pulsating_between, px,
};

use crate::app::{FlashcardsApp, Screen};
use crate::icons;
use crate::ui::card_box;

/// What to expect the first time, before the app has timed a run.
const FIRST_GUESS: Duration = Duration::from_secs(40);
/// How often the bar and the clock move on.
const TICK: Duration = Duration::from_millis(200);

/// Where the cards come from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
    Talk,
    Add,
}

impl Origin {
    fn title(self) -> &'static str {
        match self {
            Origin::Talk => "Kaarten maken uit je gesprek",
            Origin::Add => "Kaarten maken uit je tekst",
        }
    }

    fn what(self) -> &'static str {
        match self {
            Origin::Talk => "je gesprek",
            Origin::Add => "je tekst",
        }
    }
}

/// Cards on their way.
pub struct Making {
    pub origin: Origin,
    started: Instant,
    /// The model, effort and length of text, timed once the cards are in.
    run: Run,
    /// From earlier runs; `None` the first time.
    estimate: Option<Estimate>,
    _ticker: Task<()>,
}

impl Making {
    fn expected(&self) -> Duration {
        self.estimate.map_or(FIRST_GUESS, |estimate| Duration::from_secs_f64(estimate.seconds))
    }
}

/// Making cards failed; Concepten says so until it is tried again or dismissed.
pub struct MakingFailed {
    pub origin: Origin,
    pub message: String,
}

impl FlashcardsApp {
    /// Cards are being made from `text`: open Concepten, which shows the progress.
    pub fn start_making(&mut self, origin: Origin, text: &str, cx: &mut Context<Self>) {
        let settings = &self.settings;
        let run = Run {
            cli: settings.cli.id().to_string(),
            model: settings.model.clone(),
            effort: settings.effort.as_str().to_string(),
            chars: text.chars().count(),
            seconds: 0.,
        };
        let earlier = self.store.drafting_runs().unwrap_or_default();
        let estimate = drafting_time::estimate(&earlier, &run.cli, &run.model, &run.effort, run.chars);
        let ticker = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(TICK).await;
                let going = this.update(cx, |this, cx| {
                    cx.notify();
                    this.making.is_some()
                });
                if !matches!(going, Ok(true)) {
                    break;
                }
            }
        });
        self.making = Some(Making { origin, started: Instant::now(), run, estimate, _ticker: ticker });
        self.making_failed = None;
        // Straight to Concepten, where the cards will arrive.
        self.screen = Screen::Drafts;
        self.notice = None;
        cx.notify();
    }

    /// The cards are in, or there were none to make: remember how long it took.
    pub fn making_done(&mut self) {
        if let Some(making) = self.making.take() {
            let run = Run { seconds: making.started.elapsed().as_secs_f64(), ..making.run };
            if let Err(err) = self.store.record_drafting_run(&run, Self::now()) {
                crate::logging::error("kaarten maken", format!("De duur is niet bewaard: {err:#}"));
            }
        }
    }

    /// Making cards failed with `message`.
    pub fn making_failed(&mut self, origin: Origin, message: impl Into<String>) {
        self.making = None;
        self.making_failed = Some(MakingFailed { origin, message: message.into() });
    }

    /// The panel at the top of Concepten while cards are made, or when it failed.
    pub fn render_making(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if let Some(failed) = &self.making_failed {
            return Some(self.render_making_failed(failed, cx));
        }
        let making = self.making.as_ref()?;
        let theme = cx.theme().clone();
        let elapsed = making.started.elapsed();
        let expected = making.expected();
        let share = elapsed.as_secs_f32() / expected.as_secs_f32().max(1.);
        // Rises steadily at first, then ever slower: 80% at the expected time, never 100%.
        let progress = (1. - (-1.6 * share).exp()).min(0.97) * 100.;
        let late = share > 1.5;
        let model = model_name(&making.run.model);
        let expected_text = format!("verwacht ~{} s", (expected.as_secs_f32() / 5.).round().max(1.) as u64 * 5);
        let basis = match making.estimate {
            None => "eerste keer, de app leert het tempo".to_string(),
            Some(Estimate { same_model: 0, .. }) => "nog niet eerder met dit model".to_string(),
            Some(Estimate { same_model: 1, .. }) => "uit 1 eerdere keer met dit model".to_string(),
            Some(Estimate { same_model, .. }) => format!("uit {same_model} eerdere keren met dit model"),
        };

        let pulse = div()
            .size(px(64.))
            .flex_none()
            .rounded_full()
            .bg(theme.primary.opacity(0.15))
            .with_animation("making-pulse", Animation::new(Duration::from_millis(1600)).repeat().with_easing(pulsating_between(0.35, 1.)), |ring, t| {
                ring.opacity(t)
            });
        let badge = div()
            .relative()
            .size(px(64.))
            .flex_none()
            .child(div().absolute().inset_0().child(pulse))
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_color(theme.primary)
                    .child(icons::cards().size_7()),
            );
        let text = v_flex()
            .flex_1()
            .min_w_0()
            .gap_1()
            .child(div().text_xl().font_weight(FontWeight::SEMIBOLD).child(making.origin.title()))
            .child(div().text_sm().text_color(theme.muted_foreground).child(format!(
                "{model} leest {} en maakt er kaarten van: eerst je verbeterde fouten, dan nieuwe zinnen en woorden.",
                making.origin.what()
            )));
        let clock = h_flex()
            .justify_between()
            .text_xs()
            .text_color(theme.muted_foreground)
            .child(format!("{} s", elapsed.as_secs()))
            .child(if late { format!("Dit duurt langer dan verwacht ({expected_text})…") } else { format!("{expected_text} · {basis}") });
        // Grey cards where the new ones will appear.
        let placeholder = || {
            v_flex().gap_2().p_4().rounded(px(10.)).border_1().border_color(theme.border).child(Skeleton::new().h_5().w(px(320.)).rounded(px(4.))).child(
                Skeleton::new().secondary().h_4().w(px(220.)).rounded(px(4.)),
            )
        };
        let panel = card_box(cx)
            .id("making-cards")
            .p_5()
            .gap_4()
            .border_color(theme.primary.opacity(0.5))
            .child(h_flex().gap_4().items_center().child(badge).child(text))
            .child(v_flex().gap_1p5().child(Progress::new("making-progress").color(theme.primary).value(progress)).child(clock))
            .child(div().text_sm().child("Je kunt intussen verder in de app; de concepten verschijnen hier vanzelf."));
        Some(v_flex().gap_3().child(panel).child(placeholder()).child(placeholder()).into_any_element())
    }

    fn render_making_failed(&self, failed: &MakingFailed, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let again = match failed.origin {
            // The conversation is saved, so its cards can be asked for again.
            Origin::Talk => Button::new("making-retry")
                .primary()
                .icon(icons::retry())
                .label("Opnieuw proberen")
                .on_click(cx.listener(|this, _, _, cx| this.make_conversation_cards(cx))),
            // The text is still in Toevoegen.
            Origin::Add => Button::new("making-back")
                .primary()
                .icon(icons::paste())
                .label("Terug naar Toevoegen")
                .on_click(cx.listener(|this, _, window, cx| {
                    this.making_failed = None;
                    this.show(Screen::Add, window, cx);
                })),
        };
        let dismiss = Button::new("making-dismiss").ghost().label("Sluiten").on_click(cx.listener(|this, _, _, cx| {
            this.making_failed = None;
            cx.notify();
        }));
        card_box(cx)
            .id("making-failed")
            .p_5()
            .gap_3()
            .border_color(theme.danger.opacity(0.6))
            .child(
                h_flex()
                    .gap_3()
                    .items_center()
                    .child(icons::close().size_6().text_color(theme.danger))
                    .child(div().text_xl().font_weight(FontWeight::SEMIBOLD).child("Kaarten maken is mislukt")),
            )
            .child(div().text_sm().text_color(theme.muted_foreground).child(failed.message.clone()))
            .child(h_flex().gap_2().child(again).child(dismiss))
            .into_any_element()
    }
}
