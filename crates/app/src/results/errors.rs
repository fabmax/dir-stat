//! Errors and permissions (UX §10, design §9): reasons by error class, the errors popover,
//! the Full Disk Access bar and the state for an unreadable scan root.

use std::path::Path;

use dirstat_core::platform;
use dirstat_core::scan::{ErrorClass, ScanError};
use dirstat_core::tree::{ErrorRecord, NodeId, Tree};
use iced::widget::{
    Column, button, center, column, container, mouse_area, row, rule, scrollable, space,
};
use iced::{Center, Element, Fill, mouse};

use super::Results;
use super::selection_bar::tilde;
use crate::app::Message;
use crate::{fonts, format, theme};

/// The popover lists this many records; "and N more" covers the rest.
pub const POPOVER_LIMIT: usize = 200;
const ROW: f32 = 26.0;
const REASON_W: f32 = 130.0;
const LIST_MAX_H: f32 = 7.5 * ROW;

/// Tooltip text for an unreadable folder (UX §10.1).
pub fn reason(class: ErrorClass) -> &'static str {
    match class {
        ErrorClass::PrivacyBlocked => {
            "Protected by macOS privacy settings. dir-stat needs Full Disk Access to read it."
        }
        ErrorClass::PermissionDenied => {
            "Permission denied. The folder belongs to another user or the system."
        }
        ErrorClass::CloudNotDownloaded => {
            "In iCloud and not downloaded; dir-stat doesn't download it."
        }
        ErrorClass::PathTooLong => "The path is too long to read.",
        ErrorClass::Internal => "dir-stat hit an internal error here.",
        ErrorClass::Other => "This folder couldn't be read.",
    }
}

/// The reason column of the popover and the copied list.
pub fn short_reason(class: ErrorClass) -> &'static str {
    match class {
        ErrorClass::PrivacyBlocked => "Protected by macOS",
        ErrorClass::PermissionDenied => "Permission denied",
        ErrorClass::CloudNotDownloaded => "Not downloaded",
        ErrorClass::PathTooLong => "Path too long",
        ErrorClass::Internal => "Internal error",
        ErrorClass::Other => "Couldn't be read",
    }
}

/// "3 folders inside couldn't be read, so the real size may be larger."
pub fn inside_tooltip(count: u32) -> String {
    let folders = format::plural(u64::from(count), "folder", "folders");
    format!("{folders} inside couldn't be read, so the real size may be larger.")
}

pub fn record_path(tree: &Tree, record: &ErrorRecord) -> std::path::PathBuf {
    let dir = tree.path_of(record.dir);
    match &record.entry {
        Some(name) => dir.join(&**name),
        None => dir,
    }
}

/// "23 folders couldn't be read" (or items, when files are among them).
pub fn headline(tree: &Tree) -> Option<String> {
    let records = tree.errors();
    if records.is_empty() {
        return None;
    }
    let all_dirs = records.iter().all(|r| r.entry.is_none());
    let noun = if all_dirs {
        ("folder", "folders")
    } else {
        ("item", "items")
    };
    Some(format!(
        "{} couldn't be read",
        format::plural(records.len() as u64, noun.0, noun.1)
    ))
}

/// What [Copy] puts on the clipboard: every recorded "path<TAB>reason" line.
pub fn tsv(tree: &Tree) -> String {
    tree.errors()
        .iter()
        .map(|r| {
            format!(
                "{}\t{}\n",
                record_path(tree, r).display(),
                short_reason(r.error.class)
            )
        })
        .collect()
}

/// The scan root's own listing failed and nothing could be read.
pub fn root_unreadable(tree: &Tree) -> Option<ScanError> {
    if !tree.is_started() {
        return None;
    }
    let root = tree.node(NodeId::ROOT);
    let empty = root.child_count() == 0 && root.files().count == 0;
    root.error().filter(|_| empty)
}

pub fn popover<'a>(results: &'a Results, home: Option<&Path>) -> Element<'a, Message> {
    let tree = &results.tree;
    let records = tree.errors();
    let title = headline(tree).unwrap_or_default();
    let header = row![
        theme::TITLE.text(title).width(Fill),
        button(theme::SMALL.with_font(fonts::MEDIUM).text("Copy"))
            .padding([4, 10])
            .style(theme::secondary_button)
            .on_press(Message::CopyErrorList),
    ]
    .align_y(Center);

    let mut list = Column::new().spacing(0);
    for record in records.iter().take(POPOVER_LIMIT) {
        let path = tilde(&record_path(tree, record), home);
        let line = row![
            container(theme::CAPTION.text("!").style(theme::text_on_warning))
                .padding([0, 5])
                .style(theme::warning_badge),
            theme::SMALL
                .text(crate::widgets::truncate_middle(
                    &path,
                    250.0,
                    theme::SMALL.size
                ))
                .width(Fill)
                .wrapping(iced::widget::text::Wrapping::None),
            theme::SMALL
                .text(short_reason(record.error.class))
                .style(theme::text_secondary)
                .width(REASON_W)
                .wrapping(iced::widget::text::Wrapping::None),
        ]
        .spacing(8)
        .align_y(Center)
        .height(ROW)
        .padding(iced::Padding {
            right: 12.0,
            ..iced::Padding::ZERO
        });
        list = list.push(
            mouse_area(line)
                .on_press(Message::RevealInTree(record.dir))
                .interaction(mouse::Interaction::Pointer),
        );
    }
    if records.len() > POPOVER_LIMIT {
        let more = format!(
            "and {} more",
            format::thousands((records.len() - POPOVER_LIMIT) as u64)
        );
        list = list.push(theme::SMALL.text(more).style(theme::text_muted));
    }

    let mut body = column![
        header,
        rule::horizontal(1),
        // A fixed list height leaves room for the footer inside the 360 px popover.
        scrollable(list).height((records.len().min(POPOVER_LIMIT) as f32 * ROW).min(LIST_MAX_H))
    ]
    .spacing(10);
    let privacy = records
        .iter()
        .any(|r| r.error.class == ErrorClass::PrivacyBlocked);
    if privacy && platform::PRIVACY_SETTINGS_URL.is_some() {
        body = body.push(rule::horizontal(1)).push(
            row![
                theme::SMALL
                    .text("Give dir-stat Full Disk Access to include protected folders.")
                    .style(theme::text_secondary)
                    .width(Fill),
                button(
                    theme::SMALL
                        .with_font(fonts::MEDIUM)
                        .text("Open Privacy Settings")
                )
                .padding([4, 10])
                .style(theme::secondary_button)
                .on_press(Message::OpenPrivacySettings),
            ]
            .spacing(12)
            .align_y(Center),
        );
    }
    container(body)
        .padding(16)
        .width(480)
        .max_height(360)
        .style(theme::popover)
        .into()
}

