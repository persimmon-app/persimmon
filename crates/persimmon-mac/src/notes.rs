// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Persimmon contributors

//! Notifications for unattended work (spec D9, spike S4): a burst's
//! summary with Show files and Undo, or one file's note with Show file.
//! The setting is read before each one, never trusted from the first ask.

use std::ptr::NonNull;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::Duration;

use block2::{DynBlock, RcBlock};
use objc2::rc::Retained;
use objc2::runtime::{Bool, NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{AllocAnyThread, define_class, msg_send};
use objc2_foundation::{NSArray, NSBundle, NSError, NSSet, NSString, ns_string};
use objc2_user_notifications::{
    UNAuthorizationOptions, UNAuthorizationStatus, UNMutableNotificationContent, UNNotification,
    UNNotificationAction, UNNotificationActionOptions, UNNotificationCategory,
    UNNotificationCategoryOptions, UNNotificationPresentationOptions, UNNotificationRequest,
    UNNotificationResponse, UNNotificationSettings, UNUserNotificationCenter,
    UNUserNotificationCenterDelegate,
};

/// Whether notifications may be shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Permission {
    Allowed,
    /// Turned off, refused, or not possible here.
    Off,
    /// Never asked.
    NotAsked,
}

/// What a notification is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A burst's summary: Show files and Undo.
    Batch,
    /// One file: Show file.
    File,
}

/// What the person chose on a notification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Show files (or a click on the notification itself).
    Show,
    Undo,
}

/// Whether this process can have notifications: an app with a bundle
/// identifier. Under `cargo test` it has none, and the notification
/// centre would raise.
pub fn available() -> bool {
    NSBundle::mainBundle().bundleIdentifier().is_some()
}

fn center() -> Option<Retained<UNUserNotificationCenter>> {
    available().then(UNUserNotificationCenter::currentNotificationCenter)
}

/// The current setting, read from System Settings each time (spike S4:
/// the first answer can't be trusted). Waits up to 2 s.
pub fn permission() -> Permission {
    let Some(center) = center() else {
        return Permission::Off;
    };
    let (tx, rx) = mpsc::channel();
    let block = RcBlock::new(move |settings: NonNull<UNNotificationSettings>| {
        #[allow(unsafe_code)]
        // SAFETY: the framework hands the block a valid settings object for
        // the length of the call.
        let settings = unsafe { settings.as_ref() };
        let _ = tx.send(settings.authorizationStatus());
    });
    center.getNotificationSettingsWithCompletionHandler(&block);
    match rx.recv_timeout(Duration::from_secs(2)) {
        Ok(UNAuthorizationStatus::NotDetermined) => Permission::NotAsked,
        Ok(UNAuthorizationStatus::Denied) | Err(_) => Permission::Off,
        Ok(_) => Permission::Allowed,
    }
}

/// Ask for permission (macOS shows its prompt the first time). The answer
/// is ignored: [`permission`] reads the setting when it matters.
pub fn ask() {
    if let Some(center) = center() {
        let done = RcBlock::new(|_: Bool, _: *mut NSError| {});
        center.requestAuthorizationWithOptions_completionHandler(
            UNAuthorizationOptions::Alert | UNAuthorizationOptions::Sound,
            &done,
        );
    }
}

/// The two categories, with their buttons; set once.
fn categories(center: &UNUserNotificationCenter) {
    static SET: AtomicBool = AtomicBool::new(false);
    if SET.swap(true, Ordering::SeqCst) {
        return;
    }
    let action = |id: &NSString, title: &NSString| {
        UNNotificationAction::actionWithIdentifier_title_options(
            id,
            title,
            UNNotificationActionOptions::empty(),
        )
    };
    let category = |id: &NSString, actions: &[Retained<UNNotificationAction>]| {
        UNNotificationCategory::categoryWithIdentifier_actions_intentIdentifiers_options(
            id,
            &NSArray::from_retained_slice(actions),
            &NSArray::new(),
            UNNotificationCategoryOptions::empty(),
        )
    };
    let batch = category(
        ns_string!("batch"),
        &[
            action(ns_string!("show"), ns_string!("Show files")),
            action(ns_string!("undo"), ns_string!("Undo")),
        ],
    );
    let file = category(
        ns_string!("file"),
        &[action(ns_string!("show"), ns_string!("Show file"))],
    );
    center.setNotificationCategories(&NSSet::from_retained_slice(&[batch, file]));
}

