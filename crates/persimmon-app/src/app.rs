// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Persimmon contributors

//! The window's side of the app: commands, the `row` event, and quitting
//! with a conversion running.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use persimmon_core::backend::Backends;
use persimmon_core::format::FormatId;
use persimmon_core::registry::Registry;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::{AppHandle, Emitter, Manager, RunEvent, State, WindowEvent};
use tauri_plugin_dialog::DialogExt;

use crate::queue::{Queue, RowId, RowView, SetTarget};
use crate::runner::Runner;

/// The app's state, shared by every command.
pub struct AppState {
    queue: Arc<Mutex<Queue>>,
    runner: Runner,
}

impl AppState {
    fn queue(&self) -> std::sync::MutexGuard<'_, Queue> {
        self.queue.lock().unwrap_or_else(|p| p.into_inner())
    }
}

fn emit_rows(app: &AppHandle, rows: &[RowView]) {
    for row in rows {
        let _ = app.emit("row", row);
    }
}

#[tauri::command]
fn engine_version() -> String {
    persimmon_core::VERSION.to_string()
}

#[tauri::command]
fn add_paths(app: AppHandle, state: State<'_, AppState>, paths: Vec<PathBuf>) -> Vec<RowView> {
    let rows = state.queue().add(&paths);
    emit_rows(&app, &rows);
    rows
}

/// The macOS open panel, from Rust; runs off the main thread.
#[tauri::command]
async fn pick_files(app: AppHandle, state: State<'_, AppState>) -> Result<Vec<RowView>, String> {
    let picked = app
        .dialog()
        .file()
        .blocking_pick_files()
        .unwrap_or_default();
    let paths: Vec<PathBuf> = picked
        .into_iter()
        .filter_map(|f| f.into_path().ok())
        .collect();
    let rows = state.queue().add(&paths);
    emit_rows(&app, &rows);
    Ok(rows)
}

#[tauri::command]
fn set_target(
    app: AppHandle,
    state: State<'_, AppState>,
    ids: Vec<RowId>,
    to: FormatId,
) -> SetTarget {
    let result = state.queue().set_target(&ids, to);
    emit_rows(&app, &result.changed);
    result
}

#[tauri::command]
fn targets(state: State<'_, AppState>, ids: Vec<RowId>) -> Vec<FormatId> {
    state.queue().targets(&ids)
}

#[tauri::command]
fn remove_rows(state: State<'_, AppState>, ids: Vec<RowId>) -> Vec<RowId> {
    state.queue().remove(&ids)
}

#[tauri::command]
fn queue_rows(state: State<'_, AppState>) -> Vec<RowView> {
    state.queue().rows()
}

#[tauri::command]
fn start(state: State<'_, AppState>) {
    state.runner.start();
}

#[tauri::command]
fn cancel_row(state: State<'_, AppState>, id: RowId) -> bool {
    state.runner.cancel(id)
}

#[tauri::command]
fn retry_row(state: State<'_, AppState>, id: RowId) -> Option<RowView> {
    state.runner.retry(id)
}

#[tauri::command]
fn reveal_row(state: State<'_, AppState>, id: RowId) -> bool {
    let output = state.queue().get(id).and_then(|v| match v.state {
        crate::queue::RowState::Done { output, .. } => Some(output),
        _ => None,
    });
    output.is_some_and(|p| crate::reveal::reveal(&p))
}

/// Quit after the window confirmed it: cancel the running row first.
#[tauri::command]
fn quit_now(app: AppHandle, state: State<'_, AppState>) {
    state.runner.shutdown();
    app.exit(0);
}

/// What quitting does now.
#[derive(Debug, PartialEq, Eq)]
enum QuitAction {
    /// A row is running: ask in the window first.
    Confirm,
    Exit,
}

fn quit_action(busy: bool) -> QuitAction {
    if busy {
        QuitAction::Confirm
    } else {
        QuitAction::Exit
    }
}

/// The app's own Quit (⌘Q). macOS's standard Quit item calls
/// `terminate:`, which never reaches Tauri's ExitRequested, so it could
/// neither ask nor cancel the running row.
fn quit_requested(app: &AppHandle) {
    match quit_action(app.state::<AppState>().runner.is_busy()) {
        QuitAction::Confirm => {
            let _ = app.emit("confirm-quit", ());
        }
        QuitAction::Exit => app.exit(0),
    }
}

