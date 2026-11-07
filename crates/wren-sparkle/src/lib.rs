//! Wren's bridge to the Sparkle updater on macOS.
//!
//! Sparkle is loaded with `dlopen` at runtime instead of linked: the framework
//! ships only inside `Wren.app` (`Contents/Frameworks/Sparkle.framework`), so a
//! bare `cargo run` binary, a CI test build, or a bundle built without Sparkle
//! simply reports the updater as unavailable and greys out "Check for
//! Updates…". Nothing else in the app depends on it.
//!
//! The Objective-C surface is kept small: one class lookup, one controller,
//! `startUpdater:`, the two check calls, two BOOL properties, and a delegate
//! that records how each check ended. Every selector comes from Sparkle 2's
//! public headers (`SPUStandardUpdaterController.h`, `SPUUpdater.h`,
//! `SPUUpdaterDelegate.h`).
//!
//! Everything here is main-thread-only. The controller holds its delegate
//! weakly, so the dlopen handle, controller, updater, and delegate live in a
//! `thread_local!` for the life of the app (the framework is never unloaded:
//! Objective-C classes cannot be unregistered). The delegate's callbacks run on
//! the main run loop and write to a separate `thread_local!`, so a callback
//! that fires while an entry point holds the objects never re-borrows them.
//!
//! On every other platform this crate is empty.

#![cfg(target_os = "macos")]

use std::cell::RefCell;
use std::ffi::{c_void, CStr, CString};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::ptr;

use objc2::rc::{Allocated, Retained};
use objc2::runtime::{AnyClass, AnyObject, NSObject};
use objc2::{define_class, msg_send, sel, MainThreadMarker, MainThreadOnly};
use objc2_foundation::NSString;

/// Relative to `Contents/MacOS/`, through the framework's stable top-level
/// symlink.
const FRAMEWORK: &str = "../Frameworks/Sparkle.framework/Sparkle";

/// How a finished update check ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// The feed offered a newer version.
    Found,
    /// The feed had nothing newer.
    UpToDate,
    /// The check failed (network, feed, or signature problems).
    Failed,
}

impl Outcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Found => "found",
            Self::UpToDate => "up_to_date",
            Self::Failed => "failed",
        }
    }
}

/// One completed check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Check {
    pub outcome: Outcome,
    /// The offered version, for [`Outcome::Found`].
    pub version: Option<String>,
    /// Sparkle's explanation, when it gave one.
    pub detail: Option<String>,
}

/// The updater's state, as plain data.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Status {
    /// The updater loaded and started.
    pub started: bool,
    /// Why it did not, when it did not.
    pub unavailable: Option<String>,
    /// Counts finished checks, so a caller can wait for the next one.
    pub checks: u64,
    pub last: Option<Check>,
}

#[derive(Default)]
struct Record {
    checks: u64,
    last: Option<Check>,
    /// The in-flight check already recorded its outcome. Sparkle reports a
    /// check through whichever callback fits and then always through the
    /// cycle-finished one; the first to fire wins.
    recorded: bool,
}

struct Objects {
    /// Held for the app's life and never passed to `dlclose`.
    _handle: *mut c_void,
    _controller: Retained<AnyObject>,
    /// `None` when the controller came up but `startUpdater:` refused.
    updater: Option<Retained<AnyObject>>,
    /// The only strong reference: the controller holds its delegate weakly.
    _delegate: Retained<UpdaterDelegate>,
}

thread_local! {
    static OBJECTS: RefCell<Option<Objects>> = const { RefCell::new(None) };
    static RECORD: RefCell<Record> = RefCell::new(Record::default());
    /// `None` until [`start`] runs; then `Ok` or why the updater is missing.
    static STARTED: RefCell<Option<Result<(), String>>> = const { RefCell::new(None) };
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements, and this class does
    // not implement Drop.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "WrenSparkleUpdaterDelegate"]
    struct UpdaterDelegate;

    // Every `SPUUpdaterDelegate` member is optional and Sparkle probes each
    // with `respondsToSelector:`, so implementing these four is the whole
    // conformance.
    impl UpdaterDelegate {
        /// `- (void)updater:(SPUUpdater *)updater didFindValidUpdate:(SUAppcastItem *)item`
        #[unsafe(method(updater:didFindValidUpdate:))]
        fn did_find_valid_update(&self, _updater: &AnyObject, item: &AnyObject) {
            record(Outcome::Found, item_version(item), None);
        }

        /// `- (void)updaterDidNotFindUpdate:(SPUUpdater *)updater error:(NSError *)error`
        #[unsafe(method(updaterDidNotFindUpdate:error:))]
        fn did_not_find_update(&self, _updater: &AnyObject, error: &AnyObject) {
            record(Outcome::UpToDate, None, Some(error_description(error)));
        }

        /// `- (void)updater:(SPUUpdater *)updater didAbortWithError:(NSError *)error`
        #[unsafe(method(updater:didAbortWithError:))]
        fn did_abort(&self, _updater: &AnyObject, error: &AnyObject) {
            record(Outcome::Failed, None, Some(error_description(error)));
        }

        /// `- (void)updater:(SPUUpdater *)updater
        ///      didFinishUpdateCycleForUpdateCheck:(SPUUpdateCheck)updateCheck
        ///      error:(nullable NSError *)error`
        ///
        /// Fires after every cycle, so a check that reported through none of
        /// the callbacks above still counts.
        #[unsafe(method(updater:didFinishUpdateCycleForUpdateCheck:error:))]
        fn did_finish_cycle(&self, _updater: &AnyObject, _check: isize, error: Option<&AnyObject>) {
            match error {
                Some(error) => record(Outcome::Failed, None, Some(error_description(error))),
                None => record(Outcome::UpToDate, None, None),
            }
        }
    }
);

