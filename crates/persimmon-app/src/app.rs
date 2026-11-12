// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Persimmon contributors

//! The windows' side of the app: commands, the `row` and `settings`
//! events, the menu bar, the ways in (window, drop window, Dock and Open
//! With), and quitting with a conversion running.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use persimmon_core::backend::Backends;
use persimmon_core::format::FormatId;
use persimmon_core::registry::Registry;
use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::{AppHandle, Emitter, Manager, RunEvent, State, WindowEvent};
use tauri_plugin_dialog::DialogExt;

use crate::queue::{Queue, QueueMode, RowId, RowView, SetTarget};
use crate::runner::Runner;
use crate::settings::{
    OutputMode, Settings, SettingsView, load, output_policy, save, settings_path, view,
};

/// The app's state, shared by every command.
pub struct AppState {
    queue: Arc<Mutex<Queue>>,
    runner: Runner,
    settings: Mutex<Settings>,
    settings_file: PathBuf,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

impl AppState {
    fn queue(&self) -> MutexGuard<'_, Queue> {
        lock(&self.queue)
    }

    fn mode(&self) -> QueueMode {
        lock(&self.settings).mode
    }
}

fn emit_rows(app: &AppHandle, rows: &[RowView]) {
    for row in rows {
        let _ = app.emit("row", row);
    }
}

/// Change the settings, save them, apply output changes to the queue, and
/// tell every window.
fn change_settings(
    app: &AppHandle,
    state: &AppState,
    f: impl FnOnce(&mut Settings),
) -> SettingsView {
    let (new, output_changed) = {
        let mut settings = lock(&state.settings);
        let before = (settings.output.clone(), settings.fallback.clone());
        f(&mut settings);
        let _ = save(&state.settings_file, &settings);
        let changed = before != (settings.output.clone(), settings.fallback.clone());
        (settings.clone(), changed)
    };
    if output_changed {
        let rows = state.queue().set_policy(output_policy(&new));
        emit_rows(app, &rows);
    }
    let shown = view(&new);
    let _ = app.emit("settings", &shown);
    shown
}

/// Files opened before the app finished starting (a cold launch from the
/// Dock or Open With delivers them first): queued once startup is done.
#[derive(Default)]
struct Pending(Mutex<Vec<PathBuf>>);

/// File URLs from the Dock or Open With, as paths; anything else is
/// dropped.
pub fn paths_from_urls(urls: &[tauri::Url]) -> Vec<PathBuf> {
    urls.iter()
        .filter(|u| u.scheme() == "file")
        .filter_map(|u| u.to_file_path().ok())
        .collect()
}

/// Add files and start them at once: what a drop on the drop window does.
pub fn add_and_start(
    queue: &Mutex<Queue>,
    runner: &Runner,
    paths: &[PathBuf],
    mode: QueueMode,
) -> Vec<RowView> {
    let rows = lock(queue).add(paths, mode);
    runner.start();
    rows
}

#[tauri::command]
fn engine_version() -> String {
    persimmon_core::VERSION.to_string()
}

#[tauri::command]
fn add_paths(app: AppHandle, state: State<'_, AppState>, paths: Vec<PathBuf>) -> Vec<RowView> {
    let mode = state.mode();
    let rows = state.queue().add(&paths, mode);
    emit_rows(&app, &rows);
    rows
}

