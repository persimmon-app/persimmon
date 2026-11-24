// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Persimmon contributors

//! Watch rules as the window shows them: each with its command, what it's
//! doing, and how many files it converted today.

use persimmon_core::format::FormatId;
use persimmon_core::registry::{Operation, Registry};
use persimmon_core::watch::files_in;

use crate::history::History;
use crate::presets::Presets;
use crate::queue::tilde;
use crate::watch::rule::{Rule, command, spec};
use crate::watch::runner::{RuleStatus, WatchState};

/// A rule as the Watch folders view shows it.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuleView {
    #[serde(flatten)]
    pub rule: Rule,
    pub command: String,
    pub status: RuleStatus,
    /// Files converted since `since` (local midnight), including the burst
    /// not yet in History.
    pub today: usize,
    pub folder_label: String,
    pub results_label: String,
    pub done_label: Option<String>,
}

/// A rule being made or edited.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuleDraftView {
    pub rule: Rule,
    pub command: String,
    /// Files already in the folder that the rule would match.
    pub existing: usize,
}

/// Every rule as the view shows it, in the order they were made.
pub fn rule_views(
    rules: &[Rule],
    presets: &Presets,
    statuses: &[RuleStatus],
    history: &History,
    since: u64,
) -> Vec<RuleView> {
    rules
        .iter()
        .map(|r| {
            let status = statuses
                .iter()
                .find(|s| s.id == r.id)
                .cloned()
                .unwrap_or_else(|| RuleStatus {
                    id: r.id.clone(),
                    state: if r.enabled {
                        WatchState::Watching
                    } else {
                        WatchState::Paused
                    },
                    burst_done: 0,
                    last_failed: None,
                    note: None,
                });
            RuleView {
                command: command(r, presets),
                today: history.converted_since(&r.id, since) + status.burst_done,
                status,
                folder_label: tilde(&r.folder),
                results_label: tilde(&r.results),
                done_label: r.done.as_deref().map(tilde),
                rule: r.clone(),
            }
        })
        .collect()
}

/// `rule` for the editor, with the files already in its folder counted.
pub fn draft_view(rule: Rule, presets: &Presets, reg: &Registry) -> RuleDraftView {
    let (s, _) = spec(&rule, presets, reg);
    RuleDraftView {
        existing: files_in(&s).map(|f| f.len()).unwrap_or(0),
        command: command(&rule, presets),
        rule,
    }
}