/// The menu bar: the standard items, with the app's own Quit.
fn menu(app: &AppHandle) -> tauri::Result<Menu<tauri::Wry>> {
    let sep = || PredefinedMenuItem::separator(app);
    let quit = MenuItem::with_id(app, "quit", "Quit Persimmon", true, Some("CmdOrCtrl+Q"))?;
    let app_menu = Submenu::with_items(
        app,
        "Persimmon",
        true,
        &[
            &PredefinedMenuItem::about(app, None, None)?,
            &sep()?,
            &PredefinedMenuItem::services(app, None)?,
            &sep()?,
            &PredefinedMenuItem::hide(app, None)?,
            &PredefinedMenuItem::hide_others(app, None)?,
            &PredefinedMenuItem::show_all(app, None)?,
            &sep()?,
            &quit,
        ],
    )?;
    let edit = Submenu::with_items(
        app,
        "Edit",
        true,
        &[
            &PredefinedMenuItem::undo(app, None)?,
            &PredefinedMenuItem::redo(app, None)?,
            &sep()?,
            &PredefinedMenuItem::cut(app, None)?,
            &PredefinedMenuItem::copy(app, None)?,
            &PredefinedMenuItem::paste(app, None)?,
            &PredefinedMenuItem::select_all(app, None)?,
        ],
    )?;
    let window = Submenu::with_items(
        app,
        "Window",
        true,
        &[
            &PredefinedMenuItem::minimize(app, None)?,
            &PredefinedMenuItem::maximize(app, None)?,
            &sep()?,
            &PredefinedMenuItem::close_window(app, None)?,
        ],
    )?;
    Menu::with_items(app, &[&app_menu, &edit, &window])
}

/// Start the app.
pub fn run() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let queue = Arc::new(Mutex::new(Queue::new(Registry::builtin())));
            let handle = app.handle().clone();
            // FFmpeg from PATH or Homebrew until the app bundles its own.
            let runner = Runner::new(Arc::clone(&queue), Backends::new(), move |row| {
                let _ = handle.emit("row", &row);
            });
            app.manage(AppState { queue, runner });
            app.set_menu(menu(app.handle())?)?;
            Ok(())
        })
        .on_menu_event(|app, event| {
            if event.id() == "quit" {
                quit_requested(app);
            }
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event
                && window.state::<AppState>().runner.is_busy()
            {
                api.prevent_close();
                let _ = window.emit("confirm-quit", ());
            }
        })
        .invoke_handler(tauri::generate_handler![
            engine_version,
            add_paths,
            pick_files,
            set_target,
            targets,
            remove_rows,
            queue_rows,
            start,
            cancel_row,
            retry_row,
            reveal_row,
            quit_now
        ])
        .build(tauri::generate_context!())
        .expect("Persimmon couldn't start");
    app.run(|app, event| match event {
        // Closing the last window with a conversion running: ask first.
        // app.exit() from the Quit item or quit_now has a code, so it goes
        // through.
        RunEvent::ExitRequested {
            api, code: None, ..
        } if app.state::<AppState>().runner.is_busy() => {
            api.prevent_exit();
            let _ = app.emit("confirm-quit", ());
        }
        // Every way out, Dock Quit and logout included (they can't ask):
        // cancel the running row, so no .part and no FFmpeg is left.
        RunEvent::Exit => app.state::<AppState>().runner.shutdown(),
        _ => {}
    });
}

#[cfg(test)]
mod tests {
    use super::{QuitAction, quit_action};

    /// ⌘Q (the app's own Quit item) asks first while a row runs, and
    /// quits at once otherwise.
    #[test]
    fn quit_asks_only_while_a_row_runs() {
        assert_eq!(quit_action(true), QuitAction::Confirm);
        assert_eq!(quit_action(false), QuitAction::Exit);
    }

    /// The window may call the app's own commands and nothing else: no
    /// core defaults (tray, menus, paths, devtools), no plugins.
    #[test]
    fn capability_allows_only_the_apps_commands() {
        let cap: serde_json::Value =
            serde_json::from_str(include_str!("../capabilities/main.json")).unwrap();
        assert_eq!(cap["windows"], serde_json::json!(["main"]));
        assert_eq!(
            cap["permissions"],
            serde_json::json!([
                "allow-add-paths",
                "allow-cancel-row",
                "allow-engine-version",
                "allow-pick-files",
                "allow-queue-rows",
                "allow-quit-now",
                "allow-remove-rows",
                "allow-retry-row",
                "allow-reveal-row",
                "allow-set-target",
                "allow-start",
                "allow-targets",
                "core:event:allow-listen",
                "core:event:allow-unlisten"
            ])
        );
    }

    /// Every colour token has a light value too (DD-19: both themes at
    /// parity), and the light error colour is the owner's AA value.
    #[test]
    fn light_theme_defines_every_colour_token() {
        let css = include_str!("../ui/src/tokens.css");
        let (dark, light) = css
            .split_once("@media (prefers-color-scheme: light)")
            .unwrap();
        let colours = |block: &str| -> Vec<(String, String)> {
            block
                .lines()
                .filter_map(|l| l.trim().strip_prefix("--"))
                .filter_map(|l| l.split_once(':'))
                .filter(|(_, v)| v.trim().starts_with('#') || v.trim().starts_with("rgba"))
                .map(|(k, v)| (k.to_string(), v.trim().trim_end_matches(';').to_string()))
                .collect()
        };
        let light = colours(light);
        for (name, _) in colours(dark) {
            assert!(
                light.iter().any(|(k, _)| *k == name),
                "light theme has no --{name}"
            );
        }
        let value = |n: &str| light.iter().find(|(k, _)| k == n).unwrap().1.clone();
        assert_eq!(value("error"), "#c4291c");
        assert_eq!(value("error-text"), "#c4291c");
    }
}
