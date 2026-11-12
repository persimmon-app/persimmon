// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Persimmon contributors

//! The queue: one row per file, each a one-input job previewed before it
//! runs (spec D1, "How the queue behaves").

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use persimmon_core::format::{FormatId, MediaKind};
use persimmon_core::job::{Job, JobOptions, PlanMode};
use persimmon_core::output::OutputPolicy;
use persimmon_core::registry::{Operation, Registry};

use crate::wording::{Shown, shown};

pub type RowId = u64;

/// Where a row is.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum RowState {
    Queued,
    /// `fraction` is `None` while a backend can't tell (images).
    Running {
        fraction: Option<f32>,
    },
    Done {
        #[serde(serialize_with = "lossy")]
        output: PathBuf,
        size: Option<u64>,
    },
    /// Compressing wouldn't make it smaller; nothing was written.
    NotSmaller,
    Failed {
        error: Shown,
    },
    Cancelled,
}

/// A row as the window shows it.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RowView {
    pub id: RowId,
    #[serde(serialize_with = "lossy")]
    pub path: PathBuf,
    pub name: String,
    pub from: Option<FormatId>,
    pub to: Option<FormatId>,
    pub op: Operation,
    pub size: u64,
    #[serde(serialize_with = "lossy_opt")]
    pub output: Option<PathBuf>,
    #[serde(flatten)]
    pub state: RowState,
}

/// What [`Queue::set_target`] did.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct SetTarget {
    pub changed: Vec<RowView>,
    pub skipped: usize,
}

fn lossy<S: serde::Serializer>(p: &Path, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_str(&p.to_string_lossy())
}

fn lossy_opt<S: serde::Serializer>(p: &Option<PathBuf>, s: S) -> Result<S::Ok, S::Error> {
    match p {
        Some(p) => lossy(p, s),
        None => s.serialize_none(),
    }
}

struct Row {
    id: RowId,
    path: PathBuf,
    key: PathBuf,
    from: Option<FormatId>,
    job: Job,
    size: u64,
    output: Option<PathBuf>,
    state: RowState,
}

/// The rows, and the target last picked for each kind of file.
pub struct Queue {
    reg: Registry,
    rows: Vec<Row>,
    next_id: RowId,
    last: HashMap<MediaKind, FormatId>,
}

impl Queue {
    pub fn new(reg: Registry) -> Queue {
        Queue {
            reg,
            rows: Vec::new(),
            next_id: 1,
            last: HashMap::new(),
        }
    }

    /// Add files; a path already queued isn't added again. Returns the
    /// new rows only.
    pub fn add(&mut self, paths: &[PathBuf]) -> Vec<RowView> {
        let mut added = Vec::new();
        for path in paths {
            let key = std::fs::canonicalize(path).unwrap_or_else(|_| path.clone());
            if self.rows.iter().any(|r| r.key == key) {
                continue;
            }
            let id = self.next_id;
            self.next_id += 1;
            let mut row = Row {
                id,
                path: path.clone(),
                key,
                from: None,
                job: blank_job(path),
                size: 0,
                output: None,
                state: RowState::Queued,
            };
            self.refresh(&mut row, None);
            added.push(view(&row));
            self.rows.push(row);
        }
        added
    }

    /// Point rows at `to`. Rows that can't become it keep their target and
    /// count as skipped; running and finished rows are left alone.
    pub fn set_target(&mut self, ids: &[RowId], to: FormatId) -> SetTarget {
        let mut changed = Vec::new();
        let mut skipped = 0;
        for i in 0..self.rows.len() {
            if !ids.contains(&self.rows[i].id) || !editable(&self.rows[i].state) {
                continue;
            }
            let reachable = self.rows[i].job.op == Operation::Convert
                && self.rows[i]
                    .from
                    .is_some_and(|f| self.reg.find(Operation::Convert, f, Some(to)).is_some());
            if !reachable {
                skipped += 1;
                continue;
            }
            if let Some(f) = self.rows[i].from {
                self.last.insert(self.reg.format(f).kind, to);
            }
            let mut row = std::mem::replace(&mut self.rows[i], placeholder());
            self.refresh(&mut row, Some(to));
            changed.push(view(&row));
            self.rows[i] = row;
        }
        SetTarget { changed, skipped }
    }

