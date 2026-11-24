// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Persimmon contributors

//! Panels on the main thread, where AppKit needs them: this test runs
//! without the test harness, so `main` is the process's main thread.

#[cfg(target_os = "macos")]
fn main() {
    use objc2::MainThreadOnly;
    use objc2_app_kit::{NSApplication, NSBackingStoreType, NSView, NSWindow, NSWindowStyleMask};
    use objc2_foundation::{NSPoint, NSRect, NSSize};
    use raw_window_handle::{
        AppKitWindowHandle, HandleError, HasWindowHandle, RawWindowHandle, WindowHandle,
    };

    struct Handle(std::ptr::NonNull<std::ffi::c_void>);
    impl HasWindowHandle for Handle {
        fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
            let raw = RawWindowHandle::AppKit(AppKitWindowHandle::new(self.0));
            // SAFETY: the view outlives the handle; the test keeps it.
            #[allow(unsafe_code)]
            Ok(unsafe { WindowHandle::borrow_raw(raw) })
        }
    }

    let mtm = objc2::MainThreadMarker::new().expect("main thread");
    let _app = NSApplication::sharedApplication(mtm);
    let window = |mtm| {
        let rect = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(320.0, 360.0));
        #[allow(unsafe_code)]
        // SAFETY: a plain borderless window on the main thread, with the
        // designated initialiser's argument types.
        let w = unsafe {
            NSWindow::initWithContentRect_styleMask_backing_defer(
                NSWindow::alloc(mtm),
                rect,
                NSWindowStyleMask::Borderless,
                NSBackingStoreType::Buffered,
                false,
            )
        };
        #[allow(unsafe_code)]
        // SAFETY: the window stays alive in this test's frame.
        unsafe {
            w.setReleasedWhenClosed(false)
        };
        let view = NSView::new(mtm);
        w.setContentView(Some(&view));
        let ptr = std::ptr::NonNull::from(&*view).cast();
        (w, view, Handle(ptr))
    };

    let (_w1, _v1, wheel) = window(mtm);
    persimmon_mac::panel::adopt_as_panel(&wheel, "wheel", true).unwrap();
    assert_eq!(
        persimmon_mac::panel::takes_keys("wheel"),
        Ok(true),
        "the wheel must take the keyboard for Esc"
    );
    let (_w2, _v2, drop) = window(mtm);
    persimmon_mac::panel::adopt_as_panel(&drop, "drop", false).unwrap();
    assert_eq!(persimmon_mac::panel::takes_keys("drop"), Ok(false));
    assert_eq!(persimmon_mac::panel::takes_keys("nothing"), Ok(false));
    println!("keyable_panel: ok");
}

#[cfg(not(target_os = "macos"))]
fn main() {}
