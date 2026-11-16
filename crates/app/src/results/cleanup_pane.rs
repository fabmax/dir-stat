//! The cleanup pane: its state (expansion, batches, cursor) and view (UX §7).

use std::collections::{HashMap, HashSet};

use dirstat_core::catalog::Section;
use dirstat_core::tree::CandidateRef;
use iced::widget::{
    Column, button, column, container, mouse_area, row, rule, scrollable, space, text, tooltip,
};
use iced::{Center, Element, Fill, Right, mouse};

use super::cleanup::{CleanupModel, Group, GroupKey, Item};
use crate::app::Message;
use crate::{fonts, format, theme, widgets};

/// Child rows per batch.
const BATCH: usize = 50;

// Fixed heights of the scrolling list, so the cursor's position is known without a layout
// pass (keyboard scroll-follow).
const LIST_PAD: f32 = 4.0;
const SECTION_H: f32 = 34.0;
const GROUP_H: f32 = 50.0;
const COMMAND_H: f32 = 24.0;
const ITEM_H: f32 = 28.0;
const LINK_H: f32 = 30.0;

pub const LIST_ID: &str = "cleanup-list";

pub fn list_id() -> iced::widget::Id {
    iced::widget::Id::new(LIST_ID)
}

/// One focusable line of the pane, for the keyboard cursor.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Line {
    Group(GroupKey),
    Item(GroupKey, CandidateRef),
    More(GroupKey),
    Smaller(Section),
}

#[derive(Debug, Default)]
pub struct CleanupPane {
    pub model: CleanupModel,
    pub expanded: HashSet<GroupKey>,
    pub shown: HashMap<GroupKey, usize>,
    pub show_small: HashSet<Section>,
    pub cursor: Option<Line>,
    /// Trashed items that still show, struck through, until the next re-order.
    pub keep_trashed: HashSet<CandidateRef>,
    /// The list's scroll offset and viewport height, from its `on_scroll`.
    pub scroll_y: f32,
    pub viewport_h: f32,
}

impl CleanupPane {
    fn groups(&self) -> impl Iterator<Item = (&Group, bool)> {
        self.model.sections.iter().flat_map(move |s| {
            let small = self.show_small.contains(&s.section);
            s.groups
                .iter()
                .map(|g| (g, true))
                .chain(s.small.iter().map(move |g| (g, small)))
        })
    }

    pub fn group(&self, key: GroupKey) -> Option<&Group> {
        self.groups().map(|(g, _)| g).find(|g| g.key() == key)
    }

    /// The visible lines in order: what ↑/↓ walk through.
    pub fn lines(&self) -> Vec<Line> {
        self.layout().into_iter().map(|(line, ..)| line).collect()
    }

    /// Every line with its top and height inside the list, matching `view`.
    pub fn layout(&self) -> Vec<(Line, f32, f32)> {
        let mut out = Vec::new();
        let mut y = LIST_PAD;
        for section in &self.model.sections {
            y += SECTION_H;
            for group in &section.groups {
                self.layout_group(group, &mut y, &mut out);
            }
            if !section.small.is_empty() {
                if self.show_small.contains(&section.section) {
                    for group in &section.small {
                        self.layout_group(group, &mut y, &mut out);
                    }
                } else {
                    out.push((Line::Smaller(section.section), y, LINK_H));
                    y += LINK_H;
                }
            }
        }
        out
    }

    fn layout_group(&self, group: &Group, y: &mut f32, out: &mut Vec<(Line, f32, f32)>) {
        let key = group.key();
        let h = group_height(group);
        out.push((Line::Group(key), *y, h));
        *y += h;
        if group.items.len() > 1 && self.expanded.contains(&key) {
            let shown = self.shown_count(key);
            for item in group.items.iter().take(shown) {
                out.push((Line::Item(key, item.candidate), *y, ITEM_H));
                *y += ITEM_H;
            }
            if group.items.len() > shown {
                out.push((Line::More(key), *y, LINK_H));
                *y += LINK_H;
            }
        }
    }

