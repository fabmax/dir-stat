//! The tree pane's view: header, virtualized rows and their anatomy (UX §5.3, §6.2).

use std::time::{Duration, Instant};

use dirstat_core::scan::{ErrorClass, Skip};
use dirstat_core::tree::{FileFlags, NodeFlags, NodeId, NodeState, Tree};
use iced::widget::{
    Column, Id, Space, container, hover, mouse_area, progress_bar, responsive, row, scrollable,
    space, text, tooltip,
};
use iced::{Center, Element, Fill, Length, Right, mouse};

use super::order::{Column as SortColumn, split_files};
use super::rows::{self, OpenAggregate, ROW_H, Row, RowKey, SmallFiles};
use super::{Results, errors};
use crate::app::Message;
use crate::theme::{self, TextRole};
use crate::{fonts, format, widgets};

pub const TREE_ID: &str = "tree";
pub const HEADER_H: f32 = 28.0;

const INDENT: f32 = 16.0;
const SIZE_W: f32 = 100.0;
const BAR_W: f32 = 72.0;
const PERCENT_W: f32 = 40.0;
const ITEMS_W: f32 = 64.0;
const TOOLTIP_DELAY: Duration = Duration::from_millis(500);

/// Which optional columns fit (UX §6.1).
#[derive(Clone, Copy)]
struct Layout {
    items: bool,
    percent: bool,
}

impl Layout {
    fn for_width(width: f32) -> Layout {
        Layout {
            items: width >= 600.0,
            percent: width >= 480.0,
        }
    }
}

pub fn tree_id() -> Id {
    Id::new(TREE_ID)
}

pub fn view(results: &Results, now: Instant) -> Element<'_, Message> {
    let pane = responsive(move |size| pane_content(results, size, now));
    let pane = container(pane)
        .width(Fill)
        .height(Fill)
        .style(theme::surface);
    mouse_area(pane)
        .on_enter(Message::PointerIn(true))
        .on_exit(Message::PointerIn(false))
        .into()
}

/// The header and the visible rows for one layout size. Runs on every layout, so its cost
/// depends only on the rows on screen, never on the total (design §6.6).
pub fn pane_content(results: &Results, size: iced::Size, now: Instant) -> Element<'_, Message> {
    let started = Instant::now();
    let layout = Layout::for_width(size.width);
    let header = header(results, layout);
    let tree = &results.tree;
    if let Some(error) = errors::root_unreadable(tree) {
        return errors::root_unreadable_view(results, error);
    }
    if !tree.is_started() || tree.node(results.view.zoom).state() == NodeState::Pending {
        return Column::new().push(header).push(placeholder_rows()).into();
    }
    let view = &results.view;
    let height = (size.height - HEADER_H).max(ROW_H);
    let (first, last) = rows::range(view.viewport.offset_y, height, view.rows.len());
    let visible = view.rows[first..last]
        .iter()
        .map(|r| row_view(results, r, layout, now));
    let content = Column::with_capacity(last - first + 2)
        .push(Space::new().height(first as f32 * ROW_H))
        .extend(visible)
        .push(Space::new().height((view.rows.len() - last) as f32 * ROW_H))
        .width(Fill);
    let list = scrollable(content)
        .id(tree_id())
        .on_scroll(Message::TreeScrolled)
        .height(Fill)
        .style(theme::thin_scrollbar);
    if cfg!(debug_assertions) && std::env::var_os("DIRSTAT_TIMING").is_some() {
        eprintln!(
            "tree view: {} of {} rows in {:?}",
            last - first,
            view.rows.len(),
            started.elapsed()
        );
    }
    Column::new().push(header).push(list).into()
}

fn header(results: &Results, layout: Layout) -> Element<'_, Message> {
    let sort = results.view.sort;
    let cell = |label: &'static str, column: SortColumn, width: Length, right: bool| {
        let active = sort.column == column && !(label == "% of parent");
        let arrow = if sort.descending { "▾" } else { "▴" };
        let mut content = row![].spacing(4).align_y(Center);
        if right {
            content = content.push(space().width(Fill));
        }
        content = content.push(
            TextRole {
                font: fonts::MEDIUM,
                ..theme::SMALL
            }
            .text(label)
            .style(move |t| {
                if active {
                    theme::text_primary(t)
                } else {
                    theme::text_secondary(t)
                }
            }),
        );
        if active {
            content = content.push(theme::SMALL.text(arrow).style(theme::text_primary));
        }
        mouse_area(
            container(content)
                .width(width)
                .height(HEADER_H)
                .center_y(HEADER_H),
        )
        .on_press(Message::SortBy(column))
        .interaction(mouse::Interaction::Pointer)
    };
    let mut header = row![
        space().width(12),
        cell("Name", SortColumn::Name, Length::Fill, false),
        cell("Size", SortColumn::Size, Length::Fixed(SIZE_W), true),
        space().width(12),
    ];
    if layout.percent {
        let tip = tip_box("Share of the folder it is in.");
        header = header.push(
            tooltip(
                cell(
                    "% of parent",
                    SortColumn::Size,
                    Length::Fixed(BAR_W + 6.0 + PERCENT_W),
                    true,
                ),
                tip,
                tooltip::Position::Bottom,
            )
            .delay(TOOLTIP_DELAY),
        );
    } else {
        header = header.push(space().width(BAR_W));
    }
    if layout.items {
        header = header.push(space().width(12)).push(cell(
            "Items",
            SortColumn::Items,
            Length::Fixed(ITEMS_W),
            true,
        ));
    }
    container(
        header
            .push(space().width(16))
            .height(HEADER_H)
            .align_y(Center),
    )
    .style(theme::header)
    .into()
}