    /// Every format at least one of these rows can become, sorted.
    pub fn targets(&self, ids: &[RowId]) -> Vec<FormatId> {
        let mut t: Vec<FormatId> = self
            .rows
            .iter()
            .filter(|r| ids.contains(&r.id) && r.job.op == Operation::Convert)
            .filter_map(|r| r.from)
            .flat_map(|f| self.reg.targets(Operation::Convert, f))
            .collect();
        t.sort();
        t.dedup();
        t
    }

    /// Remove rows that aren't running; returns the ids removed.
    pub fn remove(&mut self, ids: &[RowId]) -> Vec<RowId> {
        let mut removed = Vec::new();
        self.rows.retain(|r| {
            let go = ids.contains(&r.id) && !matches!(r.state, RowState::Running { .. });
            if go {
                removed.push(r.id);
            }
            !go
        });
        removed
    }

    pub fn rows(&self) -> Vec<RowView> {
        self.rows.iter().map(view).collect()
    }

    pub fn get(&self, id: RowId) -> Option<RowView> {
        self.rows.iter().find(|r| r.id == id).map(view)
    }

    /// The first queued row, now marked running, with its job.
    pub fn next_queued(&mut self) -> Option<(RowId, Job)> {
        let row = self.rows.iter_mut().find(|r| r.state == RowState::Queued)?;
        row.state = RowState::Running { fraction: None };
        Some((row.id, row.job.clone()))
    }

    pub fn update(&mut self, id: RowId, state: RowState) -> Option<RowView> {
        let row = self.rows.iter_mut().find(|r| r.id == id)?;
        if let RowState::Done { output, .. } = &state {
            row.output = Some(output.clone());
        }
        row.state = state;
        Some(view(row))
    }

    /// Put a failed or cancelled row back in the queue, previewing it again.
    pub fn requeue(&mut self, id: RowId) -> Option<RowView> {
        let i = self.rows.iter().position(|r| r.id == id)?;
        if !matches!(
            self.rows[i].state,
            RowState::Failed { .. } | RowState::Cancelled
        ) {
            return None;
        }
        let target = self.rows[i].job.target;
        let mut row = std::mem::replace(&mut self.rows[i], placeholder());
        self.refresh(&mut row, target);
        let v = view(&row);
        self.rows[i] = row;
        Some(v)
    }

    /// Detect the file, choose its job (with `target` if given, else the
    /// default for its kind) and preview it.
    fn refresh(&self, row: &mut Row, target: Option<FormatId>) {
        row.size = std::fs::metadata(&row.path).map(|m| m.len()).unwrap_or(0);
        row.output = None;
        let from = match persimmon_core::detect::detect(&row.path, self.reg.formats()) {
            Ok(d) => d.format,
            Err(e) => {
                row.from = None;
                row.state = RowState::Failed { error: shown(&e) };
                return;
            }
        };
        row.from = Some(from);
        let kind = self.reg.format(from).kind;
        row.job = blank_job(&row.path);
        if kind == MediaKind::Archive {
            row.job.op = Operation::Extract;
        } else {
            row.job.target = target.or_else(|| self.default_target(from, kind));
        }
        let plan = row.job.plan_with(&self.reg, PlanMode::Preview);
        match (plan.tasks.first(), plan.errors.first()) {
            (Some(task), _) => {
                row.output = Some(task.output.clone());
                row.state = RowState::Queued;
            }
            (None, Some(e)) => row.state = RowState::Failed { error: shown(e) },
            (None, None) => row.state = RowState::Queued,
        }
    }

    /// The last target picked for this kind, else the kind's default, else
    /// the first format this file can become: never its own format.
    fn default_target(&self, from: FormatId, kind: MediaKind) -> Option<FormatId> {
        let reachable = |t: FormatId| self.reg.find(Operation::Convert, from, Some(t)).is_some();
        let natural = match kind {
            MediaKind::Image => Some(FormatId::Jpeg),
            MediaKind::Video => Some(FormatId::Mp4),
            MediaKind::Audio => Some(FormatId::Mp3),
            MediaKind::Document => Some(FormatId::Png),
            MediaKind::Archive => None,
        };
        self.last
            .get(&kind)
            .copied()
            .filter(|&t| reachable(t))
            .or(natural.filter(|&t| reachable(t)))
            .or_else(|| self.reg.targets(Operation::Convert, from).first().copied())
    }
}