/// First writer wins: `didFindValidUpdate:` fires before the cycle ends, and an
/// information-only check that found an update still ends its cycle with an
/// error-shaped "no update" result.
fn record(outcome: Outcome, version: Option<String>, detail: Option<String>) {
    RECORD.with(|cell| {
        let Ok(mut record) = cell.try_borrow_mut() else {
            return;
        };
        if record.recorded {
            return;
        }
        record.recorded = true;
        record.checks += 1;
        record.last = Some(Check {
            outcome,
            version,
            detail,
        });
    });
}

/// Load the framework beside the executable and start the updater, once.
///
/// Call after the first window opens: Sparkle drives AppKit, and touching
/// `NSApplication` before the event loop runs is unsupported. Later calls
/// return the first call's result.
pub fn start() -> Result<(), String> {
    if let Some(result) = STARTED.with(|cell| cell.borrow().clone()) {
        return result;
    }
    let result = match MainThreadMarker::new() {
        None => Err("the updater must start on the main thread".to_string()),
        Some(mtm) => load(mtm).and_then(|(objects, started)| {
            OBJECTS.with(|cell| *cell.borrow_mut() = Some(objects));
            started
        }),
    };
    STARTED.with(|cell| *cell.borrow_mut() = Some(result.clone()));
    result
}

pub fn status() -> Status {
    let started = STARTED.with(|cell| cell.borrow().clone());
    RECORD.with(|cell| {
        let record = cell.borrow();
        Status {
            started: matches!(started, Some(Ok(()))),
            unavailable: match started {
                None => Some("the updater has not been started".to_string()),
                Some(result) => result.err(),
            },
            checks: record.checks,
            last: record.last.clone(),
        }
    })
}

/// Whether "Check for Updates…" should be enabled: the updater started and
/// Sparkle's `canCheckForUpdates` holds (it is false while a check runs).
pub fn can_check() -> bool {
    with_updater(|updater| {
        // SAFETY: `canCheckForUpdates` is a readonly BOOL property of
        // SPUUpdater.
        let can: bool = unsafe { msg_send![updater, canCheckForUpdates] };
        can
    })
    .unwrap_or(false)
}

/// The user's "Check for Updates…": Sparkle shows its own progress, result,
/// and install UI, and presents its own errors.
pub fn check_for_updates() -> Result<(), String> {
    begin_check(|updater| {
        // SAFETY: `-checkForUpdates` takes no arguments and returns void.
        // This is the updater's method rather than the controller's
        // `-checkForUpdates:` action, which only adds a sender argument.
        let () = unsafe { msg_send![updater, checkForUpdates] };
    })
}

/// A silent check: fetch the feed and compare versions without any UI or
/// download. The outcome arrives in [`status`].
pub fn check_for_update_information() -> Result<(), String> {
    begin_check(|updater| {
        // SAFETY: `-checkForUpdateInformation` takes no arguments and returns
        // void.
        let () = unsafe { msg_send![updater, checkForUpdateInformation] };
    })
}

fn begin_check(call: impl FnOnce(&AnyObject)) -> Result<(), String> {
    with_updater(|updater| {
        // A check requested mid-session is a silent no-op in Sparkle. Refuse
        // it instead, so the running session's outcome is not mistaken for
        // this one's.
        // SAFETY: `sessionInProgress` is a readonly BOOL property of
        // SPUUpdater.
        let busy: bool = unsafe { msg_send![updater, sessionInProgress] };
        if busy {
            return Err("an update check is already running".to_string());
        }
        RECORD.with(|cell| cell.borrow_mut().recorded = false);
        call(updater);
        Ok(())
    })
    .unwrap_or_else(|| {
        Err(status()
            .unavailable
            .unwrap_or_else(|| "the updater is unavailable".to_string()))
    })
}

fn with_updater<T>(f: impl FnOnce(&AnyObject) -> T) -> Option<T> {
    OBJECTS.with(|cell| {
        let objects = cell.borrow();
        let updater = objects.as_ref()?.updater.as_ref()?;
        Some(f(updater))
    })
}

