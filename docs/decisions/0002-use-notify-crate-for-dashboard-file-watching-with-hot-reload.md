---
status: accepted
date: 2026-02-21
decision-makers: Josh Wood
---

# Use notify crate for dashboard file watching with hot-reload

## Context and Problem Statement

hbtui loads dashboard layouts from YAML files at startup and displays them as TUI widgets. During development — both by humans editing YAML and by AI coding agents generating or modifying dashboards programmatically — the only way to see changes was to quit and restart the app. This created a slow feedback loop, especially for agents that modify dashboard files as part of an automated workflow and need the user to see results immediately.

The app is built with Rust using tokio (async runtime), crossterm/ratatui (TUI), and an existing `AppMessage` mpsc channel pattern for delivering async results to the main event loop. Any file-watching solution needs to integrate with this architecture without blocking the 100ms UI poll cycle.

Constraints:
- Must work cross-platform (macOS primary, Linux secondary)
- Must not block the main event loop or degrade TUI responsiveness
- Must handle editor save patterns (multiple rapid writes) gracefully
- Should integrate with the existing `AppMessage` / `mpsc` channel pattern
- Should not require restructuring the event loop

## Decision

Use the `notify` crate (v6.1) to watch dashboard YAML files for changes and hot-reload them in-place.

**Scope:**
- Watch `.yml`/`.yaml` files in the dashboard directory (or parent directory for single-file mode)
- On file modification, re-parse YAML and replace the `DashboardState` in-place
- Re-fetch widget data from the Honeybadger API if the changed dashboard is currently active
- Update the dashboard display name if the YAML `title` field changed
- Debounce events (200ms) to handle rapid successive writes from editors
- Canonicalize all file paths to ensure reliable matching across platforms
- Gracefully handle parse errors by keeping the previous dashboard intact

**Non-goals:**
- Watching for new files added to the dashboard directory (only pre-loaded files are tracked)
- Watching for file deletions
- Recursive directory watching
- Logging watcher errors to a visible location (currently `eprintln!` which is invisible in TUI mode)

## Consequences

* Good, because developers and agents can edit dashboard YAML and see changes reflected immediately without restarting
* Good, because AI agents can modify dashboards programmatically and users see the results in real-time
* Good, because the implementation reuses the existing `AppMessage` channel pattern — no new concurrency model introduced
* Good, because parse errors are non-destructive — a syntax error mid-edit doesn't crash or blank the dashboard
* Bad, because adds `notify` as a dependency (pulls in platform-specific sub-crates: `fsevent-sys`, `inotify`, `kqueue`, `mio 0.8`)
* Bad, because migrated the channel from `std::sync::mpsc` to `tokio::sync::mpsc` — existing code using `try_recv()` is unaffected but this is a subtle change future contributors should be aware of
* Bad, because watcher errors (`eprintln!`) are invisible in alternate-screen TUI mode — a future change should route these to a status bar or log file
* Follow-up: consider watching for new/deleted files in the dashboard directory
* Follow-up: add a visible status indicator when a dashboard is reloaded (e.g., brief flash or status bar message)

## Implementation Plan

* **Affected paths**: `Cargo.toml`, `Cargo.lock`, `src/main.rs`
* **Dependencies**: Add `notify = "6.1"`
* **Patterns to follow**:
  - `AppMessage` enum for async-to-main-thread communication (`src/main.rs:62-72`)
  - `process_messages()` method for handling channel messages (`src/main.rs:232`)
  - `fetch_all_widgets()` / `fetch_widgets_for_dashboard()` for triggering API refetch (`src/main.rs:226-229`)
  - `DashboardState::new()` for constructing fresh dashboard state (`src/dashboard.rs:166`)
  - `canonicalize()` on all stored paths to ensure consistent matching with notify events
* **Patterns to avoid**:
  - Do not use `tokio::spawn` for the file watcher thread — notify callbacks are synchronous, use `std::thread::spawn`
  - Do not use `std::sync::mpsc` — the codebase has migrated to `tokio::sync::mpsc`; use `blocking_send()` from sync contexts
  - Do not `unwrap()` YAML parse results during reload — always fall back to keeping the previous dashboard
  - Do not pass `project_id` as a parameter to the reload method — read it from the existing `DashboardState` at the matched index

### Key Integration Points

| Location | What | Purpose |
|----------|------|---------|
| `AppMessage` enum | `DashboardFileChanged { path: PathBuf }` variant | Carries file change events to the main loop |
| `App` struct | `dashboard_paths: Vec<PathBuf>` field | Maps dashboard indices to canonical file paths |
| `process_messages()` | Match arm for `DashboardFileChanged` | Routes events to `reload_dashboard_from_path()` |
| `reload_dashboard_from_path()` | New method on `App` | Re-reads YAML, replaces state, updates name, triggers refetch |
| `spawn_file_watcher()` | New free function | Spawns OS thread with notify watcher, debounce logic, and channel send |
| `main()` after `fetch_all_widgets()` | Watcher startup | Starts watcher for directory or single-file parent |

### Verification

- [x] `cargo build` succeeds
- [x] `cargo clippy` reports zero warnings
- [x] `cargo test` passes all 21 tests (16 existing + 5 new)
- [x] New tests cover: successful reload, invalid YAML resilience, unknown path no-op, active dashboard refetch, inactive dashboard no-refetch
- [x] Manual test: run `hbtui`, edit a dashboard YAML in another editor, save — TUI updates without restart
- [x] Works for all three load modes: default `.hbtui/dashboards/`, explicit directory via `-d`, single file via `-d`

## Alternatives Considered

* **Polling with `std::fs::metadata` timestamps**: Simpler (no new dependency), but wastes CPU checking mtimes on a timer, has higher latency (limited by poll interval), and requires manual cross-platform mtime handling. Rejected because notify is zero-overhead when idle and provides near-instant notification.
* **tokio-based async file watchers (e.g., `async-watcher`)**: Would avoid the `std::thread::spawn` / `blocking_send` bridge. Rejected because these crates are less mature, and the sync callback + `blocking_send` pattern works cleanly with the existing architecture. notify is the de facto standard with 85M+ downloads.

## More Information

Implemented in commit `2f43ce5` on branch `watch-dashboards`.

**Revisit this decision if:**
- The app needs to detect new/deleted dashboard files (would require a directory scan on watcher events, not just path matching)
- A logging or status-bar infrastructure is added (route watcher errors there instead of `eprintln!`)
- The `notify` crate introduces breaking changes or a clearly superior async-native alternative emerges
