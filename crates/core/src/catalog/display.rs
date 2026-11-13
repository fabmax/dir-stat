//! Which section a candidate is shown in (design §5.3, user decision D1). Every UI place
//! that shows a candidate asks `display_section`; nothing looks at `Risk` directly.

use super::Risk;
use crate::git::GitStatus;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Section {
    SafeToDelete,
    ReviewFirst,
}

/// Only Safe is "Safe to delete". Low is restorable only by re-downloading or rebuilding,
/// so it goes under "Review first" with Review.
pub const fn section_for(risk: Risk) -> Section {
    match risk {
        Risk::Safe => Section::SafeToDelete,
        Risk::Low | Risk::Review => Section::ReviewFirst,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GitState {
    /// Not inside a git work tree: no check needed.
    NotInRepo,
    /// Inside a work tree, not checked yet.
    Pending,
    Known(GitStatus),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Display {
    /// Waiting for git: not in the pane, no pill, a neutral dialog reason.
    Pending,
    /// Contains tracked files: never suggested anywhere.
    Hidden,
    Shown {
        section: Section,
        why: Why,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Why {
    Catalog,
    GitNotIgnored,
    GitUnavailable,
}

pub const fn display_section(risk: Risk, git: GitState) -> Display {
    match git {
        GitState::Pending => Display::Pending,
        GitState::Known(GitStatus::Tracked) => Display::Hidden,
        GitState::Known(GitStatus::NotIgnored) => Display::Shown {
            section: Section::ReviewFirst,
            why: Why::GitNotIgnored,
        },
        GitState::Known(GitStatus::Unavailable) => Display::Shown {
            section: Section::ReviewFirst,
            why: Why::GitUnavailable,
        },
        GitState::NotInRepo | GitState::Known(GitStatus::Clean) => Display::Shown {
            section: section_for(risk),
            why: Why::Catalog,
        },
    }
}

/// Warning lines for the trash dialog that come from the catalog and git (design §7.3).
/// `reason` is the rule's reason, `repo` the work tree's folder name.
pub fn warnings(risk: Risk, display: Display, reason: &str, repo: &str) -> Vec<String> {
    let mut lines = Vec::new();
    match display {
        Display::Pending => return lines,
        Display::Hidden => {
            lines.push(format!("Contains files tracked by git in {repo}."));
            return lines;
        }
        Display::Shown {
            why: Why::GitNotIgnored,
            ..
        } => {
            lines.push(format!(
                "Not ignored by git in {repo}: it may contain your work."
            ));
        }
        Display::Shown {
            why: Why::GitUnavailable,
            ..
        } => lines.push("Couldn't check with git.".into()),
        Display::Shown {
            why: Why::Catalog, ..
        } => {}
    }
    match risk {
        Risk::Safe => {}
        Risk::Low => lines.push(format!(
            "Restored only by re-downloading or rebuilding: {reason}"
        )),
        Risk::Review => lines.push(format!("May contain data you want: {reason}")),
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use GitStatus::*;

    #[test]
    fn low_goes_under_review_first() {
        assert_eq!(section_for(Risk::Safe), Section::SafeToDelete);
        assert_eq!(section_for(Risk::Low), Section::ReviewFirst);
        assert_eq!(section_for(Risk::Review), Section::ReviewFirst);
    }

    #[test]
    fn the_display_table() {
        let shown = |section, why| Display::Shown { section, why };
        let safe = Section::SafeToDelete;
        let review = Section::ReviewFirst;
        let table = [
            (Risk::Safe, GitState::NotInRepo, shown(safe, Why::Catalog)),
            (
                Risk::Safe,
                GitState::Known(Clean),
                shown(safe, Why::Catalog),
            ),
            (Risk::Low, GitState::NotInRepo, shown(review, Why::Catalog)),
            (
                Risk::Low,
                GitState::Known(Clean),
                shown(review, Why::Catalog),
            ),
            (
                Risk::Review,
                GitState::NotInRepo,
                shown(review, Why::Catalog),
            ),
            (
                Risk::Review,
                GitState::Known(Clean),
                shown(review, Why::Catalog),
            ),
        ];
        for (risk, git, expected) in table {
            assert_eq!(display_section(risk, git), expected, "{risk:?} {git:?}");
        }
        for risk in [Risk::Safe, Risk::Low, Risk::Review] {
            assert_eq!(
                display_section(risk, GitState::Known(NotIgnored)),
                shown(review, Why::GitNotIgnored)
            );
            assert_eq!(
                display_section(risk, GitState::Known(Unavailable)),
                shown(review, Why::GitUnavailable)
            );
            assert_eq!(
                display_section(risk, GitState::Known(Tracked)),
                Display::Hidden
            );
            assert_eq!(display_section(risk, GitState::Pending), Display::Pending);
        }
    }

    #[test]
    fn warning_text_per_risk_and_cause() {
        let catalog = |risk| display_section(risk, GitState::NotInRepo);
        assert!(warnings(Risk::Safe, catalog(Risk::Safe), "Rebuilt.", "proj").is_empty());
        assert_eq!(
            warnings(Risk::Low, catalog(Risk::Low), "Reinstalled by npm.", "proj"),
            ["Restored only by re-downloading or rebuilding: Reinstalled by npm."]
        );
        assert_eq!(
            warnings(Risk::Review, catalog(Risk::Review), "Old backups.", "proj"),
            ["May contain data you want: Old backups."]
        );
        assert_eq!(
            warnings(
                Risk::Safe,
                display_section(Risk::Safe, GitState::Known(NotIgnored)),
                "Rebuilt.",
                "proj"
            ),
            ["Not ignored by git in proj: it may contain your work."]
        );
        assert_eq!(
            warnings(
                Risk::Low,
                display_section(Risk::Low, GitState::Known(Unavailable)),
                "Reinstalled.",
                "proj"
            ),
            [
                "Couldn't check with git.",
                "Restored only by re-downloading or rebuilding: Reinstalled."
            ]
        );
        assert_eq!(
            warnings(Risk::Safe, Display::Hidden, "Rebuilt.", "proj"),
            ["Contains files tracked by git in proj."]
        );
        assert!(warnings(Risk::Review, Display::Pending, "x", "proj").is_empty());
    }
}
