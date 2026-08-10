# aw-watcher-window-rs

[![Build](https://github.com/0xbrayo/aw-watcher-window-rs/actions/workflows/build.yml/badge.svg?branch=main)](https://github.com/0xbrayo/aw-watcher-window-rs/actions/workflows/build.yml)

macOS **window + AFK** watcher for [ActivityWatch](https://activitywatch.net), written in Rust.

Tracks:

1. **Window** → bucket `aw-watcher-window-rs_{hostname}`
2. **AFK** → bucket `aw-watcher-afk-rs_{hostname}`

## AFK rules

| Condition | Status |
|-----------|--------|
| Frontmost app is login / lock screen (`loginwindow`, screen saver, …) | **`afk`** |
| No keyboard/mouse input for ≥ `--afk-timeout` seconds (default 180) | **`afk`** |
| Otherwise | **`not-afk`** |

Uses a **different bucket and type** so it does not collide with `aw-watcher-afk`:

| | Official | This crate |
|---|----------|------------|
| Bucket | `aw-watcher-afk_{host}` | `aw-watcher-afk-rs_{host}` |
| Type | `afkstatus` | `afkstatus.rs` (override with `--afk-event-type`) |
| Payload | `{"status":"afk"\|"not-afk"}` | **TEMPORARY:** `{"title":"afk"\|"not-afk"}` (see below) |

> **Temporary payload:** AFK events currently use a `title` field (`"afk"` / `"not-afk"`) instead of the official `status` field so values are easier to see in explorers/UIs that surface titles. Restore `{"status":"afk"|"not-afk"}` before treating this as production-compatible with aw-webui AFK views (search `TODO(temporary)` in `src/afk.rs`).

## Requirements

- macOS
- ActivityWatch server (`aw-server`)
- **Accessibility** (window titles)
- **Screen Recording** recommended (CGWindowList title fallback)
- **Automation** may prompt for Chrome-family / Safari URL capture

```bash
cargo build --release
./target/release/aw-watcher-window-rs --verbose
```

## CLI

| Flag | Default | Description |
|------|---------|-------------|
| `--host` | `127.0.0.1` | aw-server host |
| `--port` | `5600` / `5666` | Port (`5666` with `--testing`) |
| `--testing` | off | Testing server |
| `--poll-time` | `1.0` | Poll interval; `pulsetime = poll_time + 1` |
| `--exclude-title` / `--exclude-titles` | — | Title privacy filters |
| `--event-type` | `currentwindow.rs` | Window bucket type |
| `--afk-timeout` | `180` | Idle seconds → afk |
| `--afk-event-type` | `afkstatus.rs` | AFK bucket type |
| `--disable-afk` | off | Window-only mode |
| `--verbose` | off | Debug logs |
| `--exit-with-parent` | off | Exit when parent dies (aw-qt) |
| `--config` | auto | TOML config path |

```bash
# Live server, both buckets
./target/release/aw-watcher-window-rs --verbose --afk-timeout 60

# Window only
./target/release/aw-watcher-window-rs --disable-afk
```

### Config file (optional)

`~/Library/Application Support/activitywatch/aw-watcher-window-rs/config.toml` or  
`~/.config/activitywatch/aw-watcher-window-rs/config.toml`

```toml
poll_time = 1.0
afk_timeout = 180
# event_type = "currentwindow"
# afk_event_type = "afkstatus"
exclude_titles = ["password"]
```

## Data model

**Window** (`aw-watcher-window-rs_*`):

```json
{ "app": "Firefox", "title": "…", "url": "https://…" }
```

**AFK** (`aw-watcher-afk-rs_*`) — **temporary** shape (use `title` for visibility):

```json
{ "title": "afk" }
```

```json
{ "title": "not-afk" }
```

Official shape to restore later: `{ "status": "afk" }` / `{ "status": "not-afk" }`.

## Development

```bash
# Install pre-commit hooks (fmt + clippy on commit, tests on push)
pip install pre-commit   # or: brew install pre-commit
pre-commit install
pre-commit install --hook-type pre-push

cargo fmt --all
cargo clippy --all-targets -- -D warnings
cargo test
```

CI (GitHub Actions) runs format check, clippy, `cargo check`, and tests on `macos-latest` for pushes/PRs to `main`. Tagged releases (`v*.*.*`) build a macOS binary draft release.

## Not included

- Linux / Windows
- Replacing official bucket names (`aw-watcher-window` / `aw-watcher-afk`) — use `-rs` suffixes on purpose
