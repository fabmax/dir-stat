//! The selection bar below the tree (UX §6.6): what is selected, and its actions.

use std::path::Path;
use std::time::Duration;

use dirstat_core::scan::{ErrorClass, Skip};
use dirstat_core::tree::{FileFlags, NodeFlags, NodeState, Tree};
use iced::widget::{Row, button, container, row, space, tooltip};
use iced::{Center, Element, Fill};

use super::Results;
use super::order::split_files;
use super::rows::{RowKey, SmallFiles};
use crate::app::Message;
use crate::{fonts, format, theme, widgets};

/// Below this window width, buttons use short labels with tooltips (D16).
const NARROW: f32 = 1000.0;

pub const REVEAL_LABEL: &str = if cfg!(target_os = "macos") {
    "Reveal in Finder"
} else if cfg!(windows) {
    "Show in Explorer"
} else {
    "Show in File Manager"
};

#[derive(Debug, PartialEq)]
pub enum Action {
    Reveal,
    CopyPath,
    /// `Err` holds why it is disabled.
    ScanVolume(Result<(), &'static str>),
    Rescan(Result<(), &'static str>),
    PrivacySettings,
    /// Move to Trash…; `Err` holds why it is disabled.
    Trash(Result<(), &'static str>),
}

/// How an action reads and what it sends; the bar and the tree's context menu share it.
pub struct Labels {
    pub full: &'static str,
    /// For narrow windows (D16).
    pub short: &'static str,
    /// Acts on the selection.
    pub message: Message,
    /// Why it is unavailable.
    pub disabled: Option<&'static str>,
}

pub const TRASH_LABEL: &str = if cfg!(windows) {
    "Move to Recycle Bin…"
} else {
    "Move to Trash…"
};

impl Action {
    pub fn labels(&self) -> Labels {
        let disabled = match self {
            Action::Trash(Err(why)) | Action::Rescan(Err(why)) | Action::ScanVolume(Err(why)) => {
                Some(*why)
            }
            _ => None,
        };
        let (full, short, message) = match self {
            Action::Trash(_) => (TRASH_LABEL, "Trash…", Message::RequestTrash),
            Action::Reveal => (REVEAL_LABEL, "Reveal", Message::RevealSelection),
            Action::CopyPath => ("Copy Path", "Copy", Message::CopySelection),
            Action::ScanVolume(_) => ("Scan This Volume", "Scan", Message::ScanSelection),
            Action::Rescan(_) => ("Rescan", "Rescan", Message::Rescan),
            Action::PrivacySettings => (
                "Open Privacy Settings",
                "Privacy",
                Message::OpenPrivacySettings,
            ),
        };
        Labels {
            full,
            short,
            message,
            disabled,
        }
    }
}

/// Whether the trash button works for a folder (G1, UX §5.5).
fn folder_trash(results: &Results, id: dirstat_core::tree::NodeId) -> Result<(), &'static str> {
    let tree = &results.tree;
    let node = tree.node(id);
    if results.rescanning_within(id) {
        return Err("Available when the rescan finishes.");
    }
    let flags = |a: dirstat_core::tree::NodeId| tree.node(a).flags();
    let busy = std::iter::once(id).chain(tree.ancestors(id)).any(|a| {
        flags(a).contains(NodeFlags::TRASHED)
            || flags(a).contains(NodeFlags::TRASH_PENDING)
            || flags(a).contains(NodeFlags::RESCANNING)
    });
    if busy {
        Err("Already in the Trash or being moved.")
    } else if !node.is_done() {
        Err("Available when this folder is fully scanned.")
    } else if node.total().boundaries > 0 {
        Err("This folder contains another volume.")
    } else {
        Ok(())
    }
}

fn rescan(results: &Results, id: dirstat_core::tree::NodeId) -> Result<(), &'static str> {
    if results.tree.node(id).has_flag(NodeFlags::RESCANNING) {
        return Err("Rescanning…");
    }
    results.rescan_refusal(id).map_or(Ok(()), Err)
}

/// What the bar shows for one selection. Pure, so the per-type rules are tested.
#[derive(Debug, PartialEq)]
pub struct Contents {
    pub warning: bool,
    pub path: String,
    pub details: String,
    pub actions: Vec<Action>,
}

/// `~/…` for paths in the home folder; display only.
pub fn tilde(path: &Path, home: Option<&Path>) -> String {
    match home.and_then(|h| path.strip_prefix(h).ok()) {
        Some(rest) if rest.as_os_str().is_empty() => "~".into(),
        Some(rest) => format!("~/{}", rest.display()),
        None => path.display().to_string(),
    }
}

fn modified(mtime: i64, now: i64) -> Option<String> {
    (mtime != i64::MIN).then(|| {
        format!(
            "modified {}",
            format::relative(Duration::from_secs(now.saturating_sub(mtime).max(0) as u64))
        )
    })
}

pub fn contents(results: &Results, key: RowKey, home: Option<&Path>, now: i64) -> Option<Contents> {
    let tree: &Tree = &results.tree;
    if key == RowKey::Unaccounted {
        let privacy = tree
            .errors()
            .iter()
            .any(|e| e.error.class == ErrorClass::PrivacyBlocked);
        return Some(Contents {
            warning: false,
            path: "Unaccounted space".into(),
            details: super::columns::UNACCOUNTED_TIP.into(),
            actions: if privacy && results.offer_privacy_settings() {
                vec![Action::PrivacySettings]
            } else {
                Vec::new()
            },
        });
    }
    let node = tree.get(key.dir())?;
    let path = tilde(&results.path_for(key)?, home);
    let mut details = Vec::new();
    let mut actions = vec![Action::Reveal, Action::CopyPath];
    let mut warning = false;
    match key {
        RowKey::Dir(id) => match node.state() {
            NodeState::Skipped(Skip::MountPoint | Skip::OtherFilesystem) => {
                details.push("Separate volume, not included".into());
                // D2: a scan of / already covers the Data volume through its firmlinks.
                let data = cfg!(target_os = "macos")
                    && tree.path_of(id) == Path::new("/System/Volumes/Data");
                actions = vec![
                    Action::ScanVolume(if data {
                        Err("Scanning / already covers this volume")
                    } else {
                        Ok(())
                    }),
                    Action::Reveal,
                ];
            }
            NodeState::Skipped(_) => details.push("Not scanned".into()),
            NodeState::Pending => {
                details.push(if tree.is_stopped() {
                    "Not scanned – the scan was stopped".into()
                } else {
                    "Waiting to be scanned".into()
                });
                if tree.is_stopped() {
                    actions.push(Action::Rescan(rescan(results, id)));
                }
            }
            _ if node.error().is_some() => {
                let class = node.error().map(|e| e.class).unwrap_or(ErrorClass::Other);
                warning = true;
                details.push(format!("Couldn't read: {}", super::errors::reason(class)));
                if class == ErrorClass::PrivacyBlocked && results.offer_privacy_settings() {
                    actions.push(Action::PrivacySettings);
                }
                actions.push(Action::Rescan(rescan(results, id)));
            }
            _ => {
                let total = node.total();
                let size = if tree.is_lower_bound(id) {
                    format::size_at_least(total.alloc)
                } else {
                    format::size(total.alloc)
                };
                details.push(size);
                details.push(format::plural(total.items, "item", "items"));
                details.extend(modified(node.newest_mtime(), now));
                actions.push(Action::Rescan(rescan(results, id)));
                actions.push(Action::Trash(folder_trash(results, id)));
            }
        },
        RowKey::File(dir, index) => {
            let file = tree.node(dir).large_files().get(index as usize)?;
            details.push(format::size(file.alloc));
            details.extend(modified(file.mtime, now));
            let gone = file.flags.contains(FileFlags::TRASHED)
                || file.flags.contains(FileFlags::TRASH_PENDING);
            actions.push(Action::Trash(if gone {
                Err("Already in the Trash or being moved.")
            } else {
                Ok(())
            }));
        }
        RowKey::SmallFile(dir, index) => {
            let SmallFiles::Loaded(files) = &results.view.open_aggregates.get(&dir)?.small else {
                return None;
            };
            let file = files.get(index as usize)?;
            details.push(format::size(file.alloc));
            details.extend(modified(file.mtime, now));
            let gone = file.flags.contains(FileFlags::TRASHED)
                || file.flags.contains(FileFlags::TRASH_PENDING);
            actions.push(Action::Trash(if gone {
                Err("Already in the Trash or being moved.")
            } else {
                Ok(())
            }));
        }
        RowKey::Unaccounted => return None,
        RowKey::Aggregate(dir) | RowKey::MoreFiles(dir) | RowKey::Placeholder(dir) => {
            let split = split_files(tree.node(dir));
            return Some(Contents {
                warning: false,
                path: format!(
                    "{} directly in {path}",
                    format::plural(split.aggregate_count, "file", "files")
                ),
                details: format::size(split.aggregate_alloc),
                actions,
            });
        }
    }
    Some(Contents {
        warning,
        path,
        details: details.join(" · "),
        actions,
    })
}

pub fn view<'a>(
    results: &'a Results,
    width: f32,
    home: Option<&Path>,
    now: i64,
) -> Element<'a, Message> {
    let bar = match results
        .view
        .selection
        .and_then(|key| contents(results, key, home, now))
    {
        None => hints(),
        Some(c) => {
            let narrow = width < NARROW;
            let path_room = (width * 0.45).max(160.0);
            let mut left = row![].spacing(8).align_y(Center);
            if c.warning {
                left = left.push(
                    container(theme::CAPTION.text("!").style(theme::text_on_warning))
                        .padding([0, 5])
                        .style(theme::warning_badge),
                );
            }
            left = left
                .push(
                    theme::BODY
                        .text(widgets::truncate_middle(
                            &c.path,
                            path_room,
                            theme::BODY.size,
                        ))
                        .wrapping(iced::widget::text::Wrapping::None),
                )
                .push(
                    theme::SMALL
                        .text(c.details)
                        .style(theme::text_secondary)
                        .wrapping(iced::widget::text::Wrapping::None),
                );
            let mut buttons: Row<'a, Message> = row![].spacing(8);
            for action in c.actions {
                let Labels {
                    full,
                    short,
                    message,
                    disabled,
                } = action.labels();
                let label = if narrow { short } else { full };
                let b = button(theme::SMALL.with_font(fonts::MEDIUM).text(label))
                    .padding([4, 10])
                    .style(theme::secondary_button)
                    .on_press_maybe(disabled.is_none().then_some(message));
                let tip_text = disabled.unwrap_or(full);
                buttons = buttons.push(if narrow || disabled.is_some() {
                    Element::from(tooltip(
                        b,
                        container(theme::SMALL.text(tip_text))
                            .padding([4, 8])
                            .style(theme::raised),
                        tooltip::Position::Top,
                    ))
                } else {
                    b.into()
                });
            }
            row![container(left).width(Fill).clip(true), buttons]
                .spacing(12)
                .align_y(Center)
                .into()
        }
    };
    container(bar)
        .padding([0, 16])
        .height(44)
        .center_y(44)
        .width(Fill)
        .style(theme::selection_bar)
        .into()
}

