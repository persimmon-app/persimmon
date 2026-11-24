// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Persimmon contributors

//! Watch folders in the app (spec D2, D9): the commands, the watch
//! runner's wiring, and notifications with Show files and Undo.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use persimmon_core::format::FormatId;
use persimmon_core::registry::Registry;
use persimmon_core::watch::Patterns;
use persimmon_mac::notes;
use tauri::{AppHandle, Emitter, Manager, State};

use super::{AppState, batch_view, change_settings, lock, pick_folder, settings_view, show_main};
use crate::settings::SettingsView;
use crate::watch::burst::QUIET;
use crate::watch::record::{Record, record_path};
use crate::watch::rule::{self, Rule};
use crate::watch::runner::{Deps, Notifier, Running, WatchRunner};
use crate::watch::views::{
    RuleDraftView, RuleView, draft_view, rule_views, watch_targets as targets,
};
use crate::wording::{Shown, app_error};

/// Notifications through macOS, remembering what each one's Show files
/// reveals.
pub(super) struct MacNotifier {
    reveal: Mutex<HashMap<String, PathBuf>>,
}

impl Notifier for MacNotifier {
    fn allowed(&self) -> bool {
        notes::permission() == notes::Permission::Allowed
    }

    fn post(&self, id: &str, body: &str, batch: bool, reveal: Option<PathBuf>) {
        if let Some(path) = reveal {
            lock(&self.reveal).insert(id.to_string(), path);
        }
        let kind = if batch {
            notes::Kind::Batch
        } else {
            notes::Kind::File
        };
        notes::post(id, body, kind);
    }
}

/// The watch runner, sharing the queue's pool and output-name claims, with
/// its batches going to History and its rules' lines to the window.
pub(super) fn runner(app: &AppHandle, queue: &crate::runner::Runner, data: &Path) -> WatchRunner {
    let notifier = Arc::new(MacNotifier {
        reveal: Mutex::new(HashMap::new()),
    });
    {
        let (app, notifier) = (app.clone(), Arc::clone(&notifier));
        notes::on_action(move |id, action| match action {
            notes::Action::Show => {
                if let Some(path) = lock(&notifier.reveal).get(&id).cloned() {
                    crate::reveal::reveal(&path);
                }
            }
            // A summary's Undo opens History at that batch (spec D9).
            notes::Action::Undo => {
                show_main(&app);
                let _ = app.emit_to("main", "show-batch", id);
            }
        });
    }
    let on_batch = {
        let app = app.clone();
        move |batch: crate::history::Batch| {
            if let Some(state) = app.try_state::<AppState>() {
                let _ = lock(&state.history).append(batch.clone());
            }
            let _ = app.emit("history", batch_view(&batch));
        }
    };
    let on_status = {
        let app = app.clone();
        move |status| {
            let _ = app.emit("watch", status);
        }
    };
    WatchRunner::new(Deps {
        reg: Registry::builtin(),
        engine: queue.engine(),
        pool: queue.pool().clone(),
        claims: queue.claims(),
        data: data.to_path_buf(),
        timing: Default::default(),
        quiet: QUIET,
        on_batch: Box::new(on_batch),
        on_status: Box::new(on_status),
        notifier,
    })
}

/// Every rule as the runner takes it.
fn running(state: &AppState) -> Vec<Running> {
    let rules = lock(&state.rules).all().to_vec();
    let presets = lock(&state.presets).clone();
    let reg = Registry::builtin();
    rules
        .into_iter()
        .map(|rule| {
            let (spec, preset_name) = rule::spec(&rule, &presets, &reg);
            Running {
                rule,
                spec,
                preset_name,
            }
        })
        .collect()
}

/// Run the rules as saved now.
pub(super) fn apply(state: &AppState) {
    state.watch.apply(running(state));
}

/// Whether any rule uses preset `id`, by its folder.
pub(super) fn used_by(state: &AppState, id: &str) -> Option<String> {
    lock(&state.rules)
        .using_preset(id)
        .map(|r| crate::queue::tilde(&r.folder))
}

fn views(state: &AppState, since: u64) -> Vec<RuleView> {
    let rules = lock(&state.rules).all().to_vec();
    let presets = lock(&state.presets).clone();
    let statuses = state.watch.statuses();
    let history = lock(&state.history);
    rule_views(&rules, &presets, &statuses, &history, since)
}

#[tauri::command]
pub(crate) fn watch_rules(state: State<'_, AppState>, since: u64) -> Vec<RuleView> {
    views(&state, since)
}

#[tauri::command]
pub(crate) fn watch_targets() -> Vec<FormatId> {
    targets(&Registry::builtin())
}

/// Match files' defaults for `target`.
#[tauri::command]
pub(crate) fn watch_patterns(target: FormatId) -> String {
    Patterns::for_target(&Registry::builtin(), target).to_string()
}