    /// The offset that brings the cursor line into view, if it isn't.
    pub fn follow_cursor(&self) -> Option<f32> {
        let cursor = self.cursor?;
        let (_, top, h) = self.layout().into_iter().find(|(l, ..)| *l == cursor)?;
        if top < self.scroll_y {
            Some((top - SECTION_H - LIST_PAD).max(0.0))
        } else if self.viewport_h > 0.0 && top + h > self.scroll_y + self.viewport_h {
            Some(top + h - self.viewport_h + LIST_PAD)
        } else {
            None
        }
    }

    fn shown_count(&self, key: GroupKey) -> usize {
        self.shown.get(&key).copied().unwrap_or(BATCH)
    }

    pub fn toggle(&mut self, key: GroupKey) {
        if !self.expanded.remove(&key) {
            self.expanded.insert(key);
        }
    }

    pub fn show_more(&mut self, key: GroupKey) {
        let shown = self.shown_count(key);
        self.shown.insert(key, shown + BATCH);
    }

    /// ↑/↓ and friends. Returns the line to activate for Return.
    pub fn move_cursor(&mut self, delta: isize) {
        let lines = self.lines();
        if lines.is_empty() {
            self.cursor = None;
            return;
        }
        let index = self.cursor.and_then(|c| lines.iter().position(|l| *l == c));
        let next = match index {
            None => 0,
            Some(i) => (i as isize + delta).clamp(0, lines.len() as isize - 1) as usize,
        };
        self.cursor = Some(lines[next]);
    }

    /// The candidate a line stands for: a child row, or a single-match group.
    pub fn candidate_of(&self, line: Line) -> Option<CandidateRef> {
        match line {
            Line::Item(_, candidate) => Some(candidate),
            Line::Group(key) => self
                .group(key)
                .filter(|g| g.items.len() == 1)
                .map(|g| g.items[0].candidate),
            _ => None,
        }
    }
}

/// The Safe total for the "Cleanup 38 GB" tab label, from 100 MB on (UX §7).
pub fn tab_total(model: &CleanupModel) -> Option<String> {
    (model.safe_total >= 100_000_000).then(|| {
        if model.scanning {
            format::size_at_least(model.safe_total)
        } else {
            format::size(model.safe_total)
        }
    })
}

