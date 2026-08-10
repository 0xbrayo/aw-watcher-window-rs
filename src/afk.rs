//! AFK detection for the merged window+AFK watcher.
//!
//! Writes to `aw-watcher-afk-rs_{hostname}` with event type `afkstatus.rs`
//! (distinct from the official `afkstatus` bucket).
//!
//! AFK when:
//! - frontmost app is the login/lock screen (`loginwindow`, screen saver, …), or
//! - system input idle time ≥ configured threshold.

use serde::{Deserialize, Serialize};
use tracing::debug;

use crate::window::WindowData;

pub const AFK_CLIENT_NAME: &str = "aw-watcher-afk-rs";
/// Distinct from official `afkstatus` so aw-webui does not double-count by type alone.
pub const DEFAULT_AFK_EVENT_TYPE: &str = "afkstatus.rs";

/// Apps treated as login / lock / session UI → always AFK.
const LOCK_SCREEN_APPS: &[&str] = &[
    "loginwindow",
    "Login Window",
    "ScreenSaverEngine",
    "Screen Saver",
    "ScreenSaverEngine.app",
    // User-switch / secure desktop-ish
    "universalAccessAuthWarn",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AfkStatus {
    Afk,
    NotAfk,
}

impl AfkStatus {
    /// Temporary payload string exposed as `title` (see [`AfkData`]).
    pub fn as_title(self) -> &'static str {
        match self {
            Self::Afk => "afk",
            Self::NotAfk => "not-afk",
        }
    }
}

/// AFK heartbeat payload.
///
/// **TEMPORARY:** field is `title` (not official `status`) so events show up in UIs /
/// explorers that surface a `title` key. Restore official shape before production use:
///
/// ```json
/// { "status": "afk" }
/// { "status": "not-afk" }
/// ```
///
/// Current temporary shape:
///
/// ```json
/// { "title": "afk" }
/// { "title": "not-afk" }
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AfkData {
    // TODO(temporary): rename `title` → `status` for official afkstatus schema compatibility.
    pub title: String,
}

impl AfkData {
    pub fn afk() -> Self {
        Self {
            title: AfkStatus::Afk.as_title().to_string(),
        }
    }

    pub fn not_afk() -> Self {
        Self {
            title: AfkStatus::NotAfk.as_title().to_string(),
        }
    }
}

/// Seconds since any keyboard/mouse/tablet input (combined session).
pub fn seconds_since_last_input() -> Option<f64> {
    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        // CGEventSourceSecondsSinceLastEventType(CGEventSourceStateID, CGEventType) -> CFTimeInterval
        fn CGEventSourceSecondsSinceLastEventType(state_id: i32, event_type: u32) -> f64;
    }

    // kCGEventSourceStateCombinedSessionState = 0
    // kCGAnyInputEventType = ~0
    const COMBINED_SESSION: i32 = 0;
    const ANY_INPUT: u32 = u32::MAX;

    let secs = unsafe { CGEventSourceSecondsSinceLastEventType(COMBINED_SESSION, ANY_INPUT) };
    if secs.is_finite() && secs >= 0.0 {
        Some(secs)
    } else {
        None
    }
}

/// True if this window sample is the macOS login / lock UI.
pub fn is_login_screen(window: &WindowData) -> bool {
    let app = window.app.as_str();
    if LOCK_SCREEN_APPS.contains(&app) {
        return true;
    }
    // Defensive: empty app sometimes appears on secure desktop
    if app.is_empty() && window.title.eq_ignore_ascii_case("loginwindow") {
        return true;
    }
    app.eq_ignore_ascii_case("loginwindow")
}

/// Decide AFK from window sample + idle threshold (seconds).
///
/// Login/lock screen always forces `afk`, regardless of idle timers.
pub fn resolve_afk_status(window: Option<&WindowData>, idle_timeout_secs: f64) -> AfkData {
    if let Some(w) = window
        && is_login_screen(w)
    {
        debug!("AFK: login/lock screen (app={:?})", w.app);
        return AfkData::afk();
    }

    match seconds_since_last_input() {
        Some(idle) if idle >= idle_timeout_secs => {
            debug!(
                "AFK: idle {:.1}s >= threshold {:.1}s",
                idle, idle_timeout_secs
            );
            AfkData::afk()
        }
        Some(idle) => {
            debug!(
                "not-afk: idle {:.1}s < threshold {:.1}s",
                idle, idle_timeout_secs
            );
            AfkData::not_afk()
        }
        None => {
            // If idle API fails, only trust lock-screen path; default not-afk
            debug!("idle API unavailable; defaulting to not-afk");
            AfkData::not_afk()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loginwindow_is_login_screen() {
        let w = WindowData {
            app: "loginwindow".into(),
            title: "loginwindow".into(),
            url: None,
        };
        assert!(is_login_screen(&w));
        assert_eq!(resolve_afk_status(Some(&w), 180.0).title, "afk");
    }

    #[test]
    fn normal_app_not_login() {
        let w = WindowData {
            app: "Ghostty".into(),
            title: "shell".into(),
            url: None,
        };
        assert!(!is_login_screen(&w));
    }

    #[test]
    fn afk_title_serde_temporary() {
        // TEMPORARY: payload uses `title`, not official `status`.
        let a = AfkData::afk();
        let j = serde_json::to_string(&a).unwrap();
        assert_eq!(j, r#"{"title":"afk"}"#);
        let n = AfkData::not_afk();
        let j = serde_json::to_string(&n).unwrap();
        assert_eq!(j, r#"{"title":"not-afk"}"#);
    }
}
