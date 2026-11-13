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

use crate::history::{Batch, Filter, History, Source, SystemTrash, UndoReport, undoable};
use crate::presets::{Preset, Presets, command, kinds, presets_dir};
use crate::queue::{Queue, QueueMode, Recipe, RowId, RowView, SetTarget, kinds_phrase};
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
    presets: Mutex<Presets>,
    history: Mutex<History>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

impl AppState {
    fn queue(&self) -> MutexGuard<'_, Queue> {
        lock(&self.queue)
    }

    /// How new files become rows: the active preset, else the mode. Locks
    /// are taken settings, then presets, and released before the queue's.
    fn recipe(&self) -> Recipe {
        let (mode, preset) = {
            let s = lock(&self.settings);
            (s.mode, s.preset.clone())
        };
        preset
            .and_then(|id| lock(&self.presets).get(&id).cloned())
            .map(Recipe::Preset)
            .unwrap_or(Recipe::Mode(mode))
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
    recipe: &Recipe,
) -> Vec<RowView> {
    let rows = lock(queue).add_with(paths, recipe);
    runner.start_from(Source::Drop);
    rows
}

/// Reveal opens only an output history recorded, never a path the window
/// makes up.
pub fn reveal_allowed(history: &History, path: &Path) -> bool {
    history
        .batches()
        .iter()
        .any(|b| b.items.iter().any(|i| i.output.as_deref() == Some(path)))
}

#[tauri::command]
fn engine_version() -> String {
    persimmon_core::VERSION.to_string()
}

#[tauri::command]
fn add_paths(app: AppHandle, state: State<'_, AppState>, paths: Vec<PathBuf>) -> Vec<RowView> {
    let recipe = state.recipe();
    let (rows, already) = {
        let mut queue = state.queue();
        let already = queue.already_queued(&paths);
        (queue.add_with(&paths, &recipe), already)
    };
    emit_rows(&app, &rows);
    note_already(&app, already);
    rows
}

#[tauri::command]
fn drop_paths(app: AppHandle, state: State<'_, AppState>, paths: Vec<PathBuf>) -> Vec<RowView> {
    let recipe = state.recipe();
    let already = state.queue().already_queued(&paths);
    let rows = add_and_start(&state.queue, &state.runner, &paths, &recipe);
    emit_rows(&app, &rows);
    note_already(&app, already);
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
    let recipe = state.recipe();
    let (rows, already) = {
        let mut queue = state.queue();
        let already = queue.already_queued(&paths);
        (queue.add_with(&paths, &recipe), already)
    };
    emit_rows(&app, &rows);
    note_already(&app, already);
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
/// Reveal a done row's output; a message when it was moved or deleted.
fn reveal_row(state: State<'_, AppState>, id: RowId) -> Option<String> {
    let output = state.queue().get(id).and_then(|v| match v.state {
        crate::queue::RowState::Done { output, .. } => Some(output),
        _ => None,
    })?;
    reveal_message(&output).or_else(|| {
        crate::reveal::reveal(&output);
        None
    })
}

#[tauri::command]
fn get_settings(state: State<'_, AppState>) -> SettingsView {
    view(&lock(&state.settings))
}

#[tauri::command]
fn set_mode(app: AppHandle, state: State<'_, AppState>, mode: QueueMode) -> SettingsView {
    // Picking a mode leaves the active preset.
    change_settings(&app, &state, |s| {
        s.mode = mode;
        s.preset = None;
    })
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

/// A preset as the windows show it: with its command line, the kinds it
/// takes, and its ⌘ number.
#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct PresetView {
    #[serde(flatten)]
    preset: Preset,
    command: String,
    kinds: String,
    shortcut: Option<u8>,
}

fn preset_views(presets: &Presets) -> Vec<PresetView> {
    let reg = Registry::builtin();
    presets
        .all()
        .iter()
        .enumerate()
        .map(|(i, p)| PresetView {
            preset: p.clone(),
            command: command(p),
            kinds: kinds_phrase(&kinds(p, &reg)),
            shortcut: (i < 9).then(|| u8::try_from(i + 1).unwrap_or(0)),
        })
        .collect()
}

#[tauri::command]
fn presets(state: State<'_, AppState>) -> Vec<PresetView> {
    preset_views(&lock(&state.presets))
}

/// Make a preset (or none) active for new files.
#[tauri::command]
fn use_preset(app: AppHandle, state: State<'_, AppState>, id: Option<String>) -> SettingsView {
    let mode = id
        .as_deref()
        .and_then(|id| lock(&state.presets).get(id).map(|p| p.mode));
    change_settings(&app, &state, |s| {
        if let Some(mode) = mode {
            s.mode = mode;
        }
        s.preset = id.filter(|_| mode.is_some());
    })
}

#[tauri::command]
fn apply_preset(
    app: AppHandle,
    state: State<'_, AppState>,
    ids: Vec<RowId>,
    id: String,
) -> Vec<RowView> {
    let Some(preset) = lock(&state.presets).get(&id).cloned() else {
        return Vec::new();
    };
    let rows = state.queue().apply_preset(&ids, &preset);
    emit_rows(&app, &rows);
    rows
}

#[tauri::command]
fn save_preset(
    app: AppHandle,
    state: State<'_, AppState>,
    preset: Preset,
) -> Result<Vec<PresetView>, String> {
    let views = {
        let mut presets = lock(&state.presets);
        presets.save(preset)?;
        preset_views(&presets)
    };
    let _ = app.emit("presets", &views);
    Ok(views)
}

#[tauri::command]
fn delete_preset(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
) -> Result<Vec<PresetView>, String> {
    let views = {
        let mut presets = lock(&state.presets);
        presets.delete(&id)?;
        preset_views(&presets)
    };
    if lock(&state.settings).preset.as_deref() == Some(id.as_str()) {
        change_settings(&app, &state, |s| s.preset = None);
    }
    let _ = app.emit("presets", &views);
    Ok(views)
}

/// A batch as the windows show it: with whether it can still be undone.
#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct BatchView {
    #[serde(flatten)]
    batch: Batch,
    undoable: bool,
}

