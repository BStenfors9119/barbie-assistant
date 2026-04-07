# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Commands

```bash
cargo build          # compile
cargo run            # run
cargo test           # run all tests
cargo test <name>    # run a single test by name
cargo clippy         # lint
cargo fmt            # format
```

## Project

A desktop application for generating SQL queries compatible with SAP Travel Management (STM) data. Users select templates, fill in parameters, and copy the resulting SQL.
Make the application have a SQL query generator wysiwig control for end users that are not as technical. The application should be cross-platform (Windows, macOS, Linux) and have a clean, modern UI.

## Tech Stack

- **Rust** — all application logic and UI
- **Iced** — desktop GUI framework (Elm-like: State + Message + update + view)
- **serde / serde_json** — serialization
- **sqlx** (optional) — SQL handling if direct DB connections are added
- **Axum** (optional) — backend API if needed

## Architecture

Iced follows an Elm architecture: `App` holds state, `Message` describes events, `update()` mutates state, `view()` renders the UI. No HTML/JS — the entire UI is Rust.

Module layout under `src/`:

- `main.rs` — wires Iced application entry point
- `app.rs` — `App` struct, `Message` enum, `update()`, `view()`
- `commands/mod.rs` — pure functions for query generation; called from `update()`
- `templates/mod.rs` — SQL template definitions and placeholder rendering
- `utils/mod.rs` — shared helpers
- Domain objects each get their own folder with a `mod.rs` (e.g. `src/travel_request/mod.rs`)

## UI

- Scandinavian palette (blues, greens, grays) with dark/light theme toggle
- Configurable font size
- Clipboard copy for generated SQL
- Saved queries management within the app

## Distribution

Cross-compile to Windows, macOS, and Linux via Iced's built-in platform support.
