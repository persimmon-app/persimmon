// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Persimmon contributors

//! Windows as non-activating floating panels (spec D4, D5): on every
//! Space, over full-screen apps, and never taking focus from the app you
//! drag out of. The drop window and the wheel each become one, kept by
//! name. The window's contents move into a native `NSPanel`; the window's
//! own class is never changed, since Tauri's window class carries state of
//! its own.

use std::cell::RefCell;
use std::collections::HashMap;

use objc2::rc::Retained;
use objc2::{MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{
    NSBackingStoreType, NSPanel, NSResponder, NSScreen, NSStatusWindowLevel, NSView, NSWindow,
    NSWindowCollectionBehavior, NSWindowStyleMask,
};
use objc2_foundation::{NSObject, NSPoint, NSRect, NSSize};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

thread_local! {
    /// The panels made so far, by name: only ever touched on the main
    /// thread.
    static PANELS: RefCell<HashMap<String, Retained<NSPanel>>> = RefCell::new(HashMap::new());
}

define_class!(
    // SAFETY: NSPanel has no subclassing requirements beyond NSWindow's;
    // this class adds no instance variables and overrides one getter.
    #[unsafe(super(NSPanel, NSWindow, NSResponder, NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "PersimmonKeyablePanel"]
    struct KeyablePanel;

    impl KeyablePanel {
        /// A borderless window can't take the keyboard unless it says so:
        /// the wheel needs it for Esc when it waits for a click.
        #[unsafe(method(canBecomeKeyWindow))]
        fn can_become_key_window(&self) -> bool {
            true
        }
    }
);

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
/// Show and hide it with [`set_panel_visible`], by `name`. `takes_keys`
/// lets it take the keyboard when asked ([`focus_panel`]), still without
/// activating the app. Add anything that belongs over the webview (the
/// promise overlay) before this. Main thread only.
pub fn adopt_as_panel(
    window: &impl HasWindowHandle,
    name: &str,
    takes_keys: bool,
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
    let style = NSWindowStyleMask::Borderless | NSWindowStyleMask::NonactivatingPanel;
    let panel: Retained<NSPanel> = if takes_keys {
        let this = KeyablePanel::alloc(mtm).set_ivars(());
        #[allow(unsafe_code)]
        // SAFETY: initWithContentRect:styleMask:backing:defer: is NSWindow's
        // designated initialiser, with these argument types; the subclass
        // has no instance variables of its own to set first.
        let keyable: Retained<KeyablePanel> = unsafe {
            msg_send![super(this), initWithContentRect: frame, styleMask: style, backing: NSBackingStoreType::Buffered, defer: false]
        };
        Retained::into_super(keyable)
    } else {
        NSPanel::initWithContentRect_styleMask_backing_defer(
            NSPanel::alloc(mtm),
            frame,
            style,
            NSBackingStoreType::Buffered,
            false,
        )
    };
    #[allow(unsafe_code)]
    // SAFETY: the panel is kept in PANELS for the app's life, so closing it
    // must not release it as well.
    unsafe {
        panel.setReleasedWhenClosed(false);
    }
    // Moving the content view moves the webview with everything attached
    // to it (its drop handling, its scripts, any overlay already on it).
    // The window gets an empty view in its place: Tauri expects every
    // window to have one.
    // Detached first: had the panel taken it while the window still held
    // it, the window's swap would pull it back out of the panel.
    window.setContentView(Some(&NSView::new(mtm)));
    panel.setContentView(Some(&content));
    let moved = panel.contentView().is_some_and(|v| v == content)
        && content.window().is_some_and(|w| *w == **panel);
    if !moved {
        return Err("the window's contents didn't move into the panel".into());
    }
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
    PANELS.with(|p| p.borrow_mut().insert(name.to_string(), panel));
    Ok(())
}

/// Run `f` on the panel called `name`, if there is one. Main thread only.
fn with_panel<R>(name: &str, f: impl FnOnce(&NSPanel) -> R) -> Result<Option<R>, String> {
    if MainThreadMarker::new().is_none() {
        return Err("not on the main thread".into());
    }
    Ok(PANELS.with(|p| p.borrow().get(name).map(|panel| f(panel))))
}

/// Show or hide the panel called `name`; `Ok(false)` when there's none (so
/// the window itself is shown instead). Main thread only.
pub fn set_panel_visible(name: &str, show: bool) -> Result<bool, String> {
    with_panel(name, |panel| {
        if show {
            panel.orderFrontRegardless();
        } else {
            panel.orderOut(None);
        }
    })
    .map(|done| done.is_some())
}

/// Whether the panel called `name` can take the keyboard; `Ok(false)`
/// when there's none. Main thread only.
pub fn takes_keys(name: &str) -> Result<bool, String> {
    with_panel(name, |panel| panel.canBecomeKeyWindow()).map(|k| k.unwrap_or(false))
}

/// Give the panel called `name` the keyboard (for Esc), without
/// activating the app; `Ok(false)` when there's none. Main thread only.
pub fn focus_panel(name: &str) -> Result<bool, String> {
    with_panel(name, |panel| panel.makeKeyAndOrderFront(None)).map(|done| done.is_some())
}

/// Centre the panel called `name` on `centre`, a point in AppKit's screen
/// coordinates (origin at the bottom left of the main screen), kept whole
/// on the screen that point is on; `Ok(false)` when there's no such
/// panel. Main thread only.
pub fn place_panel(name: &str, centre: (f64, f64)) -> Result<bool, String> {
    let Some(mtm) = MainThreadMarker::new() else {
        return Err("not on the main thread".into());
    };
    let point = NSPoint::new(centre.0, centre.1);
    let screens = NSScreen::screens(mtm);
    let screen = screens
        .iter()
        .map(|s| s.frame())
        .find(|f| contains(*f, point))
        .or_else(|| screens.iter().next().map(|s| s.frame()));
    with_panel(name, |panel| {
        let size = panel.frame().size;
        let origin = match screen {
            Some(frame) => placed(size, centre, frame),
            None => NSPoint::new(centre.0 - size.width / 2.0, centre.1 - size.height / 2.0),
        };
        panel.setFrameOrigin(origin);
    })
    .map(|done| done.is_some())
}

/// Whether `rect` holds `p`, its left and bottom edges included.
fn contains(rect: NSRect, p: NSPoint) -> bool {
    p.x >= rect.origin.x
        && p.x < rect.origin.x + rect.size.width
        && p.y >= rect.origin.y
        && p.y < rect.origin.y + rect.size.height
}

/// The origin that centres a `size` window on `centre`, moved the least
/// needed to keep it whole inside `screen`.
fn placed(size: NSSize, centre: (f64, f64), screen: NSRect) -> NSPoint {
    let clamp = |v: f64, lo: f64, span: f64, len: f64| v.max(lo).min(lo + span - len);
    NSPoint::new(
        clamp(
            centre.0 - size.width / 2.0,
            screen.origin.x,
            screen.size.width,
            size.width,
        ),
        clamp(
            centre.1 - size.height / 2.0,
            screen.origin.y,
            screen.size.height,
            size.height,
        ),
    )
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
        let refused = Err("not on the main thread".to_string());
        assert_eq!(
            adopt_as_panel(&Untouched, "drop", false),
            Err("not on the main thread".to_string())
        );
        assert_eq!(set_panel_visible("drop", true), refused);
        assert_eq!(place_panel("drop", (100.0, 100.0)), refused);
        assert_eq!(focus_panel("wheel"), refused);
    }

    #[test]
    fn panels_are_kept_by_name() {
        let refused = Err("not on the main thread".to_string());
        for name in ["drop", "wheel", "unknown"] {
            assert_eq!(set_panel_visible(name, false), refused, "{name}");
            assert_eq!(place_panel(name, (0.0, 0.0)), refused, "{name}");
        }
        // No panel by that name: nothing to show, and the caller shows the
        // window itself.
        assert!(!PANELS.with(|p| p.borrow().contains_key("unknown")));
    }

    #[test]
    fn a_panel_is_centred_on_the_point_and_kept_on_its_screen() {
        let screen = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(1440.0, 900.0));
        let size = NSSize::new(320.0, 360.0);
        let at = |x, y| placed(size, (x, y), screen);
        assert_eq!(at(720.0, 450.0), NSPoint::new(560.0, 270.0));
        // Pushed back inside near every edge.
        assert_eq!(at(10.0, 10.0), NSPoint::new(0.0, 0.0));
        assert_eq!(at(1435.0, 895.0), NSPoint::new(1120.0, 540.0));
        // A second display to the left, at negative x.
        let left = NSRect::new(NSPoint::new(-1920.0, 0.0), NSSize::new(1920.0, 1080.0));
        assert_eq!(
            placed(size, (-1900.0, 500.0), left),
            NSPoint::new(-1920.0, 320.0)
        );
    }
}