fn batch_view(b: &Batch) -> BatchView {
    BatchView {
        batch: b.clone(),
        undoable: undoable(b),
    }
}

#[tauri::command]
fn history(state: State<'_, AppState>, text: String, filter: Filter) -> Vec<BatchView> {
    lock(&state.history)
        .search(&text, filter)
        .into_iter()
        .map(batch_view)
        .collect()
}

#[tauri::command]
fn undo_batch(app: AppHandle, state: State<'_, AppState>, id: String) -> UndoReport {
    let (report, batch) = {
        let mut h = lock(&state.history);
        let report = h.undo(&id, &SystemTrash);
        (report, h.get(&id).cloned())
    };
    if let Some(b) = batch {
        let _ = app.emit("history", batch_view(&b));
    }
    report
}

/// Undo the newest batch, if it can still be undone.
#[tauri::command]
fn undo_last(app: AppHandle, state: State<'_, AppState>) -> Option<UndoReport> {
    let id = lock(&state.history).newest_undoable()?.id.clone();
    Some(undo_batch(app, state, id))
}

/// Forget every batch; the files stay.
#[tauri::command]
fn clear_history(app: AppHandle, state: State<'_, AppState>) -> bool {
    let ok = lock(&state.history).clear().is_ok();
    let _ = app.emit("history-cleared", ());
    ok
}

#[tauri::command]
fn reveal_path(state: State<'_, AppState>, path: PathBuf) -> Option<String> {
    if !reveal_allowed(&lock(&state.history), &path) {
        return None;
    }
    reveal_message(&path).or_else(|| {
        crate::reveal::reveal(&path);
        None
    })
}

/// What the Engines pane shows.
#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct EnginesView {
    ffmpeg: Option<persimmon_core::backend::FfmpegInfo>,
    ffmpeg_error: Option<crate::wording::Shown>,
    images: bool,
    rar: bool,
    version: String,
}

/// The FFmpeg the packaged app carries: `ffmpeg`, with `ffprobe` beside
/// it, in the folder of the running binary (`Contents/MacOS`). Found the
/// same way when the app runs from the mounted DMG or a translocated copy.
pub fn bundled_ffmpeg(exe: &Path) -> Option<PathBuf> {
    let dir = exe.parent()?;
    let ffmpeg = dir.join("ffmpeg");
    (ffmpeg.is_file() && dir.join("ffprobe").is_file()).then_some(ffmpeg)
}