/// The open panel, for a folder to watch or to put results in.
#[tauri::command]
pub(crate) async fn pick_watch_folder(app: AppHandle) -> Result<Option<PathBuf>, String> {
    Ok(pick_folder(&app))
}

#[tauri::command]
pub(crate) fn new_rule(
    state: State<'_, AppState>,
    folder: PathBuf,
    target: FormatId,
) -> RuleDraftView {
    let reg = Registry::builtin();
    let presets = lock(&state.presets).clone();
    draft_view(rule::draft(&reg, folder, target), &presets, &reg)
}

#[tauri::command]
pub(crate) fn rule_command(state: State<'_, AppState>, rule: Rule) -> String {
    rule::command(&rule, &lock(&state.presets))
}

/// Save a rule after its checks, which read its folder so macOS asks for
/// access now (D2). A new rule leaves the files already there alone unless
/// `existing`. The first rule ever saved asks for notifications (D9).
#[tauri::command]
pub(crate) fn save_rule(
    app: AppHandle,
    state: State<'_, AppState>,
    rule: Rule,
    existing: bool,
    since: u64,
) -> Result<Vec<RuleView>, Shown> {
    let reg = Registry::builtin();
    let presets = lock(&state.presets).clone();
    let others = lock(&state.rules).all().to_vec();
    let (spec, _) = rule::check(&rule, &others, &presets, &reg)?;
    match others.iter().find(|r| r.id == rule.id) {
        None if !existing => {
            state.watch.baseline(&rule.id, &spec);
        }
        None => {}
        // An edit leaves alone what it newly brings in (another folder,
        // more patterns, subfolders), as a new rule does.
        Some(old) => {
            let (before, _) = rule::spec(old, &presets, &reg);
            state
                .watch
                .baseline_files(&rule.id, &rule::newly_matched(&before, &spec));
        }
    }
    lock(&state.rules).put(rule.clone()).map_err(|e| {
        app_error(
            persimmon_core::error::ErrorCode::OutputNotWritable,
            &rule.folder,
            &format!("Persimmon couldn't save this rule ({e})"),
            None,
        )
    })?;
    if !lock(&state.settings).notifications_asked {
        notes::ask();
        change_settings(&app, &state, |s| s.notifications_asked = true);
    }
    apply(&state);
    Ok(views(&state, since))
}

#[tauri::command]
pub(crate) fn toggle_rule(
    state: State<'_, AppState>,
    id: String,
    on: bool,
    since: u64,
) -> Vec<RuleView> {
    let rule = lock(&state.rules).get(&id).cloned();
    if let Some(mut rule) = rule {
        rule.enabled = on;
        let _ = lock(&state.rules).put(rule);
        apply(&state);
    }
    views(&state, since)
}

/// Stop watching a folder: the rule and its record go; files stay.
#[tauri::command]
pub(crate) fn remove_rule(state: State<'_, AppState>, id: String, since: u64) -> Vec<RuleView> {
    let _ = lock(&state.rules).remove(&id);
    apply(&state);
    Record::forget(&record_path(&state.data, &id));
    views(&state, since)
}

#[tauri::command]
pub(crate) fn scan_rule(state: State<'_, AppState>, id: String) {
    state.watch.scan(&id);
}

#[tauri::command]
pub(crate) fn copy_command(state: State<'_, AppState>, id: String) -> bool {
    let rule = lock(&state.rules).get(&id).cloned();
    rule.is_some_and(|r| persimmon_mac::copy_text(&rule::command(&r, &lock(&state.presets))))
}

/// Open Persimmon at login, or stop (spec D2's "Start with session").
#[tauri::command]
pub(crate) fn set_start_at_login(
    app: AppHandle,
    state: State<'_, AppState>,
    on: bool,
) -> Result<SettingsView, String> {
    persimmon_mac::login::set(on).map_err(|e| {
        if on {
            format!("Persimmon couldn't be added to Login Items ({e})")
        } else {
            format!("Persimmon couldn't be removed from Login Items ({e})")
        }
    })?;
    let view = settings_view(&lock(&state.settings));
    let _ = app.emit("settings", &view);
    Ok(view)
}

/// "allowed", "off" or "notAsked", read from System Settings now.
#[tauri::command]
pub(crate) fn notifications() -> &'static str {
    match notes::permission() {
        notes::Permission::Allowed => "allowed",
        notes::Permission::Off => "off",
        notes::Permission::NotAsked => "notAsked",
    }
}

#[tauri::command]
pub(crate) fn open_notification_settings() -> bool {
    std::process::Command::new("/usr/bin/open")
        .arg("x-apple.systempreferences:com.apple.Notifications-Settings.extension")
        .status()
        .is_ok_and(|s| s.success())
}
