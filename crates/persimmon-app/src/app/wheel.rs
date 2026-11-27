// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Persimmon contributors

//! The wheel in the app (spec D4, D3): what it offers for the dragged
//! files, and what a drop or a click on it means.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use persimmon_core::format::FormatId;
use persimmon_core::inputs::{expand_folder, is_package};
use persimmon_core::registry::{Operation, Registry};
use persimmon_mac::gesture::{Gesture, READ_BEFORE_DROP};
use tauri::{AppHandle, Emitter, Manager};

use super::{AppState, change_settings, emit_note, emit_rows, lock, show_main};
use crate::history::Source;
use crate::queue::{QueueMode, Recipe};
use crate::settings::Settings;
use crate::wheel::{self as geometry, Hit};

/// What the wheel offers for a drag: its files, the formats they can all
/// become, and the hub's text.
#[derive(Debug, Clone, PartialEq)]
pub struct Offer {
    pub files: Vec<PathBuf>,
    pub formats: Vec<FormatId>,
    pub hub: String,
}

/// What a drop or a click on the wheel means.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Choice {
    /// Convert to this format, and start.
    Convert(FormatId),
    /// Add to the queue and show the window, without starting.
    Queue,
    Cancel,
}

fn count(n: usize) -> String {
    match n {
        1 => "1 FILE".to_string(),
        n => format!("{n} FILES"),
    }
}

/// The offer for `paths`: each file detected, a folder counted by the
/// files a drop would add from it, subfolders too. A file nothing can read leaves the
/// files nothing in common.
pub fn offer(reg: &Registry, paths: &[PathBuf], recent: &[FormatId]) -> Offer {
    let detect = |p: &Path| {
        persimmon_core::detect::detect(p, reg.formats())
            .ok()
            .map(|d| d.format)
    };
    let mut found: Vec<Option<FormatId>> = Vec::new();
    for p in paths {
        if p.is_dir() && !is_package(p) {
            if let Ok(folder) = expand_folder(reg, Operation::Convert, None, p, true) {
                found.extend(folder.files.iter().map(|f| detect(f)));
            }
        } else {
            found.push(detect(p));
        }
    }
    let (formats, hub) = match found.iter().copied().collect::<Option<Vec<_>>>() {
        Some(ids) if !ids.is_empty() => (
            geometry::formats(reg, &ids, recent),
            geometry::hub(reg, &ids),
        ),
        // Nothing read: the count alone. Some read and some not: MIXED.
        _ if found.iter().all(Option::is_none) => (Vec::new(), count(found.len())),
        _ => (Vec::new(), format!("MIXED · {}", count(found.len()))),
    };
    Offer {
        files: paths.to_vec(),
        formats,
        hub,
    }
}

/// The offer for `n` promised files and nothing else, which can't be read
/// before they arrive: what a photo can become, recent formats first.
pub fn promised(reg: &Registry, n: usize, recent: &[FormatId]) -> Offer {
    Offer {
        files: Vec::new(),
        formats: geometry::formats(reg, &[FormatId::Heic], recent),
        hub: count(n),
    }
}

/// What shows while the files are still being read: their count, no
/// formats. Opens nothing, so it can run inside the event monitor.
pub fn placeholder(paths: &[PathBuf]) -> Offer {
    Offer {
        files: paths.to_vec(),
        formats: Vec::new(),
        hub: count(paths.len()),
    }
}

/// What landing at `hit` means for `offer`.
pub fn decide(offer: &Offer, hit: Hit) -> Choice {
    match hit {
        Hit::Hub => Choice::Queue,
        Hit::Format(i) => offer
            .formats
            .get(i)
            .map_or(Choice::Cancel, |&t| Choice::Convert(t)),
        Hit::Outside => Choice::Cancel,
    }
}

/// A click from the wheel's window: a segment, the hub (`None`), or `-1`.
pub fn pick_hit(index: Option<i64>) -> Hit {
    match index {
        None => Hit::Hub,
        Some(i) if i >= 0 => Hit::Format(i as usize),
        Some(_) => Hit::Outside,
    }
}

