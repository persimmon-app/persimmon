// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Persimmon contributors

//! The drop window as a non-activating floating panel (spec D5): on every
//! Space, over full-screen apps, and never taking focus from the app you
//! drag out of. The window's contents move into a native `NSPanel`; the
//! window's own class is never changed, since Tauri's window class carries
//! state of its own.

use std::cell::RefCell;

use objc2::rc::Retained;
use objc2::{MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSBackingStoreType, NSPanel, NSStatusWindowLevel, NSView, NSWindowCollectionBehavior,
    NSWindowStyleMask,
};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

thread_local! {
    /// The drop panel, once made: only ever touched on the main thread.
    static PANEL: RefCell<Option<Retained<NSPanel>>> = const { RefCell::new(None) };
}

/// Every Space, over full-screen apps, and not moved by Exposé.
pub fn panel_behaviour() -> NSWindowCollectionBehavior {
    NSWindowCollectionBehavior::CanJoinAllSpaces
        | NSWindowCollectionBehavior::FullScreenAuxiliary
        | NSWindowCollectionBehavior::Stationary
}

/// Move `window`'s contents (its webview) into a new non-activating
/// floating panel of the same size and place, and keep the window hidden
/// from then on: the panel floats over full-screen apps on every Space,
/// and clicking or dropping on it never takes focus from the app in front.
/// Show and hide it with [`set_panel_visible`]. Add anything that belongs
/// over the webview (the promise overlay) before this. Main thread only.
pub fn adopt_as_panel(window: &impl HasWindowHandle) -> Result<(), String> {
    let Some(mtm) = MainThreadMarker::new() else {
        return Err("not on the main thread".into());
    };
    let handle = window.window_handle().map_err(|e| e.to_string())?;
    let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
        return Err("not an AppKit window".into());
    };
    #[allow(unsafe_code)]
    // SAFETY: `window_handle` borrows the window for this call, so its
    // `ns_view` points at a live NSView; we're on the main thread, where
    // AppKit objects may be used.
    let view: &NSView = unsafe { handle.ns_view.cast::<NSView>().as_ref() };
    let Some(window) = view.window() else {
        return Err("the view has no window".into());
    };
    let Some(content) = window.contentView() else {
        return Err("the window has no content view".into());
    };
    let frame = window.frame();
    let panel = NSPanel::initWithContentRect_styleMask_backing_defer(
        NSPanel::alloc(mtm),
        frame,
        NSWindowStyleMask::Borderless | NSWindowStyleMask::NonactivatingPanel,
        NSBackingStoreType::Buffered,
        false,
    );
    #[allow(unsafe_code)]
    // SAFETY: the panel is kept in PANEL for the app's life, so closing it
    // must not release it as well.
    unsafe {
        panel.setReleasedWhenClosed(false);
    }
    // Moving the content view moves the webview with everything attached
    // to it (its drop handling, its scripts, any overlay already on it).
    // The window gets an empty view in its place: Tauri expects every
    // window to have one.
    panel.setContentView(Some(&content));
    window.setContentView(Some(&NSView::new(mtm)));
    panel.setFrame_display(frame, false);
    panel.setBackgroundColor(Some(&window.backgroundColor()));
    panel.setOpaque(false);
    panel.setHasShadow(true);
    panel.setFloatingPanel(true);
    panel.setHidesOnDeactivate(false);
    panel.setBecomesKeyOnlyIfNeeded(true);
    panel.setLevel(NSStatusWindowLevel);
    panel.setCollectionBehavior(panel_behaviour());
    window.orderOut(None);
    PANEL.with(|p| *p.borrow_mut() = Some(panel));
    Ok(())
}

/// Show or hide the drop panel; `Ok(false)` when there's none (so the
/// window itself is shown instead). Main thread only.
pub fn set_panel_visible(show: bool) -> Result<bool, String> {
    if MainThreadMarker::new().is_none() {
        return Err("not on the main thread".into());
    }
    Ok(PANEL.with(|p| match p.borrow().as_ref() {
        Some(panel) => {
            if show {
                panel.orderFrontRegardless();
            } else {
                panel.orderOut(None);
            }
            true
        }
        None => false,
    }))
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
            adopt_as_panel(&Untouched),
            Err("not on the main thread".to_string())
        );
        assert_eq!(
            set_panel_visible(true),
            Err("not on the main thread".to_string())
        );
    }
}
