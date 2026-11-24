// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Persimmon contributors

//! Start at login (spec D2, spike S2): `SMAppService.mainApp`, which works
//! for an ad-hoc signed app.

use objc2_foundation::NSAppleEventManager;
use objc2_service_management::{SMAppService, SMAppServiceStatus};

/// `keyAEPropData` and `keyAELaunchedAsLogInItem` ('prdt', 'lgit').
const KEY_AE_PROP_DATA: u32 = 0x7072_6474;
const KEY_AE_LAUNCHED_AS_LOG_IN_ITEM: u32 = 0x6c67_6974;

/// Whether Persimmon opens at login.
pub fn enabled() -> bool {
    #[allow(unsafe_code)]
    // SAFETY: `mainAppService` and `status` take no arguments and only
    // read this app's own login item; both may be called from any thread.
    unsafe {
        SMAppService::mainAppService().status() == SMAppServiceStatus::Enabled
    }
}

/// Open Persimmon at login, or stop; the error is macOS's own words.
pub fn set(on: bool) -> Result<(), String> {
    #[allow(unsafe_code)]
    // SAFETY: registering or unregistering this app's own login item takes
    // no arguments; failure comes back as an NSError, not an exception.
    let done = unsafe {
        let service = SMAppService::mainAppService();
        if on {
            service.registerAndReturnError()
        } else {
            service.unregisterAndReturnError()
        }
    };
    done.map_err(|e| e.localizedDescription().to_string())
}

/// Whether macOS opened Persimmon as a login item: the open-application
/// event says so. Read it while the app starts.
pub fn launched_at_login() -> bool {
    NSAppleEventManager::sharedAppleEventManager()
        .currentAppleEvent()
        .and_then(|event| event.paramDescriptorForKeyword(KEY_AE_PROP_DATA))
        .is_some_and(|d| d.enumCodeValue() == KEY_AE_LAUNCHED_AS_LOG_IN_ITEM)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_test_process_wasnt_launched_at_login() {
        assert!(!launched_at_login());
    }

    #[test]
    fn login_status_reads_without_failing() {
        let _ = enabled();
    }
}
