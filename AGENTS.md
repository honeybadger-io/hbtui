# AGENTS.md

This file provides guidance to AI coding agents when working with this repository.

## Project Overview

A terminal dashboard for Honeybadger.io, built with Ratatui (Rust). Displays configurable dashboard widgets powered by the Honeybadger Insights API.

## CLI Usage

```bash
# Required: project ID and auth token
hbtui -p <PROJECT_ID> --auth-token <TOKEN>

# Or via environment variables
HONEYBADGER_PROJECT_ID=12345 \
HONEYBADGER_PERSONAL_AUTH_TOKEN=xxx \
hbtui

# Custom dashboard file or directory
hbtui -p 12345 -d ./my-dashboard.yml
hbtui -p 12345 -d ./dashboards/

# Help and version
hbtui --help
hbtui --version
```

### Environment Variables

| Variable | CLI Flag | Description |
|----------|----------|-------------|
| `HONEYBADGER_PROJECT_ID` | `-p, --project-id` | Honeybadger project ID (required) |
| `HONEYBADGER_PERSONAL_AUTH_TOKEN` | `--auth-token` | API auth token (required) |
| `HONEYBADGER_DASHBOARDS` | `-d, --dashboards` | Dashboard file or directory |

### Dashboard Locations

If `-d` is not specified, hbtui looks for dashboards in order:

1. `./.hbtui/dashboards/` (project-local)
2. `~/.hbtui/dashboards/` (user default)

## Development Commands

```bash
cargo build          # Build
cargo run -- --help  # Run with args
cargo test           # Run tests (16 tests)
cargo clippy         # Lint (should be 0 warnings)
cargo fmt            # Format
```

## Key Bindings

| Key | Action |
|-----|--------|
| `q` | Quit |
| `r` | Refresh current dashboard |
| `[` / `]` | Previous / next dashboard |
| `1-9` | Jump to dashboard by number |
| Arrow keys | Navigate widgets |
| `Enter` | Maximize selected widget |
| `Esc` | Exit maximized view |
| Up/Down (maximized histogram) | Filter by series |

## Architecture

### Module Structure

- **src/main.rs** - CLI parsing (clap), terminal setup, event loop, UI rendering
- **src/honeybadger.rs** - API client with 30s timeout, Insights query support
- **src/dashboard.rs** - Dashboard YAML parsing, widget state management
- **src/layout.rs** - 12-column grid layout, widget navigation
- **src/widgets.rs** - Widget rendering (line charts, histograms, tables, billboards)

### Application Flow

1. Parse CLI args (clap) - project ID and auth token required
2. Load dashboards from file or directory
3. Validate at least one dashboard exists (exit with error if not)
4. Initialize terminal (raw mode, alternate screen)
5. Spawn async tasks to fetch widget data via Insights API
6. Event loop: render UI, poll keyboard, process API responses
7. Restore terminal on exit

### Dashboard Format

Dashboards are YAML files with widgets positioned on a 12-column grid:

```yaml
title: My Dashboard
widgets:
  - id: requests
    type: insights
    grid: { x: 0, y: 0, w: 6, h: 2 }
    presentation:
      title: Requests
    config:
      query: |
        SELECT count(*) as count, bin(ts, 1h) as time
        FROM requests
        GROUP BY time
      vis:
        view: line  # or: histogram, table, billboard
        chart_config:
          xField: time
          yField: count
```

### Key Dependencies

- **ratatui 0.29** - TUI framework
- **crossterm 0.28** - Terminal backend
- **clap 4** - CLI argument parsing
- **tokio 1** - Async runtime
- **reqwest 0.12** - HTTP client (30s timeout)
- **serde_yaml** - Dashboard YAML parsing

## Honeybadger API

Uses the Insights API for widget queries:

```
POST /v2/projects/{id}/insights/queries
Authorization: Basic base64(token:)
Content-Type: application/json

{"query": "SELECT ..."}
```

## Testing

```bash
cargo test  # 16 tests across honeybadger and layout modules
```

Tests cover:
- API response parsing (honeybadger)
- Client timeout configuration
- Widget navigation logic (layout)
- Grid calculations

## Commit Messages

Use [Conventional Commits](https://www.conventionalcommits.org/) format:

```
<type>: <subject>
```

Types: `feat`, `fix`, `docs`, `style`, `refactor`, `test`, `chore`
