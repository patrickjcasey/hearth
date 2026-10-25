//! Keeps the display awake while a recipe or cook mode is open.
//!
//! Views hold a [`Lease`] while they want the screen on. The platform call is
//! made only when the first lease is taken and the last one dropped, so views
//! never fight over the single macOS activity.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use dioxus::prelude::*;

thread_local! {
    static HOLDERS: Cell<Holders> = const { Cell::new(Holders(0)) };
}

/// Keeps the screen awake until dropped. Must stay on the main thread,
/// which is where Dioxus runs components.
pub struct Lease(());

impl Lease {
    pub fn acquire() -> Self {
        if update(Holders::acquire) {
            imp::keep_awake(true);
        }
        Lease(())
    }
}

impl Drop for Lease {
    fn drop(&mut self) {
        if update(Holders::release) {
            imp::keep_awake(false);
        }
    }
}

fn update(f: impl FnOnce(&mut Holders) -> bool) -> bool {
    HOLDERS.with(|cell| {
        let mut holders = cell.get();
        let changed = f(&mut holders);
        cell.set(holders);
        changed
    })
}

/// Holds a lease while `want()` is true, re-checked whenever the signals it
/// reads change, and releases it when the component unmounts.
pub fn use_keep_awake(want: impl Fn() -> bool + 'static) {
    let lease = use_hook(|| Rc::new(RefCell::new(None::<Lease>)));
    let held = lease.clone();
    use_effect(move || {
        let want = want();
        let mut slot = held.borrow_mut();
        if want != slot.is_some() {
            *slot = want.then(Lease::acquire);
        }
    });
    use_drop(move || drop(lease.borrow_mut().take()));
}

/// Number of live leases. `acquire`/`release` return true on the 0->1 and
/// 1->0 transitions, the only times the platform needs to hear about it.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Holders(usize);

impl Holders {
    fn acquire(&mut self) -> bool {
        self.0 += 1;
        self.0 == 1
    }

    fn release(&mut self) -> bool {
        debug_assert!(self.0 > 0, "wake lease released twice");
        let was = self.0;
        self.0 = was.saturating_sub(1);
        was == 1
    }
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
                let reason = NSString::from_str("Hestia keeps the screen awake while cooking");
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn platform_hears_only_first_and_last() {
        let mut h = Holders(0);
        assert!(h.acquire(), "0 -> 1 turns it on");
        assert!(!h.acquire(), "1 -> 2 is silent");
        assert!(!h.release(), "2 -> 1 is silent");
        assert!(h.release(), "1 -> 0 turns it off");
        assert_eq!(h, Holders(0));
        assert!(h.acquire(), "and on again");
    }

    #[test]
    #[cfg(not(debug_assertions))]
    fn extra_release_never_underflows() {
        let mut h = Holders(0);
        assert!(!h.release());
        assert_eq!(h, Holders(0));
    }

    #[test]
    #[should_panic(expected = "released twice")]
    #[cfg(debug_assertions)]
    fn extra_release_is_caught_in_debug() {
        Holders(0).release();
    }
}