/// The engine's backends: the app's own FFmpeg when it carries one (a
/// user's FFmpeg is never needed), else the lookup on PATH, as development
/// builds have no bundled copy.
pub fn app_backends(exe: Option<&Path>) -> Backends {
    // Through a symlink, the bundle is where the link points.
    let exe = exe.map(|e| std::fs::canonicalize(e).unwrap_or_else(|_| e.to_path_buf()));
    match exe.as_deref().and_then(bundled_ffmpeg) {
        Some(ffmpeg) => Backends::new().with_ffmpeg(ffmpeg),
        None => Backends::new(),
    }
}

/// What the window says when an output to reveal isn't there any more;
/// None when it is.
pub fn reveal_message(path: &Path) -> Option<String> {
    (!path.exists()).then(|| {
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        format!("{name} was moved or deleted")
    })
}

/// Tell the main window how many dropped or picked files were already in
/// the queue (none: nothing is said).
fn note_already(app: &AppHandle, n: usize) {
    if n > 0 {
        let _ = app.emit_to("main", "queue-note", n);
    }
}

/// Where a version's release keeps the sources of everything bundled and
/// FFmpeg's configure line.
pub fn sources_url(version: &str) -> String {
    format!("https://github.com/persimmon-app/persimmon/releases/tag/v{version}")
}

#[tauri::command]
async fn engines() -> Result<EnginesView, String> {
    let backends = app_backends(std::env::current_exe().ok().as_deref());
    let (ffmpeg, ffmpeg_error) = match backends.ffmpeg_info() {
        Ok(info) => (Some(info), None),
        Err(e) => (None, Some(crate::wording::shown(&e))),
    };
    Ok(EnginesView {
        ffmpeg,
        ffmpeg_error,
        images: cfg!(feature = "vips"),
        rar: cfg!(feature = "rar"),
        version: env!("CARGO_PKG_VERSION").to_string(),
    })
}

#[tauri::command]
fn set_drop_window_at_launch(app: AppHandle, state: State<'_, AppState>, on: bool) -> SettingsView {
    let view = change_settings(&app, &state, |s| s.drop_window = on);
    set_drop_window(&app, on);
    view
}

/// Open the bundled licence list, a local file. False until the packaged
/// app carries it.
#[tauri::command]
fn open_licences(app: AppHandle) -> bool {
    app.path()
        .resource_dir()
        .map(|d| d.join("THIRD-PARTY-LICENSES.html"))
        .ok()
        .filter(|p| p.is_file())
        .is_some_and(|p| {
            std::process::Command::new("/usr/bin/open")
                .arg(&p)
                .status()
                .is_ok_and(|s| s.success())
        })
}

/// Open this version's release page, which carries the sources, in the
/// browser. The browser fetches it; the app holds no network code (DD-11).
#[tauri::command]
fn open_sources() -> bool {
    std::process::Command::new("/usr/bin/open")
        .arg(sources_url(env!("CARGO_PKG_VERSION")))
        .status()
        .is_ok_and(|s| s.success())
}

/// Every window hides when closed. Closing the main window must not leave
/// the app running with nothing on screen (the hidden drop and Settings
/// windows keep it alive), so it hides too and the Dock icon brings it
/// back; a running conversion carries on.
pub fn hides_on_close(label: &str) -> bool {
    matches!(label, "main" | "drop" | "settings")
}