#[tauri::command]
fn drop_paths(app: AppHandle, state: State<'_, AppState>, paths: Vec<PathBuf>) -> Vec<RowView> {
    let rows = add_and_start(&state.queue, &state.runner, &paths, state.mode());
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
    let mode = state.mode();
    let rows = state.queue().add(&paths, mode);
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
fn reorder_input(
    app: AppHandle,
    state: State<'_, AppState>,
    id: RowId,
    from: usize,
    to: usize,
) -> Option<RowView> {
    let row = state.queue().reorder(id, from, to);
    emit_rows(&app, row.as_slice());
    row
}

/// `None` when the Combine row is gone with its last input.
#[tauri::command]
fn remove_input(
    app: AppHandle,
    state: State<'_, AppState>,
    id: RowId,
    index: usize,
) -> Option<RowView> {
    let row = state.queue().remove_input(id, index);
    emit_rows(&app, row.as_slice());
    row
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

#[tauri::command]
fn get_settings(state: State<'_, AppState>) -> SettingsView {
    view(&lock(&state.settings))
}

#[tauri::command]
fn set_mode(app: AppHandle, state: State<'_, AppState>, mode: QueueMode) -> SettingsView {
    change_settings(&app, &state, |s| s.mode = mode)
}

#[tauri::command]
fn set_output_beside(app: AppHandle, state: State<'_, AppState>) -> SettingsView {
    change_settings(&app, &state, |s| s.output = OutputMode::Beside)
}

fn pick_folder(app: &AppHandle) -> Option<PathBuf> {
    app.dialog()
        .file()
        .blocking_pick_folder()
        .and_then(|f| f.into_path().ok())
}

/// Always save into a folder the user picks; cancelled leaves it as is.
#[tauri::command]
async fn choose_output_folder(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<SettingsView, String> {
    Ok(match pick_folder(&app) {
        Some(folder) => change_settings(&app, &state, |s| s.output = OutputMode::Folder { folder }),
        None => view(&lock(&state.settings)),
    })
}

#[tauri::command]
async fn choose_fallback_folder(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<SettingsView, String> {
    Ok(match pick_folder(&app) {
        Some(folder) => change_settings(&app, &state, |s| s.fallback = folder),
        None => view(&lock(&state.settings)),
    })
}

/// Show or hide the drop window, remember it, and tick its menu item.
fn set_drop_window(app: &AppHandle, show: bool) {
    if let Some(window) = app.get_webview_window("drop") {
        let _ = if show { window.show() } else { window.hide() };
    }
    if let Some(menu) = app.menu()
        && let Some(item) = menu.get("drop-window")
        && let Some(check) = item.as_check_menuitem()
    {
        let _ = check.set_checked(show);
    }
}

#[tauri::command]
fn toggle_drop_window(app: AppHandle, state: State<'_, AppState>) -> bool {
    let show = !lock(&state.settings).drop_window;
    change_settings(&app, &state, |s| s.drop_window = show);
    set_drop_window(&app, show);
    show
}

#[tauri::command]
fn open_settings(app: AppHandle) {
    if let Some(window) = app.get_webview_window("settings") {
        let _ = window.show();
        let _ = window.set_focus();
    }
}

/// Quit after the window confirmed it: cancel the running row first, off
/// the main thread so the window doesn't freeze while it stops.
#[tauri::command]
async fn quit_now(app: AppHandle) -> Result<(), String> {
    let handle = app.clone();
    std::thread::spawn(move || {
        handle.state::<AppState>().runner.shutdown();
        handle.exit(0);
    });
    Ok(())
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

/// The menu bar: the standard items, the app's own Quit and Settings, and
/// the drop window under Window.
fn menu(app: &AppHandle, drop_window: bool) -> tauri::Result<Menu<tauri::Wry>> {
    let sep = || PredefinedMenuItem::separator(app);
    let settings = MenuItem::with_id(app, "settings", "Settings…", true, Some("CmdOrCtrl+,"))?;
    let quit = MenuItem::with_id(app, "quit", "Quit Persimmon", true, Some("CmdOrCtrl+Q"))?;
    let drop = CheckMenuItem::with_id(
        app,
        "drop-window",
        "Drop Window",
        true,
        drop_window,
        Some("Alt+CmdOrCtrl+D"),
    )?;
    let app_menu = Submenu::with_items(
        app,
        "Persimmon",
        true,
        &[
            &PredefinedMenuItem::about(app, None, None)?,
            &sep()?,
            &settings,
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
            &drop,
            &sep()?,
            &PredefinedMenuItem::close_window(app, None)?,
        ],
    )?;
    Menu::with_items(app, &[&app_menu, &edit, &window])
}

/// Where the settings live: `~/Library/Application Support/Persimmon`
/// (DD-48), not Tauri's folder named after the bundle identifier.
fn settings_file(app: &AppHandle) -> PathBuf {
    let data = app
        .path()
        .data_dir()
        .unwrap_or_else(|_| Path::new("/tmp").to_path_buf());
    settings_path(&data)
}

/// Start the app.
pub fn run() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(Pending::default())
        .setup(|app| {
            let file = settings_file(app.handle());
            let settings = load(&file);
            let queue = Arc::new(Mutex::new(Queue::new(
                Registry::builtin(),
                output_policy(&settings),
            )));
            let handle = app.handle().clone();
            // FFmpeg from PATH or Homebrew until the app bundles its own.
            let runner = Runner::new(Arc::clone(&queue), Backends::new(), move |row| {
                let _ = handle.emit("row", &row);
            });
            let drop_window = settings.drop_window;
            app.manage(AppState {
                queue,
                runner,
                settings: Mutex::new(settings),
                settings_file: file,
            });
            let pending = std::mem::take(&mut *lock(&app.state::<Pending>().0));
            if !pending.is_empty() {
                let state = app.state::<AppState>();
                let mode = state.mode();
                state.queue().add(&pending, mode);
            }
            app.set_menu(menu(app.handle(), drop_window)?)?;
            if drop_window {
                set_drop_window(app.handle(), true);
            }
            Ok(())
        })
        .on_menu_event(|app, event| match event.id().as_ref() {
            "quit" => quit_requested(app),
            "settings" => open_settings(app.clone()),
            "drop-window" => {
                let state = app.state::<AppState>();
                toggle_drop_window(app.clone(), state);
            }
            _ => {}
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                match window.label() {
                    // The drop and Settings windows hide; they're shown again
                    // from the menu.
                    "drop" | "settings" => {
                        api.prevent_close();
                        let _ = window.hide();
                        if window.label() == "drop" {
                            let app = window.app_handle();
                            let state = app.state::<AppState>();
                            change_settings(app, &state, |s| s.drop_window = false);
                            set_drop_window(app, false);
                        }
                    }
                    _ if window.state::<AppState>().runner.is_busy() => {
                        api.prevent_close();
                        let _ = window.emit("confirm-quit", ());
                    }
                    _ => {}
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            engine_version,
            add_paths,
            drop_paths,
            pick_files,
            set_target,
            targets,
            remove_rows,
            reorder_input,
            remove_input,
            queue_rows,
            start,
            cancel_row,
            retry_row,
            reveal_row,
            get_settings,
            set_mode,
            set_output_beside,
            choose_output_folder,
            choose_fallback_folder,
            toggle_drop_window,
            open_settings,
            quit_now
        ])
        .build(tauri::generate_context!())
        .expect("Persimmon couldn't start");
    app.run(|app, event| match event {
        // Files dropped on the Dock icon or opened with Persimmon: queued
        // with the current mode, shown when the window loads its rows.
        RunEvent::Opened { urls } => {
            let paths = paths_from_urls(&urls);
            match app.try_state::<AppState>() {
                Some(state) => {
                    let mode = state.mode();
                    let rows = state.queue().add(&paths, mode);
                    emit_rows(app, &rows);
                }
                // Not started yet: setup queues these.
                None => lock(&app.state::<Pending>().0).extend(paths),
            }
        }
        // Closing the last window with a conversion running: ask first.
        // app.exit() from the Quit item or quit_now has a code, so it goes
        // through.
        RunEvent::ExitRequested {
            api, code: None, ..
        } if app
            .try_state::<AppState>()
            .is_some_and(|s| s.runner.is_busy()) =>
        {
            api.prevent_exit();
            let _ = app.emit("confirm-quit", ());
        }
        // Every way out, Dock Quit and logout included (they can't ask):
        // cancel the running row, so no .part and no FFmpeg is left.
        RunEvent::Exit => {
            if let Some(state) = app.try_state::<AppState>() {
                state.runner.shutdown();
            }
        }
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

    fn permissions(file: &str) -> serde_json::Value {
        let cap: serde_json::Value = serde_json::from_str(file).unwrap();
        cap["permissions"].clone()
    }

    /// Each window may call exactly its own commands: no core defaults
    /// (tray, menus, paths, devtools), no plugins.
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
                "allow-get-settings",
                "allow-open-settings",
                "allow-pick-files",
                "allow-queue-rows",
                "allow-quit-now",
                "allow-remove-input",
                "allow-remove-rows",
                "allow-reorder-input",
                "allow-retry-row",
                "allow-reveal-row",
                "allow-set-mode",
                "allow-set-target",
                "allow-start",
                "allow-targets",
                "allow-toggle-drop-window",
                "core:event:allow-listen",
                "core:event:allow-unlisten"
            ])
        );
    }

    #[test]
    fn the_drop_window_may_only_drop_and_watch() {
        let file = include_str!("../capabilities/drop.json");
        let cap: serde_json::Value = serde_json::from_str(file).unwrap();
        assert_eq!(cap["windows"], serde_json::json!(["drop"]));
        assert_eq!(
            permissions(file),
            serde_json::json!([
                "allow-drop-paths",
                "allow-queue-rows",
                "core:event:allow-listen",
                "core:event:allow-unlisten"
            ])
        );
    }

    #[test]
    fn the_settings_window_may_only_read_and_set_output() {
        let file = include_str!("../capabilities/settings.json");
        let cap: serde_json::Value = serde_json::from_str(file).unwrap();
        assert_eq!(cap["windows"], serde_json::json!(["settings"]));
        assert_eq!(
            permissions(file),
            serde_json::json!([
                "allow-choose-fallback-folder",
                "allow-choose-output-folder",
                "allow-get-settings",
                "allow-set-output-beside",
                "core:event:allow-listen",
                "core:event:allow-unlisten"
            ])
        );
    }

    /// Open With lists Persimmon for every format it reads, and it never
    /// takes over as any type's default app.
    #[test]
    fn file_associations_list_every_readable_extension_as_alternate() {
        use persimmon_core::registry::{Operation, Registry};
        let conf: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        let assoc = conf["bundle"]["fileAssociations"].as_array().unwrap();
        assert!(!assoc.is_empty());
        let mut listed = std::collections::BTreeSet::new();
        for a in assoc {
            assert_eq!(a["rank"], "Alternate", "{a}");
            assert_eq!(a["role"], "Viewer", "{a}");
            for e in a["ext"].as_array().unwrap() {
                listed.insert(e.as_str().unwrap().to_string());
            }
        }
        let reg = Registry::builtin();
        let mut readable = std::collections::BTreeSet::new();
        for f in reg.formats() {
            let read = [
                Operation::Convert,
                Operation::Compress,
                Operation::Extract,
                Operation::Merge,
            ]
            .iter()
            .any(|&op| {
                reg.capabilities()
                    .iter()
                    .any(|c| c.op == op && c.from == f.id)
            });
            if read {
                readable.extend(f.extensions.iter().map(|e| e.to_string()));
            }
        }
        assert_eq!(listed, readable);
    }

    #[test]
    fn opened_files_are_queued_without_a_window() {
        let dir = std::env::temp_dir().join(format!("prsm-app-opened-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let png = dir.join("a.png");
        std::fs::write(&png, persimmon_corpus::PNG_1X1).unwrap();
        let urls = [
            tauri::Url::from_file_path(&png).unwrap(),
            tauri::Url::parse("https://example.com/b.png").unwrap(),
        ];
        let paths = super::paths_from_urls(&urls);
        assert_eq!(paths, vec![png.clone()]);
        let mut q = crate::queue::Queue::new(
            persimmon_core::registry::Registry::builtin(),
            persimmon_core::output::OutputPolicy::default(),
        );
        assert_eq!(q.add(&paths, crate::queue::QueueMode::Convert).len(), 1);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[cfg(feature = "vips")]
    #[test]
    fn drop_paths_starts_the_queue() {
        use std::sync::{Arc, Mutex};
        let dir = std::env::temp_dir().join(format!("prsm-app-drop-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let png = dir.join("a.png");
        std::fs::write(&png, persimmon_corpus::PNG_1X1).unwrap();
        let queue = Arc::new(Mutex::new(crate::queue::Queue::new(
            persimmon_core::registry::Registry::builtin(),
            persimmon_core::output::OutputPolicy::default(),
        )));
        let runner = crate::runner::Runner::new(
            Arc::clone(&queue),
            persimmon_core::backend::Backends::new(),
            |_| {},
        );
        let rows = super::add_and_start(&queue, &runner, &[png], crate::queue::QueueMode::Convert);
        let id = rows[0].id;
        let end = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while !matches!(
            queue.lock().unwrap().get(id).unwrap().state,
            crate::queue::RowState::Done { .. }
        ) {
            assert!(std::time::Instant::now() < end, "the dropped row never ran");
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        drop(runner);
        std::fs::remove_dir_all(&dir).unwrap();
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