/// `t` was picked on the wheel: it leads the wheel next time.
pub fn chosen(settings: &mut Settings, t: FormatId) {
    geometry::remember(&mut settings.wheel_recent, t);
}

/// The wheel's switch: `true` when turning it on should ask for
/// notifications now (spec D9: once, at the first watch rule or the
/// wheel, whichever comes first).
pub fn switch(settings: &mut Settings, on: bool) -> bool {
    settings.wheel = on;
    let ask = on && !settings.notifications_asked;
    if ask {
        settings.notifications_asked = true;
    }
    ask
}

/// The wheel between showing and a drop or a click.
#[derive(Debug, Default)]
pub(super) struct WheelState {
    offer: Option<Offer>,
    /// Shown by Convert with Persimmon: it waits for a click.
    click: bool,
    /// The segment last lit under a drag's pointer.
    hover: Option<usize>,
}

/// `at`, in AppKit's screen points (y up), as a point in a window whose
/// frame is `(x, y, width, height)`, from its top left (y down).
pub fn local_point(at: (f64, f64), frame: (f64, f64, f64, f64)) -> (f64, f64) {
    let (x, y, _, height) = frame;
    (at.0 - x, y + height - at.1)
}

/// The segment of `n` under the pointer at `at`, for a wheel whose window
/// has `frame`; `None` over the hub or outside the ring.
pub fn hover(at: (f64, f64), frame: (f64, f64, f64, f64), n: usize) -> Option<usize> {
    let (x, y) = local_point(at, frame);
    match geometry::hit(x, y, n) {
        Hit::Format(i) => Some(i),
        Hit::Hub | Hit::Outside => None,
    }
}

/// Each showing's number, so a late hide never hides a newer showing.
static SHOWN: AtomicU64 = AtomicU64::new(0);

/// What the wheel's window draws.
#[derive(Debug, Clone, serde::Serialize)]
struct WheelShow<'a> {
    formats: &'a [FormatId],
    labels: Vec<String>,
    hub: &'a str,
}

fn recent(app: &AppHandle) -> Vec<FormatId> {
    app.try_state::<AppState>()
        .map(|s| lock(&s.settings).wheel_recent.clone())
        .unwrap_or_default()
}

/// Hide the wheel, from any thread.
fn hide(app: &AppHandle) {
    let _ = app.run_on_main_thread(|| {
        let _ = persimmon_mac::panel::set_panel_visible("wheel", false);
    });
}

/// Show the wheel at `at` (AppKit's screen points) with `offer`; `false`
/// when there's no wheel panel. Main thread only.
fn show(app: &AppHandle, at: (f64, f64), offer: Offer, click: bool) -> bool {
    SHOWN.fetch_add(1, Ordering::SeqCst);
    if persimmon_mac::panel::place_panel("wheel", at) != Ok(true) {
        return false;
    }
    let _ = persimmon_mac::panel::set_panel_visible("wheel", true);
    if click {
        let _ = persimmon_mac::panel::focus_panel("wheel");
    }
    let shown = SHOWN.load(Ordering::SeqCst);
    let pending = !offer.files.is_empty();
    let files = offer.files.clone();
    put(
        app,
        if pending { placeholder(&files) } else { offer },
        click,
    );
    if pending {
        // Reading the files (and listing folders) can take a while, or
        // start an iCloud download: never in the event monitor.
        let app = app.clone();
        let recent = recent(&app);
        std::thread::spawn(move || {
            let real = self::offer(&Registry::builtin(), &files, &recent);
            // Only while this showing still waits: a drop or a click
            // before the files were read decided on what was drawn.
            let waiting = app
                .try_state::<AppState>()
                .is_some_and(|s| lock(&s.wheel).offer.is_some());
            if SHOWN.load(Ordering::SeqCst) == shown && waiting {
                put(&app, real, click);
            }
        });
    }
    true
}

