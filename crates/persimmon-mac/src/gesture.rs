// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Persimmon contributors

//! The wheel's gesture (spec D4): ⇧ held before or during a file drag
//! anywhere. Seen with global event monitors and the drag pasteboard
//! alone, which need no permission: no Accessibility, no Input Monitoring,
//! no key events.

use std::cell::RefCell;
use std::path::PathBuf;
use std::ptr::NonNull;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{ClassType, MainThreadMarker};
use objc2_app_kit::{
    NSEvent, NSEventMask, NSEventModifierFlags, NSEventType, NSFilePromiseReceiver, NSPasteboard,
    NSPasteboardNameDrag, NSPasteboardTypeFileURL,
};
use objc2_foundation::{NSArray, NSURL};

use crate::services::{unique_paths, url_path};

/// Whether the drag's files are read when the wheel shows, before the
/// drop. Reading the drag pasteboard reported "always allow" with no
/// alert; if a macOS release asks, this turns off and the wheel offers the
/// recent formats until the drop says what the files are.
pub const READ_BEFORE_DROP: bool = true;

/// What the app hears.
#[derive(Debug, Clone, PartialEq)]
pub enum Gesture {
    /// ⇧ is held during a drag: show the wheel at `at`, the pointer in
    /// AppKit's screen coordinates (origin at the main screen's bottom
    /// left). `files` and `promises` are what the drag carries (both empty
    /// when [`READ_BEFORE_DROP`] is off).
    Show {
        at: (f64, f64),
        files: Vec<PathBuf>,
        promises: usize,
    },
    /// The pointer moved while the wheel shows, to `at` (the same
    /// coordinates as `Show`).
    Move { at: (f64, f64) },
    /// The drag ended, wherever it was released.
    Hide,
}

/// What a session's event calls for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Show,
    /// The pointer moved while shown.
    Move,
    Hide,
}

/// One drag at a time, from the events the monitors see. A drag is a
/// file drag (or any drag that carries data) when the drag pasteboard's
/// `changeCount` moved since the last one; a plain mouse drag, selecting
/// text or moving a window, leaves it alone.
#[derive(Debug, Clone)]
pub struct Session {
    /// The count the last drag left (or the one at start).
    baseline: isize,
    /// The count the current drag has.
    count: isize,
    dragging: bool,
    shown: bool,
}

impl Session {
    /// A session that knows the drag pasteboard's count now.
    pub fn new(change_count: isize) -> Session {
        Session {
            baseline: change_count,
            count: change_count,
            dragging: false,
            shown: false,
        }
    }

    /// The mouse moved with its button down.
    pub fn on_drag(&mut self, change_count: isize, shift: bool) -> Option<Step> {
        self.count = change_count;
        if !self.dragging {
            if change_count == self.baseline {
                return None;
            }
            self.dragging = true;
        }
        if self.shown {
            return Some(Step::Move);
        }
        self.show_if(shift)
    }

    /// A modifier key went down or up.
    pub fn on_flags(&mut self, shift: bool) -> Option<Step> {
        if self.dragging {
            self.show_if(shift)
        } else {
            None
        }
    }

    /// The button went up: the drag is over, dropped or not.
    pub fn on_up(&mut self) -> Option<Step> {
        let shown = self.shown;
        if self.dragging {
            self.baseline = self.count;
        }
        self.dragging = false;
        self.shown = false;
        shown.then_some(Step::Hide)
    }

    fn show_if(&mut self, shift: bool) -> Option<Step> {
        if shift && !self.shown {
            self.shown = true;
            Some(Step::Show)
        } else {
            None
        }
    }
}

thread_local! {
    /// The monitors [`start`] added, kept for [`stop`]: main thread only.
    static MONITORS: RefCell<Vec<Retained<AnyObject>>> = const { RefCell::new(Vec::new()) };
}

/// Whether `PERSIMMON_WHEEL_LOG=1` asks for a line per gesture on stderr.
fn logging() -> bool {
    std::env::var_os("PERSIMMON_WHEEL_LOG").is_some_and(|v| v == "1")
}

