# Velora Poker

Velora Poker is a Windows-first desktop poker HUD (Heads-Up Display), built with Tauri 2 (Rust) + React 19/TypeScript. It watches a PokerStars hand-history folder, parses hands (cash, tournament, Zoom) into a local SQLite database, computes per-opponent statistics (VPIP, PFR, 3-bet, fold-to-3-bet, C-bet, fold-to-C-bet, AF, WTSD, W$SD), classifies opponents into archetypes (TAG / LAG / Maniac / Loose-Passive / Recreational, with manual override), and displays everything on a real always-on-top transparent overlay with draggable per-player HUD cards.

Everything runs locally. Accounts, billing and cloud sync are part of the long-term vision but are intentionally out of scope for the current MVP.

## Current status


## Documentation


## Setup

**Prerequisites:**

- [Node.js](https://nodejs.org/) 20+ and npm
- [Rust](https://www.rust-lang.org/tools/install) (stable toolchain, MSVC)
- On Windows: the [Tauri prerequisites](https://tauri.app/start/prerequisites/) — Microsoft C++ Build Tools (Desktop development with C++ workload) and WebView2 Runtime (preinstalled on most modern Windows systems)

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

**Frontend-only commands** (quick UI iteration without the Rust shell):

```sh
npm run dev        # start the Vite dev server
npm run build      # type-check and build the frontend
npm run preview    # preview the built frontend
```

**Rust tests:**

```sh
cd src-tauri && cargo test
```

## Project structure

```
velora-poker/
├── src/            # React + TypeScript frontend (views, hud/, overlay/, onboarding/)
├── src-tauri/      # Rust backend: parser, import, watcher, db, stats, classification, hud, overlay
│   └── tests/      # Rust integration tests + fixtures
├── overlay.html    # entry point for the transparent overlay window
└── assets/         # non-code project assets
```
