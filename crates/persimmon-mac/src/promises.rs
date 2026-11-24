// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Persimmon contributors

//! Photos' originals, through file promises (spec D5): a transparent view
//! over a window takes a drag that carries promises (Photos offers the
//! original that way, and a preview as a plain file), has each file
//! written into a staging folder of its own, and says when each arrives.
//! Finder drags carry no promises and go on to the window's own handling.

use std::path::{Path, PathBuf};
use std::ptr::NonNull;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2::{ClassType, DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{
    NSAutoresizingMaskOptions, NSDragOperation, NSDraggingDestination, NSDraggingInfo,
    NSFilePromiseReceiver, NSView, NSWindowOrderingMode,
};
use objc2_foundation::{
    NSArray, NSDictionary, NSError, NSObjectProtocol, NSOperationQueue, NSPoint, NSString, NSURL,
};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

/// What Photos puts on a drag besides its promises.
const PHOTOS_ASSET: &str = "com.apple.photos.object-reference.asset";

/// A promised file's progress.
#[derive(Debug, Clone, PartialEq)]
pub enum PromiseEvent {
    /// A promised file is on its way; `name` when the sender gave one,
    /// `photos` when the sender is Photos (whose originals may download
    /// from iCloud first).
    Started {
        item: u64,
        name: Option<String>,
        photos: bool,
        /// Where it was dropped, in points from the window's top left.
        at: Option<(f64, f64)>,
    },
    /// It's complete at `path`.
    Arrived { item: u64, path: PathBuf },
    /// It couldn't be written; the system's words.
    Failed { item: u64, reason: String },
}

type Sink = Arc<dyn Fn(PromiseEvent) + Send + Sync>;

static DROPS: AtomicU64 = AtomicU64::new(1);
static ITEMS: AtomicU64 = AtomicU64::new(1);

/// `base/<drop>-<item>`, made: one folder per promised file, so two files
/// with the same name never meet.
pub fn staging_folder(base: &Path, drop: u64, item: u64) -> std::io::Result<PathBuf> {
    let dir = base.join(format!("{drop}-{item}"));
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// Remove `base` and everything in it: what's left from an earlier run.
pub fn clear_staging(base: &Path) {
    let _ = std::fs::remove_dir_all(base);
}

pub struct OverlayIvars {
    staging: PathBuf,
    sink: Sink,
}

define_class!(
    // SAFETY: NSView has no subclassing requirements this class breaks; its
    // instance variables are plain Rust values, set before init.
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[name = "PersimmonPromiseOverlay"]
    #[ivars = OverlayIvars]
    struct Overlay;

    impl Overlay {
        // Clicks and scrolling go through to the webview below.
        #[unsafe(method(hitTest:))]
        fn hit_test(&self, _point: NSPoint) -> *mut NSView {
            std::ptr::null_mut()
        }
    }

    // SAFETY: NSObjectProtocol has no requirements beyond NSObject's.
    unsafe impl NSObjectProtocol for Overlay {}

    // SAFETY: the methods match the protocol's signatures.
    unsafe impl NSDraggingDestination for Overlay {
        #[unsafe(method(draggingEntered:))]
        fn dragging_entered(&self, sender: &ProtocolObject<dyn NSDraggingInfo>) -> NSDragOperation {
            if carries_promises(sender) {
                NSDragOperation::Copy
            } else {
                NSDragOperation::None
            }
        }

        #[unsafe(method(performDragOperation:))]
        fn perform(&self, sender: &ProtocolObject<dyn NSDraggingInfo>) -> bool {
            self.receive(sender)
        }
    }
);

/// A point in a view of `height`, measured from its top left: AppKit's
/// views count up from the bottom unless flipped.
fn top_left(p: NSPoint, height: f64, flipped: bool) -> (f64, f64) {
    (p.x, if flipped { p.y } else { height - p.y })
}

/// Whether a drag offers file promises (or a Photos asset with them).
fn carries_promises(sender: &ProtocolObject<dyn NSDraggingInfo>) -> bool {
    let wanted: Vec<String> = NSFilePromiseReceiver::readableDraggedTypes()
        .iter()
        .map(|t| t.to_string())
        .collect();
    sender
        .draggingPasteboard()
        .types()
        .is_some_and(|types| types.iter().any(|t| wanted.contains(&t.to_string())))
}

impl Overlay {
    fn receive(&self, sender: &ProtocolObject<dyn NSDraggingInfo>) -> bool {
        let pasteboard = sender.draggingPasteboard();
        let classes = NSArray::from_slice(&[NSFilePromiseReceiver::class()]);
        #[allow(unsafe_code)]
        // SAFETY: the classes array holds a class object, which is what the
        // call reads; no options are passed.
        let objects = unsafe { pasteboard.readObjectsForClasses_options(&classes, None) };
        let receivers: Vec<Retained<NSFilePromiseReceiver>> = objects
            .map(|a| {
                a.iter()
                    .filter_map(|o| o.downcast::<NSFilePromiseReceiver>().ok())
                    .collect()
            })
            .unwrap_or_default();
        if receivers.is_empty() {
            return false;
        }
        let ivars = self.ivars();
        let photos = pasteboard
            .types()
            .is_some_and(|types| types.iter().any(|t| t.to_string() == PHOTOS_ASSET));
        let drop = DROPS.fetch_add(1, Ordering::Relaxed);
        let point = self.convertPoint_fromView(sender.draggingLocation(), None);
        let at = Some(top_left(point, self.bounds().size.height, self.isFlipped()));
        let queue = NSOperationQueue::new();
        for receiver in receivers {
            let item = ITEMS.fetch_add(1, Ordering::Relaxed);
            let name = receiver.fileNames().iter().next().map(|n| n.to_string());
            (ivars.sink)(PromiseEvent::Started {
                item,
                name,
                photos,
                at,
            });
            let folder = match staging_folder(&ivars.staging, drop, item) {
                Ok(f) => f,
                Err(e) => {
                    (ivars.sink)(PromiseEvent::Failed {
                        item,
                        reason: e.to_string(),
                    });
                    continue;
                }
            };
            let destination =
                NSURL::fileURLWithPath(&NSString::from_str(&folder.to_string_lossy()));
            let sink = Arc::clone(&ivars.sink);
            let reader = RcBlock::new(move |url: NonNull<NSURL>, error: *mut NSError| {
                #[allow(unsafe_code)]
                // SAFETY: the system calls the reader with a valid URL and a
                // valid error or null, alive for the call.
                let (url, error) = unsafe { (url.as_ref(), error.as_ref()) };
                let event = match (error, url.path()) {
                    (Some(e), _) => PromiseEvent::Failed {
                        item,
                        reason: e.localizedDescription().to_string(),
                    },
                    (None, Some(path)) => PromiseEvent::Arrived {
                        item,
                        path: PathBuf::from(path.to_string()),
                    },
                    (None, None) => PromiseEvent::Failed {
                        item,
                        reason: "the file didn't arrive".into(),
                    },
                };
                sink(event);
            });
            #[allow(unsafe_code)]
            // SAFETY: the options dictionary is empty, the queue is an
            // ordinary operation queue, and the reader block takes (URL,
            // error) as the API calls it.
            unsafe {
                receiver.receivePromisedFilesAtDestination_options_operationQueue_reader(
                    &destination,
                    &NSDictionary::new(),
                    &queue,
                    &reader,
                );
            }
        }
        true
    }
}

/// Lay a transparent view over `window` that takes promised files into
/// folders under `staging` and reports each through `on_event`. Main
/// thread only.
pub fn install_overlay(
    window: &impl HasWindowHandle,
    staging: PathBuf,
    on_event: impl Fn(PromiseEvent) + Send + Sync + 'static,
) -> Result<(), String> {
    let Some(mtm) = MainThreadMarker::new() else {
        return Err("not on the main thread".into());
    };
    let handle = window.window_handle().map_err(|e| e.to_string())?;
    let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
        return Err("not an AppKit window".into());
    };
    #[allow(unsafe_code)]
    // SAFETY: `window_handle` borrows the window for this call, so its
    // `ns_view` is a live NSView; we're on the main thread.
    let view: &NSView = unsafe { handle.ns_view.cast::<NSView>().as_ref() };
    let Some(content) = view.window().and_then(|w| w.contentView()) else {
        return Err("the window has no content view".into());
    };
    let overlay = Overlay::alloc(mtm).set_ivars(OverlayIvars {
        staging,
        sink: Arc::new(on_event),
    });
    #[allow(unsafe_code)]
    // SAFETY: `initWithFrame:` is NSView's designated initialiser, called
    // once on a freshly allocated object whose ivars are set.
    let overlay: Retained<Overlay> =
        unsafe { msg_send![super(overlay), initWithFrame: content.bounds()] };
    overlay.setAutoresizingMask(
        NSAutoresizingMaskOptions::ViewWidthSizable | NSAutoresizingMaskOptions::ViewHeightSizable,
    );
    let mut types: Vec<Retained<NSString>> = NSFilePromiseReceiver::readableDraggedTypes()
        .iter()
        .collect();
    types.push(NSString::from_str(PHOTOS_ASSET));
    overlay.registerForDraggedTypes(&NSArray::from_retained_slice(&types));
    content.addSubview_positioned_relativeTo(&overlay, NSWindowOrderingMode::Above, None);
    Ok(())
}

#[cfg(test)]
mod point_tests {
    use super::*;

    #[test]
    fn a_drop_point_is_measured_from_the_top_left() {
        let p = NSPoint::new(100.0, 40.0);
        assert_eq!(top_left(p, 360.0, false), (100.0, 320.0));
        assert_eq!(top_left(p, 360.0, true), (100.0, 40.0));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use raw_window_handle::{HandleError, HasWindowHandle, WindowHandle};
    use std::collections::HashSet;

    struct Untouched;
    impl HasWindowHandle for Untouched {
        fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
            panic!("the handle was read off the main thread")
        }
    }

    fn base(name: &str) -> PathBuf {
        let d =
            std::env::temp_dir().join(format!("prsm-mac-promised-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn staging_folders_never_repeat() {
        let b = base("repeat");
        let mut seen = HashSet::new();
        for drop in 0..40 {
            for item in 0..25 {
                let f = staging_folder(&b, drop, item).unwrap();
                assert!(f.is_dir());
                assert!(f.starts_with(&b));
                assert!(seen.insert(f));
            }
        }
        assert_eq!(seen.len(), 1_000);
        clear_staging(&b);
    }

    #[test]
    fn clear_staging_empties_the_promised_folder() {
        let b = base("clear");
        for item in 0..2 {
            let f = staging_folder(&b, 7, item).unwrap();
            std::fs::write(f.join("IMG_0001.JPG"), b"x").unwrap();
        }
        clear_staging(&b);
        assert!(!b.exists());
        clear_staging(&b);
    }

    #[test]
    fn installing_off_the_main_thread_refuses() {
        assert_eq!(
            install_overlay(&Untouched, base("thread"), |_| {}),
            Err("not on the main thread".to_string())
        );
    }
}
