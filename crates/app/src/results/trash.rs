//! Move to Trash on the app side (design §7.1, §7.3): planning a target into a confirmation
//! dialog, the dialog itself, and the values that become a `TrashJob`.

use std::time::{Duration, SystemTime};

use dirstat_core::catalog::model::{Kind, Removal, Scope};
use dirstat_core::catalog::{
    self, Display, Engine, GitState, Risk, Section, Verdict, Why, display_section,
};
use dirstat_core::platform::EntryKind;
use dirstat_core::safety::{self, Guards, Refusal};
use dirstat_core::scan::FileMatch;
use dirstat_core::trash::{ContentsEntry, Identity, Item, Mode};
use dirstat_core::tree::{CandidateRef, FileFlags, NodeFlags, NodeId, NodeState, Tree};
use iced::widget::{button, column, container, row, space, text};
use iced::{Center, Element, Fill};

use super::Results;
use super::rows::SmallFiles;
use crate::app::Message;
use crate::{fonts, format, theme, widgets};

/// What the user asked to move to the Trash.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TrashTarget {
    Dir(NodeId),
    File(NodeId, u32),
    SmallFile(NodeId, u32),
}

impl TrashTarget {
    pub fn dir(self) -> NodeId {
        match self {
            TrashTarget::Dir(id) | TrashTarget::File(id, _) | TrashTarget::SmallFile(id, _) => id,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct TrashPlan {
    pub target: TrashTarget,
    pub items: Vec<Item>,
    pub name: String,
    pub title: String,
    pub path: String,
    pub facts: String,
    /// The section icon and the catalog reason, or a neutral note while git is pending.
    pub reason: Option<(Option<Section>, String)>,
    pub warnings: Vec<String>,
    /// Return activates "Move to Trash" only for unwarned Safe catalog matches.
    pub default_is_trash: bool,
    /// The size freed when the Trash is emptied, and whether it may be less.
    pub frees: u64,
    pub shared_space: bool,
}

/// Several folders of one cleanup group in one dialog: every item planned on its own, with
/// all guards; the ones that can't go are listed with their reason.
#[derive(Clone, Debug, PartialEq)]
pub struct BatchPlan {
    pub title: String,
    /// "Cargo target · 124 GB"
    pub subtitle: String,
    pub reason: Option<(Option<Section>, String)>,
    pub plans: Vec<TrashPlan>,
    /// Name and why it stays.
    pub skipped: Vec<(String, String)>,
    pub warnings: Vec<String>,
    /// Return moves them only if it would for every single item, and together they are at
    /// most a quarter of the scan.
    pub default_is_trash: bool,
    pub frees: u64,
    pub shared_space: bool,
}

/// At most this many warning lines in the batch dialog.
const BATCH_WARNINGS: usize = 6;
/// Folders listed by name in the batch dialog.
pub const BATCH_LISTED: usize = 8;

/// Plans every target of a cleanup group (D8 relaxed at the user's request, 2026-11-16).
#[allow(clippy::too_many_arguments)]
pub fn plan_batch(
    results: &Results,
    engine: &Engine,
    guards: &Guards,
    group_title: &str,
    targets: &[TrashTarget],
    git: impl Fn(CandidateRef) -> GitState,
    now: SystemTime,
) -> Result<BatchPlan, Refused> {
    let mut plans = Vec::new();
    let mut skipped = Vec::new();
    for &target in targets {
        match plan(results, engine, guards, target, &git, now) {
            Ok(p) => plans.push(p),
            Err(refused) => {
                let name = results
                    .tree
                    .get(target.dir())
                    .map(|n| n.name().to_string_lossy().into_owned())
                    .unwrap_or_default();
                skipped.push((name, refused.message));
            }
        }
    }
    if plans.is_empty() {
        return Err(Refused {
            title: format!("Can't move these folders to the {TRASH_WORD}"),
            message: skipped
                .first()
                .map(|(name, why)| format!("“{name}”: {why}"))
                .unwrap_or_else(|| "Nothing left to move.".into()),
        });
    }
    let frees: u64 = plans.iter().map(|p| p.frees).sum();
    let shared_space = plans.iter().any(|p| p.shared_space);
    let count = plans.len();
    // Identical lines once; a line only some items have names them. Each item's "this is X%
    // of everything scanned" gives way to one line for the whole batch.
    let mut warnings: Vec<String> = Vec::new();
    let mut seen: Vec<(String, Vec<&str>)> = Vec::new();
    for p in &plans {
        for w in &p.warnings {
            if w.ends_with("of everything scanned.") {
                continue;
            }
            match seen.iter_mut().find(|(text, _)| text == w) {
                Some((_, names)) => names.push(&p.name),
                None => seen.push((w.clone(), vec![&p.name])),
            }
        }
    }
    for (text, names) in &seen {
        warnings.push(if names.len() == count {
            text.clone()
        } else if names.len() == 1 {
            format!("“{}”: {text}", names[0])
        } else {
            format!("{} of them: {text}", names.len())
        });
    }
    let root_total = results.tree.node(NodeId::ROOT).total().alloc;
    let together = plans
        .iter()
        .filter_map(|p| results.tree.get(p.target.dir()).map(|n| n.total().alloc))
        .sum::<u64>();
    let big = root_total > 0 && together.saturating_mul(4) > root_total;
    if big {
        warnings.push(format!(
            "Together this is {} of everything scanned.",
            format::percent(together, root_total)
        ));
    }
    if warnings.len() > BATCH_WARNINGS {
        let more = warnings.len() - (BATCH_WARNINGS - 1);
        warnings.truncate(BATCH_WARNINGS - 1);
        warnings.push(format!(
            "and {} more",
            format::plural(more as u64, "warning", "warnings")
        ));
    }
    let default_is_trash = !big && plans.iter().all(|p| p.default_is_trash);
    // The catalog reason, if all items share it.
    let reason = plans[0]
        .reason
        .clone()
        .filter(|r| plans.iter().all(|p| p.reason.as_ref() == Some(r)));
    let what = if plans
        .iter()
        .all(|p| matches!(p.target, TrashTarget::Dir(_)))
    {
        format::plural(count as u64, "folder", "folders")
    } else {
        format::plural(count as u64, "item", "items")
    };
    let up_to = if shared_space { "up to " } else { "" };
    Ok(BatchPlan {
        title: format!("Move {what} to the {TRASH_WORD}?"),
        subtitle: format!("{group_title} · frees {up_to}{}", format::size(frees)),
        reason,
        plans,
        skipped,
        warnings,
        default_is_trash,
        frees,
        shared_space,
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Focus {
    Cancel,
    Trash,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Modal {
    Confirm {
        plan: Box<TrashPlan>,
        focus: Focus,
    },
    ConfirmBatch {
        batch: Box<BatchPlan>,
        focus: Focus,
    },
    Refuse {
        title: String,
        message: String,
    },
    /// New Scan while scanning (UX §5.2). `Focus::Trash` is "Stop and Start Over".
    StopScan {
        title: String,
        focus: Focus,
    },
}

const SHARED_SPACE: &str = "Some files are shared with other folders (hard links or clones); this may free less than shown.";
const TRASH_WORD: &str = if cfg!(windows) {
    "Recycle Bin"
} else {
    "Trash"
};

/// Why a target can't be trashed at all (G1, or the G2–G5 path rules).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Refused {
    pub title: String,
    pub message: String,
}

/// Plans a target: eligibility, guards, the catalog and git reason, warnings, the default.
/// Pure apart from reading the tree, catalog and cached guards.
pub fn plan(
    results: &Results,
    engine: &Engine,
    guards: &Guards,
    target: TrashTarget,
    git: impl Fn(CandidateRef) -> GitState,
    now: SystemTime,
) -> Result<TrashPlan, Refused> {
    let tree = &results.tree;
    let dir = target.dir();
    let node = tree
        .get(dir)
        .ok_or_else(|| refused("This item", "It is no longer in the tree."))?;
    let blocked = |id: NodeId| {
        std::iter::once(id).chain(tree.ancestors(id)).any(|a| {
            let n = tree.node(a);
            n.has_flag(NodeFlags::TRASHED)
                || n.has_flag(NodeFlags::TRASH_PENDING)
                || n.has_flag(NodeFlags::RESCANNING)
        })
    };
    if blocked(dir) || results.rescanning_within(dir) {
        return Err(refused(
            "This item",
            "It is already being moved, rescanned or in the Trash.",
        ));
    }

    let dir_path = tree.path_of(dir);
    let dev = tree.root_dev();
    let (path, name, alloc, items_count, mtime, expect, candidate) = match target {
        TrashTarget::Dir(id) => {
            let name = node.name().to_string_lossy().into_owned();
            // G1: only complete, readable folders that hold no other volume.
            match node.state() {
                NodeState::Done => {}
                NodeState::Skipped(_) => return Err(refused(&name, "It isn't part of this scan.")),
                _ => {
                    return Err(refused(
                        &name,
                        "Available when this folder is fully scanned.",
                    ));
                }
            }
            if node.error().is_some() {
                return Err(refused(
                    &name,
                    "It couldn't be read, so its contents are unknown.",
                ));
            }
            if node.total().boundaries > 0 {
                return Err(Refused {
                    title: format!("Can't move “{name}”"),
                    message: Refusal::ContainsOtherVolume.message(&name),
                });
            }
            let total = node.total();
            let expect = Identity {
                dev,
                ino: node.ino(),
                kind: EntryKind::Dir,
                len_mtime: None,
            };
            (
                dir_path.clone(),
                name,
                total.alloc,
                Some(total.items),
                node.newest_mtime(),
                expect,
                Some(CandidateRef::Dir(id)),
            )
        }
        TrashTarget::File(_, index) => {
            let file = node
                .large_files()
                .get(index as usize)
                .ok_or_else(|| refused("This file", "It is no longer listed."))?;
            if file.flags.contains(FileFlags::TRASHED)
                || file.flags.contains(FileFlags::TRASH_PENDING)
            {
                return Err(refused("This file", "It is already in the Trash."));
            }
            let len_mtime = cfg!(windows).then_some((file.apparent, file.mtime));
            let expect = Identity {
                dev,
                ino: file.ino,
                kind: EntryKind::File,
                len_mtime,
            };
            let candidate = matches!(file.matched, Some(FileMatch::Target(_)))
                .then_some(CandidateRef::File(dir, index));
            (
                dir_path.join(&*file.name),
                file.name.to_string_lossy().into_owned(),
                file.alloc,
                None,
                file.mtime,
                expect,
                candidate,
            )
        }
        TrashTarget::SmallFile(_, index) => {
            let Some(SmallFiles::Loaded(files)) =
                results.view.open_aggregates.get(&dir).map(|o| &o.small)
            else {
                return Err(refused("This file", "It is no longer listed."));
            };
            let file = files
                .get(index as usize)
                .ok_or_else(|| refused("This file", "It is no longer listed."))?;
            if file.flags.contains(FileFlags::TRASHED) {
                return Err(refused("This file", "It is already in the Trash."));
            }
            let len_mtime = (cfg!(windows) && file.kind == EntryKind::File)
                .then_some((file.apparent, file.mtime));
            let expect = Identity {
                dev,
                ino: file.ino,
                kind: file.kind,
                len_mtime,
            };
            (
                dir_path.join(&*file.name),
                file.name.to_string_lossy().into_owned(),
                file.alloc,
                None,
                file.mtime,
                expect,
                None,
            )
        }
    };

    // G2–G5 path rules. Read-only is a live check too, so a volume mounted after startup
    // is refused before the dialog rather than after confirming (one `statfs`).
    let read_only = dirstat_core::platform::is_read_only(std::path::Path::new(&path))
        .then_some(Refusal::ReadOnlyVolume);
    if let Err(refusal) =
        safety::check(guards, tree.root_path(), &path).and(read_only.map_or(Ok(()), Err))
    {
        return Err(Refused {
            title: format!("Can't move “{name}”"),
            message: refusal.message(&name),
        });
    }

    // The catalog rule, if the target is a candidate itself.
    let rule_index = match target {
        TrashTarget::Dir(id) => match tree.node(id).verdict() {
            Verdict::Target { rule, .. } => Some(rule),
            _ => None,
        },
        TrashTarget::File(_, index) => match node.large_files()[index as usize].matched {
            Some(FileMatch::Target(rule)) => Some(rule),
            _ => None,
        },
        TrashTarget::SmallFile(..) => None,
    };
    let rule = rule_index.map(|r| engine.rule(r));
    let display = match (rule, candidate) {
        (Some(rule), Some(candidate)) => Some(display_section(
            rule.risk().unwrap_or(Risk::Review),
            git(candidate),
        )),
        _ => None,
    };

    let mut warnings = Vec::new();
    // Inside a package: a folder's ancestors, a file's folder and its ancestors.
    let around: Vec<NodeId> = match target {
        TrashTarget::Dir(_) => tree.ancestors(dir).collect(),
        _ => std::iter::once(dir).chain(tree.ancestors(dir)).collect(),
    };
    if let Some(package) = around
        .into_iter()
        .find(|&a| super::package::is_package(tree, a))
    {
        let package_name = tree.node(package).name().to_string_lossy().into_owned();
        warnings.push(format!(
            "Inside the package “{package_name}”: removing parts of it can break it."
        ));
    }
    // A protected zone around it. Self-only zones (`~`, `/Volumes/X`) cover only the folder
    // itself, not what is inside.
    let zone = std::iter::once(dir)
        .chain(tree.ancestors(dir))
        .find_map(|a| match tree.node(a).verdict() {
            Verdict::Protected(r) => {
                let rule = engine.rule(r);
                let self_only = matches!(rule.kind, Kind::Protect(Scope::SelfOnly));
                (a == dir || !self_only).then_some(rule)
            }
            _ => None,
        });
    if let Some(zone) = zone {
        warnings.push(if zone.reason.starts_with(zone.name) {
            zone.reason.to_string()
        } else {
            format!("{}: {}", zone.name, zone.reason)
        });
    }
    let repo = candidate
        .and_then(|c| dirstat_core::git::worktree_of(tree, c))
        .and_then(|w| w.file_name().map(|n| n.to_string_lossy().into_owned()))
        .unwrap_or_default();
    if let (Some(rule), Some(display)) = (rule, display) {
        // The reason has its own line in the dialog (unless git hides it): don't repeat it.
        let repeat = if matches!(display, Display::Shown { .. }) {
            ""
        } else {
            rule.reason
        };
        warnings.extend(catalog::warnings(
            rule.risk().unwrap_or(Risk::Review),
            display,
            repeat,
            &repo,
        ));
        if let Some(caution) = rule.caution {
            warnings.push(caution.to_string());
        }
        if rule.removal == Removal::ToolOnly
            && let Some(index) = rule_index
            && let Some(command) = engine
                .cleanup_for(index, &name, &path.to_string_lossy())
                .first()
        {
            warnings.push(format!("Recommended instead: {command}"));
        }
    }
    if let TrashTarget::Dir(id) = target {
        let total = tree.node(id).total();
        if total.cloud_items > 0 {
            warnings.push(format!(
                "{} inside are stored only in iCloud. Moving it to the Trash removes them from iCloud on all your devices and frees almost no space here.",
                format::plural(u64::from(total.cloud_items), "item", "items")
            ));
        }
        if total.error_dirs > 0 {
            warnings.push(format!(
                "{} inside couldn't be read; their contents are unknown.",
                format::plural(u64::from(total.error_dirs), "folder", "folders")
            ));
        }
    }
    let linked = match target {
        TrashTarget::Dir(id) => {
            let total = tree.node(id).total();
            total.linked_alloc.saturating_mul(10) >= total.alloc && total.linked_alloc > 0
        }
        TrashTarget::File(_, index) => node.large_files()[index as usize].nlink > 1,
        TrashTarget::SmallFile(..) => false,
    };
    let shared_space = linked || rule.is_some_and(|r| r.shares_space);
    if shared_space {
        warnings.push(SHARED_SPACE.into());
    }
    let root_total = tree.node(NodeId::ROOT).total().alloc;
    if root_total > 0 && alloc.saturating_mul(4) > root_total {
        warnings.push(format!(
            "This is {} of everything scanned.",
            format::percent(alloc, root_total)
        ));
    }

    let reason = match display {
        Some(Display::Shown {
            section,
            why: Why::Catalog,
        }) => Some((
            Some(section),
            rule.map(|r| r.reason.to_string()).unwrap_or_default(),
        )),
        Some(Display::Shown { .. }) => Some((
            Some(Section::ReviewFirst),
            rule.map(|r| r.reason.to_string()).unwrap_or_default(),
        )),
        Some(Display::Pending) => Some((None, "Checking with git…".into())),
        Some(Display::Hidden) | None => None,
    };
    let removal = rule.map_or(Removal::Item, |r| r.removal);
    let default_is_trash = display
        == Some(Display::Shown {
            section: Section::SafeToDelete,
            why: Why::Catalog,
        })
        && matches!(removal, Removal::Item | Removal::Contents)
        // Shared space only lowers what is freed; it doesn't make the item riskier (user
        // decision 2026-11-13: Cargo hard-links its build output, so nearly every target has it).
        && warnings.iter().all(|w| w == SHARED_SPACE);

    let mode = match (removal, target) {
        (Removal::Contents, TrashTarget::Dir(id)) => Mode::Contents {
            entries: contents_entries(tree, id),
            scan_started: results.scan_started,
        },
        _ => Mode::Item,
    };
    let mut facts = vec![format::size(alloc)];
    if let Some(items) = items_count {
        facts.push(format::plural(items, "item", "items"));
    }
    if mtime != i64::MIN {
        let age = now
            .duration_since(SystemTime::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs() as i64)
            .saturating_sub(mtime)
            .max(0);
        facts.push(format!(
            "modified {}",
            format::relative(Duration::from_secs(age as u64))
        ));
    }
    let title = match mode {
        Mode::Contents { .. } => format!("Move the contents of “{name}” to the {TRASH_WORD}?"),
        Mode::Item => format!("Move “{name}” to the {TRASH_WORD}?"),
    };
    Ok(TrashPlan {
        target,
        items: vec![Item {
            path: path.clone(),
            expect,
            mode,
        }],
        title,
        path: path.display().to_string(),
        facts: facts.join(" · "),
        reason,
        warnings,
        default_is_trash,
        frees: alloc,
        shared_space,
        name,
    })
}

/// Contents mode: the scanned subfolders and large files of `id` (G9).
fn contents_entries(tree: &Tree, id: NodeId) -> Vec<ContentsEntry> {
    let dev = tree.root_dev();
    let mut entries: Vec<ContentsEntry> = tree
        .children(id)
        .filter(|&c| !matches!(tree.node(c).state(), NodeState::Skipped(_)))
        .map(|c| ContentsEntry {
            name: tree.node(c).name().into(),
            expect: Identity {
                dev,
                ino: tree.node(c).ino(),
                kind: EntryKind::Dir,
                len_mtime: None,
            },
        })
        .collect();
    entries.extend(tree.node(id).large_files().iter().map(|f| ContentsEntry {
        name: f.name.clone(),
        expect: Identity {
            dev,
            ino: f.ino,
            kind: EntryKind::File,
            len_mtime: cfg!(windows).then_some((f.apparent, f.mtime)),
        },
    }));
    entries
}

fn refused(name: &str, message: &str) -> Refused {
    Refused {
        title: format!("Can't move “{name}” to the {TRASH_WORD}"),
        message: message.into(),
    }
}

/// The confirmation or refusal dialog over a scrim (UX §9).
pub fn modal_view<'a>(modal: &'a Modal, home: Option<&std::path::Path>) -> Element<'a, Message> {
    let card = match modal {
        Modal::Refuse { title, message } => column![
            theme::TITLE.text(title.clone()),
            theme::BODY
                .text(message.clone())
                .style(theme::text_secondary),
            row![
                space().width(Fill),
                dialog_button("OK", true, false, Message::CancelModal)
            ],
        ]
        .spacing(16),
        Modal::StopScan { title, focus } => column![
            theme::TITLE.text(title.clone()),
            theme::BODY
                .text("The results so far will be lost.")
                .style(theme::text_secondary),
            row![
                space().width(Fill),
                dialog_button(
                    "Keep Scanning",
                    *focus == Focus::Cancel,
                    false,
                    Message::CancelModal
                ),
                dialog_button(
                    "Stop and Start Over",
                    *focus == Focus::Trash,
                    false,
                    Message::StopAndStartOver
                ),
            ]
            .spacing(8),
        ]
        .spacing(16),
        Modal::ConfirmBatch { batch, focus } => {
            let mut body = column![
                theme::TITLE.text(batch.title.clone()),
                theme::SMALL
                    .text(batch.subtitle.clone())
                    .style(theme::text_secondary),
            ]
            .spacing(16);
            let mut list = column![].spacing(2);
            for p in batch.plans.iter().take(BATCH_LISTED) {
                let shown = super::selection_bar::tilde(std::path::Path::new(&p.path), home);
                list = list.push(
                    row![
                        theme::SMALL
                            .text(widgets::truncate_middle(&shown, 300.0, theme::SMALL.size))
                            .wrapping(text::Wrapping::None)
                            .width(Fill),
                        theme::SMALL
                            .text(format::size(p.frees))
                            .style(theme::text_secondary),
                    ]
                    .spacing(8),
                );
            }
            if batch.plans.len() > BATCH_LISTED {
                list = list.push(
                    theme::SMALL
                        .text(format!("and {} more", batch.plans.len() - BATCH_LISTED))
                        .style(theme::text_secondary),
                );
            }
            body = body.push(list);
            if let Some((section, reason)) = &batch.reason {
                let icon = match section {
                    Some(Section::SafeToDelete) => theme::BODY.text("✓").style(theme::text_good),
                    Some(Section::ReviewFirst) => theme::BODY.text("!").style(theme::text_warning),
                    None => theme::BODY.text("·").style(theme::text_muted),
                };
                body = body.push(row![icon, theme::BODY.text(reason.clone())].spacing(10));
            }
            for warning in &batch.warnings {
                body = body.push(
                    row![
                        container(theme::CAPTION.text("!").style(theme::text_on_warning))
                            .padding([0, 5])
                            .style(theme::warning_badge),
                        theme::SMALL.text(warning.clone()),
                    ]
                    .spacing(10),
                );
            }
            if !batch.skipped.is_empty() {
                let mut text = format!(
                    "{} stay:",
                    format::plural(batch.skipped.len() as u64, "item", "items")
                );
                for (name, why) in batch.skipped.iter().take(3) {
                    text += &format!(" “{name}” ({why})");
                }
                if batch.skipped.len() > 3 {
                    text += &format!(" and {} more", batch.skipped.len() - 3);
                }
                body = body.push(theme::SMALL.text(text).style(theme::text_secondary));
            }
            body = body.push(
                theme::SMALL
                    .text("The space is freed when you empty the Trash.")
                    .style(theme::text_secondary),
            );
            body.push(
                row![
                    space().width(Fill),
                    dialog_button(
                        "Cancel",
                        *focus == Focus::Cancel,
                        false,
                        Message::CancelModal
                    ),
                    dialog_button(
                        if cfg!(windows) {
                            "Move All to Recycle Bin"
                        } else {
                            "Move All to Trash"
                        },
                        *focus == Focus::Trash,
                        true,
                        Message::ConfirmBatch
                    ),
                ]
                .spacing(8),
            )
        }
        Modal::Confirm { plan, focus } => {
            let shown_path = super::selection_bar::tilde(std::path::Path::new(&plan.path), home);
            let mut body = column![
                theme::TITLE.text(plan.title.clone()),
                row![
                    theme::BODY.text("▭").style(theme::text_secondary),
                    column![
                        theme::BODY
                            .text(widgets::truncate_middle(
                                &shown_path,
                                360.0,
                                theme::BODY.size
                            ))
                            .wrapping(text::Wrapping::None),
                        theme::SMALL
                            .text(plan.facts.clone())
                            .style(theme::text_secondary),
                    ]
                    .spacing(2),
                ]
                .spacing(10),
            ]
            .spacing(16);
            if let Some((section, reason)) = &plan.reason {
                let icon = match section {
                    Some(Section::SafeToDelete) => theme::BODY.text("✓").style(theme::text_good),
                    Some(Section::ReviewFirst) => theme::BODY.text("!").style(theme::text_warning),
                    None => theme::BODY.text("·").style(theme::text_muted),
                };
                body = body.push(row![icon, theme::BODY.text(reason.clone())].spacing(10));
            }
            for warning in &plan.warnings {
                body = body.push(
                    row![
                        container(theme::CAPTION.text("!").style(theme::text_on_warning))
                            .padding([0, 5])
                            .style(theme::warning_badge),
                        theme::SMALL.text(warning.clone()),
                    ]
                    .spacing(10),
                );
            }
            body = body.push(
                theme::SMALL
                    .text("The space is freed when you empty the Trash.")
                    .style(theme::text_secondary),
            );
            body.push(
                row![
                    space().width(Fill),
                    dialog_button(
                        "Cancel",
                        *focus == Focus::Cancel,
                        false,
                        Message::CancelModal
                    ),
                    dialog_button(
                        if cfg!(windows) {
                            "Move to Recycle Bin"
                        } else {
                            "Move to Trash"
                        },
                        *focus == Focus::Trash,
                        true,
                        Message::ConfirmTrash
                    ),
                ]
                .spacing(8),
            )
        }
    };
    let card = container(card).padding(24).width(440).style(theme::dialog);
    container(iced::widget::center(card))
        .width(Fill)
        .height(Fill)
        .style(theme::scrim)
        .into()
}

fn dialog_button<'a>(
    label: &'a str,
    focused: bool,
    destructive: bool,
    message: Message,
) -> Element<'a, Message> {
    button(
        theme::BODY_STRONG
            .with_font(fonts::MEDIUM)
            .text(label)
            .align_x(Center),
    )
    .padding([7, 16])
    .style(move |t, s| theme::dialog_button(t, s, focused, destructive))
    .on_press(message)
    .into()
}

#[cfg(test)]
mod tests;