/// Muted bars until the root's own listing arrives (UX §5.1).
fn placeholder_rows<'a>() -> Element<'a, Message> {
    let widths = [220.0, 160.0, 260.0, 140.0, 200.0, 120.0];
    let rows = widths.into_iter().map(|w| {
        row![
            space().width(40),
            container(space())
                .width(w)
                .height(8)
                .style(theme::placeholder_bar)
        ]
        .height(ROW_H)
        .align_y(Center)
        .into()
    });
    Column::with_children(rows).into()
}

/// One row, exactly ROW_H tall.
fn row_view<'a>(
    results: &'a Results,
    r: &Row,
    layout: Layout,
    now: Instant,
) -> Element<'a, Message> {
    let tree = &results.tree;
    let cells = match (r.key, results.view.unaccounted) {
        (RowKey::Unaccounted, Some(bytes)) => unaccounted_cells(bytes),
        _ => cells_for(
            tree,
            r.key,
            results.view.zoom,
            &results.view.open_aggregates,
            now,
            results.started_at,
        ),
    };
    let indent = 12.0 + f32::from(r.depth) * INDENT;

    let disclosure: Element<'a, Message> = match cells.disclosure {
        Some(open) => mouse_area(
            container(
                theme::BODY
                    .text(if open { "▾" } else { "▸" })
                    .style(theme::text_secondary),
            )
            .width(16)
            .center_x(16),
        )
        .on_press(Message::ToggleExpand(r.key))
        .interaction(mouse::Interaction::Pointer)
        .into(),
        None => space().width(16).into(),
    };
    let mut name_cell = row![
        space().width(indent),
        disclosure,
        space().width(4),
        container(theme::BODY.text(cells.icon).style(theme::text_secondary))
            .width(16)
            .center_x(16),
        space().width(6),
    ]
    .align_y(Center);
    let name_max = 520.0 - indent;
    let shown_name = widgets::truncate_end(&cells.name, name_max, theme::BODY.size);
    let name: Element<'a, Message> = if cells.strike {
        // `on_link_click(never)` fixes the otherwise uninferable link type.
        iced::widget::rich_text([iced::widget::span(shown_name).strikethrough(true)])
            .size(theme::BODY.size)
            .font(fonts::INTER)
            .wrapping(text::Wrapping::None)
            .style(theme::text_muted)
            .on_link_click(iced::never)
            .into()
    } else {
        theme::BODY
            .text(shown_name)
            .wrapping(text::Wrapping::None)
            .style(move |t| tone_style(t, cells.name_tone))
            .into()
    };
    let name: Element<'a, Message> = match &cells.name_tip {
        Some(tip) => tooltip(name, tip_box(tip.clone()), tooltip::Position::Bottom)
            .delay(TOOLTIP_DELAY)
            .into(),
        None => name,
    };
    name_cell = name_cell.push(container(name).clip(true));
    if let Some((section, label)) = pill(results, r.key) {
        name_cell = name_cell
            .push(space().width(6))
            .push(pill_view(section, label));
    }
    if let Some(badge) = cells.badge {
        name_cell = name_cell.push(space().width(6)).push(badge_view(badge));
    }

    let size_text = theme::BODY
        .text(cells.size)
        .style(move |t| tone_style(t, cells.size_tone));
    let marker = match cells.marker {
        Marker::None => Element::from(space().width(10)),
        Marker::Waiting => theme::SMALL
            .text("○")
            .style(theme::text_muted)
            .width(10)
            .into(),
        Marker::Scanning(alpha) => theme::SMALL
            .text("•")
            .style(move |t| theme::text_accent_alpha(t, alpha))
            .width(10)
            .into(),
    };
    let size_cell = row![space().width(Fill), marker, space().width(4), size_text]
        .align_y(Center)
        .width(SIZE_W);
    let size_cell: Element<'a, Message> = match cells.size_tip {
        Some(tip) => tooltip(size_cell, tip_box(tip), tooltip::Position::Bottom)
            .delay(TOOLTIP_DELAY)
            .into(),
        None => size_cell.into(),
    };

    let mut line = row![
        container(name_cell).width(Fill).clip(true),
        size_cell,
        space().width(12)
    ]
    .align_y(Center)
    .height(ROW_H);
    match cells.share {
        Some((share, ref percent_tip)) => {
            let bar_value = if share > 0.0 {
                share.max(2.0 / BAR_W)
            } else {
                0.0
            };
            let bar = progress_bar(0.0..=1.0, bar_value)
                .length(BAR_W)
                .girth(6)
                .style(theme::share_bar);
            let mut share_cell = row![bar].align_y(Center);
            if layout.percent {
                share_cell = share_cell.push(space().width(6)).push(
                    theme::SMALL
                        .text(cells.percent.clone())
                        .style(theme::text_secondary)
                        .width(PERCENT_W)
                        .align_x(Right),
                );
            }
            line = line.push(
                tooltip(
                    share_cell,
                    tip_box(percent_tip.clone()),
                    tooltip::Position::Bottom,
                )
                .delay(TOOLTIP_DELAY),
            );
        }
        None => {
            line = line.push(space().width(if layout.percent {
                BAR_W + 6.0 + PERCENT_W
            } else {
                BAR_W
            }))
        }
    }
    if layout.items {
        line = line.push(space().width(12)).push(
            theme::SMALL
                .text(cells.items)
                .style(theme::text_secondary)
                .width(ITEMS_W)
                .align_x(Right),
        );
    }
    let line = line.push(space().width(16));

    let selected = results.view.selection == Some(r.key);
    let focused = results.focus == super::Pane::Tree;
    // Revealed from the cleanup pane: two flashes over 600 ms.
    let flashing = results.view.flash.is_some_and(|(key, at)| {
        let ms = now.saturating_duration_since(at).as_millis();
        key == r.key && ms < super::FLASH.as_millis() && (ms / 150).is_multiple_of(2)
    });
    let base = container(line)
        .width(Fill)
        .height(ROW_H)
        .style(move |t| theme::tree_row_state(t, selected, focused, flashing));
    let highlight = container(space())
        .width(Fill)
        .height(Fill)
        .style(theme::row_hover);
    mouse_area(hover(base, highlight))
        .on_press(Message::RowPressed(r.key))
        .on_double_click(Message::RowActivated(r.key))
        .into()
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tone {
    Primary,
    Secondary,
    Muted,
    /// A clickable row ("Show 50 more").
    Accent,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Marker {
    None,
    /// ○ not read yet.
    Waiting,
    /// • pulsing, with the current opacity.
    Scanning(f32),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Badge {
    /// Filled warning badge: this folder couldn't be read.
    Unreadable(ErrorClass),
    /// Outline badge: this many folders inside couldn't be read.
    UnreadableInside(u32),
    Tag(&'static str),
}

/// Everything a row shows, computed from the tree. Pure, so the per-state rules are tested.
#[derive(Debug, PartialEq)]
pub struct Cells {
    pub disclosure: Option<bool>,
    pub icon: &'static str,
    pub name: String,
    pub name_tone: Tone,
    pub name_tip: Option<String>,
    pub badge: Option<Badge>,
    pub marker: Marker,
    pub size: String,
    pub size_tone: Tone,
    pub size_tip: Option<String>,
    /// Share of the parent and its tooltip.
    pub share: Option<(f32, String)>,
    pub percent: String,
    pub items: String,
    /// In the Trash: struck through until the next re-order.
    pub strike: bool,
}

/// The cells of a row, including its trash state.
pub fn cells_for(
    tree: &Tree,
    key: RowKey,
    zoom: NodeId,
    open_aggregates: &std::collections::HashMap<NodeId, OpenAggregate>,
    now: Instant,
    started: Instant,
) -> Cells {
    let mut cells = base_cells(tree, key, zoom, open_aggregates, now, started);
    let (trashed, pending, outdated) =
        match key {
            RowKey::Dir(id) => {
                let f = tree.node(id).flags();
                (
                    f.contains(NodeFlags::TRASHED),
                    f.contains(NodeFlags::TRASH_PENDING),
                    f.contains(NodeFlags::OUTDATED),
                )
            }
            RowKey::File(dir, index) => tree.node(dir).large_files().get(index as usize).map_or(
                (false, false, false),
                |file| {
                    (
                        file.flags.contains(FileFlags::TRASHED),
                        file.flags.contains(FileFlags::TRASH_PENDING),
                        false,
                    )
                },
            ),
            RowKey::SmallFile(dir, index) => match open_aggregates.get(&dir).map(|o| &o.small) {
                Some(SmallFiles::Loaded(files)) => {
                    files
                        .get(index as usize)
                        .map_or((false, false, false), |file| {
                            (
                                file.flags.contains(FileFlags::TRASHED),
                                file.flags.contains(FileFlags::TRASH_PENDING),
                                false,
                            )
                        })
                }
                _ => (false, false, false),
            },
            _ => (false, false, false),
        };
    if trashed {
        cells.strike = true;
        cells.name_tone = Tone::Muted;
        cells.size = "In Trash".into();
        cells.size_tone = Tone::Muted;
        cells.size_tip = None;
        cells.share = None;
        cells.percent.clear();
        cells.items.clear();
        cells.disclosure = None;
        cells.marker = Marker::None;
    } else if pending {
        cells.size = "Moving to Trash…".into();
        cells.size_tone = Tone::Muted;
        cells.marker = Marker::Scanning(0.8);
    } else if outdated {
        cells.size_tip = Some("Rescan to update".into());
    }
    // A rescan in progress: the old numbers stay, in secondary color, the target pulsing
    // (UX §6.6).
    if !trashed && !pending {
        let dir = key.dir();
        if let Some(target) = std::iter::once(dir)
            .chain(tree.ancestors(dir))
            .find(|&a| tree.node(a).has_flag(NodeFlags::RESCANNING))
        {
            cells.size_tone = Tone::Secondary;
            if key == RowKey::Dir(target) {
                let phase = (now.saturating_duration_since(started).as_secs_f32() / 1.2).fract();
                cells.marker = Marker::Scanning(0.35 + 0.65 * (1.0 - (phase * 2.0 - 1.0).abs()));
                cells.size_tip = Some("Rescanning…".into());
            }
        }
    }
    cells
}

fn base_cells(
    tree: &Tree,
    key: RowKey,
    zoom: NodeId,
    open_aggregates: &std::collections::HashMap<NodeId, OpenAggregate>,
    now: Instant,
    started: Instant,
) -> Cells {
    let pulse = {
        let phase = (now.saturating_duration_since(started).as_secs_f32() / 1.2).fract();
        0.35 + 0.65 * (1.0 - (phase * 2.0 - 1.0).abs())
    };
    let mut cells = Cells {
        disclosure: None,
        icon: "",
        name: String::new(),
        name_tone: Tone::Primary,
        name_tip: None,
        badge: None,
        marker: Marker::None,
        size: String::new(),
        size_tone: Tone::Primary,
        size_tip: None,
        share: None,
        percent: String::new(),
        items: String::new(),
        strike: false,
    };
    let share = |part: u64, parent: NodeId| {
        let whole = tree.node(parent).total().alloc;
        let root = tree.node(zoom).total().alloc;
        let tip = format!(
            "{} of {} · {} of {}",
            format::percent(part, whole),
            display_name(tree, parent),
            format::percent(part, root),
            display_name(tree, zoom)
        );
        let fraction = if whole == 0 {
            0.0
        } else {
            part as f32 / whole as f32
        };
        (fraction.min(1.0), tip, format::percent(part, whole))
    };
    let bytes_tip = |alloc: u64, apparent: u64| {
        let mut tip = format!("{} on disk", format::bytes_exact(alloc));
        if apparent != alloc {
            tip += &format!(" ({} apparent)", format::size(apparent));
        }
        tip
    };

    match key {
        RowKey::Dir(id) => {
            let node = tree.node(id);
            let total = node.total();
            cells.icon = "▭";
            cells.name = node.name().to_string_lossy().into_owned();
            cells.name_tip = Some(tree.path_of(id).display().to_string());
            if rows::expandable(tree, id) {
                cells.disclosure = Some(node.has_flag(NodeFlags::EXPANDED));
            }
            match node.state() {
                NodeState::Pending => {
                    cells.marker = if tree.is_stopped() {
                        Marker::None
                    } else {
                        Marker::Waiting
                    };
                    cells.size = format::PENDING.into();
                    cells.size_tone = Tone::Muted;
                    cells.items = format::PENDING.into();
                    if tree.is_stopped() {
                        cells.size_tip =
                            Some("Scan was stopped before this folder was measured.".into());
                    }
                }
                NodeState::Skipped(skip) => {
                    cells.size = format::PENDING.into();
                    cells.size_tone = Tone::Muted;
                    cells.name_tone = Tone::Secondary;
                    cells.badge = Some(Badge::Tag(match skip {
                        Skip::MountPoint | Skip::OtherFilesystem => "other volume",
                        Skip::CloudOnly => "Cloud folder – not scanned",
                        Skip::Excluded if node.name() == "Backups.backupdb" => {
                            "Time Machine backups – not scanned"
                        }
                        Skip::Excluded => "System folder – not scanned",
                        Skip::LinkedDir => "counted elsewhere",
                    }));
                }
                NodeState::Scanning | NodeState::Done | NodeState::Removed => {
                    let lower = tree.is_lower_bound(id);
                    if node.error().is_some() && node.child_count() == 0 && node.files().count == 0
                    {
                        cells.size = format::PENDING.into();
                        cells.size_tone = Tone::Muted;
                    } else {
                        cells.size = if lower {
                            format::size_at_least(total.alloc)
                        } else {
                            format::size(total.alloc)
                        };
                        cells.size_tone = if !node.is_done() || total.alloc < 1_000_000 {
                            Tone::Secondary
                        } else {
                            Tone::Primary
                        };
                        cells.size_tip = Some(if lower {
                            "Scan was stopped before this folder was fully measured.".into()
                        } else {
                            bytes_tip(total.alloc, total.apparent)
                        });
                        cells.items = format::count(total.items);
                    }
                    if node.state() == NodeState::Scanning && !tree.is_stopped() {
                        cells.marker = Marker::Scanning(pulse);
                    }
                    if let Some(error) = node.error() {
                        cells.badge = Some(Badge::Unreadable(error.class));
                        cells.name_tone = Tone::Secondary;
                    } else if total.error_dirs > 0 {
                        cells.badge = Some(Badge::UnreadableInside(total.error_dirs));
                    } else if cfg!(target_os = "macos") && super::package::is_package(tree, id) {
                        cells.badge = Some(Badge::Tag("package"));
                    }
                    if node.has_flag(NodeFlags::VANISHED) {
                        cells.name_tone = Tone::Muted;
                        cells.name_tip = Some("Removed during the scan".into());
                    }
                }
            }
            let parent = node.parent().unwrap_or(zoom);
            if !matches!(node.state(), NodeState::Pending | NodeState::Skipped(_)) {
                let (fraction, tip, percent) = share(total.alloc, parent);
                cells.share = Some((fraction, tip));
                cells.percent = percent;
            }
        }
        RowKey::File(dir, index) => {
            let Some(file) = tree.node(dir).large_files().get(index as usize) else {
                return cells;
            };
            cells.name = file.name.to_string_lossy().into_owned();
            cells.name_tip = Some(tree.path_of(dir).join(&*file.name).display().to_string());
            cells.size = format::size(file.alloc);
            cells.size_tone = if file.alloc < 1_000_000 {
                Tone::Secondary
            } else {
                Tone::Primary
            };
            cells.size_tip = Some(bytes_tip(file.alloc, file.apparent));
            let (fraction, tip, percent) = share(file.alloc, dir);
            cells.share = Some((fraction, tip));
            cells.percent = percent;
        }
        RowKey::Aggregate(dir) => {
            let node = tree.node(dir);
            let split = split_files(node);
            cells.icon = "≡";
            cells.disclosure = Some(open_aggregates.contains_key(&dir));
            let noun = if split.shown.is_empty() {
                "file"
            } else {
                "other file"
            };
            cells.name = format::plural(split.aggregate_count, noun, &format!("{noun}s"));
            cells.name_tone = Tone::Secondary;
            cells.size = format::size(split.aggregate_alloc);
            cells.size_tone = Tone::Secondary;
            cells.size_tip = Some(format!(
                "{} on disk",
                format::bytes_exact(split.aggregate_alloc)
            ));
            cells.items = format::count(split.aggregate_count);
            let (fraction, tip, percent) = share(split.aggregate_alloc, dir);
            cells.share = Some((fraction, tip));
            cells.percent = percent;
        }
        RowKey::SmallFile(dir, index) => {
            let Some(SmallFiles::Loaded(files)) = open_aggregates.get(&dir).map(|o| &o.small)
            else {
                return cells;
            };
            let Some(file) = files.get(index as usize) else {
                return cells;
            };
            cells.name = file.name.to_string_lossy().into_owned();
            cells.name_tip = Some(tree.path_of(dir).join(&*file.name).display().to_string());
            cells.size = format::size(file.alloc);
            cells.size_tone = Tone::Secondary;
            cells.size_tip = Some(bytes_tip(file.alloc, file.apparent));
            let (fraction, tip, percent) = share(file.alloc, dir);
            cells.share = Some((fraction, tip));
            cells.percent = percent;
        }
        RowKey::MoreFiles(dir) => {
            cells.name_tone = Tone::Muted;
            let Some(open) = open_aggregates.get(&dir) else {
                return cells;
            };
            let all = rows::aggregate_entries(
                tree,
                dir,
                &OpenAggregate {
                    shown: usize::MAX,
                    ..open.clone()
                },
            )
            .0
            .len();
            let split = split_files(tree.node(dir));
            cells.name = if all > open.shown {
                cells.name_tone = Tone::Accent;
                format!("Show {} more", (all - open.shown).min(rows::FILE_BATCH))
            } else {
                match &open.small {
                    SmallFiles::Loading => "Listing files…".into(),
                    SmallFiles::Failed(error) => format!("Couldn't list the files: {error}"),
                    SmallFiles::Loaded(_) => {
                        let rest = split
                            .aggregate_count
                            .saturating_sub(split.kept_rest.len() as u64);
                        format!("and {} smaller", format::plural(rest, "file", "files"))
                    }
                }
            };
        }
        RowKey::Placeholder(dir) => {
            let pending = tree.node(dir).state() == NodeState::Pending;
            cells.name = match (pending, tree.is_stopped()) {
                (true, false) => "Waiting to be scanned…".into(),
                (true, true) => "Not scanned – the scan was stopped.".into(),
                (false, _) => "This folder is empty.".into(),
            };
            cells.name_tone = Tone::Muted;
        }
        // Filled in by `unaccounted_cells`; the tree knows nothing about it.
        RowKey::Unaccounted => {}
    }
    cells
}

/// Tooltip and selection-bar text of the Unaccounted row (UX §5.6).
pub const UNACCOUNTED_TIP: &str = "Used space the scan couldn't see: APFS snapshots, purgeable data, system volumes (VM, Preboot, Recovery), other users' files, and folders dir-stat isn't allowed to read.";

/// The Unaccounted row: not part of any share, so the children still add up to 100%.
pub fn unaccounted_cells(bytes: u64) -> Cells {
    Cells {
        disclosure: None,
        icon: "⋯",
        name: "Unaccounted space".into(),
        name_tone: Tone::Secondary,
        name_tip: Some(UNACCOUNTED_TIP.into()),
        badge: None,
        marker: Marker::None,
        size: format::size(bytes),
        size_tone: Tone::Secondary,
        size_tip: Some(UNACCOUNTED_TIP.into()),
        share: None,
        percent: "—".into(),
        items: String::new(),
        strike: false,
    }
}

/// The cleanup pill of a candidate row, from `display_section` (no pill while git is pending
/// or for tracked folders).
fn pill(results: &Results, key: RowKey) -> Option<(dirstat_core::catalog::Section, &'static str)> {
    use dirstat_core::catalog::{Display, Verdict, display_section};
    use dirstat_core::scan::FileMatch;
    use dirstat_core::tree::CandidateRef;
    let engine = results.engine.as_ref()?;
    let tree = &results.tree;
    let (rule, candidate) = match key {
        RowKey::Dir(id) => match tree.node(id).verdict() {
            Verdict::Target { rule, .. } => (rule, CandidateRef::Dir(id)),
            _ => return None,
        },
        RowKey::File(dir, index) => match tree.node(dir).large_files().get(index as usize)?.matched
        {
            Some(FileMatch::Target(rule)) => (rule, CandidateRef::File(dir, index)),
            _ => return None,
        },
        _ => return None,
    };
    let rule = engine.rule(rule);
    match display_section(rule.risk()?, results.candidates.state(candidate)) {
        Display::Shown { section, .. } => Some((section, rule.category.pill())),
        _ => None,
    }
}

fn pill_view<'a>(
    section: dirstat_core::catalog::Section,
    label: &'static str,
) -> Element<'a, Message> {
    let safe = section == dirstat_core::catalog::Section::SafeToDelete;
    container(
        row![
            container(space().width(6).height(6)).style(move |t| theme::status_dot(t, safe)),
            theme::CAPTION.text(label).style(theme::text_secondary),
        ]
        .spacing(5)
        .align_y(Center),
    )
    .padding([1, 8])
    .style(move |t| theme::pill(t, safe))
    .into()
}

fn tone_style(theme: &iced::Theme, tone: Tone) -> iced::widget::text::Style {
    match tone {
        Tone::Primary => theme::text_primary(theme),
        Tone::Secondary => theme::text_secondary(theme),
        Tone::Muted => theme::text_muted(theme),
        Tone::Accent => theme::text_accent_link(theme),
    }
}

fn display_name(tree: &Tree, id: NodeId) -> String {
    tree.node(id).name().to_string_lossy().into_owned()
}

fn badge_view<'a>(badge: Badge) -> Element<'a, Message> {
    match badge {
        Badge::Unreadable(class) => tooltip(
            container(theme::CAPTION.text("!").style(theme::text_on_warning))
                .padding([0, 5])
                .style(theme::warning_badge),
            tip_box(errors::reason(class)),
            tooltip::Position::Bottom,
        )
        .delay(TOOLTIP_DELAY)
        .into(),
        Badge::UnreadableInside(count) => tooltip(
            container(theme::CAPTION.text("!").style(theme::text_warning))
                .padding([0, 4])
                .style(theme::warning_outline),
            tip_box(errors::inside_tooltip(count)),
            tooltip::Position::Bottom,
        )
        .delay(TOOLTIP_DELAY)
        .into(),
        Badge::Tag(label) => theme::CAPTION
            .with_font(fonts::INTER)
            .text(label)
            .style(theme::text_muted)
            .into(),
    }
}

fn tip_box<'a>(content: impl Into<String>) -> Element<'a, Message> {
    container(theme::SMALL.text(content.into()))
        .padding([4, 8])
        .max_width(360)
        .style(theme::raised)
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use dirstat_core::tree::testing::{TreeBuilder, dir, large};
    use std::collections::HashMap;

    fn cells(tree: &Tree, key: RowKey) -> Cells {
        let now = Instant::now();
        cells_for(tree, key, NodeId::ROOT, &HashMap::new(), now, now)
    }

    #[test]
    fn rescanning_rows_keep_old_numbers_in_secondary() {
        let mut tree = TreeBuilder::new(
            "/r",
            dir("r").child(dir("a").small_files(1, 5_000_000).child(dir("in"))),
        )
        .build();
        let a = tree.find_path(std::path::Path::new("/r/a")).unwrap();
        let inner = tree.find_path(std::path::Path::new("/r/a/in")).unwrap();
        assert_eq!(cells(&tree, RowKey::Dir(a)).size_tone, Tone::Primary);
        tree.set_flag(a, NodeFlags::RESCANNING, true);
        let target = cells(&tree, RowKey::Dir(a));
        assert_eq!(target.size_tone, Tone::Secondary);
        assert_eq!(target.size, "5.01 MB", "old numbers stay");
        assert!(matches!(target.marker, Marker::Scanning(_)));
        assert_eq!(cells(&tree, RowKey::Dir(inner)).size_tone, Tone::Secondary);
        tree.set_flag(a, NodeFlags::RESCANNING, false);
        tree.set_flag(NodeId::ROOT, NodeFlags::RESCANNING, true);
        assert_eq!(
            cells(&tree, RowKey::Dir(a)).size_tone,
            Tone::Secondary,
            "root rescan"
        );
    }

    #[test]
    fn per_row_scan_states() {
        let builder = TreeBuilder::new(
            "/r",
            dir("r")
                .dir_alloc(0)
                .child(dir("done").small_files(1, 5_000_000))
                .child(dir("pending"))
                .child(dir("mnt").skip(Skip::MountPoint))
                .child(dir("bad").error(ErrorClass::PermissionDenied)),
        );
        let mut tree = builder.started();
        for l in builder.listings().into_iter().filter(|l| l.id != NodeId(2)) {
            tree.apply(l);
        }
        let done = cells(&tree, RowKey::Dir(NodeId(1)));
        assert_eq!(
            (done.size.as_str(), done.size_tone, done.marker),
            ("5.00 MB", Tone::Primary, Marker::None)
        );
        assert_eq!(done.percent, "100%");
        let pending = cells(&tree, RowKey::Dir(NodeId(2)));
        assert_eq!(
            (
                pending.size.as_str(),
                pending.marker,
                pending.items.as_str()
            ),
            ("—", Marker::Waiting, "—")
        );
        assert_eq!(
            pending.disclosure,
            Some(false),
            "pending folders can be expanded"
        );
        let root = cells_for(
            &tree,
            RowKey::Dir(NodeId::ROOT),
            NodeId::ROOT,
            &HashMap::new(),
            Instant::now(),
            Instant::now(),
        );
        assert!(matches!(root.marker, Marker::Scanning(_)));
        assert_eq!(root.size_tone, Tone::Secondary, "a growing lower bound");
        let mnt = cells(&tree, RowKey::Dir(NodeId(3)));
        assert_eq!(
            (mnt.size.as_str(), mnt.badge, mnt.disclosure),
            ("—", Some(Badge::Tag("other volume")), None)
        );
        let bad = cells(&tree, RowKey::Dir(NodeId(4)));
        assert_eq!(
            bad.badge,
            Some(Badge::Unreadable(ErrorClass::PermissionDenied))
        );
        assert_eq!((bad.size.as_str(), bad.disclosure), ("—", None));

        tree.mark_stopped();
        let pending = cells(&tree, RowKey::Dir(NodeId(2)));
        assert_eq!(pending.marker, Marker::None);
        let root = cells(&tree, RowKey::Dir(NodeId::ROOT));
        assert!(root.size.starts_with('≥'));
        assert_eq!(
            cells(&tree, RowKey::Placeholder(NodeId(2))).name,
            "Not scanned – the scan was stopped."
        );
    }

    /// 160k rows expanded: building the rows is linear, the view only touches visible ones.
    #[test]
    fn view_cost_is_independent_of_the_row_count() {
        use crate::results::{
            Results,
            scan_task::{ActiveScan, ScanId},
        };
        let mut root = dir("r");
        for i in 0..400 {
            let mut d = dir(format!("d{i:03}"));
            for j in 0..400 {
                d = d.child(dir(format!("c{j:03}")).small_files(1, (i * 400 + j) as u64));
            }
            root = root.child(d);
        }
        let tree = TreeBuilder::new("/r", root).build();
        let mut results = Results::new("/r".into(), "r".into(), None, ActiveScan::inert(ScanId(1)));
        results.tree = tree;
        let now = Instant::now();
        let ids: Vec<NodeId> = results.tree.children(NodeId::ROOT).collect();
        let expand = Instant::now();
        for id in ids {
            results.toggle(RowKey::Dir(id), now);
        }
        results.view.orders.insert(
            NodeId::ROOT,
            crate::results::order::exact(&results.tree, NodeId::ROOT, Default::default()),
        );
        results.view.rebuild(&results.tree, now);
        let built = expand.elapsed();
        assert_eq!(results.view.rows.len(), 400 + 400 * 400);

        results.view.viewport.offset_y = 80_000.0 * ROW_H;
        let mut best = Duration::MAX;
        for _ in 0..20 {
            let t = Instant::now();
            let element = pane_content(&results, iced::Size::new(1200.0, 700.0), now);
            best = best.min(t.elapsed());
            drop(element);
        }
        eprintln!("rows built in {built:?}; view {best:?}");
        // Debug builds are slower; the 2 ms target is checked with `cargo test --release`.
        let limit = if cfg!(debug_assertions) {
            Duration::from_millis(20)
        } else {
            Duration::from_millis(2)
        };
        assert!(best < limit, "view took {best:?}");
    }

    #[test]
    fn unreadable_folders_and_their_parents() {
        let tree = TreeBuilder::new(
            "/r",
            dir("r").child(dir("outer").child(dir("locked").error(ErrorClass::PermissionDenied))),
        )
        .build();
        let locked = cells(&tree, RowKey::Dir(NodeId(2)));
        assert_eq!(
            locked.badge,
            Some(Badge::Unreadable(ErrorClass::PermissionDenied)),
            "filled badge"
        );
        assert_eq!(locked.name_tone, Tone::Secondary);
        let outer = cells(&tree, RowKey::Dir(NodeId(1)));
        assert_eq!(
            outer.badge,
            Some(Badge::UnreadableInside(1)),
            "outline badge with the count"
        );
        let tags = TreeBuilder::new(
            "/r",
            dir("r")
                .child(dir("Backups.backupdb").skip(Skip::Excluded))
                .child(dir("icloud").skip(Skip::CloudOnly))
                .child(dir(".vol").skip(Skip::Excluded)),
        )
        .build();
        assert_eq!(
            cells(&tags, RowKey::Dir(NodeId(1))).badge,
            Some(Badge::Tag("Time Machine backups – not scanned"))
        );
        assert_eq!(
            cells(&tags, RowKey::Dir(NodeId(2))).badge,
            Some(Badge::Tag("Cloud folder – not scanned"))
        );
        assert_eq!(
            cells(&tags, RowKey::Dir(NodeId(3))).badge,
            Some(Badge::Tag("System folder – not scanned"))
        );
    }

    #[test]
    fn files_aggregate_and_tooltips() {
        let tree = TreeBuilder::new(
            "/r",
            dir("r")
                .dir_alloc(0)
                .large(large("disk.img", 300_000_000))
                .small_files(1_234, 2_000_000),
        )
        .build();
        let file = cells(&tree, RowKey::File(NodeId::ROOT, 0));
        assert_eq!(file.size, "300 MB");
        assert_eq!(file.size_tip.as_deref(), Some("300,000,000 bytes on disk"));
        let aggregate = cells(&tree, RowKey::Aggregate(NodeId::ROOT));
        assert_eq!(aggregate.name, "1,234 other files");
        assert_eq!(aggregate.items, "1,234");
        assert_eq!(aggregate.share.unwrap().1, "0.7% of r · 0.7% of r");
    }
}
