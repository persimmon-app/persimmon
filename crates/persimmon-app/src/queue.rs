// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Persimmon contributors

//! The queue: one row per file, or one Combine row for many, each a job
//! previewed before it runs (spec D1, "How the queue behaves").

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use persimmon_core::error::ErrorCode;
use persimmon_core::format::{FormatId, MediaKind};
use persimmon_core::job::{Job, JobOptions, PlanMode};
use persimmon_core::output::OutputPolicy;
use persimmon_core::registry::{Operation, Registry};

use crate::wording::{Shown, app_error, shown};

pub type RowId = u64;

/// How newly added files become rows (spec D1, the sidebar's modes).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum QueueMode {
    #[default]
    Convert,
    Compress,
    Extract,
    Combine,
}

/// Where a row is.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(
    tag = "state",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum RowState {
    Queued,
    /// `fraction` is `None` while a backend can't tell (images).
    Running {
        fraction: Option<f32>,
    },
    /// `output` is also the row's own `output`, so only `outputSize` is
    /// serialised here: a flattened duplicate key would hide one of them.
    Done {
        #[serde(skip)]
        output: PathBuf,
        output_size: Option<u64>,
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
    /// The first input; a Combine row has more in `inputs`.
    #[serde(serialize_with = "lossy")]
    pub path: PathBuf,
    #[serde(serialize_with = "lossy_all")]
    pub inputs: Vec<PathBuf>,
    /// The file's name; a Combine row's output name.
    pub name: String,
    pub from: Option<FormatId>,
    pub to: Option<FormatId>,
    pub op: Operation,
    /// The input's size, or a Combine row's inputs' total.
    pub size: u64,
    #[serde(serialize_with = "lossy_opt")]
    pub output: Option<PathBuf>,
    /// Said under the name: where the output went when it fell back.
    pub note: Option<String>,
    /// Goes up with every change, so the window keeps the newest view.
    pub rev: u64,
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

fn lossy_all<S: serde::Serializer>(ps: &[PathBuf], s: S) -> Result<S::Ok, S::Error> {
    s.collect_seq(ps.iter().map(|p| p.to_string_lossy()))
}

struct Row {
    id: RowId,
    combine: bool,
    /// The target was picked, not defaulted: a Combine row keeps it as
    /// inputs are added; a defaulted one follows them.
    picked: bool,
    inputs: Vec<PathBuf>,
    keys: Vec<PathBuf>,
    from: Option<FormatId>,
    job: Job,
    size: u64,
    output: Option<PathBuf>,
    note: Option<String>,
    state: RowState,
    rev: u64,
}

/// The rows, the output policy, and the target last picked for each kind
/// of file.
pub struct Queue {
    reg: Registry,
    policy: OutputPolicy,
    rows: Vec<Row>,
    next_id: RowId,
    next_rev: u64,
    last: HashMap<MediaKind, FormatId>,
}

impl Queue {
    pub fn new(reg: Registry, policy: OutputPolicy) -> Queue {
        Queue {
            reg,
            policy,
            rows: Vec::new(),
            next_id: 1,
            next_rev: 1,
            last: HashMap::new(),
        }
    }

    /// A new output policy: queued and failed rows are previewed again.
    pub fn set_policy(&mut self, policy: OutputPolicy) -> Vec<RowView> {
        self.policy = policy;
        let mut changed = Vec::new();
        for i in 0..self.rows.len() {
            if editable(&self.rows[i].state) {
                let target = self.rows[i].job.target;
                changed.push(self.refresh_at(i, target));
            }
        }
        changed
    }

    /// Add files the way `mode` says; a path already queued isn't added
    /// again. Returns the rows added or changed.
    pub fn add(&mut self, paths: &[PathBuf], mode: QueueMode) -> Vec<RowView> {
        let mut fresh: Vec<(PathBuf, PathBuf)> = Vec::new();
        for path in paths {
            let key = std::fs::canonicalize(path).unwrap_or_else(|_| path.clone());
            let known = self.rows.iter().any(|r| r.keys.contains(&key))
                || fresh.iter().any(|(_, k)| *k == key);
            if !known {
                fresh.push((path.clone(), key));
            }
        }
        if fresh.is_empty() {
            return Vec::new();
        }
        if mode == QueueMode::Combine {
            let i = match self
                .rows
                .iter()
                .rposition(|r| r.combine && editable(&r.state))
            {
                Some(i) => i,
                None => {
                    let row = self.blank_row(true, mode);
                    self.rows.push(row);
                    self.rows.len() - 1
                }
            };
            for (path, key) in fresh {
                self.rows[i].inputs.push(path);
                self.rows[i].keys.push(key);
            }
            let target = self.rows[i].job.target.filter(|_| self.rows[i].picked);
            return vec![self.refresh_at(i, target)];
        }
        let mut added = Vec::new();
        for (path, key) in fresh {
            let mut row = self.blank_row(false, mode);
            row.inputs.push(path);
            row.keys.push(key);
            self.rows.push(row);
            let i = self.rows.len() - 1;
            added.push(self.refresh_at(i, None));
        }
        added
    }

    /// Point rows at `to`. Rows that can't become it keep their target and
    /// count as skipped; running, finished and cancelled rows are left alone.
    pub fn set_target(&mut self, ids: &[RowId], to: FormatId) -> SetTarget {
        let mut changed = Vec::new();
        let mut skipped = 0;
        for i in 0..self.rows.len() {
            if !ids.contains(&self.rows[i].id) || !editable(&self.rows[i].state) {
                continue;
            }
            if !self.targets_of(&self.rows[i]).contains(&to) {
                skipped += 1;
                continue;
            }
            if !self.rows[i].combine
                && let Some(f) = self.rows[i].from
            {
                self.last.insert(self.reg.format(f).kind, to);
            }
            self.rows[i].picked = true;
            changed.push(self.refresh_at(i, Some(to)));
        }
        SetTarget { changed, skipped }
    }

    /// Every format at least one of these rows can become, sorted.
    pub fn targets(&self, ids: &[RowId]) -> Vec<FormatId> {
        let mut t: Vec<FormatId> = self
            .rows
            .iter()
            .filter(|r| ids.contains(&r.id))
            .flat_map(|r| self.targets_of(r))
            .collect();
        t.sort();
        t.dedup();
        t
    }

    /// Move one of a Combine row's inputs.
    pub fn reorder(&mut self, id: RowId, from: usize, to: usize) -> Option<RowView> {
        let i = self.rows.iter().position(|r| r.id == id)?;
        let row = &mut self.rows[i];
        if !row.combine
            || !editable(&row.state)
            || from >= row.inputs.len()
            || to >= row.inputs.len()
        {
            return None;
        }
        let input = row.inputs.remove(from);
        row.inputs.insert(to, input);
        let key = row.keys.remove(from);
        row.keys.insert(to, key);
        let target = row.job.target;
        Some(self.refresh_at(i, target))
    }

    /// Take one input out of a Combine row. `None` when the row is gone:
    /// it had no inputs left, or there was no such row.
    pub fn remove_input(&mut self, id: RowId, index: usize) -> Option<RowView> {
        let i = self.rows.iter().position(|r| r.id == id)?;
        let row = &mut self.rows[i];
        if !row.combine || !editable(&row.state) || index >= row.inputs.len() {
            return Some(view(row));
        }
        row.inputs.remove(index);
        row.keys.remove(index);
        if row.inputs.is_empty() {
            self.rows.remove(i);
            return None;
        }
        let target = self.rows[i].job.target;
        Some(self.refresh_at(i, target))
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
        let rev = self.bump();
        let row = self.rows.iter_mut().find(|r| r.state == RowState::Queued)?;
        row.state = RowState::Running { fraction: None };
        row.rev = rev;
        Some((row.id, row.job.clone()))
    }

    pub fn update(&mut self, id: RowId, state: RowState) -> Option<RowView> {
        let rev = self.bump();
        let note = match &state {
            RowState::Done { output, .. } => self.fallback_note(id, output),
            _ => None,
        };
        let row = self.rows.iter_mut().find(|r| r.id == id)?;
        if let RowState::Done { output, .. } = &state {
            row.output = Some(output.clone());
            row.note = note;
        }
        row.state = state;
        row.rev = rev;
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
        Some(self.refresh_at(i, target))
    }

    fn bump(&mut self) -> u64 {
        self.next_rev += 1;
        self.next_rev
    }

    fn blank_row(&mut self, combine: bool, mode: QueueMode) -> Row {
        let id = self.next_id;
        self.next_id += 1;
        let mut job = blank_job(&self.policy);
        job.op = match mode {
            QueueMode::Convert => Operation::Convert,
            QueueMode::Compress => Operation::Compress,
            QueueMode::Extract => Operation::Extract,
            QueueMode::Combine => Operation::Merge,
        };
        Row {
            id,
            combine,
            picked: false,
            inputs: Vec::new(),
            keys: Vec::new(),
            from: None,
            job,
            size: 0,
            output: None,
            note: None,
            state: RowState::Queued,
            rev: 0,
        }
    }

    /// Detect, choose the job (with `target` if given), preview, and bump
    /// the row's revision. Returns its view.
    fn refresh_at(&mut self, i: usize, target: Option<FormatId>) -> RowView {
        let rev = self.bump();
        let mut row = std::mem::replace(&mut self.rows[i], placeholder());
        if row.combine {
            self.refresh_combine(&mut row, target);
        } else {
            self.refresh_one(&mut row, target);
        }
        row.rev = rev;
        let v = view(&row);
        self.rows[i] = row;
        v
    }

    fn refresh_one(&self, row: &mut Row, target: Option<FormatId>) {
        let path = row.inputs[0].clone();
        row.size = std::fs::metadata(&path)
            .ok()
            .filter(|m| m.is_file())
            .map(|m| m.len())
            .unwrap_or(0);
        row.output = None;
        row.note = None;
        row.from = None;
        row.job.inputs = vec![path.clone()];
        row.job.output = self.policy.clone();
        if let Some(refused) = refusal(&path, false) {
            row.state = RowState::Failed { error: refused };
            return;
        }
        let from = match persimmon_core::detect::detect(&path, self.reg.formats()) {
            Ok(d) => d.format,
            Err(e) => {
                row.state = RowState::Failed { error: shown(&e) };
                return;
            }
        };
        row.from = Some(from);
        let kind = self.reg.format(from).kind;
        // Convert mode sends archives to Extract (spec D1).
        if row.job.op == Operation::Convert && kind == MediaKind::Archive {
            row.job.op = Operation::Extract;
        }
        row.job.target = match row.job.op {
            Operation::Convert => target.or_else(|| self.default_target(from, kind)),
            _ => None,
        };
        self.preview(row);
    }

    fn refresh_combine(&self, row: &mut Row, target: Option<FormatId>) {
        row.output = None;
        row.note = None;
        row.from = None;
        row.size = row
            .inputs
            .iter()
            .filter_map(|p| std::fs::metadata(p).ok().filter(|m| m.is_file()))
            .map(|m| m.len())
            .sum();
        row.job.op = Operation::Merge;
        row.job.inputs = row.inputs.clone();
        row.job.output = self.policy.clone();
        if let Some(refused) = row.inputs.iter().find_map(|p| refusal(p, true)) {
            row.state = RowState::Failed { error: refused };
            return;
        }
        row.job.target = target.or_else(|| {
            Some(
                if self.combine_targets(&row.inputs).contains(&FormatId::Pdf) {
                    FormatId::Pdf
                } else {
                    FormatId::Zip
                },
            )
        });
        self.preview(row);
    }

    fn preview(&self, row: &mut Row) {
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

    fn targets_of(&self, row: &Row) -> Vec<FormatId> {
        if row.combine {
            return self.combine_targets(&row.inputs);
        }
        match (row.job.op, row.from) {
            (Operation::Convert, Some(f)) => self.reg.targets(Operation::Convert, f),
            _ => Vec::new(),
        }
    }

    /// What these inputs can be combined into: ZIP and tar.gz always, PDF
    /// when every input is a file PDF can take.
    fn combine_targets(&self, inputs: &[PathBuf]) -> Vec<FormatId> {
        let pdf = !inputs.is_empty()
            && inputs.iter().all(|p| {
                p.is_file()
                    && persimmon_core::detect::detect(p, self.reg.formats()).is_ok_and(|d| {
                        self.reg
                            .find(Operation::Merge, d.format, Some(FormatId::Pdf))
                            .is_some()
                    })
            });
        let mut t = vec![FormatId::Zip, FormatId::TarGz];
        if pdf {
            t.push(FormatId::Pdf);
        }
        t.sort();
        t
    }

    /// The last target picked for this kind, else the first of the kind's
    /// usual formats this file can become, else anything it can become:
    /// never its own format, and a video never defaults to GIF.
    fn default_target(&self, from: FormatId, kind: MediaKind) -> Option<FormatId> {
        let reachable = |t: &FormatId| self.reg.find(Operation::Convert, from, Some(*t)).is_some();
        let usual: &[FormatId] = match kind {
            MediaKind::Image => &[FormatId::Jpeg, FormatId::Png, FormatId::Webp],
            MediaKind::Video => &[FormatId::Mp4, FormatId::Mov, FormatId::Webm],
            MediaKind::Audio => &[FormatId::Mp3, FormatId::M4a],
            MediaKind::Document => &[FormatId::Png, FormatId::Jpeg],
            MediaKind::Archive => &[],
        };
        self.last
            .get(&kind)
            .filter(|t| reachable(t))
            .or_else(|| usual.iter().find(|t| reachable(t)))
            .copied()
            .or_else(|| self.reg.targets(Operation::Convert, from).first().copied())
    }

    /// When a done row's output isn't beside its first input and the user
    /// asked for "beside", it fell back: say where it went.
    fn fallback_note(&self, id: RowId, output: &Path) -> Option<String> {
        let row = self.rows.iter().find(|r| r.id == id)?;
        let fallback = self.policy.fallback.as_ref()?;
        let source_dir = row.inputs.first()?.parent()?;
        let out_dir = output.parent()?;
        (self.policy.dir.is_none() && out_dir != source_dir).then(|| {
            format!(
                "Saved to {} — the original's folder isn't writable",
                tilde(fallback)
            )
        })
    }
}

/// Inputs the app refuses before the engine sees them (spec D1, D2).
fn refusal(path: &Path, combine: bool) -> Option<Shown> {
    let from_photos = path.ancestors().any(|a| {
        a.file_name()
            .is_some_and(|n| n.to_string_lossy().ends_with(".photoslibrary"))
    });
    if from_photos {
        return Some(app_error(
            ErrorCode::UnsupportedInput,
            path,
            "This is Photos' preview, not your original",
            Some(
                "In Photos, choose File → Export → Export Unmodified Original, then drop the exported file.",
            ),
        ));
    }
    if !combine && path.is_dir() {
        return Some(app_error(
            ErrorCode::UnsupportedInput,
            path,
            "Folders can only be combined into an archive in this version",
            Some("Open the folder and drop the files inside"),
        ));
    }
    None
}

/// A path with the home folder shown as `~`.
pub fn tilde(path: &Path) -> String {
    match std::env::var_os("HOME").map(PathBuf::from) {
        Some(home) if path.starts_with(&home) => match path.strip_prefix(&home) {
            Ok(rest) if rest.as_os_str().is_empty() => "~".into(),
            Ok(rest) => format!("~/{}", rest.to_string_lossy()),
            Err(_) => path.to_string_lossy().into_owned(),
        },
        _ => path.to_string_lossy().into_owned(),
    }
}

/// Queued and failed rows can be pointed at another format. A cancelled
/// row stays cancelled until retried: picking a format mustn't run it.
fn editable(state: &RowState) -> bool {
    matches!(state, RowState::Queued | RowState::Failed { .. })
}

fn blank_job(policy: &OutputPolicy) -> Job {
    Job {
        op: Operation::Convert,
        inputs: Vec::new(),
        target: None,
        // The app never overwrites (DD-42).
        output: OutputPolicy {
            overwrite: false,
            ..policy.clone()
        },
        options: JobOptions::default(),
    }
}

fn placeholder() -> Row {
    Row {
        id: 0,
        combine: false,
        picked: false,
        inputs: Vec::new(),
        keys: Vec::new(),
        from: None,
        job: blank_job(&OutputPolicy::default()),
        size: 0,
        output: None,
        note: None,
        state: RowState::Queued,
        rev: 0,
    }
}

fn view(row: &Row) -> RowView {
    let file_name = |p: &Path| {
        p.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
    };
    let name = if row.combine {
        row.output
            .as_deref()
            .map(file_name)
            .unwrap_or_else(|| "Combined".into())
    } else {
        row.inputs.first().map(|p| file_name(p)).unwrap_or_default()
    };
    RowView {
        id: row.id,
        path: row.inputs.first().cloned().unwrap_or_default(),
        inputs: row.inputs.clone(),
        name,
        from: row.from,
        to: row.job.target,
        op: row.job.op,
        size: row.size,
        output: row.output.clone(),
        note: row.note.clone(),
        rev: row.rev,
        state: row.state.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use persimmon_core::error::ErrorCode;
    use persimmon_corpus::{PNG_1X1, WAV_HEADER, ZIP_EMPTY, minimal_pdf};

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
        Queue::new(Registry::builtin(), OutputPolicy::default())
    }

    #[test]
    fn add_previews_outputs_beside_the_original() {
        let d = Dir::new("add");
        let p = d.put("photo.png", &PNG_1X1);
        let mut q = queue();
        let rows = q.add(std::slice::from_ref(&p), QueueMode::Convert);
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
        assert_eq!(q.add(std::slice::from_ref(&p), QueueMode::Convert).len(), 1);
        assert_eq!(
            q.add(&[p.clone(), d.0.join("./photo.png")], QueueMode::Convert)
                .len(),
            0
        );
        assert_eq!(q.rows().len(), 1);
    }

    #[test]
    fn default_target_follows_the_last_one_for_that_kind() {
        let d = Dir::new("last");
        let a = d.put("a.png", &PNG_1X1);
        let b = d.put("b.png", &PNG_1X1);
        let mut q = queue();
        let ids: Vec<RowId> = q
            .add(&[a, b], QueueMode::Convert)
            .iter()
            .map(|r| r.id)
            .collect();
        q.set_target(&ids, FormatId::Webp);
        let c = q.add(&[d.put("c.png", &PNG_1X1)], QueueMode::Convert);
        assert_eq!(c[0].to, Some(FormatId::Webp));
        let pdf = q.add(&[d.put("doc.pdf", &minimal_pdf())], QueueMode::Convert);
        assert_eq!(pdf[0].to, Some(FormatId::Png));
    }

    #[test]
    fn a_file_never_defaults_to_its_own_format() {
        let d = Dir::new("own");
        let png = d.put("a.png", &PNG_1X1);
        let mut q = queue();
        let ids: Vec<RowId> = q
            .add(&[png], QueueMode::Convert)
            .iter()
            .map(|r| r.id)
            .collect();
        q.set_target(&ids, FormatId::Webp);
        // A WebP made from it would default to webp, its own format: it
        // gets the first format it can become instead.
        let mut other = queue();
        other.last.insert(MediaKind::Image, FormatId::Png);
        let r = other.add(&[d.put("b.png", &PNG_1X1)], QueueMode::Convert);
        assert_ne!(r[0].to, Some(FormatId::Png));
        assert_eq!(r[0].state, RowState::Queued);
    }

    #[test]
    fn set_target_skips_rows_that_cant_take_it() {
        let d = Dir::new("skip");
        let mut q = queue();
        let ids: Vec<RowId> = q
            .add(
                &[
                    d.put("photo.png", &PNG_1X1),
                    d.put("doc.pdf", &minimal_pdf()),
                ],
                QueueMode::Convert,
            )
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
            .add(
                &[
                    d.put("photo.png", &PNG_1X1),
                    d.put("doc.pdf", &minimal_pdf()),
                ],
                QueueMode::Convert,
            )
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
        let r = q.add(&[d.0.join("gone.png")], QueueMode::Convert);
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
        let r = q.add(&[d.put("a.zip", &ZIP_EMPTY)], QueueMode::Convert);
        assert_eq!((r[0].op, r[0].to), (Operation::Extract, None));
        assert_eq!(r[0].output.as_deref(), Some(d.0.join("a").as_path()));
    }

    #[test]
    fn remove_never_removes_a_running_row() {
        let d = Dir::new("remove");
        let mut q = queue();
        let ids: Vec<RowId> = q
            .add(
                &[d.put("a.png", &PNG_1X1), d.put("b.png", &PNG_1X1)],
                QueueMode::Convert,
            )
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
        let id = q.add(&[d.put("a.png", &PNG_1X1)], QueueMode::Convert)[0].id;
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
        let r = q.add(&[d.put("a.png", &PNG_1X1)], QueueMode::Convert)[0].clone();
        let v = serde_json::to_value(&r).unwrap();
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
        // A done row: one "size" (the source's) and one "output" (the
        // path), never a duplicate key from the flattened state.
        let done = q
            .update(
                r.id,
                RowState::Done {
                    output: d.0.join("a.jpg"),
                    output_size: Some(7),
                },
            )
            .unwrap();
        let text = serde_json::to_string(&done).unwrap();
        assert_eq!(text.matches("\"size\"").count(), 1, "{text}");
        assert_eq!(text.matches("\"output\"").count(), 1, "{text}");
        let v: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(v["outputSize"], 7);
        assert_eq!(v["size"], PNG_1X1.len() as u64);
    }

    #[test]
    fn video_never_defaults_to_gif() {
        // Sniffed as MP4 from its header; MP4 can't become MP4, and the
        // fallback must stay a video format, never a whole-video GIF.
        let d = Dir::new("mp4");
        let mut head = b"\x00\x00\x00\x18ftypisom\x00\x00\x02\x00isomiso2".to_vec();
        head.resize(64, 0);
        let r = queue().add(&[d.put("clip.mp4", &head)], QueueMode::Convert);
        assert_eq!(r[0].from, Some(FormatId::Mp4), "{:?}", r[0]);
        assert_eq!(r[0].to, Some(FormatId::Mov));
    }

    #[test]
    fn picking_a_format_leaves_cancelled_rows_cancelled() {
        let d = Dir::new("cancelled");
        let mut q = queue();
        let ids: Vec<RowId> = q
            .add(
                &[d.put("a.png", &PNG_1X1), d.put("b.png", &PNG_1X1)],
                QueueMode::Convert,
            )
            .iter()
            .map(|r| r.id)
            .collect();
        q.update(ids[0], RowState::Cancelled);
        let result = q.set_target(&ids, FormatId::Webp);
        assert_eq!(result.changed.len(), 1);
        assert_eq!(result.skipped, 0);
        assert_eq!(q.get(ids[0]).unwrap().state, RowState::Cancelled);
    }

    #[test]
    fn compress_mode_keeps_the_format() {
        let d = Dir::new("compress");
        let r = queue().add(&[d.put("photo.png", &PNG_1X1)], QueueMode::Compress);
        assert_eq!((r[0].op, r[0].to), (Operation::Compress, None));
        assert_eq!(
            r[0].output.as_deref(),
            Some(d.0.join("photo (compressed).png").as_path())
        );
    }

    #[test]
    fn extract_mode_on_a_png_fails_with_the_engines_reason() {
        let d = Dir::new("extract-png");
        let r = queue().add(&[d.put("photo.png", &PNG_1X1)], QueueMode::Extract);
        let RowState::Failed { error } = &r[0].state else {
            panic!("{:?}", r[0].state)
        };
        assert_eq!(error.code, ErrorCode::UnsupportedConversion);
    }

    #[test]
    fn combine_collects_into_one_row() {
        let d = Dir::new("combine");
        let mut q = queue();
        q.add(
            &[d.put("a.png", &PNG_1X1), d.put("b.png", &PNG_1X1)],
            QueueMode::Combine,
        );
        let r = q.add(&[d.put("c.pdf", &minimal_pdf())], QueueMode::Combine);
        assert_eq!(q.rows().len(), 1);
        assert_eq!(
            r[0].inputs,
            vec![d.0.join("a.png"), d.0.join("b.png"), d.0.join("c.pdf")]
        );
        assert_eq!((r[0].op, r[0].to), (Operation::Merge, Some(FormatId::Pdf)));
        assert_eq!(r[0].name, "merged.pdf");
        assert_eq!(r[0].state, RowState::Queued);
    }

    #[test]
    fn combine_target_follows_its_inputs() {
        let d = Dir::new("combine-wav");
        let mut q = queue();
        q.add(&[d.put("a.png", &PNG_1X1)], QueueMode::Combine);
        let r = q.add(&[d.put("b.wav", &WAV_HEADER)], QueueMode::Combine);
        // A WAV can't go into a PDF: the row now defaults to ZIP.
        assert_eq!(r[0].to, Some(FormatId::Zip));
        assert_eq!(q.targets(&[r[0].id]), vec![FormatId::Zip, FormatId::TarGz]);
        let pdf = q.set_target(&[r[0].id], FormatId::Pdf);
        assert_eq!(pdf.skipped, 1);
        let gz = q.set_target(&[r[0].id], FormatId::TarGz);
        assert_eq!(gz.changed[0].name, "merged.tar.gz");
    }

    #[test]
    fn combine_with_a_file_pdf_cant_take_fails_until_retargeted() {
        let d = Dir::new("combine-fail");
        let mut q = queue();
        let id = q.add(&[d.put("a.png", &PNG_1X1)], QueueMode::Combine)[0].id;
        assert_eq!(q.get(id).unwrap().to, Some(FormatId::Pdf));
        // Kept at PDF by an explicit pick, a WAV makes it fail at preview.
        q.set_target(&[id], FormatId::Pdf);
        let r = q.add(&[d.put("b.wav", &WAV_HEADER)], QueueMode::Combine);
        let RowState::Failed { error } = &r[0].state else {
            panic!("{:?}", r[0].state)
        };
        assert!(!error.reason.contains("--"));
        let zip = q.set_target(&[id], FormatId::Zip);
        assert_eq!(zip.changed[0].state, RowState::Queued);
    }

    #[test]
    fn combine_reorders_and_removes_inputs() {
        let d = Dir::new("reorder");
        let mut q = queue();
        let (a, b, c) = (
            d.put("a.png", &PNG_1X1),
            d.put("b.png", &PNG_1X1),
            d.put("c.png", &PNG_1X1),
        );
        let id = q.add(&[a.clone(), b.clone(), c.clone()], QueueMode::Combine)[0].id;
        assert_eq!(
            q.reorder(id, 2, 0).unwrap().inputs,
            vec![c.clone(), a.clone(), b.clone()]
        );
        assert_eq!(
            q.remove_input(id, 1).unwrap().inputs,
            vec![c.clone(), b.clone()]
        );
        assert!(q.remove_input(id, 0).is_some());
        assert!(
            q.remove_input(id, 0).is_none(),
            "the row is removed with its last input"
        );
        assert!(q.rows().is_empty());
    }

    #[test]
    fn a_new_combine_row_after_start() {
        let d = Dir::new("combine-new");
        let mut q = queue();
        q.add(&[d.put("a.png", &PNG_1X1)], QueueMode::Combine);
        q.next_queued().unwrap();
        q.add(&[d.put("b.png", &PNG_1X1)], QueueMode::Combine);
        assert_eq!(q.rows().len(), 2);
    }

    #[test]
    fn folders_only_in_combine() {
        let d = Dir::new("folders");
        let folder = d.0.join("photos");
        std::fs::create_dir(&folder).unwrap();
        std::fs::write(folder.join("x.png"), PNG_1X1).unwrap();
        let mut q = queue();
        let r = q.add(std::slice::from_ref(&folder), QueueMode::Convert);
        let RowState::Failed { error } = &r[0].state else {
            panic!("{:?}", r[0].state)
        };
        assert_eq!(
            error.reason,
            "Folders can only be combined into an archive in this version"
        );
        assert_eq!(
            error.fix.as_deref(),
            Some("Open the folder and drop the files inside")
        );
        let mut q = queue();
        let r = q.add(&[folder], QueueMode::Combine);
        assert_eq!(r[0].state, RowState::Queued);
        assert_eq!(q.targets(&[r[0].id]), vec![FormatId::Zip, FormatId::TarGz]);
    }

    #[test]
    fn photos_library_paths_are_refused() {
        let d = Dir::new("photos");
        let lib =
            d.0.join("Photos Library.photoslibrary/resources/derivatives");
        std::fs::create_dir_all(&lib).unwrap();
        let p = lib.join("x.jpeg");
        std::fs::write(&p, PNG_1X1).unwrap();
        let r = queue().add(&[p], QueueMode::Convert);
        let RowState::Failed { error } = &r[0].state else {
            panic!("{:?}", r[0].state)
        };
        assert_eq!(error.reason, "This is Photos' preview, not your original");
        assert_eq!(
            error.fix.as_deref(),
            Some(
                "In Photos, choose File → Export → Export Unmodified Original, then drop the exported file."
            )
        );
    }

    #[test]
    fn revisions_only_go_up() {
        let d = Dir::new("revs");
        let mut q = queue();
        let a = q.add(&[d.put("a.png", &PNG_1X1)], QueueMode::Convert)[0].clone();
        let b = q.set_target(&[a.id], FormatId::Webp).changed[0].clone();
        q.next_queued().unwrap();
        let c = q.get(a.id).unwrap();
        let e = q.update(a.id, RowState::Cancelled).unwrap();
        let f = q.requeue(a.id).unwrap();
        let revs = [a.rev, b.rev, c.rev, e.rev, f.rev];
        assert!(revs.windows(2).all(|w| w[0] < w[1]), "{revs:?}");
    }

    #[test]
    fn tilde_shows_home_as_a_tilde() {
        let home = PathBuf::from(std::env::var_os("HOME").unwrap());
        assert_eq!(
            tilde(&home.join("Downloads/Persimmon")),
            "~/Downloads/Persimmon"
        );
        assert_eq!(tilde(Path::new("/Volumes/x")), "/Volumes/x");
    }
}
