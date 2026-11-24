// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Persimmon contributors

//! Finder's Services (spec D3): Convert with Persimmon and Open in
//! Persimmon, declared in the app's Info.plist for any file or folder.
//! macOS calls the provider with the selected items' file URLs.

use std::path::PathBuf;
use std::sync::Arc;

use objc2::rc::Retained;
use objc2::{ClassType, DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{NSApplication, NSPasteboard, NSUpdateDynamicServices};
use objc2_foundation::{NSArray, NSObject, NSObjectProtocol, NSString, NSURL};

/// Which Finder item was chosen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Service {
    /// Convert with Persimmon.
    Convert,
    /// Open in Persimmon.
    Open,
}

/// A Services request: the item chosen and the selected files and folders.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceRequest {
    pub service: Service,
    pub paths: Vec<PathBuf>,
}

/// `file://` URLs as paths, percent-decoded, in order, each once; anything
/// that isn't a file URL is left out.
pub fn service_paths(urls: &[String]) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    for url in urls {
        let Some(rest) = url.strip_prefix("file://") else {
            continue;
        };
        // `file://localhost/…` names the same file as `file:///…`.
        let rest = rest.strip_prefix("localhost").unwrap_or(rest);
        let mut path = percent_decode(rest);
        while path.len() > 1 && path.ends_with('/') {
            path.pop();
        }
        let path = PathBuf::from(path);
        if !out.contains(&path) {
            out.push(path);
        }
    }
    out
}

/// `%XX` escapes as the bytes they stand for, read as UTF-8.
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = |at: usize| bytes.get(at).and_then(|b| (*b as char).to_digit(16));
        if bytes[i] == b'%'
            && let (Some(h), Some(l)) = (hex(i + 1), hex(i + 2))
        {
            out.push((h * 16 + l) as u8);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

type Sink = Arc<dyn Fn(ServiceRequest) + Send + Sync>;

pub struct ProviderIvars {
    sink: Sink,
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements; the instance
    // variables are plain Rust values, set before init.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "PersimmonServices"]
    #[ivars = ProviderIvars]
    struct Provider;

    impl Provider {
        #[unsafe(method(convertFiles:userData:error:))]
        fn convert_files(&self, pasteboard: &NSPasteboard, _data: Option<&NSString>, _error: *mut *mut NSString) {
            self.request(Service::Convert, pasteboard);
        }

        #[unsafe(method(openFiles:userData:error:))]
        fn open_files(&self, pasteboard: &NSPasteboard, _data: Option<&NSString>, _error: *mut *mut NSString) {
            self.request(Service::Open, pasteboard);
        }
    }

    // SAFETY: NSObjectProtocol has no requirements beyond NSObject's.
    unsafe impl NSObjectProtocol for Provider {}
);

impl Provider {
    fn request(&self, service: Service, pasteboard: &NSPasteboard) {
        let classes = NSArray::from_slice(&[NSURL::class()]);
        #[allow(unsafe_code)]
        // SAFETY: the classes array holds a class object, which is what the
        // call reads; no options are passed.
        let objects = unsafe { pasteboard.readObjectsForClasses_options(&classes, None) };
        let urls: Vec<String> = objects
            .map(|a| {
                a.iter()
                    .filter_map(|o| o.downcast::<NSURL>().ok())
                    .filter_map(|u| u.absoluteString().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default();
        let paths = service_paths(&urls);
        if !paths.is_empty() {
            (self.ivars().sink)(ServiceRequest { service, paths });
        }
    }
}

/// Answer Finder's Services with `on_request`. Main thread only.
pub fn install(on_request: impl Fn(ServiceRequest) + Send + Sync + 'static) -> Result<(), String> {
    let Some(mtm) = MainThreadMarker::new() else {
        return Err("not on the main thread".into());
    };
    let provider = Provider::alloc(mtm).set_ivars(ProviderIvars {
        sink: Arc::new(on_request),
    });
    #[allow(unsafe_code)]
    // SAFETY: `init` is NSObject's initialiser, called once on a freshly
    // allocated object whose ivars are set.
    let provider: Retained<Provider> = unsafe { msg_send![super(provider), init] };
    #[allow(unsafe_code)]
    // SAFETY: the provider answers the two messages the Info.plist names,
    // with the signatures Services calls them with.
    unsafe {
        NSApplication::sharedApplication(mtm).setServicesProvider(Some(&provider));
    }
    NSUpdateDynamicServices();
    // Kept for the app's life, whether or not AppKit holds it strongly.
    std::mem::forget(provider);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn service_paths_take_files_and_folders() {
        let urls = [
            "file:///Users/me/a%20b.png".to_string(),
            "file:///Users/me/Folder/".to_string(),
            "file:///Users/me/a%20b.png".to_string(),
            "https://example.com/x.png".to_string(),
            "file:///Users/me/caf%C3%A9.jpg".to_string(),
            "file:///Users/me/odd%".to_string(),
            "file:///Users/me/odd%4".to_string(),
        ];
        assert_eq!(
            service_paths(&urls),
            vec![
                PathBuf::from("/Users/me/a b.png"),
                PathBuf::from("/Users/me/Folder"),
                PathBuf::from("/Users/me/café.jpg"),
                PathBuf::from("/Users/me/odd%"),
                PathBuf::from("/Users/me/odd%4"),
            ]
        );
    }

    #[test]
    fn installing_services_off_the_main_thread_refuses() {
        assert_eq!(install(|_| {}), Err("not on the main thread".to_string()));
    }
}
