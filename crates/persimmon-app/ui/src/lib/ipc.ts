// SPDX-License-Identifier: GPL-3.0-or-later
// Typed calls to the app's commands (crates/persimmon-app/src/app.rs).
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { FormatId, RowId, RowView, SetTarget } from "./types";

export const addPaths = (paths: string[]) => invoke<RowView[]>("add_paths", { paths });
export const pickFiles = () => invoke<RowView[]>("pick_files");
export const setTarget = (ids: RowId[], to: FormatId) => invoke<SetTarget>("set_target", { ids, to });
export const targets = (ids: RowId[]) => invoke<FormatId[]>("targets", { ids });
export const removeRows = (ids: RowId[]) => invoke<RowId[]>("remove_rows", { ids });
export const queueRows = () => invoke<RowView[]>("queue_rows");
export const start = () => invoke<void>("start");
export const cancelRow = (id: RowId) => invoke<boolean>("cancel_row", { id });
export const retryRow = (id: RowId) => invoke<RowView | null>("retry_row", { id });
/** Null when revealed; otherwise why not ("… was moved or deleted"). */
export const revealRow = (id: RowId) => invoke<string | null>("reveal_row", { id });
export const quitNow = () => invoke<void>("quit_now");

export const onRow = (f: (row: RowView) => void): Promise<UnlistenFn> =>
  listen<RowView>("row", (e) => f(e.payload));
/** Quitting asks first: how many rows are running and rules are active. */
export const onConfirmQuit = (f: (p: { running: number; rules: number } | null) => void): Promise<UnlistenFn> =>
  listen<{ running: number; rules: number } | null>("confirm-quit", (e) => f(e.payload));

import type { QueueMode, SettingsView, WheelShow } from "./types";

export const dropPaths = (paths: string[]) => invoke<RowView[]>("drop_paths", { paths });
/** The wheel: a drop with its point, in the wheel window's points. */
export const wheelDrop = (paths: string[], x: number, y: number) => invoke<void>("wheel_drop", { paths, x, y });
/** The wheel: a click on a segment's index, the hub (null), or -1 to cancel. */
export const wheelPick = (index: number | null) => invoke<void>("wheel_pick", { index });
export const onWheelShow = (f: (w: WheelShow) => void): Promise<UnlistenFn> =>
  listen<WheelShow>("wheel-show", (e) => f(e.payload));
export const reorderInput = (id: RowId, from: number, to: number) => invoke<RowView | null>("reorder_input", { id, from, to });
export const removeInput = (id: RowId, index: number) => invoke<RowView | null>("remove_input", { id, index });
export const getSettings = () => invoke<SettingsView>("get_settings");
export const setMode = (mode: QueueMode) => invoke<SettingsView>("set_mode", { mode });
export const setOutputBeside = () => invoke<SettingsView>("set_output_beside");
export const chooseOutputFolder = () => invoke<SettingsView>("choose_output_folder");
export const chooseFallbackFolder = () => invoke<SettingsView>("choose_fallback_folder");
export const toggleDropWindow = () => invoke<boolean>("toggle_drop_window");
export const openSettings = () => invoke<void>("open_settings");
export const onSettings = (f: (s: SettingsView) => void): Promise<UnlistenFn> =>
  listen<SettingsView>("settings", (e) => f(e.payload));

import type { BatchView, EnginesView, HistoryFilter, JobOptions, PresetView, UndoReport } from "./types";
import type { AddNote } from "./format";

