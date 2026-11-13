//! Compile + runtime probes for APIs that are not on the main dir-stat UI path:
//! Task combinators, Subscription constructors, and a widget "playground".
use crate::style::{self, ThemeChoice};
use crate::{App, FILTER_INPUT, Message};

use iced::futures::{SinkExt, Stream, stream};
use iced::widget::{
    button, center, checkbox, column, container, keyed_column, lazy, pick_list, pin,
    progress_bar, responsive, row, rule, space, stack, text, text_input, toggler, tooltip,
};
use iced::{Center, Element, Font, Shrink, Subscription, Task, Theme, font};

use std::time::Duration;

/// Task: done / perform / future / run / stream / chain / batch / collect / then / map /
/// discard / and_then — all in one pipeline. Produces two Log messages at boot.
pub fn task_combinators() -> Task<Message> {
    let a: Task<u32> = Task::done(1);
    let b: Task<u32> = Task::perform(async { 2 }, |x| x);
    let c: Task<u32> = Task::future(async { 3 });
    let d: Task<u32> = Task::run(stream::iter(vec![4u32, 5]), |x| x);
    let e: Task<u32> = Task::stream(stream::iter([6u32]));

    let sequential = a.chain(b); // b starts after a completes
    let parallel = Task::batch([sequential, c, d, e]); // run concurrently
    let sum = parallel
        .collect() // Task<Vec<u32>>
        .then(|values| Task::done(values.iter().sum::<u32>()));

    let side_effect: Task<Message> = Task::done(()).discard();

    let maybe: Task<Option<u32>> = Task::done(Some(41));
    let and_then = maybe.and_then(|x| Task::done(x + 1));

    Task::batch([
        sum.map(|sum| Message::Log(format!("Task combinators OK: sum = {sum} (expect 21)"))),
        side_effect,
        and_then.map(|x| Message::Log(format!("Task::and_then OK: {x} (expect 42)"))),
    ])
}

/// Subscription::run takes a *fn pointer* `fn() -> S` (non-capturing).
fn heartbeat() -> impl Stream<Item = Message> {
    // 0.14: stream::channel(size, impl AsyncFnOnce(mpsc::Sender<T>))
    iced::stream::channel(1, async |mut output| {
        let mut beats = 0u64;
        loop {
            tokio::time::sleep(Duration::from_secs(1)).await;
            beats += 1;
            if output.send(Message::Heartbeat(beats)).await.is_err() {
                break;
            }
        }
    })
}

/// Subscription::run_with(data, fn(&D) -> S): data is hashed into the identity.
fn countdown(from: &u32) -> impl Stream<Item = u32> + use<> {
    stream::iter((0..=*from).rev().collect::<Vec<_>>())
}

pub fn subscriptions(enabled: bool) -> Subscription<Message> {
    if !enabled {
        return Subscription::none();
    }

    Subscription::batch([
        Subscription::run(heartbeat),
        Subscription::run_with(3u32, countdown)
            // .with(value) prepends context (value: Hash + Clone + Send + Sync + 'static)
            .with("countdown")
            // map/filter_map closures MUST be non-capturing (checked at compile time)
            .map(|(label, n)| Message::Log(format!("{label}: {n}"))),
    ])
}