/// Queued, failed and cancelled rows can be pointed at another format.
fn editable(state: &RowState) -> bool {
    matches!(
        state,
        RowState::Queued | RowState::Failed { .. } | RowState::Cancelled
    )
}

fn blank_job(path: &Path) -> Job {
    Job {
        op: Operation::Convert,
        inputs: vec![path.to_path_buf()],
        target: None,
        // Beside the original; the app never overwrites (DD-42).
        output: OutputPolicy::default(),
        options: JobOptions::default(),
    }
}

fn placeholder() -> Row {
    Row {
        id: 0,
        path: PathBuf::new(),
        key: PathBuf::new(),
        from: None,
        job: blank_job(Path::new("")),
        size: 0,
        output: None,
        state: RowState::Queued,
    }
}

fn view(row: &Row) -> RowView {
    RowView {
        id: row.id,
        path: row.path.clone(),
        name: row
            .path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
        from: row.from,
        to: row.job.target,
        op: row.job.op,
        size: row.size,
        output: row.output.clone(),
        state: row.state.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use persimmon_core::error::ErrorCode;
    use persimmon_corpus::{PNG_1X1, ZIP_EMPTY, minimal_pdf};

    struct Dir(PathBuf);
    impl Dir {
        fn new(name: &str) -> Dir {
            let d =
                std::env::temp_dir().join(format!("prsm-app-queue-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&d);
            std::fs::create_dir_all(&d).unwrap();
            Dir(d)
        }
        fn put(&self, name: &str, bytes: &[u8]) -> PathBuf {
            let p = self.0.join(name);
            std::fs::write(&p, bytes).unwrap();
            p
        }
        fn listing(&self) -> Vec<String> {
            let mut v: Vec<String> = std::fs::read_dir(&self.0)
                .unwrap()
                .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                .collect();
            v.sort();
            v
        }
    }
    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn queue() -> Queue {
        Queue::new(Registry::builtin())
    }

    #[test]
    fn add_previews_outputs_beside_the_original() {
        let d = Dir::new("add");
        let p = d.put("photo.png", &PNG_1X1);
        let mut q = queue();
        let rows = q.add(std::slice::from_ref(&p));
        assert_eq!(rows.len(), 1);
        let r = &rows[0];
        assert_eq!(r.state, RowState::Queued);
        assert_eq!(
            (r.from, r.to, r.op),
            (
                Some(FormatId::Png),
                Some(FormatId::Jpeg),
                Operation::Convert
            )
        );
        assert_eq!(r.output.as_deref(), Some(d.0.join("photo.jpg").as_path()));
        assert_eq!(r.name, "photo.png");
        assert_eq!(r.size, PNG_1X1.len() as u64);
        assert_eq!(d.listing(), vec!["photo.png"]);
    }

    #[test]
    fn adding_a_path_twice_keeps_one_row() {
        let d = Dir::new("twice");
        let p = d.put("photo.png", &PNG_1X1);
        let mut q = queue();
        assert_eq!(q.add(std::slice::from_ref(&p)).len(), 1);
        assert_eq!(q.add(&[p.clone(), d.0.join("./photo.png")]).len(), 0);
        assert_eq!(q.rows().len(), 1);
    }

    #[test]
    fn default_target_follows_the_last_one_for_that_kind() {
        let d = Dir::new("last");
        let a = d.put("a.png", &PNG_1X1);
        let b = d.put("b.png", &PNG_1X1);
        let mut q = queue();
        let ids: Vec<RowId> = q.add(&[a, b]).iter().map(|r| r.id).collect();
        q.set_target(&ids, FormatId::Webp);
        let c = q.add(&[d.put("c.png", &PNG_1X1)]);
        assert_eq!(c[0].to, Some(FormatId::Webp));
        let pdf = q.add(&[d.put("doc.pdf", &minimal_pdf())]);
        assert_eq!(pdf[0].to, Some(FormatId::Png));
    }

    #[test]
    fn a_file_never_defaults_to_its_own_format() {
        let d = Dir::new("own");
        let png = d.put("a.png", &PNG_1X1);
        let mut q = queue();
        let ids: Vec<RowId> = q.add(&[png]).iter().map(|r| r.id).collect();
        q.set_target(&ids, FormatId::Webp);
        // A WebP made from it would default to webp, its own format: it
        // gets the first format it can become instead.
        let mut other = queue();
        other.last.insert(MediaKind::Image, FormatId::Png);
        let r = other.add(&[d.put("b.png", &PNG_1X1)]);
        assert_ne!(r[0].to, Some(FormatId::Png));
        assert_eq!(r[0].state, RowState::Queued);
    }

    #[test]
    fn set_target_skips_rows_that_cant_take_it() {
        let d = Dir::new("skip");
        let mut q = queue();
        let ids: Vec<RowId> = q
            .add(&[
                d.put("photo.png", &PNG_1X1),
                d.put("doc.pdf", &minimal_pdf()),
            ])
            .iter()
            .map(|r| r.id)
            .collect();
        let none = q.set_target(&ids, FormatId::Mp4);
        assert_eq!(none.skipped, 2);
        assert!(none.changed.is_empty());
        let some = q.set_target(&ids, FormatId::Webp);
        assert_eq!(some.skipped, 1);
        assert_eq!(some.changed.len(), 1);
        assert_eq!(some.changed[0].to, Some(FormatId::Webp));
        assert_eq!(
            some.changed[0].output.as_deref(),
            Some(d.0.join("photo.webp").as_path())
        );
        assert_eq!(q.get(ids[1]).unwrap().to, Some(FormatId::Png));
    }

    #[test]
    fn targets_is_the_union_for_the_rows() {
        let d = Dir::new("union");
        let mut q = queue();
        let ids: Vec<RowId> = q
            .add(&[
                d.put("photo.png", &PNG_1X1),
                d.put("doc.pdf", &minimal_pdf()),
            ])
            .iter()
            .map(|r| r.id)
            .collect();
        let t = q.targets(&ids);
        assert!(
            t.contains(&FormatId::Webp) && t.contains(&FormatId::Png),
            "{t:?}"
        );
        assert!(t.windows(2).all(|w| w[0] < w[1]));
    }

    #[test]
    fn a_missing_file_is_a_failed_row() {
        let d = Dir::new("missing");
        let mut q = queue();
        let r = q.add(&[d.0.join("gone.png")]);
        let RowState::Failed { error } = &r[0].state else {
            panic!("{:?}", r[0].state)
        };
        assert_eq!(error.code, ErrorCode::InputNotFound);
        assert!(!error.reason.contains("--"));
    }

    #[test]
    fn archives_extract() {
        let d = Dir::new("zip");
        let mut q = queue();
        let r = q.add(&[d.put("a.zip", &ZIP_EMPTY)]);
        assert_eq!((r[0].op, r[0].to), (Operation::Extract, None));
        assert_eq!(r[0].output.as_deref(), Some(d.0.join("a").as_path()));
    }

    #[test]
    fn remove_never_removes_a_running_row() {
        let d = Dir::new("remove");
        let mut q = queue();
        let ids: Vec<RowId> = q
            .add(&[d.put("a.png", &PNG_1X1), d.put("b.png", &PNG_1X1)])
            .iter()
            .map(|r| r.id)
            .collect();
        let (running, _) = q.next_queued().unwrap();
        assert_eq!(running, ids[0]);
        assert_eq!(q.remove(&ids), vec![ids[1]]);
        assert_eq!(q.rows().len(), 1);
    }

    #[test]
    fn update_and_requeue() {
        let d = Dir::new("requeue");
        let mut q = queue();
        let id = q.add(&[d.put("a.png", &PNG_1X1)])[0].id;
        q.next_queued().unwrap();
        let v = q.update(id, RowState::Cancelled).unwrap();
        assert_eq!(v.state, RowState::Cancelled);
        assert_eq!(q.requeue(id).unwrap().state, RowState::Queued);
        assert!(q.next_queued().is_some());
        assert!(q.requeue(id).is_none(), "a running row can't be requeued");
    }

    #[test]
    fn row_view_json_shape() {
        let d = Dir::new("json");
        let mut q = queue();
        let r = &q.add(&[d.put("a.png", &PNG_1X1)])[0];
        let v = serde_json::to_value(r).unwrap();
        assert_eq!(v["state"], "queued");
        assert_eq!(v["from"], "png");
        assert_eq!(v["to"], "jpeg");
        assert_eq!(v["op"], "convert");
        assert_eq!(v["name"], "a.png");
        let running = serde_json::to_value(RowState::Running {
            fraction: Some(0.5),
        })
        .unwrap();
        assert_eq!(
            running,
            serde_json::json!({"state": "running", "fraction": 0.5})
        );
    }
}
