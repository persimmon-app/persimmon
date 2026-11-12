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
export const revealRow = (id: RowId) => invoke<boolean>("reveal_row", { id });
export const quitNow = () => invoke<void>("quit_now");

export const onRow = (f: (row: RowView) => void): Promise<UnlistenFn> =>
  listen<RowView>("row", (e) => f(e.payload));
export const onConfirmQuit = (f: () => void): Promise<UnlistenFn> =>
  listen("confirm-quit", () => f());

import type { QueueMode, SettingsView } from "./types";

export const dropPaths = (paths: string[]) => invoke<RowView[]>("drop_paths", { paths });
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