pub fn view<'a>(pane: &'a CleanupPane, focused: bool) -> Element<'a, Message> {
    let m = &pane.model;
    let mut body = Column::new().spacing(8).padding([12, 12]);
    let mut list = Column::new().padding([LIST_PAD, 12.0]);
    if let Some(info) = &m.info {
        body = body.push(theme::SMALL.text(info.clone()).style(theme::text_secondary));
    } else if m.is_empty() && !m.scanning {
        body = body
            .push(
                theme::SMALL
                    .text(m.scope.clone())
                    .style(theme::text_secondary),
            )
            .push(
                row![
                    theme::BODY.text("✓").style(theme::text_muted),
                    theme::BODY.text(format!(
                        "No caches or build folders found {}.",
                        m.scope.to_lowercase()
                    ))
                ]
                .spacing(8),
            )
            .push(
                theme::SMALL
                    .text("The largest folders are at the top of the list. Review them yourself.")
                    .style(theme::text_secondary),
            );
    } else {
        let total = if m.scanning {
            format::size_at_least(m.safe_total)
        } else {
            format::size(m.safe_total)
        };
        let mut sub = "likely reclaimable".to_string();
        if m.review_total > 0 {
            sub += &format!(" · + {} to review", format::size(m.review_total));
        }
        body = body.push(
            column![
                theme::SMALL
                    .text(m.scope.clone())
                    .style(theme::text_secondary),
                theme::STAT.text(total),
                theme::SMALL.text(sub).style(theme::text_secondary)
            ]
            .spacing(2),
        );
        if m.scanning {
            body = body.push(
                theme::SMALL
                    .text("Still scanning, more will appear.")
                    .style(theme::text_secondary),
            );
        }
        if m.incomplete {
            body = body.push(
                row![
                    container(theme::CAPTION.text("!").style(theme::text_on_warning))
                        .padding([0, 5])
                        .style(theme::warning_badge),
                    theme::SMALL
                        .text("Some folders couldn't be read, so suggestions may be incomplete.")
                        .style(theme::text_secondary),
                ]
                .spacing(8),
            );
        }
        for section in &m.sections {
            let (icon, label, icon_style): (&str, &str, fn(&iced::Theme) -> text::Style) =
                match section.section {
                    Section::SafeToDelete => (
                        "✓",
                        "SAFE TO DELETE · rebuilt automatically",
                        theme::text_good,
                    ),
                    Section::ReviewFirst => (
                        "!",
                        "REVIEW FIRST · costs a re-download, or may hold your data",
                        theme::text_warning,
                    ),
                };
            list = list.push(
                container(
                    row![
                        theme::CAPTION
                            .with_font(fonts::INTER)
                            .text(icon)
                            .style(icon_style),
                        theme::CAPTION.text(label).style(theme::text_secondary)
                    ]
                    .spacing(6),
                )
                .height(SECTION_H)
                .align_y(iced::Bottom)
                .padding(iced::Padding::ZERO.bottom(8)),
            );
            for group in &section.groups {
                list = list.push(group_view(pane, group, focused));
            }
            if !section.small.is_empty() {
                if pane.show_small.contains(&section.section) {
                    for group in &section.small {
                        list = list.push(group_view(pane, group, focused));
                    }
                } else {
                    let label = format!("Show {} smaller suggestions", section.small.len());
                    list = list.push(link_line(
                        label,
                        Message::CleanupSmaller(section.section),
                        pane.cursor == Some(Line::Smaller(section.section)) && focused,
                    ));
                }
            }
        }
    }
    let disclaimer = container(
        theme::CAPTION
            .with_font(fonts::INTER)
            .text("Based on names and locations. Check before deleting.")
            .style(theme::text_muted),
    )
    .padding([8, 12]);
    let pane_body = column![
        body,
        (!m.sections.is_empty()).then(|| rule::horizontal(1)),
        scrollable(list)
            .id(list_id())
            .on_scroll(Message::CleanupScrolled)
            .height(Fill)
            .style(theme::thin_scrollbar),
        rule::horizontal(1),
        disclaimer
    ];
    mouse_area(
        container(pane_body)
            .width(Fill)
            .height(Fill)
            .style(theme::surface),
    )
    .on_press(Message::FocusCleanup)
    .on_enter(Message::PointerIn(true))
    .on_exit(Message::PointerIn(false))
    .into()
}

