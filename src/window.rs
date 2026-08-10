use serde::{Deserialize, Serialize};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use tracing::{debug, warn};

use core_foundation::array::{CFArray, CFArrayRef};
use core_foundation::base::{CFType, TCFType};
use core_foundation::dictionary::{CFDictionary, CFDictionaryRef};
use core_foundation::number::CFNumber;
use core_foundation::string::CFString;
use objc2_app_kit::NSWorkspace;

use axuielement::ax_attribute::{
    AX_FOCUSED_WINDOW_ATTRIBUTE, AX_MAIN_ATTRIBUTE, AX_MAIN_WINDOW_ATTRIBUTE, AX_TITLE_ATTRIBUTE,
    AX_WINDOWS_ATTRIBUTE,
};
use axuielement::prelude::*;

/// Chrome-family browsers (AppleScript dictionary supports mode + active tab URL).
const CHROME_BROWSERS: &[&str] = &[
    "Google Chrome",
    "Google Chrome Canary",
    "Google Chrome Beta",
    "Google Chrome Dev",
    "Chromium",
    "Brave Browser",
    "Brave Browser Beta",
    "Microsoft Edge",
    "Microsoft Edge Beta",
    "Microsoft Edge Dev",
    "Microsoft Edge Canary",
    "Vivaldi",
    "Opera",
    "Arc",
    "Dia",
    "Sidekick",
    "Wavebox",
];

/// Bundle-id prefixes treated as Chrome-family for URL/incognito scripting.
const CHROME_BUNDLE_PREFIXES: &[&str] = &[
    "com.google.Chrome",
    "com.google.chrome",
    "org.chromium.Chromium",
    "com.brave.Browser",
    "com.microsoft.edgemac",
    "com.operasoftware.Opera",
    "com.vivaldi.Vivaldi",
    "company.thebrowser.Browser", // Arc
    "com.kagi.kagimacOS",         // Orion is Safari-like; skip
];

