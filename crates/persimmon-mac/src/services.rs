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

/// A file URL's path, including for a file reference URL
/// (`file:///.file/id=…`), which Finder often hands over: macOS resolves it
/// to where the file is now. `None` for anything that isn't a file.
pub fn url_path(url: &NSURL) -> Option<PathBuf> {
    url.filePathURL()
        .and_then(|u| u.path())
        .map(|p| PathBuf::from(p.to_string()))
}

/// Paths in order, each once.
pub fn unique_paths(paths: impl IntoIterator<Item = PathBuf>) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    for path in paths {
        if !out.contains(&path) {
            out.push(path);
        }
    }
    out
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
        let paths = unique_paths(
            objects
                .map(|a| {
                    a.iter()
                        .filter_map(|o| o.downcast::<NSURL>().ok())
                        .filter_map(|u| url_path(&u))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default(),
        );
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
        let a = PathBuf::from("/Users/me/a b.png");
        let folder = PathBuf::from("/Users/me/Folder");
        assert_eq!(
            unique_paths([a.clone(), folder.clone(), a.clone()]),
            vec![a, folder]
        );
        let folder_url = NSURL::fileURLWithPath(&NSString::from_str("/Users/me/Folder/"));
        assert_eq!(
            url_path(&folder_url),
            Some(PathBuf::from("/Users/me/Folder"))
        );
    }

    /// Finder often hands over file reference URLs (`file:///.file/id=…`),
    /// whose text is no path at all: the path comes from the URL itself.
    #[test]
    fn a_file_reference_url_gives_the_files_path() {
        let dir = std::env::temp_dir().join(format!("prsm-mac-ref-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("a b.png");
        std::fs::write(&file, b"x").unwrap();
        let url = NSURL::fileURLWithPath(&NSString::from_str(&file.to_string_lossy()));
        let reference = url.fileReferenceURL().expect("a reference URL");
        let text = reference.absoluteString().unwrap().to_string();
        assert!(text.starts_with("file:///.file/id="), "{text}");
        assert_eq!(
            url_path(&reference).map(|p| std::fs::canonicalize(p).unwrap()),
            Some(std::fs::canonicalize(&file).unwrap())
        );
        assert_eq!(url_path(&url), Some(file.clone()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn installing_services_off_the_main_thread_refuses() {
        assert_eq!(install(|_| {}), Err("not on the main thread".to_string()));
    }
}