fn group_view<'a>(pane: &'a CleanupPane, group: &'a Group, focused: bool) -> Element<'a, Message> {
    let key = group.key();
    let multi = group.items.len() > 1;
    let open = pane.expanded.contains(&key);
    let mut title = row![].spacing(6).align_y(Center);
    title = title.push(if multi {
        Element::from(
            theme::BODY
                .text(if open { "▾" } else { "▸" })
                .style(theme::text_secondary)
                .width(12),
        )
    } else {
        Element::from(space().width(12))
    });
    let label = if multi {
        format!(
            "{} · {}",
            group.title,
            format::thousands(group.items.len() as u64)
        )
    } else {
        group.title.clone()
    };
    title = title.push(
        theme::BODY
            .text(widgets::truncate_end(&label, 220.0, theme::BODY.size))
            .wrapping(text::Wrapping::None),
    );
    if group.redownloadable {
        title = title.push(
            theme::CAPTION
                .with_font(fonts::INTER)
                .text("Re-downloadable")
                .style(theme::text_muted),
        );
    }
    let size = if group.items.iter().any(|i| i.growing) {
        format::size_at_least(group.total)
    } else {
        format::size(group.total)
    };
    let line1 = row![
        container(title).width(Fill).clip(true),
        theme::BODY.text(size)
    ]
    .align_y(Center);
    // Several folders left to move, and no tool needed: all of them in one dialog.
    let batch = group.command.is_none() && group.items.iter().filter(|i| !i.trashed).count() > 1;
    let reason = theme::SMALL
        .text(widgets::truncate_end(
            group.reason,
            if batch { 190.0 } else { 300.0 },
            theme::SMALL.size,
        ))
        .style(theme::text_secondary)
        .wrapping(text::Wrapping::None);
    let line2: Element<'a, Message> = if batch {
        let label = if cfg!(windows) {
            "Move All to Recycle Bin…"
        } else {
            "Move All to Trash…"
        };
        row![
            container(reason).width(Fill).clip(true),
            button(theme::CAPTION.with_font(fonts::MEDIUM).text(label))
                .padding([0, 8])
                .height(18)
                .style(theme::secondary_button)
                .on_press(Message::TrashGroup(key)),
        ]
        .spacing(8)
        .align_y(Center)
        .height(18)
        .into()
    } else {
        reason.into()
    };
    let mut lines = column![line1, line2].spacing(2);
    if let Some(command) = &group.command {
        lines = lines.push(
            row![
                theme::SMALL
                    .with_font(iced::Font::MONOSPACE)
                    .text(widgets::truncate_end(command, 240.0, theme::SMALL.size))
                    .width(Fill)
                    .wrapping(text::Wrapping::None),
                button(theme::CAPTION.text("Copy"))
                    .padding([1, 8])
                    .style(theme::secondary_button)
                    .on_press(Message::CopyCommand(command.clone())),
            ]
            .spacing(8)
            .align_y(Center),
        );
    }
    let cursor = pane.cursor == Some(Line::Group(key)) && focused;
    let mut tip = String::new();
    if let Some(hint) = &group.hint {
        tip += hint;
    }
    if let Some(caution) = group.caution {
        if !tip.is_empty() {
            tip += "\n";
        }
        tip += caution;
    }
    let row_box = container(lines)
        .padding([6, 8])
        .height(group_height(group))
        .width(Fill)
        .style(move |t| theme::tree_row(t, cursor));
    let mut area: Element<'a, Message> = mouse_area(row_box)
        .on_press(Message::CleanupGroupPressed(key))
        .interaction(mouse::Interaction::Pointer)
        .into();
    if !tip.is_empty() {
        area = tooltip(
            area,
            container(theme::SMALL.text(tip))
                .padding([4, 8])
                .max_width(320)
                .style(theme::raised),
            tooltip::Position::Bottom,
        )
        .delay(std::time::Duration::from_millis(500))
        .into();
    }
    let mut out = column![area];
    if multi && open {
        let shown = pane.shown.get(&key).copied().unwrap_or(BATCH);
        for item in group.items.iter().take(shown) {
            out = out.push(item_view(
                key,
                item,
                pane.cursor == Some(Line::Item(key, item.candidate)) && focused,
            ));
        }
        if group.items.len() > shown {
            let label = format!(
                "Show {} more",
                format::thousands((group.items.len() - shown) as u64)
            );
            out = out.push(link_line(
                label,
                Message::CleanupMore(key),
                pane.cursor == Some(Line::More(key)) && focused,
            ));
        }
    }
    out.into()
}

fn group_height(group: &Group) -> f32 {
    GROUP_H
        + if group.command.is_some() {
            COMMAND_H
        } else {
            0.0
        }
}

fn item_view<'a>(key: GroupKey, item: &'a Item, cursor: bool) -> Element<'a, Message> {
    let path = widgets::truncate_middle(&item.path, 190.0, theme::SMALL.size);
    let path: Element<'a, Message> = if item.trashed {
        iced::widget::rich_text([iced::widget::span(path).strikethrough(true)])
            .size(theme::SMALL.size)
            .font(fonts::INTER)
            .style(theme::text_muted)
            .on_link_click(iced::never)
            .wrapping(text::Wrapping::None)
            .into()
    } else {
        theme::SMALL
            .text(path)
            .wrapping(text::Wrapping::None)
            .into()
    };
    let size = if item.trashed {
        "In Trash".to_string()
    } else if item.growing {
        format::size_at_least(item.size)
    } else {
        format::size(item.size)
    };
    let mut line = row![space().width(18), container(path).width(Fill).clip(true)]
        .spacing(6)
        .align_y(Center);
    if let Some(note) = item.note {
        line = line.push(
            theme::CAPTION
                .with_font(fonts::INTER)
                .text(note)
                .style(theme::text_muted),
        );
    }
    line = line
        .push(
            theme::SMALL
                .text(size)
                .style(theme::text_secondary)
                .width(72)
                .align_x(Right),
        )
        .push(
            theme::CAPTION
                .with_font(fonts::INTER)
                .text(item.age.clone().unwrap_or_default())
                .style(theme::text_muted)
                .width(32)
                .align_x(Right),
        );
    let row_box = container(line)
        .padding([0, 8])
        .height(ITEM_H)
        .center_y(ITEM_H)
        .width(Fill)
        .style(move |t| theme::tree_row(t, cursor));
    mouse_area(row_box)
        .on_press(Message::CleanupItemPressed(key, item.candidate))
        .interaction(mouse::Interaction::Pointer)
        .into()
}