static OPENED_ACCESSIBILITY_SETTINGS: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
pub struct WindowData {
    pub app: String,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

fn open_accessibility_settings_once() {
    if OPENED_ACCESSIBILITY_SETTINGS.swap(true, Ordering::SeqCst) {
        return;
    }
    let _ = Command::new("open")
        .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility")
        .status();
}

struct Frontmost {
    app_name: String,
    pid: i32,
    bundle_id: Option<String>,
}

/// Frontmost app via NSWorkspace (does not need Accessibility).
fn frontmost_app() -> Option<Frontmost> {
    let workspace = NSWorkspace::sharedWorkspace();
    let app = workspace.frontmostApplication()?;
    let pid = app.processIdentifier();
    let app_name = app
        .localizedName()
        .map(|s| s.to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "Unknown".to_string());
    let bundle_id = app.bundleIdentifier().map(|s| s.to_string());

    Some(Frontmost {
        app_name,
        pid,
        bundle_id,
    })
}

fn is_chrome_family(app_name: &str, bundle_id: Option<&str>) -> bool {
    if CHROME_BROWSERS.contains(&app_name) {
        return true;
    }
    if let Some(bid) = bundle_id {
        return CHROME_BUNDLE_PREFIXES
            .iter()
            .any(|p| bid == *p || bid.starts_with(&format!("{p}.")));
    }
    false
}

fn is_safari(app_name: &str, bundle_id: Option<&str>) -> bool {
    app_name == "Safari"
        || app_name == "Safari Technology Preview"
        || bundle_id == Some("com.apple.Safari")
        || bundle_id == Some("com.apple.SafariTechnologyPreview")
}

/// Title via AX for a given app pid.
fn ax_window_title(pid: i32) -> Option<String> {
    let ax_app = AXUIElement::from_pid(pid)?;

    // 1) Focused window
    match ax_app.element_attribute(AX_FOCUSED_WINDOW_ATTRIBUTE) {
        Ok(Some(window)) => match window.string_attribute(AX_TITLE_ATTRIBUTE) {
            Ok(Some(t)) if !t.is_empty() => return Some(t),
            Ok(_) => debug!("AX focused window has empty/missing title"),
            Err(e) => debug!("AX focused window title error: {:?}", e),
        },
        Ok(None) => debug!("AX focused window is None for pid={}", pid),
        Err(e) => debug!("AX focused window error for pid={}: {:?}", pid, e),
    }

    // 2) Main window attribute
    if let Ok(Some(window)) = ax_app.element_attribute(AX_MAIN_WINDOW_ATTRIBUTE)
        && let Ok(Some(t)) = window.string_attribute(AX_TITLE_ATTRIBUTE)
        && !t.is_empty()
    {
        return Some(t);
    }

    // 3) Windows list — prefer AXMain, else first with a title
    match ax_app.attribute(AX_WINDOWS_ATTRIBUTE) {
        Ok(Some(val)) => {
            if let Some(windows) = val.as_array() {
                let mut first_title: Option<String> = None;
                for wval in windows {
                    let Some(win) = wval.as_element() else {
                        continue;
                    };
                    let is_main = win
                        .bool_attribute(AX_MAIN_ATTRIBUTE)
                        .ok()
                        .flatten()
                        .unwrap_or(false);
                    let title = win
                        .string_attribute(AX_TITLE_ATTRIBUTE)
                        .ok()
                        .flatten()
                        .unwrap_or_default();
                    if title.is_empty() {
                        continue;
                    }
                    if is_main {
                        return Some(title);
                    }
                    if first_title.is_none() {
                        first_title = Some(title);
                    }
                }
                if first_title.is_some() {
                    return first_title;
                }
            }
        }
        Ok(None) => debug!("AXWindows is None for pid={}", pid),
        Err(e) => debug!("AXWindows error for pid={}: {:?}", pid, e),
    }

    // 4) System-wide focused window
    if let Some(system) = system_wide()
        && let Ok(Some(win)) = system.focused_window()
        && let Ok(Some(t)) = win.string_attribute(AX_TITLE_ATTRIBUTE)
        && !t.is_empty()
    {
        return Some(t);
    }

    None
}

/// Request Screen Recording so CGWindowList can return window names on modern macOS.
fn request_screen_capture_access() {
    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        fn CGPreflightScreenCaptureAccess() -> bool;
        fn CGRequestScreenCaptureAccess() -> bool;
    }
    unsafe {
        if !CGPreflightScreenCaptureAccess() {
            let ok = CGRequestScreenCaptureAccess();
            debug!("CGRequestScreenCaptureAccess => {}", ok);
        }
    }
}

/// Title via CGWindowListCopyWindowInfo for the given owner pid (layer 0, on-screen).
fn cg_window_title(pid: i32) -> Option<String> {
    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        fn CGWindowListCopyWindowInfo(option: u32, relative_to_window: u32) -> CFArrayRef;
    }

    const ON_SCREEN_ONLY: u32 = 1 << 0;
    const EXCLUDE_DESKTOP: u32 = 1 << 4;
    const NULL_WINDOW: u32 = 0;

    unsafe {
        let arr_ref = CGWindowListCopyWindowInfo(ON_SCREEN_ONLY | EXCLUDE_DESKTOP, NULL_WINDOW);
        if arr_ref.is_null() {
            return None;
        }
        let arr = CFArray::<CFType>::wrap_under_create_rule(arr_ref);
        let key_name = CFString::new("kCGWindowName");
        let key_pid = CFString::new("kCGWindowOwnerPID");
        let key_layer = CFString::new("kCGWindowLayer");

        for i in 0..arr.len() {
            let Some(item) = arr.get(i) else {
                continue;
            };
            let dict_ref = item.as_CFTypeRef() as CFDictionaryRef;
            if dict_ref.is_null() {
                continue;
            }
            let dict = CFDictionary::<CFString, CFType>::wrap_under_get_rule(dict_ref);

            let Some(pid_val) = dict.find(&key_pid) else {
                continue;
            };
            let Some(owner_pid) = pid_val
                .clone()
                .downcast_into::<CFNumber>()
                .and_then(|n| n.to_i64())
            else {
                continue;
            };
            if owner_pid != i64::from(pid) {
                continue;
            }

            let layer = dict
                .find(&key_layer)
                .and_then(|v| v.clone().downcast_into::<CFNumber>())
                .and_then(|n| n.to_i64())
                .unwrap_or(-1);
            if layer != 0 {
                continue;
            }

            let title = dict
                .find(&key_name)
                .and_then(|v| v.clone().downcast_into::<CFString>())
                .map(|s| s.to_string())
                .unwrap_or_default();
            if title.is_empty() {
                continue;
            }
            return Some(title);
        }
        None
    }
}