fn hints<'a>() -> Element<'a, Message> {
    let back = if cfg!(target_os = "macos") {
        "⌫ back"
    } else {
        "Backspace back"
    };
    let text = format!("Nothing selected   ·   ↑↓ select   → expand   ⏎ zoom in   {back}");
    row![
        theme::SMALL.text(text).style(theme::text_secondary),
        space().width(Fill)
    ]
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::results::scan_task::{ActiveScan, ScanId};
    use dirstat_core::tree::NodeId;
    use dirstat_core::tree::testing::{TreeBuilder, dir};

    fn results(builder: TreeBuilder) -> Results {
        let mut r = Results::new(
            "/Users/u/r".into(),
            "r".into(),
            None,
            ActiveScan::inert(ScanId(1)),
        );
        r.tree = builder.build();
        r.active = None;
        r.phase = crate::results::Phase::Complete;
        r
    }

    #[test]
    fn contents_per_row_type() {
        let home = Some(Path::new("/Users/u"));
        let r = results(TreeBuilder::new(
            "/Users/u/r",
            dir("r")
                .child(dir("docs").small_files(3, 3_000_000).mtime(1_000))
                .child(dir("mnt").skip(Skip::MountPoint))
                .child(dir("secret").error(ErrorClass::PrivacyBlocked)),
        ));
        let folder = contents(&r, RowKey::Dir(NodeId(1)), home, 1_000 + 2 * 86_400).unwrap();
        assert_eq!(folder.path, "~/r/docs");
        assert_eq!(folder.details, "3.00 MB · 3 items · modified 2 days ago");
        assert_eq!(
            folder.actions,
            [
                Action::Reveal,
                Action::CopyPath,
                Action::Rescan(Ok(())),
                Action::Trash(Ok(()))
            ]
        );

        let mount = contents(&r, RowKey::Dir(NodeId(2)), home, 0).unwrap();
        assert_eq!(mount.details, "Separate volume, not included");
        assert_eq!(mount.actions, [Action::ScanVolume(Ok(())), Action::Reveal]);

        let secret = contents(&r, RowKey::Dir(NodeId(3)), home, 0).unwrap();
        assert!(secret.warning);
        assert!(
            secret
                .details
                .starts_with("Couldn't read: Protected by macOS privacy settings")
        );
        assert_eq!(
            secret.actions.contains(&Action::PrivacySettings),
            cfg!(target_os = "macos")
        );

        let aggregate = contents(&r, RowKey::Aggregate(NodeId(1)), home, 0).unwrap();
        assert_eq!(
            (aggregate.path.as_str(), aggregate.details.as_str()),
            ("3 files directly in ~/r/docs", "3.00 MB")
        );
        assert_eq!(tilde(Path::new("/Users/u"), home), "~");
        assert_eq!(tilde(Path::new("/opt/x"), home), "/opt/x");
    }
}
