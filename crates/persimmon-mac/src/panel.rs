// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Persimmon contributors

//! The drop window as a non-activating floating panel (spec D5): on every
//! Space, over full-screen apps, and never taking focus from the app you
//! drag out of.

use objc2::runtime::AnyObject;
use objc2::{ClassType, MainThreadMarker, MainThreadOnly, define_class};
use objc2_app_kit::{
    NSPanel, NSResponder, NSStatusWindowLevel, NSView, NSWindow, NSWindowCollectionBehavior,
    NSWindowStyleMask,
};
use objc2_foundation::NSObject;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

define_class!(
    // SAFETY: NSPanel has no subclassing requirements this class breaks,
    // and it declares no instance variables.
    #[unsafe(super(NSPanel, NSWindow, NSResponder, NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "PersimmonDropPanel"]
    struct DropPanel;
);

/// Every Space, over full-screen apps, and not moved by Exposé.
pub fn panel_behaviour() -> NSWindowCollectionBehavior {
    NSWindowCollectionBehavior::CanJoinAllSpaces
        | NSWindowCollectionBehavior::FullScreenAuxiliary
        | NSWindowCollectionBehavior::Stationary
}

/// Turn `window` into a non-activating floating panel: it floats over
/// full-screen apps on every Space, and clicking or dropping on it never
/// takes focus from the app in front. Main thread only.
pub fn make_floating_panel(window: &impl HasWindowHandle) -> Result<(), String> {
    if MainThreadMarker::new().is_none() {
        return Err("not on the main thread".into());
    }
    let handle = window.window_handle().map_err(|e| e.to_string())?;
    let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
        return Err("not an AppKit window".into());
    };
    #[allow(unsafe_code)]
    // SAFETY: `window_handle` borrows the window for this call, so its
    // `ns_view` points at a live NSView; we're on the main thread, where
    // AppKit objects may be used.
    let view: &NSView = unsafe { handle.ns_view.cast::<NSView>().as_ref() };
    let Some(ns_window) = view.window() else {
        return Err("the view has no window".into());
    };
    let object: &AnyObject = &ns_window;
    #[allow(unsafe_code)]
    // SAFETY: NSPanel adds no instance variables over NSWindow, and
    // DropPanel adds none over NSPanel, so the object's size and layout
    // are what they were; only its class (and so its methods) changes.
    unsafe {
        AnyObject::set_class(object, DropPanel::class());
    }
    let Some(panel) = object.downcast_ref::<NSPanel>() else {
        return Err("the window didn't become a panel".into());
    };
    panel.setStyleMask(panel.styleMask() | NSWindowStyleMask::NonactivatingPanel);
    panel.setFloatingPanel(true);
    panel.setHidesOnDeactivate(false);
    panel.setBecomesKeyOnlyIfNeeded(true);
    panel.setLevel(NSStatusWindowLevel);
    panel.setCollectionBehavior(panel_behaviour());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use raw_window_handle::{HandleError, HasWindowHandle, WindowHandle};

    /// A window handle that must never be asked for: the main-thread check
    /// comes first.
    struct Untouched;
    impl HasWindowHandle for Untouched {
        fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
            panic!("the handle was read off the main thread")
        }
    }

    #[test]
    fn the_panel_floats_on_every_space_and_over_full_screen() {
        let b = panel_behaviour();
        assert!(b.contains(NSWindowCollectionBehavior::CanJoinAllSpaces));
        assert!(b.contains(NSWindowCollectionBehavior::FullScreenAuxiliary));
    }

    #[test]
    fn off_the_main_thread_it_refuses() {
        assert_eq!(
            make_floating_panel(&Untouched),
            Err("not on the main thread".to_string())
        );
    }
}