/// Draw `offer` and keep it for the drop or the click.
fn put(app: &AppHandle, offer: Offer, click: bool) {
    let shown = WheelShow {
        formats: &offer.formats,
        labels: offer.formats.iter().map(|&f| geometry::label(f)).collect(),
        hub: &offer.hub,
    };
    let _ = app.emit_to("wheel", "wheel-show", &shown);
    if let Some(state) = app.try_state::<AppState>() {
        *lock(&state.wheel) = WheelState {
            offer: Some(offer),
            click,
            hover: None,
        };
    }
}

/// A ⇧-drag seen anywhere. On the main thread, inside the event monitor:
/// the panel shows before anything else happens (the 100 ms target).
fn on_gesture(app: &AppHandle, gesture: Gesture) {
    match gesture {
        Gesture::Show {
            at,
            files,
            promises,
        } => {
            if READ_BEFORE_DROP && !geometry::should_show(files.len(), promises) {
                return;
            }
            let reg = Registry::builtin();
            let recent = recent(app);
            let offer = if !files.is_empty() {
                // Read off the main thread once it shows (see `show`).
                placeholder(&files)
            } else if promises > 0 {
                promised(&reg, promises, &recent)
            } else {
                // Nothing read before the drop: the recent formats.
                Offer {
                    files: Vec::new(),
                    formats: recent.iter().copied().take(geometry::SEGMENTS).collect(),
                    hub: "⇧ DROP".to_string(),
                }
            };
            show(app, at, offer, false);
        }
        Gesture::Move { at } => {
            // The drag's pointer, whatever it carries: a Photos drag never
            // reaches the wheel's page, so the page can't tell for itself.
            let Some(state) = app.try_state::<AppState>() else {
                return;
            };
            let Ok(Some(frame)) = persimmon_mac::panel::panel_frame("wheel") else {
                return;
            };
            let lit = {
                let mut wheel = lock(&state.wheel);
                let Some(n) = wheel.offer.as_ref().map(|o| o.formats.len()) else {
                    return;
                };
                let lit = hover(at, frame, n);
                if lit == wheel.hover {
                    return;
                }
                wheel.hover = lit;
                lit
            };
            let _ = app.emit_to("wheel", "wheel-hover", lit);
        }
        Gesture::Hide => {
            // The drop, if it landed on the wheel, may come just after the
            // drag's end: hide a moment later, unless it showed again.
            let shown = SHOWN.load(Ordering::SeqCst);
            let app = app.clone();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(300));
                let click = app
                    .try_state::<AppState>()
                    .is_some_and(|s| lock(&s.wheel).click);
                if SHOWN.load(Ordering::SeqCst) == shown && !click {
                    hide(&app);
                }
            });
        }
    }
}

/// Watch for the ⇧-drag when `on`, else stop. Any thread.
pub(super) fn apply(app: &AppHandle, on: bool) {
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        if on {
            let app = handle.clone();
            if let Err(e) = persimmon_mac::gesture::start(move |g| on_gesture(&app, g)) {
                eprintln!("persimmon: couldn't watch for the wheel's gesture: {e}");
            }
        } else {
            persimmon_mac::gesture::stop();
            let _ = persimmon_mac::panel::set_panel_visible("wheel", false);
        }
    });
}

/// Carry out `choice` for `paths`: converting and starting (tagged
/// `source`), or queueing and showing the window.
fn act(app: &AppHandle, paths: &[PathBuf], choice: Choice, source: Source) {
    let Some(state) = app.try_state::<AppState>() else {
        return;
    };
    match choice {
        Choice::Convert(t) => {
            let (rows, note) = {
                let mut queue = state.queue();
                let rows = queue.add_with(paths, &Recipe::Mode(QueueMode::Convert));
                let ids: Vec<_> = rows.iter().map(|r| r.id).collect();
                let changed = queue.set_target(&ids, t).changed;
                let rows: Vec<_> = rows
                    .into_iter()
                    .map(|r| changed.iter().find(|c| c.id == r.id).cloned().unwrap_or(r))
                    .collect();
                (rows, queue.take_note())
            };
            emit_rows(app, &rows);
            emit_note(app, note);
            change_settings(app, &state, |s| chosen(s, t));
            state.runner.start_from(source);
        }
        Choice::Queue => {
            let recipe = state.recipe();
            let (rows, note) = {
                let mut queue = state.queue();
                (queue.add_with(paths, &recipe), queue.take_note())
            };
            emit_rows(app, &rows);
            emit_note(app, note);
            show_main(app);
        }
        Choice::Cancel => {}
    }
}