/// Show a notification titled "Persimmon"; false where notifications
/// can't work. Read [`permission`] first.
pub fn post(id: &str, body: &str, kind: Kind) -> bool {
    let Some(center) = center() else {
        return false;
    };
    categories(&center);
    let content = UNMutableNotificationContent::new();
    content.setTitle(ns_string!("Persimmon"));
    content.setBody(&NSString::from_str(body));
    content.setCategoryIdentifier(match kind {
        Kind::Batch => ns_string!("batch"),
        Kind::File => ns_string!("file"),
    });
    let request = UNNotificationRequest::requestWithIdentifier_content_trigger(
        &NSString::from_str(id),
        &content,
        None,
    );
    center.addNotificationRequest_withCompletionHandler(&request, None);
    true
}

type Handler = Box<dyn Fn(String, Action) + Send + Sync>;
static HANDLER: OnceLock<Handler> = OnceLock::new();

define_class!(
    // SAFETY: NSObject has no subclassing requirements, and the delegate
    // holds no state of its own.
    #[unsafe(super(NSObject))]
    #[name = "PersimmonNotificationDelegate"]
    struct Delegate;

    // SAFETY: NSObjectProtocol has no requirements beyond NSObject's.
    unsafe impl NSObjectProtocol for Delegate {}

    // SAFETY: the two methods match the protocol's signatures, and each
    // calls its completion handler exactly once.
    unsafe impl UNUserNotificationCenterDelegate for Delegate {
        #[unsafe(method(userNotificationCenter:didReceiveNotificationResponse:withCompletionHandler:))]
        fn did_receive(
            &self,
            _center: &UNUserNotificationCenter,
            response: &UNNotificationResponse,
            done: &DynBlock<dyn Fn()>,
        ) {
            let action = match response.actionIdentifier().to_string().as_str() {
                "undo" => Action::Undo,
                _ => Action::Show,
            };
            let id = response.notification().request().identifier().to_string();
            if let Some(handler) = HANDLER.get() {
                handler(id, action);
            }
            done.call(());
        }

        // Banners show while Persimmon is in front, too.
        #[unsafe(method(userNotificationCenter:willPresentNotification:withCompletionHandler:))]
        fn will_present(
            &self,
            _center: &UNUserNotificationCenter,
            _notification: &UNNotification,
            done: &DynBlock<dyn Fn(UNNotificationPresentationOptions)>,
        ) {
            done.call((
                UNNotificationPresentationOptions::Banner | UNNotificationPresentationOptions::List,
            ));
        }
    }
);

/// Be told when the person picks an action on one of Persimmon's
/// notifications: its id and the action. Set once.
pub fn on_action(f: impl Fn(String, Action) + Send + Sync + 'static) {
    let _ = HANDLER.set(Box::new(f));
    static INSTALLED: AtomicBool = AtomicBool::new(false);
    let Some(center) = center() else {
        return;
    };
    if INSTALLED.swap(true, Ordering::SeqCst) {
        return;
    }
    #[allow(unsafe_code)]
    // SAFETY: `new` on an NSObject subclass with no ivars returns an
    // initialised object.
    let delegate: Retained<Delegate> = unsafe { msg_send![Delegate::alloc(), init] };
    center.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
    // The centre holds its delegate weakly: keep this one for the app's life.
    std::mem::forget(delegate);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notifications_are_unavailable_in_a_test_process() {
        assert!(!available());
        assert_eq!(permission(), Permission::Off);
        assert!(!post("x", "a.jpg → a.webp", Kind::File));
        ask();
        on_action(|_, _| {});
    }
}
