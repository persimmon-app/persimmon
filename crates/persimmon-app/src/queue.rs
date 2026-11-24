// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Persimmon contributors

//! The queue: one row per file, or one Combine row for many, each a job
//! previewed before it runs (spec D1, "How the queue behaves").

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use persimmon_core::error::ErrorCode;
use persimmon_core::format::{FormatId, MediaKind};
use persimmon_core::inputs::{expand_folder, is_package};
use persimmon_core::job::{Job, JobOptions, PlanMode};
use persimmon_core::output::OutputPolicy;
use persimmon_core::registry::{Operation, Registry};

use crate::presets::{Preset, kinds, operation};
use crate::wording::{Shown, app_error, shown};

pub type RowId = u64;

/// Why files inside a dropped folder were left out.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum SkipWhy {
    /// Convert mode: nothing can be made from them.
    Convert,
    /// A preset's target: they can't become it.
    Become { name: String },
    /// Compress mode: they can't be compressed.
    Compress,
}

/// The status bar's note after files inside a folder were left out.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct SkipNote {
    pub count: usize,
    pub why: SkipWhy,
}

/// What the status bar says after an add: files that were already queued
/// (counted after folders are expanded), and files a folder left out.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
pub struct AddNote {
    pub already: usize,
    pub skipped: Option<SkipNote>,
}

/// How new files become rows: a mode, or a preset (spec D4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Recipe {
    Mode(QueueMode),
    Preset(Preset),
}

impl Recipe {
    pub fn mode(&self) -> QueueMode {
        match self {
            Recipe::Mode(m) => *m,
            Recipe::Preset(p) => p.mode,
        }
    }
}

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
    /// A file still coming from Photos (spec D5): no input yet. `slow`
    /// once it has taken a while, which means an iCloud download.
    Receiving {
        slow: bool,
    },
}

/// What became of a promised file that arrived.
#[derive(Debug, Clone, PartialEq)]
pub enum Received {
    /// Its row now holds it; `start` when the row should run at once.
    Queued { view: Box<RowView>, start: bool },
    /// Its row was cancelled or removed meanwhile: the file isn't wanted.
    Dropped,
}