fn link_line<'a>(label: String, message: Message, cursor: bool) -> Element<'a, Message> {
    let row_box = container(theme::SMALL.text(label).style(theme::text_accent_link))
        .padding([0, 26])
        .height(LINK_H)
        .center_y(LINK_H)
        .width(Fill)
        .style(move |t| theme::tree_row(t, cursor));
    mouse_area(row_box)
        .on_press(message)
        .interaction(mouse::Interaction::Pointer)
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::results::cleanup::SectionModel;
    use dirstat_core::tree::NodeId;

    fn group(rule: u16, items: usize) -> Group {
        Group {
            section: Section::SafeToDelete,
            rule,
            title: format!("rule {rule}"),
            items: (0..items)
                .map(|i| Item {
                    candidate: CandidateRef::Dir(NodeId(i as u32 + 1)),
                    path: format!("p{i}"),
                    size: 1,
                    growing: false,
                    age: None,
                    note: None,
                    trashed: false,
                })
                .collect(),
            total: items as u64,
            reason: "",
            caution: None,
            hint: None,
            redownloadable: false,
            command: None,
        }
    }

    #[test]
    fn lines_batches_and_cursor() {
        let mut pane = CleanupPane::default();
        pane.model.sections.push(SectionModel {
            section: Section::SafeToDelete,
            groups: vec![group(1, 120), group(2, 1)],
            small: vec![group(3, 1)],
        });
        assert_eq!(pane.lines().len(), 3, "two groups and the 'smaller' line");
        pane.toggle((Section::SafeToDelete, 1));
        assert_eq!(
            pane.lines().len(),
            3 + 50 + 1,
            "a batch of 50 and a More line"
        );
        pane.show_more((Section::SafeToDelete, 1));
        assert_eq!(pane.lines().len(), 3 + 100 + 1);
        pane.show_more((Section::SafeToDelete, 1));
        assert_eq!(pane.lines().len(), 3 + 120);
        pane.move_cursor(1);
        assert_eq!(pane.cursor, Some(Line::Group((Section::SafeToDelete, 1))));
        assert_eq!(
            pane.candidate_of(Line::Group((Section::SafeToDelete, 1))),
            None,
            "a multi-match group is no single target"
        );
        assert_eq!(
            pane.candidate_of(Line::Group((Section::SafeToDelete, 2))),
            Some(CandidateRef::Dir(NodeId(1)))
        );
        // Scroll-follow: line tops match the fixed heights; moving below the viewport scrolls.
        let layout = pane.layout();
        assert_eq!(layout[0].1, LIST_PAD + SECTION_H);
        assert_eq!(layout[1].1, LIST_PAD + SECTION_H + GROUP_H);
        pane.viewport_h = 200.0;
        assert_eq!(pane.follow_cursor(), None);
        pane.move_cursor(10);
        let y = pane.follow_cursor().expect("below the viewport");
        let (_, top, h) = layout[10];
        assert_eq!(y, top + h - 200.0 + LIST_PAD);
        pane.scroll_y = y;
        assert_eq!(pane.follow_cursor(), None);
        pane.move_cursor(-10);
        assert_eq!(
            pane.follow_cursor(),
            Some(0.0),
            "back to the top, header included"
        );
        pane.scroll_y = 0.0;
        pane.move_cursor(-5);
        assert_eq!(
            pane.cursor,
            Some(Line::Group((Section::SafeToDelete, 1))),
            "clamped at the top"
        );
    }
}