/// The offer the wheel showed, taken (a drop or a click ends it).
fn take(app: &AppHandle) -> Option<(Offer, bool)> {
    let state = app.try_state::<AppState>()?;
    let mut wheel = lock(&state.wheel);
    let click = wheel.click;
    wheel.click = false;
    wheel.offer.take().map(|o| (o, click))
}

/// Files dropped on the wheel's window at `(x, y)`, its points.
#[tauri::command]
pub(super) fn wheel_drop(app: AppHandle, paths: Vec<PathBuf>, x: f64, y: f64) {
    let shown = take(&app).map(|(o, _)| o);
    let offer = shown.unwrap_or_else(|| offer(&Registry::builtin(), &paths, &recent(&app)));
    act(
        &app,
        &paths,
        decide(&offer, geometry::hit(x, y, offer.formats.len())),
        Source::Wheel,
    );
    hide(&app);
}

/// What a click from the wheel's window does.
#[derive(Debug, Clone, PartialEq)]
pub enum Pick {
    /// The wheel waited for a click: carry it out, then close.
    Act(Offer, Choice),
    /// A drag's wheel, and a stray `-1`: leave it for the drop.
    Keep(Offer),
    /// Close the wheel.
    Close,
}

/// What a click `index` means for the wheel's state `taken` (its offer,
/// and whether it waits for a click). A drag's wheel gets no real click
/// while the drag goes on, so one means the drag ended unseen.
pub fn on_pick(taken: Option<(Offer, bool)>, index: Option<i64>) -> Pick {
    match taken {
        Some((offer, true)) => {
            let choice = decide(&offer, pick_hit(index));
            Pick::Act(offer, choice)
        }
        Some((offer, false)) if index == Some(-1) => Pick::Keep(offer),
        _ => Pick::Close,
    }
}

/// A click on the wheel: a segment's index, `None` for the hub, `-1` to
/// cancel. Carried out for the wheel Convert with Persimmon showed.
#[tauri::command]
pub(super) fn wheel_pick(app: AppHandle, index: Option<i64>) {
    match on_pick(take(&app), index) {
        Pick::Act(offer, choice) => act(&app, &offer.files, choice, Source::Finder),
        Pick::Keep(offer) => {
            if let Some(state) = app.try_state::<AppState>() {
                lock(&state.wheel).offer = Some(offer);
            }
            return;
        }
        Pick::Close => {}
    }
    hide(&app);
}

/// A promised file dropped on the wheel at `at` (the window's points):
/// a row that waits for it, set to the format it landed on.
pub(super) fn on_promise_started(
    app: &AppHandle,
    item: u64,
    name: Option<String>,
    photos: bool,
    at: Option<(f64, f64)>,
) {
    let Some(state) = app.try_state::<AppState>() else {
        return;
    };
    let offer = lock(&state.wheel)
        .offer
        .clone()
        .unwrap_or_else(|| promised(&Registry::builtin(), 1, &recent(app)));
    let hit = at.map_or(Hit::Hub, |(x, y)| geometry::hit(x, y, offer.formats.len()));
    let choice = decide(&offer, hit);
    let name = name.as_deref().unwrap_or("");
    let view = match choice {
        Choice::Convert(t) => {
            change_settings(app, &state, |s| chosen(s, t));
            state.queue().add_receiving(
                name,
                &Recipe::Mode(QueueMode::Convert),
                true,
                photos,
                Some(t),
            )
        }
        Choice::Queue => {
            let recipe = state.recipe();
            let view = state
                .queue()
                .add_receiving(name, &recipe, false, photos, None);
            show_main(app);
            view
        }
        // Not wanted: when it arrives, nothing is waiting for it.
        Choice::Cancel => {
            hide(app);
            return;
        }
    };
    lock(&state.receiving).insert(item, view.id);
    emit_rows(app, &[view]);
    hide(app);
}

