// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Persimmon contributors

//! The native drag layer in the app (spec D3, D5): the floating drop
//! panel, Photos originals arriving as rows, and Finder's Services.

use std::path::{Path, PathBuf};
use std::time::Duration;

use persimmon_mac::promises::PromiseEvent;
use persimmon_mac::services::{Service, ServiceRequest};
use tauri::{AppHandle, Manager};

use super::{AppState, emit_note, emit_rows, lock, show_main};
use crate::history::{Batch, ItemOutcome, Source};
use crate::queue::{Queue, Received, Recipe, RowView};

/// After this long, a file still coming from Photos is downloading from
/// iCloud.
const SLOW: Duration = Duration::from_secs(2);

/// Where promised files are written: `cache` is `~/Library/Caches`.
pub(super) fn staging_dir(cache: &Path) -> PathBuf {
    cache.join("Persimmon").join("promised")
}

/// What a Services request adds: the paths as a drop adds them.
pub(super) fn add_service_paths(
    queue: &mut Queue,
    paths: &[PathBuf],
    recipe: &Recipe,
) -> Vec<RowView> {
    queue.add_with(paths, recipe)
}

/// What a Finder batch with failures says when the window isn't in front
/// (spec D9); `None` for any other batch.
pub(super) fn finder_failure_note(batch: &Batch) -> Option<String> {
    if batch.source != Source::Finder {
        return None;
    }
    let n = batch.items.len();
    let done = batch
        .items
        .iter()
        .filter(|i| i.outcome == ItemOutcome::Done)
        .count();
    let failed = batch
        .items
        .iter()
        .filter(|i| matches!(i.outcome, ItemOutcome::Failed { .. }))
        .count();
    (failed > 0).then(|| format!("{done} of {n} converted, {failed} failed"))
}

/// A promised file's progress, from the main window's overlay or the drop
/// panel's (`from_panel`, whose files start as they arrive).
pub(super) fn on_promise(app: &AppHandle, event: PromiseEvent, from_panel: bool) {
    let Some(state) = app.try_state::<AppState>() else {
        return;
    };
    match event {
        PromiseEvent::Started { item, name } => {
            let recipe = state.recipe();
            let view =
                state
                    .queue()
                    .add_receiving(name.as_deref().unwrap_or(""), &recipe, from_panel);
            let id = view.id;
            lock(&state.receiving).insert(item, id);
            emit_rows(app, &[view]);
            let app = app.clone();
            std::thread::spawn(move || {
                std::thread::sleep(SLOW);
                if let Some(state) = app.try_state::<AppState>() {
                    let slow = state.queue().slow(id);
                    if let Some(view) = slow {
                        emit_rows(&app, &[view]);
                    }
                }
            });
        }
        PromiseEvent::Arrived { item, path } => {
            let id = lock(&state.receiving).remove(&item);
            let received = id.map(|id| state.queue().received(id, &path));
            match received {
                Some(Received::Queued { view, start }) => {
                    emit_rows(app, &[*view]);
                    if start {
                        state.runner.start_from(Source::Drop);
                    }
                }
                // Cancelled meanwhile, or not ours: the file isn't wanted.
                _ => {
                    if let Some(folder) = path.parent() {
                        let _ = std::fs::remove_dir_all(folder);
                    }
                }
            }
        }
        PromiseEvent::Failed { item, .. } => {
            let id = lock(&state.receiving).remove(&item);
            let failed = id.and_then(|id| state.queue().receive_failed(id));
            if let Some(view) = failed {
                emit_rows(app, &[view]);
            }
        }
    }
}

/// A Services request from Finder: Open adds the files and shows the
/// window; Convert adds them and starts at once (the wheel comes in week 6).
pub(super) fn on_service(app: &AppHandle, request: ServiceRequest) {
    let Some(state) = app.try_state::<AppState>() else {
        return;
    };
    let recipe = state.recipe();
    let (rows, note) = {
        let mut queue = state.queue();
        (
            add_service_paths(&mut queue, &request.paths, &recipe),
            queue.take_note(),
        )
    };
    emit_rows(app, &rows);
    emit_note(app, note);
    match request.service {
        Service::Open => show_main(app),
        Service::Convert => state.runner.start_from(Source::Finder),
    }
}