/// The outer `Err` means there is no updater at all. A controller whose
/// `startUpdater:` refused still returns its objects, with `updater: None` and
/// the reason as the inner `Err`: they must stay alive, because Sparkle may
/// still hold the weakly referenced delegate.
fn load(mtm: MainThreadMarker) -> Result<(Objects, Result<(), String>), String> {
    let path = framework_path()?;
    let handle = dlopen(&path)?;
    let class = AnyClass::get(c"SPUStandardUpdaterController").ok_or_else(|| {
        format!(
            "{} loaded without SPUStandardUpdaterController",
            path.display()
        )
    })?;

    // SAFETY: `init` on a freshly allocated instance of our own NSObject
    // subclass.
    let delegate: Retained<UpdaterDelegate> =
        unsafe { msg_send![UpdaterDelegate::alloc(mtm), init] };

    // `startingUpdater: false`, then `startUpdater:` below: when the bundle is
    // misconfigured the controller's own start shows the user a modal "contact
    // the developer" alert, while the direct call returns an NSError.
    // SAFETY: `-initWithStartingUpdater:(BOOL)updaterDelegate:(nullable
    // id)userDriverDelegate:(nullable id)` on a fresh allocation of the class
    // looked up above; the delegate outlives the controller (both live in
    // OBJECTS for the app's life).
    let controller: Retained<AnyObject> = unsafe {
        let allocated: Allocated<AnyObject> = msg_send![class, alloc];
        msg_send![
            allocated,
            initWithStartingUpdater: false,
            updaterDelegate: &*delegate as &AnyObject,
            userDriverDelegate: ptr::null::<AnyObject>(),
        ]
    };
    // SAFETY: `updater` is a readonly, nonnull object property of
    // SPUStandardUpdaterController.
    let updater: Retained<AnyObject> = unsafe { msg_send![&*controller, updater] };

    let mut error: *mut AnyObject = ptr::null_mut();
    // SAFETY: `- (BOOL)startUpdater:(NSError **)error`; `error` is a valid
    // out-pointer.
    let started: bool = unsafe { msg_send![&*updater, startUpdater: &mut error] };
    let (updater, result) = if started {
        (Some(updater), Ok(()))
    } else if error.is_null() {
        (None, Err("the updater refused to start".to_string()))
    } else {
        // SAFETY: on failure `startUpdater:` stores an autoreleased NSError,
        // valid for the rest of this call.
        (None, Err(error_description(unsafe { &*error })))
    };
    let objects = Objects {
        _handle: handle,
        _controller: controller,
        updater,
        _delegate: delegate,
    };
    Ok((objects, result))
}

/// `Contents/MacOS/wren` → `Contents/Frameworks/Sparkle.framework/Sparkle`.
/// For a binary outside a bundle this names a path that does not exist.
fn framework_path() -> Result<PathBuf, String> {
    let exe = std::env::current_exe()
        .map_err(|error| format!("could not find the executable: {error}"))?;
    // Resolve symlinks so a linked launch still finds the real Contents/.
    let exe = exe.canonicalize().unwrap_or(exe);
    let dir = exe
        .parent()
        .ok_or_else(|| format!("{} has no parent directory", exe.display()))?;
    Ok(dir.join(FRAMEWORK))
}

fn dlopen(path: &Path) -> Result<*mut c_void, String> {
    // For a missing path dyld falls back to searching by leaf name; check
    // first so a stray "Sparkle" library elsewhere is never loaded.
    if !path.exists() {
        return Err(format!("{} does not exist", path.display()));
    }
    let c_path = CString::new(path.as_os_str().as_bytes())
        .map_err(|_| format!("{} contains a NUL byte", path.display()))?;
    // SAFETY: a valid NUL-terminated path and a documented flag. The handle
    // is never closed.
    let handle = unsafe { libc::dlopen(c_path.as_ptr(), libc::RTLD_LAZY) };
    if handle.is_null() {
        // SAFETY: `dlerror` returns null or a NUL-terminated string owned by
        // the loader, valid until the next dl* call on this thread.
        let detail = unsafe {
            let raw = libc::dlerror();
            if raw.is_null() {
                "no detail".to_string()
            } else {
                CStr::from_ptr(raw).to_string_lossy().into_owned()
            }
        };
        return Err(format!("could not load {}: {detail}", path.display()));
    }
    Ok(handle)
}

fn item_version(item: &AnyObject) -> Option<String> {
    // SAFETY: `respondsToSelector:` is NSObject's; when it answers yes,
    // `displayVersionString` is SUAppcastItem's readonly NSString property.
    unsafe {
        let responds: bool = msg_send![item, respondsToSelector: sel!(displayVersionString)];
        if !responds {
            return None;
        }
        let value: Option<Retained<NSString>> = msg_send![item, displayVersionString];
        value.map(|string| string.to_string())
    }
}

fn error_description(error: &AnyObject) -> String {
    // SAFETY: Sparkle passes NSError objects to these callbacks, and
    // `localizedDescription` is NSError's readonly NSString property.
    let description: Option<Retained<NSString>> = unsafe { msg_send![error, localizedDescription] };
    description.map_or_else(|| "unknown error".to_string(), |text| text.to_string())
}