fn drag_board() -> Retained<NSPasteboard> {
    #[allow(unsafe_code)]
    // SAFETY: NSPasteboardNameDrag is an immutable string constant AppKit
    // defines for the process's whole life.
    let name = unsafe { NSPasteboardNameDrag };
    NSPasteboard::pasteboardWithName(name)
}

/// Watch for a ⇧-drag anywhere and tell `on`, on the main thread. Stops
/// any watch already running first. Main thread only.
pub fn start(on: impl Fn(Gesture) + Send + Sync + 'static) -> Result<(), String> {
    if MainThreadMarker::new().is_none() {
        return Err("not on the main thread".into());
    }
    stop();
    let on = Arc::new(on);
    let board = drag_board();
    let session = Arc::new(Mutex::new(Session::new(board.changeCount())));
    if logging() {
        eprintln!(
            "wheel: watching; drag pasteboard accessBehavior {:?}",
            board.accessBehavior()
        );
    }
    // AppKit calls a global monitor's handler on the main thread, for
    // events sent to other apps (Apple's "Monitoring Events" guide).
    let handler = RcBlock::new(move |event: NonNull<NSEvent>| {
        #[allow(unsafe_code)]
        // SAFETY: AppKit passes a live event that outlives the handler call.
        let event = unsafe { event.as_ref() };
        let shift = NSEvent::modifierFlags_class().contains(NSEventModifierFlags::Shift);
        let step = {
            let mut s = session.lock().unwrap_or_else(|p| p.into_inner());
            match event.r#type() {
                NSEventType::LeftMouseDragged => s.on_drag(drag_board().changeCount(), shift),
                NSEventType::FlagsChanged => s.on_flags(shift),
                NSEventType::LeftMouseUp => s.on_up(),
                _ => None,
            }
        };
        match step {
            Some(Step::Show) => on(show()),
            Some(Step::Move) => on(Gesture::Move { at: pointer() }),
            Some(Step::Hide) => {
                if logging() {
                    eprintln!("wheel: hide");
                }
                on(Gesture::Hide)
            }
            None => {}
        }
    });
    let mask = NSEventMask::LeftMouseDragged | NSEventMask::FlagsChanged | NSEventMask::LeftMouseUp;
    let monitor = NSEvent::addGlobalMonitorForEventsMatchingMask_handler(mask, &handler)
        .ok_or("macOS didn't add the event monitor")?;
    MONITORS.with(|m| m.borrow_mut().push(monitor));
    Ok(())
}

/// Where the pointer is, in AppKit's screen coordinates (origin at the
/// main screen's bottom left).
pub fn pointer() -> (f64, f64) {
    let p = NSEvent::mouseLocation();
    (p.x, p.y)
}

/// Stop watching (the wheel switch turned off). Harmless when nothing is
/// watched, or off the main thread, where nothing ever is.
pub fn stop() {
    MONITORS.with(|m| {
        for monitor in m.borrow_mut().drain(..) {
            #[allow(unsafe_code)]
            // SAFETY: every entry is a monitor object that
            // addGlobalMonitorForEventsMatchingMask:handler: returned, and
            // each is removed once (drained here).
            unsafe {
                NSEvent::removeMonitor(&monitor)
            };
        }
    });
}

/// The Show for the drag in progress: where the pointer is and, when
/// [`READ_BEFORE_DROP`], what the drag carries.
fn show() -> Gesture {
    let started = Instant::now();
    let p = NSEvent::mouseLocation();
    let board = drag_board();
    let (files, promises) = if READ_BEFORE_DROP {
        carried(&board)
    } else {
        (Vec::new(), 0)
    };
    if logging() {
        eprintln!(
            "wheel: show at ({:.0}, {:.0}); {} files, {} promises; read in {:?}; accessBehavior {:?}",
            p.x,
            p.y,
            files.len(),
            promises,
            started.elapsed(),
            board.accessBehavior()
        );
    }
    Gesture::Show {
        at: (p.x, p.y),
        files,
        promises,
    }
}