/// Set up the native layer: an empty staging folder, the floating drop
/// panel, Photos' promises in both windows, and Finder's Services. Runs on
/// the main thread; a piece that can't be set up is reported and skipped.
pub(super) fn install(app: &AppHandle, staging: &Path) {
    persimmon_mac::promises::clear_staging(staging);
    let report = |what: &str, r: Result<(), String>| {
        if let Err(e) = r {
            eprintln!("persimmon: couldn't set up {what}: {e}");
        }
    };
    for (label, from_panel) in [("main", false), ("drop", true)] {
        if let Some(window) = app.get_webview_window(label) {
            let app = app.clone();
            report(
                "Photos drops",
                persimmon_mac::promises::install_overlay(
                    &window,
                    staging.to_path_buf(),
                    move |event| on_promise(&app, event, from_panel),
                ),
            );
        }
    }
    // After the overlays: the drop window's overlay moves into the panel
    // with its webview.
    if let Some(drop) = app.get_webview_window("drop") {
        report(
            "the drop panel",
            persimmon_mac::panel::adopt_as_panel(&drop),
        );
    }
    let app = app.clone();
    report(
        "Finder's Services",
        persimmon_mac::services::install(move |request| on_service(&app, request)),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history::{Batch, Item, ItemOutcome, Source};
    use crate::queue::{Queue, QueueMode, Recipe, RowState};
    use persimmon_core::output::OutputPolicy;
    use persimmon_core::registry::Registry;
    use std::path::PathBuf;

    #[test]
    fn staging_lives_in_caches() {
        assert_eq!(
            staging_dir(std::path::Path::new("/c")),
            PathBuf::from("/c/Persimmon/promised")
        );
    }

    #[test]
    fn a_service_request_adds_like_a_drop() {
        let d =
            std::env::temp_dir().join(format!("prsm-app-native-service-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("folder")).unwrap();
        for p in ["a.png", "folder/b.png", "folder/c.png"] {
            std::fs::write(d.join(p), persimmon_corpus::PNG_1X1).unwrap();
        }
        let preview = d.join("Photos Library.photoslibrary/resources/x.png");
        std::fs::create_dir_all(preview.parent().unwrap()).unwrap();
        std::fs::write(&preview, persimmon_corpus::PNG_1X1).unwrap();
        let mut q = Queue::new(Registry::builtin(), OutputPolicy::default());
        let rows = add_service_paths(
            &mut q,
            &[d.join("a.png"), d.join("folder"), preview.clone()],
            &Recipe::Mode(QueueMode::Convert),
        );
        assert_eq!(rows.len(), 4);
        assert_eq!(
            rows.iter().filter(|r| r.state == RowState::Queued).count(),
            3
        );
        let refused = rows.iter().find(|r| r.path == preview).unwrap();
        let RowState::Failed { error } = &refused.state else {
            panic!("{refused:?}")
        };
        assert_eq!(error.reason, "This is Photos' preview, not your original");
        let _ = std::fs::remove_dir_all(&d);
    }

    fn batch(source: Source, outcomes: &[bool]) -> Batch {
        let items = outcomes
            .iter()
            .map(|ok| Item {
                inputs: vec![PathBuf::from("/x/a.png")],
                output: None,
                output_size: None,
                output_mtime: None,
                size: 1,
                outcome: if *ok {
                    ItemOutcome::Done
                } else {
                    ItemOutcome::Failed {
                        error: crate::wording::app_error(
                            persimmon_core::error::ErrorCode::CorruptInput,
                            std::path::Path::new("/x/a.png"),
                            "it's damaged",
                            None,
                        ),
                    }
                },
                undone: false,
                to: None,
                mode: None,
                moved: None,
                restored: false,
            })
            .collect();
        Batch {
            id: "b".into(),
            started: 1,
            finished: 2,
            source,
            preset: None,
            mode: QueueMode::Convert,
            items,
            undone: false,
            rule: None,
            folder: None,
        }
    }

    #[test]
    fn a_finder_batch_with_failures_has_a_note() {
        assert_eq!(
            finder_failure_note(&batch(Source::Finder, &[true, true, false])).as_deref(),
            Some("2 of 3 converted, 1 failed")
        );
        assert_eq!(
            finder_failure_note(&batch(Source::Finder, &[true, true])),
            None
        );
        assert_eq!(
            finder_failure_note(&batch(Source::Window, &[true, false])),
            None
        );
    }
}
