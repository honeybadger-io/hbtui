# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

This is a terminal user interface (TUI) application for Honeybadger.io, built with Ratatui (Rust). The first feature is a simple dashboard showing project statistics and fault counts.

## Development Commands

**Build the project:**
```bash
cargo build
```

**Run the application:**
```bash
HONEYBADGER_PERSONAL_AUTH_TOKEN=your_api_key cargo run
```

**Run in development with auto-reload:**
```bash
HONEYBADGER_PERSONAL_AUTH_TOKEN=your_api_key cargo watch -x run
```

**Check code without building:**
```bash
cargo check
```

**Run tests:**
```bash
cargo test
```

**Format code:**
```bash
cargo fmt
```

**Run linter:**
```bash
cargo clippy
```

## Architecture

### Module Structure

- **src/main.rs** - Main application entry point and TUI rendering logic
  - Sets up Crossterm terminal backend with raw mode and alternate screen
  - Contains the main `App` struct that holds application state (client, stats, quit flag)
  - Event loop polls for keyboard input every 100ms
  - Key bindings: 'q' to quit, 'r' to refresh data
  - UI layout: header, split main content area (stats on left, projects list on right), footer

- **src/honeybadger.rs** - Honeybadger API client implementation
  - `HoneybadgerClient` handles all API communication using reqwest
  - Authentication via Bearer token in Authorization header
  - Implements `list_projects()` and `get_fault_counts(project_id)` API calls
  - `get_project_stats()` aggregates data from multiple API calls (fetches first 10 projects only to avoid rate limiting)
  - Returns `ProjectStats` with totals and enriched project list

### Application Flow

1. Application reads `HONEYBADGER_PERSONAL_AUTH_TOKEN` from environment variable (required)
2. Terminal is initialized with Crossterm backend in raw mode with alternate screen
3. Initial data load fetches project stats asynchronously using Tokio runtime
4. Main event loop renders UI and polls for keyboard events
5. On exit, terminal is properly restored (raw mode disabled, alternate screen cleared)

### Key Dependencies

- **ratatui 0.29** - TUI framework for rendering
- **crossterm 0.28** - Terminal manipulation (raw mode, events, etc.)
- **tokio 1.x** - Async runtime with full features
- **reqwest 0.12** - HTTP client with JSON support for API calls
- **serde/serde_json** - JSON serialization/deserialization

### Data Model

- `Project`: ID, name, and fault_count (enriched from API)
- `ProjectStats`: Aggregated view with total_projects, total_faults, unresolved_faults, and recent_projects list
- API responses are deserialized into internal structs (`ProjectsResponse`, `FaultCountsResponse`)

## Honeybadger API Integration

The application uses Honeybadger's V2 API with endpoints:
- `GET /v2/projects` - List all projects
- `GET /v2/projects/{id}/fault_counts` - Get fault counts for a specific project

All requests require Bearer authentication with the API key.