fn show_main(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.set_focus();
    }
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
            // The question is asked in the main window, so show it.
            show_main(app);
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
            &sep()?,
            &MenuItem::with_id(
                app,
                "undo-last",
                "Undo Last Batch",
                true,
                Some("Alt+CmdOrCtrl+Z"),
            )?,
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
            let backends = app_backends(std::env::current_exe().ok().as_deref());
            let runner = Runner::new(Arc::clone(&queue), backends, move |row| {
                let _ = handle.emit("row", &row);
            });
            let drop_window = settings.drop_window;
            let data = file.parent().map(Path::to_path_buf).unwrap_or_default();
            let history = History::open(&data.join("history.jsonl"));
            let presets = Presets::load(&presets_dir(data.parent().unwrap_or(&data)));
            let handle = app.handle().clone();
            runner.set_on_batch(move |batch| {
                let state = handle.state::<AppState>();
                let _ = lock(&state.history).append(batch.clone());
                let _ = handle.emit("history", batch_view(&batch));
            });
            app.manage(AppState {
                queue,
                runner,
                settings: Mutex::new(settings),
                settings_file: file,
                presets: Mutex::new(presets),
                history: Mutex::new(history),
            });
            let pending = std::mem::take(&mut *lock(&app.state::<Pending>().0));
            if !pending.is_empty() {
                let state = app.state::<AppState>();
                let recipe = state.recipe();
                state.queue().add_with(&pending, &recipe);
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
            "undo-last" => {
                // The main window says what was trashed and what was left
                // alone, whichever window had the menu (spec D3).
                let state = app.state::<AppState>();
                if let Some(report) = undo_last(app.clone(), state) {
                    let _ = app.emit_to("main", "undo-report", report);
                }
            }
            _ => {}
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event
                && hides_on_close(window.label())
            {
                api.prevent_close();
                let _ = window.hide();
                if window.label() == "drop" {
                    let app = window.app_handle();
                    let state = app.state::<AppState>();
                    change_settings(app, &state, |s| s.drop_window = false);
                    set_drop_window(app, false);
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
            quit_now,
            presets,
            use_preset,
            apply_preset,
            save_preset,
            delete_preset,
            history,
            undo_batch,
            undo_last,
            clear_history,
            reveal_path,
            engines,
            set_drop_window_at_launch,
            open_licences,
            open_sources
        ])
        .build(tauri::generate_context!())
        .expect("Persimmon couldn't start");
    app.run(|app, event| match event {
        // Files dropped on the Dock icon or opened with Persimmon: queued
        // with the current mode, shown when the window loads its rows.
        // A click on the Dock icon brings the main window back.
        RunEvent::Reopen { .. } => show_main(app),
        RunEvent::Opened { urls } => {
            let paths = paths_from_urls(&urls);
            match app.try_state::<AppState>() {
                Some(state) => {
                    let recipe = state.recipe();
                    let (rows, already) = {
                        let mut queue = state.queue();
                        let already = queue.already_queued(&paths);
                        (queue.add_with(&paths, &recipe), already)
                    };
                    emit_rows(app, &rows);
                    note_already(app, already);
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
    /// Closing any window hides it: the main window comes back from the
    /// Dock, and conversions keep running.
    #[test]
    fn closing_a_window_hides_it() {
        for label in ["main", "drop", "settings"] {
            assert!(super::hides_on_close(label), "{label}");
        }
    }

    /// Reveal opens only outputs history recorded, never a path the
    /// window makes up.
    #[test]
    fn reveal_path_only_reveals_history_paths() {
        use crate::history::{Batch, History, Item, ItemOutcome, Source};
        let dir = std::env::temp_dir().join(format!("prsm-app-reveal-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut h = History::open(&dir.join("history.jsonl"));
        h.append(Batch {
            id: "a".into(),
            started: 1,
            finished: crate::history::now(),
            source: Source::Window,
            preset: None,
            mode: crate::queue::QueueMode::Convert,
            items: vec![Item {
                inputs: vec!["/x/a.png".into()],
                output: Some("/x/a.jpg".into()),
                output_size: Some(1),
                output_mtime: None,
                size: 1,
                outcome: ItemOutcome::Done,
                undone: false,
            }],
            undone: false,
        })
        .unwrap();
        assert!(super::reveal_allowed(&h, std::path::Path::new("/x/a.jpg")));
        assert!(!super::reveal_allowed(&h, std::path::Path::new("/x/a.png")));
        assert!(!super::reveal_allowed(
            &h,
            std::path::Path::new("/etc/passwd")
        ));
        std::fs::remove_dir_all(&dir).unwrap();
    }

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
                "allow-apply-preset",
                "allow-cancel-row",
                "allow-clear-history",
                "allow-engine-version",
                "allow-get-settings",
                "allow-history",
                "allow-open-settings",
                "allow-pick-files",
                "allow-presets",
                "allow-queue-rows",
                "allow-quit-now",
                "allow-remove-input",
                "allow-remove-rows",
                "allow-reorder-input",
                "allow-retry-row",
                "allow-reveal-path",
                "allow-reveal-row",
                "allow-set-mode",
                "allow-set-target",
                "allow-start",
                "allow-targets",
                "allow-toggle-drop-window",
                "allow-undo-batch",
                "allow-undo-last",
                "allow-use-preset",
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
                "allow-delete-preset",
                "allow-engines",
                "allow-get-settings",
                "allow-open-licences",
                "allow-open-sources",
                "allow-presets",
                "allow-save-preset",
                "allow-set-drop-window-at-launch",
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
        let rows = super::add_and_start(
            &queue,
            &runner,
            &[png],
            &crate::queue::Recipe::Mode(crate::queue::QueueMode::Convert),
        );
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

    /// The colour tokens of one theme, `name → #rrggbb`.
    fn theme(light: bool) -> Vec<(String, String)> {
        let css = include_str!("../ui/src/tokens.css");
        let (dark, light_block) = css
            .split_once("@media (prefers-color-scheme: light)")
            .unwrap();
        (if light { light_block } else { dark })
            .lines()
            .filter_map(|l| l.trim().strip_prefix("--"))
            .filter_map(|l| l.split_once(':'))
            .map(|(k, v)| (k.to_string(), v.trim().trim_end_matches(';').to_string()))
            .filter(|(_, v)| v.starts_with('#'))
            .collect()
    }

    /// WCAG 2.x contrast of two `#rrggbb` colours.
    fn contrast(a: &str, b: &str) -> f64 {
        let lum = |hex: &str| {
            let c = |i: usize| {
                let v = f64::from(u8::from_str_radix(&hex[i..i + 2], 16).unwrap()) / 255.0;
                if v <= 0.040_45 {
                    v / 12.92
                } else {
                    ((v + 0.055) / 1.055).powf(2.4)
                }
            };
            0.2126 * c(1) + 0.7152 * c(3) + 0.0722 * c(5)
        };
        let (x, y) = (lum(a), lum(b));
        (x.max(y) + 0.05) / (x.min(y) + 0.05)
    }

    /// Every text token is AA (4.5:1) on every background, in both themes.
    #[test]
    fn text_tokens_meet_aa_in_both_themes() {
        let mut low = Vec::new();
        for light in [false, true] {
            let t = theme(light);
            let v = |n: &str| {
                t.iter()
                    .find(|(k, _)| k == n)
                    .unwrap_or_else(|| panic!("no --{n}"))
                    .1
                    .clone()
            };
            let mut pairs: Vec<(&str, &str)> = Vec::new();
            for fg in ["text", "muted", "faint", "accent-text", "error-text"] {
                for bg in ["ground", "surface", "raised"] {
                    pairs.push((fg, bg));
                }
            }
            pairs.push(("on-accent", "accent"));
            for (fg, bg) in pairs {
                let r = contrast(&v(fg), &v(bg));
                if r < 4.5 {
                    low.push(format!(
                        "{} --{fg} on --{bg}: {r:.2}",
                        if light { "light" } else { "dark" }
                    ));
                }
            }
        }
        assert!(low.is_empty(), "under 4.5:1:\n{}", low.join("\n"));
    }

    /// No component writes a colour of its own: every colour is a token,
    /// so both themes stay at parity.
    #[test]
    fn no_colour_outside_the_tokens() {
        fn walk(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
            for e in std::fs::read_dir(dir).unwrap() {
                let p = e.unwrap().path();
                if p.is_dir() {
                    walk(&p, out);
                } else if matches!(
                    p.extension().and_then(|e| e.to_str()),
                    Some("svelte" | "css")
                ) && !p.ends_with("tokens.css")
                {
                    out.push(p);
                }
            }
        }
        let mut files = Vec::new();
        walk(
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/src"),
            &mut files,
        );
        assert!(files.len() > 10);
        let mut found = Vec::new();
        for f in files {
            let text = std::fs::read_to_string(&f).unwrap();
            for (n, line) in text.lines().enumerate() {
                let b = line.as_bytes();
                let hex = (0..b.len()).any(|i| {
                    if b[i] != b'#' || (i > 0 && b[i - 1] == b'&') {
                        return false;
                    }
                    let run = b[i + 1..]
                        .iter()
                        .take_while(|c| c.is_ascii_hexdigit())
                        .count();
                    let next = b.get(i + 1 + run).copied().unwrap_or(b' ');
                    matches!(run, 3 | 4 | 6 | 8)
                        && !(next.is_ascii_alphanumeric() || next == b'-' || next == b'_')
                });
                if hex || ["rgb(", "rgba(", "hsl("].iter().any(|c| line.contains(c)) {
                    found.push(format!("{}:{}: {}", f.display(), n + 1, line.trim()));
                }
            }
        }
        assert!(
            found.is_empty(),
            "colours outside tokens.css:\n{}",
            found.join("\n")
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

    /// A fake program that prints `ffmpeg version {version}`.
    fn fake(dir: &std::path::Path, name: &str, version: &str) -> std::path::PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let p = dir.join(name);
        std::fs::write(
            &p,
            format!("#!/bin/sh\necho 'ffmpeg version {version} Copyright'\n"),
        )
        .unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        p
    }

    /// A temp folder shaped like a mounted DMG's `Contents/MacOS`, with a
    /// space in its path as `/Volumes/Persimmon 0.2.0` has.
    fn macos_dir(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir()
            .join(format!("prsm-app-bundled-{name}-{}", std::process::id()))
            .join("Persimmon 0.2.0/MacOS");
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("persimmon-app"), b"").unwrap();
        d
    }

    #[test]
    fn bundled_ffmpeg_is_found_beside_the_binary() {
        let d = macos_dir("found");
        let exe = d.join("persimmon-app");
        let ff = fake(&d, "ffmpeg", "7.1");
        fake(&d, "ffprobe", "7.1");
        assert_eq!(super::bundled_ffmpeg(&exe), Some(ff));
        std::fs::remove_file(d.join("ffprobe")).unwrap();
        assert_eq!(super::bundled_ffmpeg(&exe), None);
        let _ = std::fs::remove_dir_all(d.parent().unwrap().parent().unwrap());
    }

    #[test]
    fn bundled_ffmpeg_wins_over_path() {
        let d = macos_dir("wins");
        let exe = d.join("persimmon-app");
        let ff = fake(&d, "ffmpeg", "7.1");
        fake(&d, "ffprobe", "7.1");
        let info = super::app_backends(Some(&exe)).ffmpeg_info().unwrap();
        // The binary's path is canonical (/var is /private/var on macOS).
        assert_eq!(
            (info.path.canonicalize().unwrap(), info.version.as_str()),
            (ff.canonicalize().unwrap(), "7.1")
        );
        let _ = std::fs::remove_dir_all(d.parent().unwrap().parent().unwrap());
    }

    #[test]
    fn reveal_of_a_missing_output_says_so() {
        let d = macos_dir("reveal");
        let here = d.join("persimmon-app");
        assert_eq!(super::reveal_message(&here), None);
        assert_eq!(
            super::reveal_message(&d.join("gone.jpg")).as_deref(),
            Some("gone.jpg was moved or deleted")
        );
        let _ = std::fs::remove_dir_all(d.parent().unwrap().parent().unwrap());
    }

    #[test]
    fn bundled_ffmpeg_follows_a_symlinked_binary() {
        let d = macos_dir("symlink");
        let ff = fake(&d, "ffmpeg", "7.1");
        fake(&d, "ffprobe", "7.1");
        let elsewhere = d.parent().unwrap().join("elsewhere");
        std::fs::create_dir_all(&elsewhere).unwrap();
        let link = elsewhere.join("persimmon-app");
        std::os::unix::fs::symlink(d.join("persimmon-app"), &link).unwrap();
        let info = super::app_backends(Some(&link)).ffmpeg_info().unwrap();
        assert_eq!(
            info.path.canonicalize().unwrap(),
            ff.canonicalize().unwrap()
        );
        let _ = std::fs::remove_dir_all(d.parent().unwrap().parent().unwrap());
    }

    #[test]
    fn sources_url_names_this_version() {
        assert_eq!(
            super::sources_url("0.2.0"),
            "https://github.com/persimmon-app/persimmon/releases/tag/v0.2.0"
        );
    }
}