/// Show the wheel at the pointer for Finder's selected files (spec D3):
/// it waits for a click. `false` when there's no wheel to show. Main
/// thread only.
pub(super) fn show_for(app: &AppHandle, paths: &[PathBuf]) -> bool {
    show(
        app,
        persimmon_mac::gesture::pointer(),
        placeholder(paths),
        true,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wheel::Hit;
    use persimmon_core::format::FormatId::*;
    use persimmon_core::registry::Registry;

    struct Dir(PathBuf);
    impl Dir {
        fn new(name: &str) -> Dir {
            let d =
                std::env::temp_dir().join(format!("prsm-app-wheel-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&d);
            std::fs::create_dir_all(&d).unwrap();
            Dir(d)
        }
        fn put(&self, name: &str, bytes: &[u8]) -> PathBuf {
            let p = self.0.join(name);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, bytes).unwrap();
            p
        }
    }
    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// A QuickTime header: enough for detection.
    const MOV: &[u8] = b"\x00\x00\x00\x14ftypqt  \x00\x00\x00\x00qt  ";

    #[test]
    fn an_offer_for_one_video() {
        let d = Dir::new("video");
        let clip = d.put("clip.mov", MOV);
        let offer = offer(&Registry::builtin(), std::slice::from_ref(&clip), &[]);
        assert_eq!(offer.files, [clip]);
        // The spec's six, less MOV itself, which Convert can't make.
        assert_eq!(offer.formats, [Mp4, Gif, Mp3, Webm, M4a, Wav]);
        assert_eq!(offer.hub, "MOV · 1 FILE");
    }

    #[test]
    fn a_folder_counts_by_its_files() {
        let d = Dir::new("folder");
        d.put("shots/a.png", &persimmon_corpus::PNG_1X1);
        d.put("shots/b.png", &persimmon_corpus::PNG_1X1);
        d.put("shots/notes.txt", b"plain text");
        let offer = offer(&Registry::builtin(), &[d.0.join("shots")], &[Webp]);
        assert_eq!(offer.hub, "PNG · 2 FILES");
        assert_eq!(offer.formats[0], Webp);
    }

    /// The offer counts what a drop would add: subfolders too, so the
    /// formats fit every file that will be converted.
    #[test]
    fn a_folder_offer_counts_its_subfolders_as_a_drop_does() {
        let d = Dir::new("nested");
        d.put("shots/a.png", &persimmon_corpus::PNG_1X1);
        d.put("shots/sub/clip.mov", MOV);
        let offer = offer(&Registry::builtin(), &[d.0.join("shots")], &[]);
        assert_eq!(offer.hub, "MIXED · 2 FILES");
        assert!(offer.formats.is_empty(), "{:?}", offer.formats);
    }

    #[test]
    fn an_unknown_file_leaves_nothing_in_common() {
        let d = Dir::new("unknown");
        let a = d.put("a.png", &persimmon_corpus::PNG_1X1);
        let b = d.put("b.xyz", b"what is this");
        let offer = offer(&Registry::builtin(), &[a, b], &[]);
        assert!(offer.formats.is_empty());
        assert_eq!(offer.hub, "MIXED · 2 FILES");
    }

    #[test]
    fn files_with_no_readable_kind_show_their_count() {
        let d = Dir::new("no-kind");
        let a = d.put("a.xyz", b"what is this");
        assert_eq!(offer(&Registry::builtin(), &[a], &[]).hub, "1 FILE");
        assert_eq!(promised(&Registry::builtin(), 1, &[]).hub, "1 FILE");
    }

    #[test]
    fn promises_alone_offer_what_photos_become() {
        let offer = promised(&Registry::builtin(), 3, &[Mp4, Webp]);
        assert_eq!(offer.hub, "3 FILES");
        assert_eq!(offer.formats[0], Webp, "a photo can't become MP4");
        assert!(offer.formats.contains(&Jpeg));
        assert!(offer.files.is_empty());
    }

    /// What shows at once, inside the event monitor: no file is opened
    /// (a missing one, here, would read as MIXED).
    #[test]
    fn a_placeholder_reads_no_file() {
        let gone = PathBuf::from("/nonexistent/a.mov");
        let p = placeholder(std::slice::from_ref(&gone));
        assert_eq!(p.hub, "1 FILE");
        assert!(p.formats.is_empty());
        assert_eq!(p.files, [gone]);
        assert_eq!(
            placeholder(&[PathBuf::new(), PathBuf::new()]).hub,
            "2 FILES"
        );
    }

    /// A real click on a drag's wheel means the drag is over (Esc ended
    /// it, and the button came up over the wheel, unseen by the monitor):
    /// it closes. A `-1` (a click wheel losing the keyboard late) leaves a
    /// drag's wheel alone.
    #[test]
    fn picks_on_each_kind_of_wheel() {
        let o = Offer {
            files: vec![PathBuf::from("/x/a.mov")],
            formats: vec![Mp4],
            hub: "MOV · 1 FILE".into(),
        };
        assert_eq!(
            on_pick(Some((o.clone(), true)), Some(0)),
            Pick::Act(o.clone(), Choice::Convert(Mp4))
        );
        assert_eq!(
            on_pick(Some((o.clone(), false)), Some(-1)),
            Pick::Keep(o.clone())
        );
        assert_eq!(on_pick(Some((o.clone(), false)), Some(0)), Pick::Close);
        assert_eq!(on_pick(Some((o, false)), None), Pick::Close);
        assert_eq!(on_pick(None, Some(-1)), Pick::Close);
    }

    /// The pointer in AppKit's screen points (y up) as a point in the
    /// wheel's window from its top left: a panel at (100, 200), 320×360.
    #[test]
    fn a_screen_point_becomes_a_wheel_point() {
        let frame = (100.0, 200.0, 320.0, 360.0);
        assert_eq!(local_point((260.0, 400.0), frame), (160.0, 160.0));
        assert_eq!(local_point((100.0, 560.0), frame), (0.0, 0.0));
        assert_eq!(hover((260.0, 520.0), frame, 6), Some(0));
        assert_eq!(hover((260.0, 400.0), frame, 6), None, "the hub");
        assert_eq!(hover((0.0, 0.0), frame, 6), None, "outside");
    }

    #[test]
    fn decide_maps_hits() {
        let offer = Offer {
            files: vec![PathBuf::from("/x/a.mov")],
            formats: vec![Mp4, Gif],
            hub: "MOV · 1 FILE".into(),
        };
        assert_eq!(decide(&offer, Hit::Format(1)), Choice::Convert(Gif));
        assert_eq!(decide(&offer, Hit::Format(2)), Choice::Cancel);
        assert_eq!(decide(&offer, Hit::Hub), Choice::Queue);
        assert_eq!(decide(&offer, Hit::Outside), Choice::Cancel);
    }

    #[test]
    fn a_pick_is_a_hit() {
        assert_eq!(pick_hit(None), Hit::Hub);
        assert_eq!(pick_hit(Some(-1)), Hit::Outside);
        assert_eq!(pick_hit(Some(4)), Hit::Format(4));
    }

    /// Spec D9: notifications are asked for when the first watch rule is
    /// saved or the wheel is turned on, once.
    #[test]
    fn turning_the_wheel_on_asks_for_notifications_once() {
        let mut s = crate::settings::Settings::default();
        assert!(!switch(&mut s, false), "turning it off asks nothing");
        assert!(switch(&mut s, true));
        assert!(s.wheel && s.notifications_asked);
        assert!(!switch(&mut s, false));
        assert!(!switch(&mut s, true), "asked already");
    }

    #[test]
    fn a_wheel_conversion_is_remembered() {
        let mut s = crate::settings::Settings {
            wheel_recent: vec![Mp4, Gif],
            ..Default::default()
        };
        chosen(&mut s, Gif);
        assert_eq!(s.wheel_recent, [Gif, Mp4]);
        chosen(&mut s, Webp);
        assert_eq!(s.wheel_recent, [Webp, Gif, Mp4]);
    }
}
