//! Keeps the display awake while "Keep screen on" is switched on for a recipe.

/// Stop (`true`) or allow (`false`) the screen from dimming and locking.
/// Must be called on the main thread, which is where Dioxus runs components.
pub fn keep_awake(on: bool) {
    imp::keep_awake(on);
}

#[cfg(target_os = "ios")]
mod imp {
    use objc2::MainThreadMarker;
    use objc2_ui_kit::UIApplication;

    pub fn keep_awake(on: bool) {
        let Some(mtm) = MainThreadMarker::new() else {
            eprintln!("keep_awake called off the main thread");
            return;
        };
        UIApplication::sharedApplication(mtm).setIdleTimerDisabled(on);
    }
}

#[cfg(target_os = "macos")]
mod imp {
    use std::cell::RefCell;

    use objc2::rc::Retained;
    use objc2::runtime::ProtocolObject;
    use objc2_foundation::{NSActivityOptions, NSObjectProtocol, NSProcessInfo, NSString};

    thread_local! {
        static ACTIVITY: RefCell<Option<Retained<ProtocolObject<dyn NSObjectProtocol>>>> =
            const { RefCell::new(None) };
    }

    pub fn keep_awake(on: bool) {
        ACTIVITY.with_borrow_mut(|slot| {
            let info = NSProcessInfo::processInfo();
            if let Some(activity) = slot.take() {
                unsafe { info.endActivity(&activity) };
            }
            if on {
                let options =
                    NSActivityOptions::IdleDisplaySleepDisabled | NSActivityOptions::UserInitiated;
                let reason = NSString::from_str("Cook mode");
                *slot = Some(info.beginActivityWithOptions_reason(options, &reason));
            }
        });
    }
}

// TODO(android): set FLAG_KEEP_SCREEN_ON on the activity window via JNI.
#[cfg(not(any(target_os = "ios", target_os = "macos")))]
mod imp {
    pub fn keep_awake(_on: bool) {}
}
