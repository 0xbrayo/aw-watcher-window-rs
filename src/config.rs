use clap::Parser;
use serde::Deserialize;
use std::fs;
use std::path::PathBuf;
use tracing::{debug, warn};

use crate::afk::DEFAULT_AFK_EVENT_TYPE;

pub const CLIENT_NAME: &str = "aw-watcher-window-rs";

/// Default while testing so aw-webui does not treat this as the primary window bucket.
pub const DEFAULT_EVENT_TYPE: &str = "currentwindow.rs";

/// Official AFK idle timeout default is often 180s; keep configurable.
pub const DEFAULT_AFK_TIMEOUT: f64 = 180.0;

/// File-config defaults (CLI overrides these).
#[derive(Debug, Default, Deserialize)]
pub struct FileConfig {
    #[serde(default)]
    pub host: Option<String>,
    #[serde(default)]
    pub port: Option<u16>,
    #[serde(default)]
    pub poll_time: Option<f64>,
    #[serde(default)]
    pub exclude_title: Option<bool>,
    #[serde(default)]
    pub exclude_titles: Option<Vec<String>>,
    #[serde(default)]
    pub event_type: Option<String>,
    #[serde(default)]
    pub afk_timeout: Option<f64>,
    #[serde(default)]
    pub afk_event_type: Option<String>,
    #[serde(default)]
    pub disable_afk: Option<bool>,
}

/// CLI: window watcher + merged AFK (`aw-watcher-afk-rs` bucket).
#[derive(Debug, Parser)]
#[command(
    name = "aw-watcher-window-rs",
    about = "macOS window + AFK watcher for ActivityWatch"
)]
pub struct Args {
    /// aw-server host
    #[arg(long, default_value = "127.0.0.1")]
    pub host: String,

    /// aw-server port (defaults to 5600, or 5666 with --testing)
    #[arg(long)]
    pub port: Option<u16>,

    /// Connect to the testing server (port 5666 unless --port is set)
    #[arg(long, default_value_t = false)]
    pub testing: bool,

    /// Poll interval in seconds
    #[arg(long, default_value_t = 1.0)]
    pub poll_time: f64,

    /// Replace all window titles with "excluded"
    #[arg(long, default_value_t = false)]
    pub exclude_title: bool,

    /// Regexes (case-insensitive); matching titles become "excluded"
    #[arg(long, num_args = 1.., value_name = "REGEX")]
    pub exclude_titles: Vec<String>,

    /// Window bucket event type (default avoids webui clash while testing)
    #[arg(long, default_value = DEFAULT_EVENT_TYPE)]
    pub event_type: String,

    /// Seconds without input before status becomes afk (login screen always afk)
    #[arg(long, default_value_t = DEFAULT_AFK_TIMEOUT)]
    pub afk_timeout: f64,

    /// AFK bucket event type (default `afkstatus.rs`, distinct from official `afkstatus`)
    #[arg(long, default_value = DEFAULT_AFK_EVENT_TYPE)]
    pub afk_event_type: String,

    /// Do not write the AFK bucket (window-only)
    #[arg(long, default_value_t = false)]
    pub disable_afk: bool,

    /// More verbose logging
    #[arg(long, default_value_t = false)]
    pub verbose: bool,

    /// Exit if the parent process dies (for launching under aw-qt)
    #[arg(long, default_value_t = false)]
    pub exit_with_parent: bool,

    /// Optional path to a TOML config file
    #[arg(long)]
    pub config: Option<PathBuf>,
}

impl Args {
    pub fn load() -> Self {
        let mut args = Self::parse();
        let path = args.config.clone().or_else(default_config_path);

        if let Some(path) = path {
            if path.is_file() {
                match load_file_config(&path) {
                    Ok(file) => {
                        debug!("Loaded config from {}", path.display());
                        apply_file_config(&mut args, file);
                    }
                    Err(e) => warn!("Failed to load config {}: {}", path.display(), e),
                }
            } else if args.config.is_some() {
                warn!("Config path {} does not exist", path.display());
            }
        }
        args
    }

    pub fn server_port(&self) -> u16 {
        self.port.unwrap_or(if self.testing { 5666 } else { 5600 })
    }

    pub fn pulsetime(&self) -> f64 {
        self.poll_time + 1.0
    }
}

fn default_config_path() -> Option<PathBuf> {
    if let Some(home) = dirs_home() {
        let mac =
            home.join("Library/Application Support/activitywatch/aw-watcher-window-rs/config.toml");
        if mac.is_file() {
            return Some(mac);
        }
        let xdg = home.join(".config/activitywatch/aw-watcher-window-rs/config.toml");
        if xdg.is_file() {
            return Some(xdg);
        }
    }
    None
}

fn dirs_home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

fn load_file_config(path: &PathBuf) -> Result<FileConfig, String> {
    let text = fs::read_to_string(path).map_err(|e| e.to_string())?;
    let value: toml::Value = text.parse().map_err(|e: toml::de::Error| e.to_string())?;
    let table = if let Some(t) = value.get("aw-watcher-window-rs") {
        t.clone()
    } else {
        value
    };
    table.try_into::<FileConfig>().map_err(|e| e.to_string())
}

fn apply_file_config(args: &mut Args, file: FileConfig) {
    if let Some(h) = file.host
        && args.host == "127.0.0.1"
    {
        args.host = h;
    }
    if args.port.is_none()
        && let Some(p) = file.port
    {
        args.port = Some(p);
    }
    if (args.poll_time - 1.0).abs() < f64::EPSILON
        && let Some(pt) = file.poll_time
    {
        args.poll_time = pt;
    }
    if !args.exclude_title
        && let Some(et) = file.exclude_title
    {
        args.exclude_title = et;
    }
    if args.exclude_titles.is_empty()
        && let Some(titles) = file.exclude_titles
    {
        args.exclude_titles = titles;
    }
    if args.event_type == DEFAULT_EVENT_TYPE
        && let Some(et) = file.event_type
    {
        args.event_type = et;
    }
    if (args.afk_timeout - DEFAULT_AFK_TIMEOUT).abs() < f64::EPSILON
        && let Some(t) = file.afk_timeout
    {
        args.afk_timeout = t;
    }
    if args.afk_event_type == DEFAULT_AFK_EVENT_TYPE
        && let Some(t) = file.afk_event_type
    {
        args.afk_event_type = t;
    }
    if !args.disable_afk
        && let Some(d) = file.disable_afk
    {
        args.disable_afk = d;
    }
}