/// A receiving row's name when Photos gave none.
const PHOTO_NAME: &str = "Photo from Photos";

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
    /// The name of the preset applied, if any.
    pub preset: Option<String>,
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
    preset: Option<Preset>,
    inputs: Vec<PathBuf>,
    keys: Vec<PathBuf>,
    from: Option<FormatId>,
    job: Job,
    size: u64,
    output: Option<PathBuf>,
    note: Option<String>,
    state: RowState,
    rev: u64,
    /// A promised file's staging folder (spec D5): its output never goes
    /// there, and the folder goes when the row does.
    staged: Option<PathBuf>,
    /// Run as soon as the file arrives (a drop on the drop panel, or Start
    /// while it was still coming).
    start_when_ready: bool,
    /// The name shown while the file is still coming.
    waiting: Option<String>,
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
    /// What the last add has to say: already queued, and left out.
    note: AddNote,
    /// Staging folders of rows removed since last asked.
    staged_removed: Vec<PathBuf>,
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
            note: AddNote::default(),
            staged_removed: Vec::new(),
        }
    }

    /// A new output policy: queued and failed rows are previewed again.
    pub fn set_policy(&mut self, policy: OutputPolicy) -> Vec<RowView> {
        self.policy = policy;
        let mut changed = Vec::new();
        for i in 0..self.rows.len() {
            // Queued rows, and rows that failed because their folder can't
            // be written, are planned again; a file that failed for its own
            // reasons (damaged, unsupported) stays failed.
            let replan = match &self.rows[i].state {
                RowState::Queued => true,
                RowState::Failed { error } => error.code == ErrorCode::OutputNotWritable,
                _ => false,
            };
            if replan {
                let target = self.rows[i].job.target;
                changed.push(self.refresh_at(i, target));
            }
        }
        changed
    }

    /// How many of `paths` a row already holds, so adding them again does
    /// nothing; the window says so rather than staying silent.
    pub fn already_queued(&self, paths: &[PathBuf]) -> usize {
        let mut seen = std::collections::HashSet::new();
        paths
            .iter()
            .map(|p| std::fs::canonicalize(p).unwrap_or_else(|_| p.clone()))
            .filter(|k| seen.insert(k.clone()))
            .filter(|k| self.rows.iter().any(|r| r.keys.contains(k)))
            .count()
    }

    /// A new Combine row of exactly `paths`, never joining the one still
    /// queued: Retry from History puts a combine back as it was.
    pub fn add_combine_row(&mut self, paths: &[PathBuf], recipe: &Recipe) -> Vec<RowView> {
        if paths.is_empty() {
            return Vec::new();
        }
        let mut row = self.blank_row(true, recipe);
        for p in paths {
            row.inputs.push(p.clone());
            row.keys
                .push(std::fs::canonicalize(p).unwrap_or_else(|_| p.clone()));
        }
        let target = row.preset.as_ref().and_then(|p| p.target);
        self.rows.push(row);
        let i = self.rows.len() - 1;
        vec![self.refresh_at(i, target)]
    }

    /// Add files the way `mode` says.
    pub fn add(&mut self, paths: &[PathBuf], mode: QueueMode) -> Vec<RowView> {
        self.add_with(paths, &Recipe::Mode(mode))
    }

    /// Add files the way `recipe` says; a path already queued isn't added
    /// again. Returns the rows added or changed.
    pub fn add_with(&mut self, paths: &[PathBuf], recipe: &Recipe) -> Vec<RowView> {
        let mode = recipe.mode();
        let paths = self.expand(paths, recipe);
        let queued: HashSet<PathBuf> = self
            .rows
            .iter()
            .flat_map(|r| r.keys.iter().cloned())
            .collect();
        let mut seen = HashSet::new();
        let mut fresh: Vec<(PathBuf, PathBuf)> = Vec::new();
        for path in &paths {
            let key = std::fs::canonicalize(path).unwrap_or_else(|_| path.clone());
            if queued.contains(&key) {
                self.note.already += 1;
            } else if seen.insert(key.clone()) {
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
                    let row = self.blank_row(true, recipe);
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
            let mut row = self.blank_row(false, recipe);
            row.inputs.push(path);
            row.keys.push(key);
            let target = row.preset.as_ref().and_then(|p| p.target);
            self.rows.push(row);
            let i = self.rows.len() - 1;
            added.push(self.refresh_at(i, target));
        }
        added
    }

    /// What the last add has to say, once; `None` when there's nothing.
    pub fn take_note(&mut self) -> Option<AddNote> {
        let note = std::mem::take(&mut self.note);
        (note.already > 0 || note.skipped.is_some()).then_some(note)
    }

    /// Paths with each folder, in Convert or Compress, replaced by the
    /// files inside it, subfolders included (a drop means "this folder's
    /// contents"). A folder that yields nothing, and each subfolder that
    /// can't be read, stays as a path whose row fails and says why.
    fn expand(&mut self, paths: &[PathBuf], recipe: &Recipe) -> Vec<PathBuf> {
        self.note = AddNote::default();
        let (op, target) = folder_scope(recipe);
        let mut out = Vec::new();
        let mut count = 0;
        for path in paths {
            if !path.is_dir()
                || is_package(path)
                || !matches!(op, Operation::Convert | Operation::Compress)
            {
                out.push(path.clone());
                continue;
            }
            match expand_folder(&self.reg, op, target, path, true) {
                Ok(ff) => {
                    out.extend(ff.files);
                    out.extend(ff.errors.into_iter().filter_map(|e| e.file));
                    count += ff.skipped.len();
                }
                Err(_) => out.push(path.clone()),
            }
        }
        if count > 0 {
            let why = match (op, target) {
                (Operation::Compress, _) => SkipWhy::Compress,
                (_, Some(to)) => SkipWhy::Become {
                    name: self.reg.format(to).name.to_string(),
                },
                (_, None) => SkipWhy::Convert,
            };
            self.note.skipped = Some(SkipNote { count, why });
        }
        out
    }

    /// Point rows at `to`, dropping any preset on them. Rows that can't
    /// become it keep their target and count as skipped; running, finished
    /// and cancelled rows are left alone.
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
            // A picked output replaces the row's preset, options and all.
            self.rows[i].picked = true;
            self.rows[i].preset = None;
            changed.push(self.refresh_at(i, Some(to)));
        }
        SetTarget { changed, skipped }
    }

    /// Apply a preset to queued and failed rows. Rows it doesn't fit fail
    /// with an error that names it and what it's for.
    pub fn apply_preset(&mut self, ids: &[RowId], preset: &Preset) -> Vec<RowView> {
        let mut changed = Vec::new();
        for i in 0..self.rows.len() {
            if !ids.contains(&self.rows[i].id) || !editable(&self.rows[i].state) {
                continue;
            }
            self.rows[i].preset = Some(preset.clone());
            self.rows[i].picked = true;
            changed.push(self.refresh_at(i, preset.target));
        }
        changed
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
        // A picked target sticks; a defaulted one follows the inputs.
        let target = row.job.target.filter(|_| row.picked);
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
        let target = self.rows[i].job.target.filter(|_| self.rows[i].picked);
        Some(self.refresh_at(i, target))
    }

    /// Remove rows that aren't running; returns the ids removed.
    pub fn remove(&mut self, ids: &[RowId]) -> Vec<RowId> {
        let mut removed = Vec::new();
        let mut staged = Vec::new();
        self.rows.retain(|r| {
            let go = ids.contains(&r.id) && !matches!(r.state, RowState::Running { .. });
            if go {
                removed.push(r.id);
                staged.extend(r.staged.clone());
            }
            !go
        });
        self.staged_removed.extend(staged);
        removed
    }

    /// The staging folders of rows removed since last asked, to delete.
    pub fn take_staged_removed(&mut self) -> Vec<PathBuf> {
        std::mem::take(&mut self.staged_removed)
    }

    /// A row for a file still coming from Photos, added the way `recipe`
    /// says once it arrives.
    pub fn add_receiving(
        &mut self,
        name: &str,
        recipe: &Recipe,
        start_when_ready: bool,
    ) -> RowView {
        let mut row = self.blank_row(false, recipe);
        let name = name.trim();
        row.waiting = Some(if name.is_empty() {
            PHOTO_NAME.into()
        } else {
            name.into()
        });
        row.start_when_ready = start_when_ready;
        row.state = RowState::Receiving { slow: false };
        row.rev = self.bump();
        let v = view(&row);
        self.rows.push(row);
        v
    }

    /// A receiving row that has taken a while: it's downloading.
    pub fn slow(&mut self, id: RowId) -> Option<RowView> {
        let rev = self.bump();
        let row = self.rows.iter_mut().find(|r| r.id == id)?;
        if !matches!(row.state, RowState::Receiving { .. }) {
            return None;
        }
        row.state = RowState::Receiving { slow: true };
        row.rev = rev;
        Some(view(row))
    }

    /// The promised file for row `id` is at `path`.
    pub fn received(&mut self, id: RowId, path: &Path) -> Received {
        let Some(i) = self
            .rows
            .iter()
            .position(|r| r.id == id && matches!(r.state, RowState::Receiving { .. }))
        else {
            return Received::Dropped;
        };
        let row = &mut self.rows[i];
        row.inputs = vec![path.to_path_buf()];
        row.keys = vec![std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())];
        row.staged = path.parent().map(Path::to_path_buf);
        row.waiting = None;
        let start = row.start_when_ready;
        let target = row.preset.as_ref().and_then(|p| p.target);
        let view = Box::new(self.refresh_at(i, target));
        Received::Queued { view, start }
    }

    /// The promised file for row `id` never came.
    pub fn receive_failed(&mut self, id: RowId) -> Option<RowView> {
        let rev = self.bump();
        let row = self.rows.iter_mut().find(|r| r.id == id)?;
        if !matches!(row.state, RowState::Receiving { .. }) {
            return None;
        }
        let name = row.waiting.clone().unwrap_or_else(|| PHOTO_NAME.into());
        row.state = RowState::Failed {
            error: app_error(
                ErrorCode::InputNotFound,
                Path::new(&name),
                "The original couldn't be downloaded from iCloud",
                Some("Open it in Photos to download it, then try again"),
            ),
        };
        row.rev = rev;
        Some(view(row))
    }

    /// Stop waiting for a file still coming: the row is cancelled, and the
    /// file is dropped if it arrives.
    pub fn cancel_receiving(&mut self, id: RowId) -> Option<RowView> {
        let rev = self.bump();
        let row = self.rows.iter_mut().find(|r| r.id == id)?;
        if !matches!(row.state, RowState::Receiving { .. }) {
            return None;
        }
        row.state = RowState::Cancelled;
        row.rev = rev;
        Some(view(row))
    }

    /// Start was asked for: files still coming run when they arrive.
    pub fn start_requested(&mut self) {
        for row in &mut self.rows {
            if matches!(row.state, RowState::Receiving { .. }) {
                row.start_when_ready = true;
            }
        }
    }

    /// A row's staging folder, if its file came from Photos.
    pub fn staged(&self, id: RowId) -> Option<PathBuf> {
        self.rows.iter().find(|r| r.id == id)?.staged.clone()
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

    /// The first queued row `ok` accepts, now marked running, with its job.
    pub fn next_queued_where(
        &mut self,
        mut ok: impl FnMut(&RowView) -> bool,
    ) -> Option<(RowId, Job)> {
        let i = self
            .rows
            .iter()
            .position(|r| r.state == RowState::Queued && ok(&view(r)))?;
        let rev = self.bump();
        let row = &mut self.rows[i];
        row.state = RowState::Running { fraction: None };
        row.rev = rev;
        Some((row.id, row.job.clone()))
    }

    /// Whether any row is waiting to run.
    pub fn any_queued(&self) -> bool {
        self.rows.iter().any(|r| r.state == RowState::Queued)
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
        // A row whose file never came has nothing to run again.
        if self.rows[i].inputs.is_empty() {
            return None;
        }
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

    fn blank_row(&mut self, combine: bool, recipe: &Recipe) -> Row {
        let id = self.next_id;
        self.next_id += 1;
        let mut job = blank_job(&self.policy);
        job.op = operation(recipe.mode());
        let preset = match recipe {
            Recipe::Preset(p) => Some(p.clone()),
            Recipe::Mode(_) => None,
        };
        Row {
            id,
            combine,
            picked: preset.is_some(),
            preset,
            inputs: Vec::new(),
            keys: Vec::new(),
            from: None,
            job,
            size: 0,
            output: None,
            note: None,
            state: RowState::Queued,
            rev: 0,
            staged: None,
            start_when_ready: false,
            waiting: None,
        }
    }

    /// Detect, choose the job (with `target` if given), preview, and bump
    /// the row's revision. Returns its view.
    fn refresh_at(&mut self, i: usize, target: Option<FormatId>) -> RowView {
        // A row whose file hasn't come (or never will) has nothing to plan.
        if !self.rows[i].combine && self.rows[i].inputs.is_empty() {
            return view(&self.rows[i]);
        }
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
        // A file from Photos sits in the cache: its output goes to the
        // output folder, or else the fallback folder (spec D5).
        if row.staged.is_some() {
            row.job.output.dir = self
                .policy
                .dir
                .clone()
                .or_else(|| self.policy.fallback.clone());
        }
        if let Some(refused) = refusal(&path) {
            row.state = RowState::Failed { error: refused };
            return;
        }
        let (op, target_of_folder) = match &row.preset {
            Some(p) => (operation(p.mode), p.target),
            None => (row.job.op, None),
        };
        if path.is_dir()
            && !is_package(&path)
            && matches!(op, Operation::Convert | Operation::Compress)
        {
            // Only a folder that yielded nothing, or couldn't be read, is
            // a row of its own.
            let error = match expand_folder(&self.reg, op, target_of_folder, &path, true) {
                Err(e) => shown(&e),
                Ok(_) => app_error(
                    ErrorCode::UnsupportedInput,
                    &path,
                    "This folder has files to convert now",
                    Some("Drop it again to add them."),
                ),
            };
            row.state = RowState::Failed { error };
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
        if let Some(p) = row.preset.clone() {
            if p.mode == QueueMode::Combine || !kinds(&p, &self.reg).contains(&kind) {
                row.state = RowState::Failed {
                    error: mismatch(&p, &path, &self.reg),
                };
                return;
            }
            row.job.op = operation(p.mode);
            row.job.target = p.target;
            row.job.options = p.options.clone();
            self.preview(row);
            return;
        }
        row.job.options = JobOptions::default();
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
        if let Some(refused) = row.inputs.iter().find_map(|p| refusal(p)) {
            row.state = RowState::Failed { error: refused };
            return;
        }
        if let Some(p) = row.preset.clone() {
            if p.mode != QueueMode::Combine {
                let first = row.inputs.first().cloned().unwrap_or_default();
                row.state = RowState::Failed {
                    error: mismatch(&p, &first, &self.reg),
                };
                return;
            }
            row.job.target = p.target;
            row.job.options = p.options.clone();
            self.preview(row);
            return;
        }
        row.job.options = JobOptions::default();
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
        // The policy the row ran with: set_policy leaves a running row's
        // job alone, so a setting changed mid-run can't mislabel it.
        let row = self.rows.iter().find(|r| r.id == id)?;
        let policy = &row.job.output;
        let fallback = policy.fallback.as_deref()?;
        if row.staged.is_some() {
            let out_dir = output.parent()?;
            return (self.policy.dir.is_none() && out_dir == fallback)
                .then(|| format!("Saved to {} — the original is in Photos", tilde(out_dir)));
        }
        let source_dir = row.inputs.first()?.parent()?;
        let out_dir = output.parent()?;
        (policy.dir.is_none() && out_dir == fallback && out_dir != source_dir).then(|| {
            format!(
                "Saved to {} — the original's folder isn't writable",
                tilde(out_dir)
            )
        })
    }
}

/// "images", "images and PDFs", "images, video and audio".
pub fn kinds_phrase(kinds: &[MediaKind]) -> String {
    let words: Vec<&str> = kinds
        .iter()
        .map(|k| match k {
            MediaKind::Image => "images",
            MediaKind::Video => "video",
            MediaKind::Audio => "audio",
            MediaKind::Document => "PDFs",
            MediaKind::Archive => "archives",
        })
        .collect();
    match words.as_slice() {
        [] => String::new(),
        [one] => (*one).to_string(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

/// A preset applied to a file it doesn't take (spec D4).
fn mismatch(preset: &Preset, file: &Path, reg: &Registry) -> Shown {
    let what = match preset.mode {
        QueueMode::Combine => "combining files".to_string(),
        _ => kinds_phrase(&kinds(preset, reg)),
    };
    app_error(
        ErrorCode::UnsupportedInput,
        file,
        &format!("{} is for {what}", preset.name),
        Some("Pick another preset, or remove this file."),
    )
}

/// What a folder dropped with `recipe` is expanded for: Convert or
/// Compress, and a preset's target if it has one.
fn folder_scope(recipe: &Recipe) -> (Operation, Option<FormatId>) {
    match recipe {
        Recipe::Mode(m) => (operation(*m), None),
        Recipe::Preset(p) => (operation(p.mode), p.target),
    }
}

/// Inputs the app refuses before the engine sees them (spec D2).
fn refusal(path: &Path) -> Option<Shown> {
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
        preset: None,
        inputs: Vec::new(),
        keys: Vec::new(),
        from: None,
        job: blank_job(&OutputPolicy::default()),
        size: 0,
        output: None,
        note: None,
        state: RowState::Queued,
        rev: 0,
        staged: None,
        start_when_ready: false,
        waiting: None,
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
        row.inputs
            .first()
            .map(|p| file_name(p))
            .or_else(|| row.waiting.clone())
            .unwrap_or_default()
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
        preset: row.preset.as_ref().map(|p| p.name.clone()),
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

    /// A queue writing beside originals, falling back to `fallback`.
    fn photos_queue(fallback: &Path) -> Queue {
        Queue::new(
            Registry::builtin(),
            OutputPolicy {
                dir: None,
                overwrite: false,
                fallback: Some(fallback.to_path_buf()),
            },
        )
    }

    /// A promised file landing in its own staging folder.
    fn land(d: &Dir, folder: &str, name: &str) -> PathBuf {
        let dir = d.0.join("promised").join(folder);
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join(name);
        std::fs::write(&p, PNG_1X1).unwrap();
        p
    }

    fn convert() -> Recipe {
        Recipe::Mode(QueueMode::Convert)
    }

    #[test]
    fn promised_files_are_rows_of_their_own() {
        let d = Dir::new("promised-rows");
        let mut q = photos_queue(&d.0.join("out"));
        let a = q.add_receiving("IMG_0001.HEIC", &convert(), false);
        let b = q.add_receiving("IMG_0002.HEIC", &convert(), false);
        let c = q.add_receiving("", &convert(), false);
        assert_eq!(a.state, RowState::Receiving { slow: false });
        assert_eq!(a.name, "IMG_0001.HEIC");
        assert_eq!(c.name, "Photo from Photos");
        let Received::Queued { view, start } = q.received(a.id, &land(&d, "1-1", "IMG_0001.png"))
        else {
            panic!("a was dropped")
        };
        assert_eq!(view.state, RowState::Queued);
        assert!(!start);
        let failed = q.receive_failed(b.id).unwrap();
        let RowState::Failed { error } = &failed.state else {
            panic!("{failed:?}")
        };
        assert_eq!(
            error.reason,
            "The original couldn't be downloaded from iCloud"
        );
        assert_eq!(
            error.fix.as_deref(),
            Some("Open it in Photos to download it, then try again")
        );
        assert_eq!(
            q.get(c.id).unwrap().state,
            RowState::Receiving { slow: false }
        );
        assert!(!q.any_queued() || q.get(a.id).unwrap().state == RowState::Queued);
        assert_eq!(
            q.next_queued().map(|(id, _)| id),
            Some(a.id),
            "only the arrived row runs"
        );
        assert!(q.next_queued().is_none());
    }

    #[test]
    fn a_receiving_row_turns_slow() {
        let d = Dir::new("slow");
        let mut q = photos_queue(&d.0.join("out"));
        let a = q.add_receiving("IMG.HEIC", &convert(), false);
        assert_eq!(
            q.slow(a.id).unwrap().state,
            RowState::Receiving { slow: true }
        );
        q.received(a.id, &land(&d, "1-1", "IMG.png"));
        assert!(q.slow(a.id).is_none());
    }

    #[test]
    fn an_arrived_original_goes_to_the_fallback_folder() {
        let d = Dir::new("promised-out");
        let fallback = d.0.join("out");
        let mut q = photos_queue(&fallback);
        let a = q.add_receiving("IMG.HEIC", &convert(), false);
        let Received::Queued { view, .. } = q.received(a.id, &land(&d, "1-1", "IMG.png")) else {
            panic!()
        };
        let planned = view.output.unwrap();
        assert!(planned.starts_with(&fallback), "{}", planned.display());
        let done = q
            .update(
                a.id,
                RowState::Done {
                    output: fallback.join("IMG.webp"),
                    output_size: Some(1),
                },
            )
            .unwrap();
        assert_eq!(
            done.note.as_deref(),
            Some(format!("Saved to {} — the original is in Photos", tilde(&fallback)).as_str())
        );
        // An output folder in Settings wins, with no note.
        let chosen = d.0.join("chosen");
        let mut q = Queue::new(
            Registry::builtin(),
            OutputPolicy {
                dir: Some(chosen.clone()),
                overwrite: false,
                fallback: Some(fallback.clone()),
            },
        );
        let b = q.add_receiving("IMG.HEIC", &convert(), false);
        let Received::Queued { view, .. } = q.received(b.id, &land(&d, "1-2", "IMG.png")) else {
            panic!()
        };
        assert!(view.output.unwrap().starts_with(&chosen));
        let done = q
            .update(
                b.id,
                RowState::Done {
                    output: chosen.join("IMG.webp"),
                    output_size: Some(1),
                },
            )
            .unwrap();
        assert_eq!(done.note, None);
    }

    #[test]
    fn a_file_arriving_after_cancel_is_dropped() {
        let d = Dir::new("promised-cancel");
        let mut q = photos_queue(&d.0.join("out"));
        let a = q.add_receiving("IMG.HEIC", &convert(), false);
        assert_eq!(q.cancel_receiving(a.id).unwrap().state, RowState::Cancelled);
        assert_eq!(
            q.received(a.id, &land(&d, "1-1", "IMG.png")),
            Received::Dropped
        );
        assert_eq!(q.get(a.id).unwrap().state, RowState::Cancelled);
        assert!(q.next_queued().is_none());
        assert_eq!(
            q.received(999, &land(&d, "1-2", "IMG.png")),
            Received::Dropped
        );
    }

    #[test]
    fn two_promised_files_with_one_name_keep_both_outputs() {
        let d = Dir::new("promised-same");
        let fallback = d.0.join("out");
        std::fs::create_dir_all(&fallback).unwrap();
        std::fs::write(fallback.join("IMG_0001.webp"), b"earlier").unwrap();
        let mut q = photos_queue(&fallback);
        let a = q.add_receiving("IMG_0001.png", &convert(), false);
        let b = q.add_receiving("IMG_0001.png", &convert(), false);
        q.received(a.id, &land(&d, "1-1", "IMG_0001.png"));
        q.received(b.id, &land(&d, "1-2", "IMG_0001.png"));
        let (oa, ob) = (
            q.get(a.id).unwrap().output.unwrap(),
            q.get(b.id).unwrap().output.unwrap(),
        );
        assert_eq!(q.get(a.id).unwrap().state, RowState::Queued);
        assert_eq!(q.get(b.id).unwrap().state, RowState::Queued);
        assert!(oa.starts_with(&fallback) && ob.starts_with(&fallback));
        assert_ne!(
            oa,
            fallback.join("IMG_0001.webp"),
            "an earlier file isn't overwritten"
        );
        assert_ne!(ob, fallback.join("IMG_0001.webp"));
    }

    #[test]
    fn start_when_ready_and_start_requested() {
        let d = Dir::new("promised-start");
        let mut q = photos_queue(&d.0.join("out"));
        let panel = q.add_receiving("A.HEIC", &convert(), true);
        let window = q.add_receiving("B.HEIC", &convert(), false);
        let later = q.add_receiving("C.HEIC", &convert(), false);
        assert!(matches!(
            q.received(panel.id, &land(&d, "1-1", "A.png")),
            Received::Queued { start: true, .. }
        ));
        assert!(matches!(
            q.received(window.id, &land(&d, "1-2", "B.png")),
            Received::Queued { start: false, .. }
        ));
        q.start_requested();
        assert!(matches!(
            q.received(later.id, &land(&d, "1-3", "C.png")),
            Received::Queued { start: true, .. }
        ));
    }

    #[test]
    fn removing_a_staged_row_hands_back_its_folder() {
        let d = Dir::new("promised-remove");
        let mut q = photos_queue(&d.0.join("out"));
        let a = q.add_receiving("A.HEIC", &convert(), false);
        let file = land(&d, "1-1", "A.png");
        q.received(a.id, &file);
        assert_eq!(q.staged(a.id), Some(file.parent().unwrap().to_path_buf()));
        assert_eq!(q.remove(&[a.id]), vec![a.id]);
        assert_eq!(
            q.take_staged_removed(),
            vec![file.parent().unwrap().to_path_buf()]
        );
        assert!(q.take_staged_removed().is_empty());
    }

    #[test]
    fn rows_without_a_file_yet_ignore_retargeting_and_retry() {
        let d = Dir::new("promised-nofile");
        let mut q = photos_queue(&d.0.join("out"));
        let waiting = q.add_receiving("A.HEIC", &convert(), false);
        let failed = q.add_receiving("B.HEIC", &convert(), false);
        q.receive_failed(failed.id);
        let cancelled = q.add_receiving("C.HEIC", &convert(), false);
        q.cancel_receiving(cancelled.id);
        for id in [waiting.id, failed.id, cancelled.id] {
            q.set_target(&[id], FormatId::Png);
            assert!(q.requeue(id).is_none(), "row {id} came back without a file");
        }
        assert_eq!(
            q.get(waiting.id).unwrap().state,
            RowState::Receiving { slow: false }
        );
        assert!(matches!(
            q.get(failed.id).unwrap().state,
            RowState::Failed { .. }
        ));
        assert_eq!(q.get(cancelled.id).unwrap().state, RowState::Cancelled);
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
    fn a_folder_in_convert_adds_its_files() {
        let d = Dir::new("folders");
        let photos = d.0.join("photos");
        std::fs::create_dir_all(photos.join("sub")).unwrap();
        std::fs::write(photos.join("x.png"), PNG_1X1).unwrap();
        std::fs::write(photos.join("sub/y.png"), PNG_1X1).unwrap();
        std::fs::write(photos.join("notes.txt"), b"plain text").unwrap();
        let mut q = queue();
        let rows = q.add(std::slice::from_ref(&photos), QueueMode::Convert);
        assert_eq!(
            rows.iter().map(|r| r.path.clone()).collect::<Vec<_>>(),
            vec![photos.join("x.png"), photos.join("sub/y.png")]
        );
        assert!(rows.iter().all(|r| r.state == RowState::Queued), "{rows:?}");
        assert_eq!(
            q.take_note(),
            Some(AddNote {
                already: 0,
                skipped: Some(SkipNote {
                    count: 1,
                    why: SkipWhy::Convert
                })
            })
        );
        assert_eq!(q.take_note(), None);
    }

    #[test]
    fn re_adding_a_folder_says_its_files_are_already_queued() {
        let d = Dir::new("folder-again");
        let photos = d.0.join("photos");
        std::fs::create_dir_all(&photos).unwrap();
        std::fs::write(photos.join("x.png"), PNG_1X1).unwrap();
        std::fs::write(photos.join("y.png"), PNG_1X1).unwrap();
        let mut q = queue();
        q.add(std::slice::from_ref(&photos), QueueMode::Convert);
        assert_eq!(q.take_note(), None);
        let again = q.add(std::slice::from_ref(&photos), QueueMode::Convert);
        assert!(again.is_empty());
        assert_eq!(
            q.take_note(),
            Some(AddNote {
                already: 2,
                skipped: None
            })
        );
    }

    #[test]
    fn a_package_dropped_on_its_own_is_one_failed_row() {
        let d = Dir::new("folder-package");
        let app = d.0.join("Foo.app");
        std::fs::create_dir_all(app.join("Contents")).unwrap();
        std::fs::write(app.join("Contents/icon.png"), PNG_1X1).unwrap();
        let r = queue().add(std::slice::from_ref(&app), QueueMode::Convert);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].path, app);
        assert!(
            matches!(r[0].state, RowState::Failed { .. }),
            "{:?}",
            r[0].state
        );
    }

    #[test]
    fn a_preset_names_its_target_in_the_skip_note() {
        let d = Dir::new("folder-preset");
        let photos = d.0.join("photos");
        std::fs::create_dir_all(&photos).unwrap();
        std::fs::write(photos.join("x.png"), PNG_1X1).unwrap();
        std::fs::write(photos.join("x.webp"), b"RIFF\x1a\0\0\0WEBPVP8 \x0e\0\0\0").unwrap();
        let mut q = queue();
        q.add_with(&[photos], &Recipe::Preset(builtin("web-image")));
        assert_eq!(
            q.take_note().and_then(|n| n.skipped),
            Some(SkipNote {
                count: 1,
                why: SkipWhy::Become {
                    name: "WebP".into()
                }
            })
        );
    }

    #[test]
    fn an_empty_folder_becomes_a_failed_row() {
        let d = Dir::new("folder-empty");
        let empty = d.0.join("empty");
        std::fs::create_dir_all(&empty).unwrap();
        let r = queue().add(std::slice::from_ref(&empty), QueueMode::Convert);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].path, empty);
        let RowState::Failed { error } = &r[0].state else {
            panic!("{:?}", r[0].state)
        };
        assert_eq!(error.reason, "the folder is empty");
    }

    #[test]
    fn combine_still_takes_a_folder_whole() {
        let d = Dir::new("folder-combine");
        let folder = d.0.join("photos");
        std::fs::create_dir(&folder).unwrap();
        std::fs::write(folder.join("x.png"), PNG_1X1).unwrap();
        let mut q = queue();
        let r = q.add(&[folder], QueueMode::Combine);
        assert_eq!(r[0].state, RowState::Queued);
        assert_eq!(q.targets(&[r[0].id]), vec![FormatId::Zip, FormatId::TarGz]);
        assert_eq!(q.take_note(), None);
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

    /// The note follows the policy the row ran with, not one changed while
    /// it ran, and names the folder the file actually went to.
    #[test]
    fn the_fallback_note_follows_the_rows_own_policy() {
        let d = Dir::new("note-policy");
        let src = d.put("a.png", &PNG_1X1);
        let folder = d.0.join("out");
        let mut q = Queue::new(
            Registry::builtin(),
            OutputPolicy {
                dir: Some(folder.clone()),
                ..OutputPolicy::default()
            },
        );
        let id = q.add(&[src], QueueMode::Convert)[0].id;
        q.next_queued().unwrap();
        q.set_policy(OutputPolicy {
            fallback: Some(d.0.join("fb")),
            ..OutputPolicy::default()
        });
        let v = q
            .update(
                id,
                RowState::Done {
                    output: folder.join("a.jpg"),
                    output_size: Some(1),
                },
            )
            .unwrap();
        assert_eq!(
            v.note, None,
            "it went where the user asked, not to a fallback"
        );
    }

    #[test]
    fn a_defaulted_combine_target_follows_removed_inputs() {
        let d = Dir::new("combine-remove");
        let mut q = queue();
        q.add(&[d.put("a.png", &PNG_1X1)], QueueMode::Combine);
        let r = q.add(&[d.put("b.wav", &WAV_HEADER)], QueueMode::Combine);
        assert_eq!(r[0].to, Some(FormatId::Zip));
        let after = q.remove_input(r[0].id, 1).unwrap();
        assert_eq!(after.to, Some(FormatId::Pdf));
    }

    fn builtin(id: &str) -> crate::presets::Preset {
        crate::presets::builtins()
            .into_iter()
            .find(|p| p.id == id)
            .unwrap()
    }

    #[test]
    fn a_preset_sets_target_and_options() {
        let d = Dir::new("preset-apply");
        let mut q = queue();
        let id = q.add(&[d.put("photo.png", &PNG_1X1)], QueueMode::Convert)[0].id;
        let r = q.apply_preset(&[id], &builtin("web-image"));
        assert_eq!(r[0].to, Some(FormatId::Webp));
        assert_eq!(r[0].preset.as_deref(), Some("Web image"));
        assert_eq!(
            r[0].output.as_deref(),
            Some(d.0.join("photo.webp").as_path())
        );
        let (_, job) = q.next_queued().unwrap();
        assert_eq!(job.options.max_size, Some(2048));
        assert!(job.options.strip_metadata);
    }

    #[test]
    fn a_preset_names_what_it_is_for() {
        let d = Dir::new("preset-mismatch");
        let mut q = queue();
        let id = q.add(&[d.put("photo.png", &PNG_1X1)], QueueMode::Convert)[0].id;
        let r = q.apply_preset(&[id], &builtin("email-video"));
        let RowState::Failed { error } = &r[0].state else {
            panic!("{:?}", r[0].state)
        };
        assert_eq!(error.reason, "Email video is for video");
        assert_eq!(
            error.fix.as_deref(),
            Some("Pick another preset, or remove this file.")
        );
        let r = q.apply_preset(&[id], &builtin("web-image"));
        assert_eq!(r[0].state, RowState::Queued);
    }

    #[test]
    fn picking_an_output_drops_the_rows_preset() {
        let d = Dir::new("preset-picked");
        let mut q = queue();
        let misfit = q.add(&[d.put("photo.png", &PNG_1X1)], QueueMode::Convert)[0].id;
        let fitted = q.add(&[d.put("other.png", &PNG_1X1)], QueueMode::Convert)[0].id;
        q.apply_preset(&[misfit], &builtin("email-video"));
        q.apply_preset(&[fitted], &builtin("web-image"));
        let r = q.set_target(&[misfit, fitted], FormatId::Jpeg).changed;
        assert_eq!(r.len(), 2);
        for row in &r {
            assert_eq!(
                (&row.state, row.to, row.preset.as_deref()),
                (&RowState::Queued, Some(FormatId::Jpeg), None)
            );
        }
        let (_, job) = q.next_queued().unwrap();
        assert_eq!(job.options.max_size, None);
        assert!(!job.options.strip_metadata);
    }

    #[test]
    fn a_compress_preset_with_a_video_preset_is_for_video() {
        let d = Dir::new("preset-video-compress");
        let mut q = queue();
        let id = q.add(&[d.put("photo.png", &PNG_1X1)], QueueMode::Compress)[0].id;
        let mut small = builtin("web-image");
        small.name = "Small video".into();
        small.mode = QueueMode::Compress;
        small.target = None;
        small.options = persimmon_core::job::JobOptions {
            preset: Some(persimmon_core::job::Preset::Small),
            ..Default::default()
        };
        let r = q.apply_preset(&[id], &small);
        let RowState::Failed { error } = &r[0].state else {
            panic!("{:?}", r[0].state)
        };
        assert_eq!(error.reason, "Small video is for video");
    }

    #[test]
    fn adding_a_queued_file_again_reports_it() {
        let d = Dir::new("already");
        let mut q = queue();
        let a = d.put("a.png", &PNG_1X1);
        let b = d.put("b.png", &PNG_1X1);
        q.add(std::slice::from_ref(&a), QueueMode::Convert);
        assert_eq!(q.already_queued(&[a.clone(), b]), 1);
        assert!(q.add(&[a], QueueMode::Convert).is_empty());
    }

    #[test]
    fn changing_the_output_keeps_runtime_failures() {
        use persimmon_core::error::ErrorCode;
        let d = Dir::new("policy-failures");
        let mut q = queue();
        let corrupt = q.add(&[d.put("a.png", &PNG_1X1)], QueueMode::Convert)[0].id;
        let locked = q.add(&[d.put("b.png", &PNG_1X1)], QueueMode::Convert)[0].id;
        let fail = |code, reason: &str| RowState::Failed {
            error: crate::wording::app_error(code, &d.0, reason, None),
        };
        q.update(corrupt, fail(ErrorCode::CorruptInput, "it's damaged"));
        q.update(
            locked,
            fail(ErrorCode::OutputNotWritable, "can't write here"),
        );
        q.set_policy(OutputPolicy::default());
        assert_eq!(
            q.get(corrupt).unwrap().state,
            fail(ErrorCode::CorruptInput, "it's damaged")
        );
        assert!(!matches!(
            q.get(locked).unwrap().state,
            RowState::Failed { .. }
        ));
    }

    #[test]
    fn new_files_use_the_active_preset() {
        let d = Dir::new("preset-new");
        let mut q = queue();
        let r = q.add_with(
            &[d.put("photo.png", &PNG_1X1)],
            &Recipe::Preset(builtin("web-image")),
        );
        assert_eq!(
            (r[0].to, r[0].preset.as_deref()),
            (Some(FormatId::Webp), Some("Web image"))
        );
    }

    #[test]
    fn kinds_read_as_a_list() {
        assert_eq!(kinds_phrase(&[MediaKind::Video]), "video");
        assert_eq!(
            kinds_phrase(&[MediaKind::Image, MediaKind::Document]),
            "images and PDFs"
        );
        assert_eq!(
            kinds_phrase(&[MediaKind::Image, MediaKind::Video, MediaKind::Audio]),
            "images, video and audio"
        );
    }
}