fn title_for_pid(pid: i32, trusted: bool) -> Option<String> {
    if trusted {
        ax_window_title(pid).or_else(|| {
            let t = cg_window_title(pid);
            if t.is_some() {
                debug!("AX empty; using CGWindowList title for pid={}", pid);
            }
            t
        })
    } else {
        cg_window_title(pid)
    }
}

/// When title is still empty, fill in for apps where the official watcher does too.
fn title_fallback(app_name: &str, title: String) -> String {
    if !title.is_empty() {
        return title;
    }
    // loginwindow / Screen Saver: main Swift watcher often ends up with title == app name
    match app_name {
        "loginwindow" | "ScreenSaverEngine" | "Screen Saver" => app_name.to_string(),
        _ => title,
    }
}

/// Capture frontmost app + window title (AX when trusted, else CGWindowList).
fn get_ax_window() -> Option<(String, String, i32, Option<String>)> {
    let front = frontmost_app()?;
    let trusted = axuielement::is_process_trusted();

    // During focus changes CG/AX sometimes return an empty name for one frame.
    let mut title = title_for_pid(front.pid, trusted).unwrap_or_default();
    let (app_name, pid, bundle_id) = if title.is_empty() {
        std::thread::sleep(Duration::from_millis(40));
        let front2 = frontmost_app()?;
        title = title_for_pid(front2.pid, trusted).unwrap_or_default();
        if title.is_empty() {
            debug!(
                "Empty title for app={} pid={} trusted={}",
                front2.app_name, front2.pid, trusted
            );
        }
        (front2.app_name, front2.pid, front2.bundle_id)
    } else {
        (front.app_name, front.pid, front.bundle_id)
    };

    let title = title_fallback(&app_name, title);
    Some((app_name, title, pid, bundle_id))
}

#[derive(Debug, Deserialize)]
struct BrowserScriptResult {
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    mode: Option<String>,
}

/// Run osascript with a hard wall-clock timeout so a hung browser can't stall the poll loop.
fn run_osascript(source: &str, timeout: Duration) -> Option<String> {
    let mut child = Command::new("osascript")
        .args(["-e", source])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .ok()?;

    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if start.elapsed() > timeout => {
                debug!("osascript timed out after {:?}", timeout);
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(e) => {
                debug!("osascript wait error: {}", e);
                let _ = child.kill();
                return None;
            }
        }
    }

    let output = child.wait_with_output().ok()?;
    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr);
        debug!("osascript failed: {}", err.trim());
        return None;
    }
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if stdout.is_empty() {
        None
    } else {
        Some(stdout)
    }
}

/// Chrome-family: mode + active tab URL/title via osascript (line-separated).
fn chrome_browser_info(app_name: &str) -> Option<BrowserScriptResult> {
    let app = app_name.replace('\\', "\\\\").replace('"', "\\\"");
    let source = format!(
        r#"
tell application "{app}"
    if (count of windows) is 0 then return ""
    set w to front window
    set t to active tab of w
    set modeStr to mode of w
    set urlStr to URL of t
    set titleStr to title of t
    return modeStr & linefeed & urlStr & linefeed & titleStr
end tell
"#
    );

    let out = run_osascript(&source, Duration::from_millis(800))?;
    let mut lines = out.lines();
    let mode = lines.next().map(|s| s.to_string());
    let url = lines
        .next()
        .map(|s| s.to_string())
        .filter(|s| !s.is_empty());
    let rest: Vec<&str> = lines.collect();
    let title = if rest.is_empty() {
        None
    } else {
        Some(rest.join("\n"))
    };

    Some(BrowserScriptResult { url, title, mode })
}

