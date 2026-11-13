//! M0 placeholder screen: the type scale, fallback glyphs at every weight, and sample
//! formatting, for checking fonts and tokens in screenshots. The start screen replaces it.

use std::time::Duration;

use iced::widget::{Column, column, container, row, scrollable, space, text};
use iced::{Border, Center, Element, Fill, Font};

use crate::app::Message;
use crate::theme::{self, TextRole};
use crate::{fonts, format};

/// UX §11.7 fallback glyphs. Always their own Regular-weight `text` (design §6.1).
const GLYPHS: [&str; 9] = ["▸", "▾", "›", "·", "—", "≥", "…", "✓", "⌂"];

const ROLES: [(&str, TextRole); 7] = [
    ("Display 24/30 · 600", theme::DISPLAY),
    ("Title 15/20 · 600", theme::TITLE),
    ("Stat 28/34 · 600", theme::STAT),
    ("Body 13/18 · 400", theme::BODY),
    ("Body strong 13/18 · 500", theme::BODY_STRONG),
    ("Small 12/16 · 400", theme::SMALL),
    ("Caption 11/14 · 500", theme::CAPTION),
];

pub fn view() -> Element<'static, Message> {
    let header = column![
        theme::DISPLAY.text("dir-stat"),
        theme::SMALL
            .text("Type specimen: Inter 400 / 500 / 600, tokens and formatting")
            .style(theme::text_secondary),
    ]
    .spacing(4);

    let body = column![
        header,
        row![
            card("Type scale", type_scale()),
            column![
                card("Fallback glyphs", glyphs()),
                card("Formatting", formatting())
            ]
            .spacing(16),
        ]
        .spacing(16),
        card("Color tokens", swatches()),
    ]
    .spacing(20)
    .max_width(1136);

    // Scrollable gives the wrapping rows an unbounded height to measure against.
    scrollable(container(body).padding(32).width(Fill))
        .height(Fill)
        .into()
}

fn card<'a>(title: &'a str, content: Element<'a, Message>) -> Element<'a, Message> {
    container(
        column![
            theme::CAPTION
                .text(title.to_uppercase())
                .style(theme::text_secondary),
            content
        ]
        .spacing(12),
    )
    .padding(16)
    .width(Fill)
    .style(theme::card)
    .into()
}

fn type_scale() -> Element<'static, Message> {
    let rows = ROLES.iter().map(|(label, role)| {
        row![
            theme::SMALL
                .text(*label)
                .style(theme::text_secondary)
                .width(170),
            role.text("Library 12.3 GB"),
        ]
        .align_y(Center)
        .into()
    });
    Column::with_children(rows).spacing(8).into()
}

fn glyphs() -> Element<'static, Message> {
    let weights: [(&str, Font); 3] = [
        ("400", fonts::INTER),
        ("500", fonts::MEDIUM),
        ("600", fonts::SEMIBOLD),
    ];
    let rows = weights.into_iter().map(|(label, font)| {
        let mut line = row![
            theme::SMALL
                .text(label)
                .style(theme::text_secondary)
                .width(40),
            theme::BODY.with_font(font).text("Rescan"),
        ]
        .spacing(8)
        .align_y(Center);
        for glyph in GLYPHS {
            line = line.push(theme::BODY.text(glyph));
        }
        line.into()
    });
    column![
        Column::with_children(rows).spacing(10),
        theme::SMALL
            .text("Glyphs are separate Regular text; muted text looks like this.")
            .style(theme::text_muted),
    ]
    .spacing(12)
    .into()
}

fn formatting() -> Element<'static, Message> {
    const DAY: u64 = 86_400;
    let samples = [
        ("size", format::size(12_300_000_000)),
        ("lower bound", format::size_at_least(4_500_000_000)),
        ("pending", format::PENDING.to_string()),
        ("exact", format::bytes_exact(24_113_402_880)),
        ("count", format::count(12_345)),
        ("prose", format::plural(312_004, "item", "items")),
        ("percent", format::percent(42, 1000)),
        ("elapsed", format::elapsed(Duration::from_secs(3753))),
        ("eta", format::eta(Duration::from_secs(120))),
        ("relative", format::relative(Duration::from_secs(240 * DAY))),
        (
            "compact",
            format::relative_compact(Duration::from_secs(5 * DAY)),
        ),
        ("date", format::datetime(1_773_508_920, 3600)),
        ("rate", format::rate(9_800.0)),
    ];
    let cells = samples.into_iter().map(|(label, value)| {
        column![
            theme::CAPTION.text(label).style(theme::text_secondary),
            theme::BODY.text(value).wrapping(text::Wrapping::None),
        ]
        .spacing(2)
        .width(160)
        .into()
    });
    row(cells).spacing(12).wrap().vertical_spacing(12).into()
}

fn swatches() -> Element<'static, Message> {
    let cells = theme::LIGHT
        .named()
        .into_iter()
        .enumerate()
        .map(|(index, (name, _))| {
            let chip = container(space().width(16).height(16)).style(move |theme: &iced::Theme| {
                let tokens = theme::tokens(theme);
                container::Style {
                    background: Some(tokens.named()[index].1.into()),
                    border: Border {
                        color: tokens.border_strong,
                        width: 1.0,
                        radius: 4.0.into(),
                    },
                    ..container::Style::default()
                }
            });
            row![chip, theme::SMALL.text(name).style(theme::text_secondary)]
                .spacing(8)
                .align_y(Center)
                .width(175)
                .into()
        });
    row(cells).spacing(8).wrap().vertical_spacing(8).into()
}
