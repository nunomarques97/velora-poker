# Velora Poker

## Purpose

Velora Poker is a commercial, Windows-first poker HUD (Heads-Up Display)
application. It will provide players with a real-time table overlay showing
opponent statistics derived from hand history, along with a minimal in-game
HUD, a detailed player-profile overlay, an underlying statistics engine, user
accounts, subscription billing, and cloud synchronization of data across
devices.

This repository currently contains **only the project bootstrap** — no
application functionality has been implemented yet.

## Current Status

**Pre-development / Architecture & Product Validation**

The application shell (Tauri 2 + React + TypeScript) has been scaffolded.
No poker HUD functionality exists in this repository yet. The project is
not ready for contributions or use.

## Development Principles

- Make deliberate, documented architecture decisions before writing feature
  code — avoid technology choices made by default or by accident.
- Keep the dependency footprint minimal; add a dependency only when it earns
  its place.
- Favor a clean, professional project structure appropriate for a
  commercial desktop application from day one.
- Treat correctness and data handling (hand histories, statistics, user
  accounts) as first-class concerns given the commercial nature of the
  product.
- Document decisions as they are made, rather than after the fact.

## Architecture

**Technical foundation (chosen):**

- [Tauri 2](https://tauri.app/) — Rust-based desktop application shell
- [React 19](https://react.dev/) + [TypeScript](https://www.typescriptlang.org/) — frontend UI
- [Vite](https://vitejs.dev/) — frontend build tool
- Windows-first target platform

This is only the application shell. No overlay rendering, table detection,
hand history parsing, statistics engine, authentication, backend, or
payments have been implemented. Those remain open architecture decisions to
be made and documented here as the project progresses.

## Setup

**Prerequisites:**

- [Node.js](https://nodejs.org/) 20+ and npm
- [Rust](https://www.rust-lang.org/tools/install) (stable toolchain, MSVC)
- On Windows: the [Tauri prerequisites](https://tauri.app/start/prerequisites/)
  — Microsoft C++ Build Tools (Desktop development with C++ workload) and
  WebView2 Runtime (preinstalled on most modern Windows systems)

**Install dependencies:**

```sh
npm install
```

**Run the app in development mode:**

```sh
npm run tauri dev
```

**Build a production bundle:**

```sh
npm run tauri build
```

**Frontend-only commands** (useful for quick UI iteration without the Rust
shell):

```sh
npm run dev       # start the Vite dev server
npm run build      # type-check and build the frontend
npm run preview    # preview the built frontend
```

## Project Structure

```
velora-poker/
├── src/            # React + TypeScript frontend
├── src-tauri/      # Rust backend / Tauri application shell
├── tests/          # test suites (placeholder)
└── assets/         # non-code project assets (placeholder)
```