fn safari_browser_info(app_name: &str) -> Option<BrowserScriptResult> {
    let app = app_name.replace('\\', "\\\\").replace('"', "\\\"");
    let source = format!(
        r#"
tell application "{app}"
    if (count of windows) is 0 then return ""
    set t to current tab of front window
    set urlStr to URL of t
    set titleStr to name of t
    return urlStr & linefeed & titleStr
end tell
"#
    );
    let out = run_osascript(&source, Duration::from_millis(800))?;
    let mut lines = out.lines();
    let url = lines
        .next()
        .map(|s| s.to_string())
        .filter(|s| !s.is_empty());
    let rest: Vec<&str> = lines.collect();
    let title = if rest.is_empty() {
        None
    } else {
        Some(rest.join("\n"))
    };
    Some(BrowserScriptResult {
        url,
        title,
        mode: None,
    })
}

/// Active window info with optional browser URL (swift-strategy equivalent).
pub fn get_active_window() -> Option<WindowData> {
    let (app_name, ax_title, _pid, bundle_id) = get_ax_window()?;
    let bid = bundle_id.as_deref();

    // Chrome family: URL + incognito privacy (blank app/title when private)
    if is_chrome_family(&app_name, bid) {
        match chrome_browser_info(&app_name) {
            Some(info) if info.mode.as_deref() == Some("incognito") => {
                debug!("Chrome-family incognito — blanking app/title/url");
                return Some(WindowData {
                    app: String::new(),
                    title: String::new(),
                    url: None,
                });
            }
            Some(info) => {
                let title = info.title.filter(|t| !t.is_empty()).unwrap_or(ax_title);
                return Some(WindowData {
                    app: app_name,
                    title,
                    url: info.url,
                });
            }
            None => {
                debug!("Chrome scripting failed; falling back to AX/CG only");
                return Some(WindowData {
                    app: app_name,
                    title: ax_title,
                    url: None,
                });
            }
        }
    }

    if is_safari(&app_name, bid) {
        match safari_browser_info(&app_name) {
            Some(info) => {
                let title = info.title.filter(|t| !t.is_empty()).unwrap_or(ax_title);
                return Some(WindowData {
                    app: app_name,
                    title,
                    url: info.url,
                });
            }
            None => {
                debug!("Safari scripting failed; falling back to AX/CG only");
                return Some(WindowData {
                    app: app_name,
                    title: ax_title,
                    url: None,
                });
            }
        }
    }

    Some(WindowData {
        app: app_name,
        title: ax_title,
        url: None,
    })
}

/// Pump the Cocoa run loop briefly so NSWorkspace stays fresh.
pub fn pump_runloop() {
    use core_foundation::runloop::{CFRunLoop, kCFRunLoopDefaultMode};
    unsafe {
        CFRunLoop::run_in_mode(kCFRunLoopDefaultMode, Duration::from_millis(10), true);
    }
}

/// Log a warning if Accessibility is not granted; open Settings at most once per process.
/// Also request Screen Recording so CGWindowList can supply titles as a fallback.
pub fn ensure_accessibility_warning() {
    request_screen_capture_access();

    if is_process_trusted_with_prompt() {
        debug!("Accessibility permission granted");
        return;
    }
    warn!(
        "Accessibility permissions are not granted. \
         Enable Accessibility for this binary (e.g. target/release/aw-watcher-window-rs) \
         or your terminal in System Settings > Privacy & Security > Accessibility. \
         Without it, many window titles stay empty (CGWindowList may fill some)."
    );
    open_accessibility_settings_once();
}