/// Every format Convert can make, by name.
pub fn watch_targets(reg: &Registry) -> Vec<FormatId> {
    let mut out: Vec<FormatId> = reg
        .capabilities()
        .iter()
        .filter(|c| c.op == Operation::Convert)
        .filter_map(|c| c.to)
        .collect();
    out.sort_by_key(|f| f.as_str());
    out.dedup();
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history::{Batch, History, Item, ItemOutcome, Source};
    use crate::presets::Presets;
    use crate::queue::QueueMode;
    use crate::watch::rule::draft;
    use crate::watch::runner::{RuleStatus, WatchState};
    use std::path::PathBuf;

    struct Dir(PathBuf);
    impl Dir {
        fn new(name: &str) -> Dir {
            let d =
                std::env::temp_dir().join(format!("prsm-app-views-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&d);
            std::fs::create_dir_all(d.join("in")).unwrap();
            Dir(d)
        }
    }
    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn item() -> Item {
        Item {
            inputs: vec![PathBuf::from("/in/a.png")],
            output: None,
            output_size: None,
            output_mtime: None,
            size: 1,
            outcome: ItemOutcome::Done,
            undone: false,
            to: None,
            mode: None,
            moved: None,
            restored: false,
        }
    }

    fn batch(id: &str, rule: &str, finished: u64, n: usize) -> Batch {
        Batch {
            id: id.into(),
            started: finished,
            finished,
            source: Source::Watch,
            preset: None,
            mode: QueueMode::Convert,
            items: (0..n).map(|_| item()).collect(),
            undone: false,
            rule: Some(rule.into()),
            folder: Some(PathBuf::from("/in")),
        }
    }

    #[test]
    fn rule_views_count_today_from_history_and_the_open_burst() {
        let d = Dir::new("today");
        let reg = Registry::builtin();
        let presets = Presets::load(&d.0.join("presets"));
        let mut a = draft(&reg, d.0.join("in"), FormatId::Webp);
        a.id = "a".into();
        let mut b = draft(&reg, d.0.join("in2"), FormatId::Png);
        b.id = "b".into();
        b.enabled = false;
        let mut h = History::open(&d.0.join("history.jsonl"));
        let now = crate::history::now();
        h.append(batch("old", "a", now - 3 * 86_400, 5)).unwrap();
        h.append(batch("new", "a", now - 60, 3)).unwrap();
        let mut status = RuleStatus {
            id: "a".into(),
            state: WatchState::Converting { done: 1, total: 4 },
            burst_done: 2,
            last_failed: None,
            note: None,
        };
        let views = rule_views(
            &[a.clone(), b.clone()],
            &presets,
            std::slice::from_ref(&status),
            &h,
            now - 3_600,
        );
        assert_eq!(views.len(), 2);
        assert_eq!(views[0].today, 5);
        assert_eq!(views[0].status, status);
        assert!(views[0].command.starts_with("prsm watch "));
        assert_eq!(views[0].results_label, crate::queue::tilde(&a.results));
        assert_eq!(views[0].done_label, None);
        // No status yet: a paused rule says so, an active one is watching.
        assert_eq!(views[1].status.state, WatchState::Paused);
        status.id = "b".into();
        let mut c = a.clone();
        c.id = "c".into();
        let views = rule_views(&[c], &presets, &[], &h, now);
        assert_eq!(views[0].status.state, WatchState::Watching);
        assert_eq!(views[0].today, 0);
    }

    #[test]
    fn the_draft_counts_files_already_here() {
        let d = Dir::new("draft");
        let reg = Registry::builtin();
        let presets = Presets::load(&d.0.join("presets"));
        for n in ["a.png", "b.jpg", ".DS_Store", "notes.txt"] {
            std::fs::write(d.0.join("in").join(n), b"x").unwrap();
        }
        let rule = draft(&reg, d.0.join("in"), FormatId::Webp);
        let v = draft_view(rule.clone(), &presets, &reg);
        assert_eq!(v.existing, 2);
        assert_eq!(v.rule, rule);
        assert!(v.command.contains("--to webp"));
        let gone = draft(&reg, d.0.join("nowhere"), FormatId::Webp);
        assert_eq!(draft_view(gone, &presets, &reg).existing, 0);
    }

    #[test]
    fn watch_targets_are_every_format_convert_makes() {
        let t = watch_targets(&Registry::builtin());
        assert!(
            t.contains(&FormatId::Webp) && t.contains(&FormatId::Pdf) && t.contains(&FormatId::Mp4)
        );
        let mut sorted = t.clone();
        sorted.sort_by_key(|f| f.as_str());
        sorted.dedup();
        assert_eq!(t, sorted);
    }

    /// What the window shows for watch rules is in the app's words: no
    /// CLI flags, `prsm` or package managers (the command line aside).
    #[test]
    fn the_shells_words_are_app_words() {
        use crate::watch::rule::{ALLOW, CANT_READ};
        use crate::watch::runner::DAMAGED_NOTE;
        for text in [
            CANT_READ,
            ALLOW,
            DAMAGED_NOTE,
            "Results would be converted again",
            "Choose another folder for results.",
            "Finished originals would be converted again",
            "Choose another folder for finished originals.",
            "Another rule already watches this folder",
            "Another rule watches this folder, so results would be converted again",
            "Another rule watches this folder, so finished originals would be converted again",
            "Match files needs at least one pattern",
            "Type a pattern such as *.jpg.",
            "This folder doesn't exist",
            "This isn't a folder",
            "Choose another folder.",
            "That preset no longer exists",
            "Choose another preset.",
            "Small web is used by the watch rule for ~/Inbox. Change that rule's preset first.",
            "Persimmon couldn't be added to Login Items (x)",
        ] {
            assert_eq!(crate::wording::cli_word(text), None, "{text}");
        }
    }
}