export type PresetDraft = { id: string; name: string; builtin: boolean; mode: QueueMode; target: FormatId | null; options: JobOptions };
export const presets = () => invoke<PresetView[]>("presets");
export const usePreset = (id: string | null) => invoke<SettingsView>("use_preset", { id });
export const applyPreset = (ids: RowId[], id: string) => invoke<RowView[]>("apply_preset", { ids, id });
export const savePreset = (preset: PresetDraft) => invoke<PresetView[]>("save_preset", { preset });
export const deletePreset = (id: string) => invoke<PresetView[]>("delete_preset", { id });
export const history = (text: string, filter: HistoryFilter) => invoke<BatchView[]>("history", { text, filter });
export const undoBatch = (id: string) => invoke<UndoReport>("undo_batch", { id });
export const undoLast = () => invoke<UndoReport | null>("undo_last");
export const clearHistory = () => invoke<boolean>("clear_history");
export const retryItem = (batch: string, index: number) => invoke<RowView[]>("retry_item", { batch, index });
export const revealPath = (path: string) => invoke<string | null>("reveal_path", { path });
export const engines = () => invoke<EnginesView>("engines");
export const setDropWindowAtLaunch = (on: boolean) => invoke<SettingsView>("set_drop_window_at_launch", { on });
export const openSources = () => invoke<boolean>("open_sources");
export const openLicences = () => invoke<boolean>("open_licences");
export const onHistory = (f: (b: BatchView) => void): Promise<UnlistenFn> => listen<BatchView>("history", (e) => f(e.payload));
export const onHistoryCleared = (f: () => void): Promise<UnlistenFn> => listen("history-cleared", () => f());
export const onUndoReport = (f: (r: UndoReport) => void): Promise<UnlistenFn> => listen<UndoReport>("undo-report", (e) => f(e.payload));
export const onQueueNote = (f: (note: AddNote) => void): Promise<UnlistenFn> => listen<AddNote>("queue-note", (e) => f(e.payload));
export const onPresets = (f: (p: PresetView[]) => void): Promise<UnlistenFn> => listen<PresetView[]>("presets", (e) => f(e.payload));

// Watch folders (spec D2, D9; app.rs's watch commands).
import type { NotificationsState, Rule, RuleDraftView, RuleView } from "./types";
import { midnight } from "./watch";

/** Every rule as the view shows it; `since` is local midnight, for "today". */
export const watchRules = (since: number) => invoke<RuleView[]>("watch_rules", { since });
export const watchTargets = () => invoke<FormatId[]>("watch_targets");
export const watchPatterns = (target: FormatId) => invoke<string>("watch_patterns", { target });
/** The open panel for a folder; null if cancelled. */
export const pickWatchFolder = () => invoke<string | null>("pick_watch_folder");
export const newRule = (folder: string, target: FormatId) => invoke<RuleDraftView>("new_rule", { folder, target });
/** The rule as it stands: its command line and the files already in its folder. */
export const ruleDraft = (rule: Rule) => invoke<RuleDraftView>("rule_draft", { rule });
/** Rejects with a Shown when the rule is refused. */
export const saveRule = (rule: Rule, existing: boolean) => invoke<RuleView[]>("save_rule", { rule, existing, since: midnight(new Date()) });
export const toggleRule = (id: string, on: boolean) => invoke<RuleView[]>("toggle_rule", { id, on, since: midnight(new Date()) });
export const removeRule = (id: string) => invoke<RuleView[]>("remove_rule", { id, since: midnight(new Date()) });
export const scanRule = (id: string) => invoke<void>("scan_rule", { id });
export const copyCommand = (id: string) => invoke<boolean>("copy_command", { id });
/** Rejects with the reason when macOS refuses. */
export const setStartAtLogin = (on: boolean) => invoke<SettingsView>("set_start_at_login", { on });
export const notifications = () => invoke<NotificationsState>("notifications");
export const openNotificationSettings = () => invoke<boolean>("open_notification_settings");
export const onWatch = (f: () => void): Promise<UnlistenFn> => listen("watch", () => f());
/** A notification's Undo: show this batch in History. */
export const onShowBatch = (f: (id: string) => void): Promise<UnlistenFn> => listen<string>("show-batch", (e) => f(e.payload));

/** System Settings → Keyboard → Keyboard Shortcuts, where Services are shown or hidden. */
export const openKeyboardShortcuts = () => invoke<boolean>("open_keyboard_shortcuts");