/// Widget playground: checkbox, toggler, pick_list, text_input, rule, tooltip, stack/pin,
/// keyed_column, lazy, responsive, built-in styles, fonts and glyph fallback.
pub fn playground(app: &App) -> Element<'_, Message> {
    let bold = Font {
        weight: font::Weight::Bold,
        ..Font::DEFAULT
    };

    let controls = column![
        text("Controls").size(16).font(bold),
        // 0.14: checkbox(is_checked).label(..).on_toggle(..)
        checkbox(app.show_details)
            .label("Show file counts")
            .on_toggle(Message::ToggleDetails),
        // 0.14: toggler(is_toggled).label(..).on_toggle(..)
        toggler(app.compact)
            .label("Compact rows")
            .on_toggle(Message::ToggleCompact),
        row![
            text("Theme"),
            pick_list(
                &ThemeChoice::ALL[..],
                Some(app.theme_choice),
                Message::ThemeChanged
            )
            .width(140),
        ]
        .spacing(8)
        .align_y(Center),
        text_input("Filter… (⌘F focuses me)", &app.filter)
            .id(FILTER_INPUT)
            .on_input(Message::FilterChanged)
            .on_submit(Message::FilterSubmitted)
            .padding(6)
            .style(style::search_input),
        button("Focus filter (operation::focus)").on_press(Message::FocusFilter),
    ]
    .spacing(10);

    let buttons = row![
        button("primary").style(button::primary).on_press(Message::Noop),
        button("secondary").style(button::secondary).on_press(Message::Noop),
        button("success").style(button::success).on_press(Message::Noop),
        button("warning").style(button::warning).on_press(Message::Noop),
        button("danger").style(button::danger).on_press(Message::Noop),
        button("text").style(button::text).on_press(Message::Noop),
        button("background").style(button::background).on_press(Message::Noop),
        button("subtle").style(button::subtle).on_press(Message::Noop),
        button("disabled"), // no on_press => disabled
    ]
    .spacing(6)
    .wrap(); // 0.14: Row::wrap() -> row::Wrapping

    let glyphs = column![
        text("Glyphs (default Shaping::Auto): ▸ ▾ ● ✓ ⚠ ↻ ✕ … 📁 🗑"),
        text("Glyphs (Shaping::Basic):        ▸ ▾ ● ✓ ⚠ ↻ ✕ … 📁 🗑")
            .shaping(text::Shaping::Basic),
        text("Glyphs (Shaping::Advanced):     ▸ ▾ ● ✓ ⚠ ↻ ✕ … 📁 🗑")
            .shaping(text::Shaping::Advanced),
        text("Bold heading").size(18).font(bold),
        text("Monospace 1234567890 KB").font(Font::MONOSPACE),
        text("danger style").style(text::danger),
        text("secondary style").style(text::secondary),
        text("custom muted style").style(style::muted),
        text("Wrapping::None — a very long line that should not wrap but get clipped by its container")
            .wrapping(text::Wrapping::None),
    ]
    .spacing(4);

    let containers = row![
        container("rounded_box").padding(8).style(container::rounded_box),
        container("bordered_box").padding(8).style(container::bordered_box),
        container("dark").padding(8).style(container::dark),
        container("primary").padding(8).style(container::primary),
        container("card").padding(8).style(style::card),
    ]
    .spacing(8);

    let bars = column![
        progress_bar(0.0..=100.0, 30.0).girth(8),
        progress_bar(0.0..=100.0, 60.0).girth(8).style(progress_bar::success),
        progress_bar(0.0..=100.0, 90.0).girth(8).style(progress_bar::danger),
    ]
    .spacing(6)
    .width(220);

    let keyed = keyed_column((0..3u32).map(|i| (i, text!("keyed row {i}").into()))).spacing(2);

    // lazy: view is rebuilt only when the dependency hash changes; the closure must produce
    // an Element<'static, ..> (no borrowing from state!).
    let lazy_part = lazy(app.sort, |sort| text(format!("lazy(dep = {sort})")));

    let responsive_part = responsive(|size| {
        text(format!("responsive: {:.0} × {:.0}", size.width, size.height)).into()
    })
    .height(Shrink);

    let stacked = stack![
        container(space()).width(160).height(50).style(container::bordered_box),
        pin(text("pinned at (12, 16)").size(12)).x(12).y(16),
    ];

    let tip = tooltip(
        button("hover me (tooltip)").on_press(Message::Noop),
        container("Tooltip body").padding(8).style(container::rounded_box),
        tooltip::Position::Bottom,
    )
    .gap(4)
    .delay(Duration::from_millis(300));

    let card = container(
        column![
            text("API playground").size(20).font(bold),
            row![
                controls.width(320),
                rule::vertical(1),
                column![glyphs, containers, bars].spacing(10)
            ]
            .spacing(16),
            rule::horizontal(1),
            buttons,
            row![keyed, lazy_part, stacked, tip].spacing(16).align_y(Center),
            responsive_part,
        ]
        .spacing(12),
    )
    .padding(16)
    .max_width(1100)
    .style(style::card);

    center(card).padding(16).into()
}

/// Font fallback probe (PROBE_FONTS=1): what does each Font actually render as on this OS?
pub fn fonts() -> Element<'static, Message> {
    let weights = [
        ("Normal", font::Weight::Normal),
        ("Medium", font::Weight::Medium),
        ("Semibold", font::Weight::Semibold),
        ("Bold", font::Weight::Bold),
    ];

    let families: [(&str, Font); 6] = [
        ("with_name(\"Fira Sans\") bundled", Font::with_name("Fira Sans")),
        ("Font::DEFAULT (SansSerif)", Font::DEFAULT),
        ("with_name(\"Helvetica Neue\")", Font::with_name("Helvetica Neue")),
        ("with_name(\"Avenir Next\")", Font::with_name("Avenir Next")),
        ("with_name(\".SF NS\")", Font::with_name(".SF NS")),
        ("Font::MONOSPACE", Font::MONOSPACE),
    ];

    let mut table = column![].spacing(10);
    for (label, family) in families {
        let mut line = row![text(label).size(13).width(230).style(text::secondary)].spacing(14);
        for (weight_label, weight) in weights {
            line = line.push(
                text(format!("{weight_label} Rescan 1.2 GB ▸✓"))
                    .size(17)
                    .font(Font { weight, ..family }),
            );
        }
        table = table.push(line.align_y(Center));
    }

    center(
        container(column![text("Font fallback probe").size(20), table].spacing(16))
            .padding(20)
            .style(style::card),
    )
    .into()
}

/// Uses Theme param explicitly to show the style fn signature.
#[allow(dead_code)]
pub fn style_fn_signature(theme: &Theme) -> container::Style {
    style::card(theme)
}