/// The files a drag pasteboard holds, and how many of its items are file
/// promises with no file of their own (a Photos asset, a Mail attachment).
fn carried(board: &NSPasteboard) -> (Vec<PathBuf>, usize) {
    let classes = NSArray::from_slice(&[NSURL::class()]);
    #[allow(unsafe_code)]
    // SAFETY: the classes array holds a class object, which is what the
    // call reads; no options are passed.
    let objects = unsafe { board.readObjectsForClasses_options(&classes, None) };
    let files = unique_paths(
        objects
            .map(|a| {
                a.iter()
                    .filter_map(|o| o.downcast::<NSURL>().ok())
                    .filter_map(|u| url_path(&u))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default(),
    );
    let promised: Vec<String> = NSFilePromiseReceiver::readableDraggedTypes()
        .iter()
        .map(|t| t.to_string())
        .collect();
    #[allow(unsafe_code)]
    // SAFETY: NSPasteboardTypeFileURL is an immutable string constant
    // AppKit defines for the process's whole life.
    let file_url = unsafe { NSPasteboardTypeFileURL }.to_string();
    let promises = board
        .pasteboardItems()
        .map(|items| {
            items
                .iter()
                .filter(|item| {
                    let types: Vec<String> = item.types().iter().map(|t| t.to_string()).collect();
                    !types.contains(&file_url) && types.iter().any(|t| promised.contains(t))
                })
                .count()
        })
        .unwrap_or(0);
    (files, promises)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_shift_drag_shows_once_and_hides_on_up() {
        let mut s = Session::new(7);
        assert_eq!(s.on_drag(8, true), Some(Step::Show));
        assert_eq!(s.on_drag(8, true), Some(Step::Move));
        assert_eq!(s.on_flags(true), None);
        assert_eq!(s.on_up(), Some(Step::Hide));
        assert_eq!(s.on_up(), None);
    }

    /// Once the wheel shows, every move of the drag says where the
    /// pointer is, so the wheel can light the segment under it whatever
    /// the drag carries (a Photos drag never reaches the wheel's page).
    #[test]
    fn a_shown_drag_reports_its_moves() {
        let mut s = Session::new(7);
        assert_eq!(s.on_drag(8, false), None, "not shown: no moves");
        assert_eq!(s.on_drag(8, true), Some(Step::Show));
        assert_eq!(
            s.on_drag(8, false),
            Some(Step::Move),
            "⇧ let go: still shown"
        );
        assert_eq!(s.on_up(), Some(Step::Hide));
        assert_eq!(s.on_drag(8, true), None);
    }

    #[test]
    fn shift_pressed_mid_drag_shows_without_moving() {
        let mut s = Session::new(7);
        assert_eq!(s.on_drag(8, false), None);
        assert_eq!(s.on_flags(true), Some(Step::Show));
        assert_eq!(s.on_up(), Some(Step::Hide));
    }

    #[test]
    fn a_drag_without_shift_never_shows() {
        let mut s = Session::new(7);
        assert_eq!(s.on_drag(8, false), None);
        assert_eq!(s.on_drag(8, false), None);
        assert_eq!(s.on_flags(false), None);
        assert_eq!(s.on_up(), None);
    }

    #[test]
    fn a_new_session_needs_a_new_change_count() {
        let mut s = Session::new(7);
        // A plain mouse drag (selecting text, moving a window) leaves the
        // drag pasteboard alone: no session, whatever ⇧ does.
        assert_eq!(s.on_drag(7, true), None);
        assert_eq!(s.on_flags(true), None);
        assert_eq!(s.on_up(), None);
        assert_eq!(s.on_drag(8, true), Some(Step::Show));
        assert_eq!(s.on_up(), Some(Step::Hide));
        // The same count again is the last drag's leftovers.
        assert_eq!(s.on_drag(8, true), None);
        assert_eq!(s.on_up(), None);
        assert_eq!(s.on_drag(9, true), Some(Step::Show));
    }

    #[test]
    fn flags_outside_a_drag_do_nothing() {
        let mut s = Session::new(7);
        assert_eq!(s.on_flags(true), None);
        assert_eq!(s.on_flags(false), None);
    }

    #[test]
    fn starting_off_the_main_thread_refuses() {
        assert_eq!(start(|_| {}), Err("not on the main thread".to_string()));
        // Stopping is harmless anywhere.
        stop();
    }
}