/// The macOS info bar at the bottom of the tree pane (UX §10.3).
pub fn fda_bar<'a>(terminal: Option<&str>) -> Element<'a, Message> {
    let who = match terminal {
        Some(app) => format!("give {app} Full Disk Access (dir-stat was started from it)"),
        None => "give dir-stat Full Disk Access".into(),
    };
    let text = format!(
        "Some folders are protected by macOS, so their sizes are missing. To include them, {who} in System Settings › Privacy & Security, then restart dir-stat and scan again."
    );
    let buttons = row![
        button(
            theme::SMALL
                .with_font(fonts::MEDIUM)
                .text("Open Privacy Settings")
        )
        .padding([4, 10])
        .style(theme::secondary_button)
        .on_press(Message::OpenPrivacySettings),
        button(theme::SMALL.with_font(fonts::MEDIUM).text("Not Now"))
            .padding([4, 10])
            .style(theme::secondary_button)
            .on_press(Message::DismissFdaBar),
    ]
    .spacing(8);
    container(
        row![
            theme::BODY.text("ⓘ").style(theme::text_secondary),
            theme::SMALL.text(text).width(Fill),
            buttons,
        ]
        .spacing(12)
        .align_y(Center),
    )
    .padding([10, 16])
    .width(Fill)
    .style(theme::info_bar)
    .into()
}

/// The full-pane state when the scan root can't be read (UX §10.4).
pub fn root_unreadable_view<'a>(results: &'a Results, error: ScanError) -> Element<'a, Message> {
    let name = results
        .tree
        .node(NodeId::ROOT)
        .name()
        .to_string_lossy()
        .into_owned();
    let message = match error.class {
        ErrorClass::PrivacyBlocked => "macOS needs your permission to show this folder.",
        ErrorClass::PermissionDenied => "You don't have permission to read this folder.",
        _ => reason(error.class),
    };
    let mut buttons = row![].spacing(8);
    if error.class == ErrorClass::PrivacyBlocked && platform::PRIVACY_SETTINGS_URL.is_some() {
        buttons = buttons.push(
            button(theme::BODY_STRONG.text("Open Privacy Settings"))
                .padding([6, 14])
                .style(theme::secondary_button)
                .on_press(Message::OpenPrivacySettings),
        );
    }
    buttons = buttons
        .push(
            button(theme::BODY_STRONG.text("Choose Another Folder"))
                .padding([6, 14])
                .style(theme::secondary_button)
                .on_press(Message::ChooseFolder),
        )
        .push(
            button(theme::BODY_STRONG.text("Try Again"))
                .padding([6, 14])
                .style(theme::secondary_button)
                .on_press(Message::RescanAll),
        );
    let content = column![
        container(theme::TITLE.text("!").style(theme::text_on_warning))
            .padding([2, 10])
            .style(theme::warning_badge),
        theme::TITLE.text(format!("Can't read “{name}”")),
        theme::BODY.text(message).style(theme::text_secondary),
        space().height(8),
        buttons,
    ]
    .spacing(8)
    .align_x(Center);
    center(content).into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use dirstat_core::tree::testing::{TreeBuilder, dir};

    #[test]
    fn headline_tsv_and_root_state() {
        let tree = TreeBuilder::new(
            "/r",
            dir("r")
                .child(dir("mail").error(ErrorClass::PrivacyBlocked))
                .child(dir("guest").error(ErrorClass::PermissionDenied))
                .child(dir("ok").entry_error("locked.txt", ErrorClass::PermissionDenied)),
        )
        .build();
        assert_eq!(headline(&tree).as_deref(), Some("3 items couldn't be read"));
        let copied = tsv(&tree);
        let lines: Vec<&str> = copied.lines().collect();
        assert_eq!(lines.len(), 3);
        assert!(lines.contains(&"/r/mail\tProtected by macOS"));
        assert!(lines.contains(&"/r/ok/locked.txt\tPermission denied"));
        assert_eq!(root_unreadable(&tree), None);

        let only_dirs =
            TreeBuilder::new("/r", dir("r").child(dir("a").error(ErrorClass::Other))).build();
        assert_eq!(
            headline(&only_dirs).as_deref(),
            Some("1 folder couldn't be read")
        );

        let locked = TreeBuilder::new("/r", dir("r").error(ErrorClass::PrivacyBlocked)).build();
        assert_eq!(
            root_unreadable(&locked).map(|e| e.class),
            Some(ErrorClass::PrivacyBlocked)
        );
        assert_eq!(
            inside_tooltip(3),
            "3 folders inside couldn't be read, so the real size may be larger."
        );
    }
}
