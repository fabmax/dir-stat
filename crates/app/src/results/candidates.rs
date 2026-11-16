//! Git verification of cleanup candidates on the app side (design §5.6, D10): which
//! candidates still need a check, one git job at a time, and each candidate's `GitState`.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use dirstat_core::catalog::GitState;
use dirstat_core::git::{self, GitJob, GitStatus};
use dirstat_core::tree::{CandidateRef, Tree};

#[derive(Debug, Default)]
pub struct Candidates {
    status: HashMap<CandidateRef, GitStatus>,
    /// Not in a work tree: no check needed.
    outside: HashSet<CandidateRef>,
    /// Waiting for git, by work tree.
    pending: HashMap<PathBuf, Vec<(CandidateRef, PathBuf)>>,
    /// How many of the tree's candidates were looked at; they only ever get appended.
    seen: usize,
    running: bool,
}

impl Candidates {
    pub fn state(&self, candidate: CandidateRef) -> GitState {
        if let Some(&status) = self.status.get(&candidate) {
            GitState::Known(status)
        } else if self.outside.contains(&candidate) {
            GitState::NotInRepo
        } else {
            GitState::Pending
        }
    }

    /// Picks up candidates that appeared since the last call; true when there were any.
    pub fn collect(&mut self, tree: &Tree) -> bool {
        let new = &tree.candidates()[self.seen.min(tree.candidates().len())..];
        if new.is_empty() {
            return false;
        }
        let unknown: Vec<CandidateRef> = new
            .iter()
            .copied()
            .filter(|c| !self.status.contains_key(c))
            .collect();
        let (outside, jobs) = git::plan_jobs(tree, unknown);
        self.seen = tree.candidates().len();
        self.outside.extend(outside);
        for job in jobs {
            self.pending
                .entry(job.worktree)
                .or_default()
                .extend(job.items);
        }
        true
    }

    /// The next job: the work tree with the most waiting candidates. At most one runs.
    pub fn next_job(&mut self) -> Option<GitJob> {
        if self.running {
            return None;
        }
        let worktree = self
            .pending
            .iter()
            .max_by_key(|(_, items)| items.len())?
            .0
            .clone();
        let items = self.pending.remove(&worktree)?;
        self.running = true;
        Some(GitJob { worktree, items })
    }

    pub fn finished(&mut self, results: Vec<(CandidateRef, GitStatus)>) {
        self.running = false;
        self.status.extend(results);
    }

    /// After a rescan swapped the tree: statuses carried over by path, everything else
    /// planned again (design §4.6).
    pub fn rebuilt(
        tree: &Tree,
        known: impl IntoIterator<Item = (CandidateRef, GitStatus)>,
        running: bool,
    ) -> Candidates {
        let mut c = Candidates {
            status: known.into_iter().collect(),
            running,
            ..Candidates::default()
        };
        c.collect(tree);
        c
    }

    pub fn statuses(&self) -> impl Iterator<Item = (CandidateRef, GitStatus)> + '_ {
        self.status.iter().map(|(&c, &s)| (c, s))
    }

    pub fn is_running(&self) -> bool {
        self.running
    }

    /// Something is waiting or running: keep the tick alive.
    pub fn busy(&self) -> bool {
        self.running || !self.pending.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dirstat_core::catalog::{Verdict, m};
    use dirstat_core::tree::testing::{TreeBuilder, dir};

    #[test]
    fn candidates_inside_work_trees_wait_for_git() {
        let target = Verdict::Target {
            rule: 0,
            by_default: false,
        };
        let tree = TreeBuilder::new(
            "/r",
            dir("r")
                .child(
                    dir("repo")
                        .markers(m::GIT)
                        .child(dir("target").verdict(target))
                        .child(dir("web").child(dir("node_modules").verdict(target))),
                )
                .child(dir("loose").child(dir("target").verdict(target))),
        )
        .build();
        let mut c = Candidates::default();
        c.collect(&tree);
        let find = |p: &str| CandidateRef::Dir(tree.find_path(std::path::Path::new(p)).unwrap());
        assert_eq!(c.state(find("/r/loose/target")), GitState::NotInRepo);
        assert_eq!(c.state(find("/r/repo/target")), GitState::Pending);
        assert!(c.busy());

        let job = c.next_job().unwrap();
        assert_eq!(job.worktree, std::path::Path::new("/r/repo"));
        assert_eq!(job.items.len(), 2);
        assert!(c.next_job().is_none(), "one job at a time");
        c.finished(vec![
            (find("/r/repo/target"), GitStatus::Clean),
            (find("/r/repo/web/node_modules"), GitStatus::Tracked),
        ]);
        assert_eq!(
            c.state(find("/r/repo/target")),
            GitState::Known(GitStatus::Clean)
        );
        assert!(!c.busy());
        c.collect(&tree);
        assert!(c.next_job().is_none(), "nothing new");
    }
}
