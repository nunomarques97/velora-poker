# Velora Poker

Velora Poker is a Windows-first desktop poker HUD (Heads-Up Display), built with Tauri 2 (Rust) + React 19/TypeScript. It watches a PokerStars hand-history folder, parses hands (cash, tournament, Zoom) into a local SQLite database, computes per-opponent statistics (VPIP, PFR, 3-bet, fold-to-3-bet, C-bet, fold-to-C-bet, AF, WTSD, W$SD), classifies opponents into archetypes (TAG / LAG / Maniac / Loose-Passive / Recreational, with manual override), and displays everything on a real always-on-top transparent overlay with draggable per-player HUD cards.

Everything runs locally. Accounts, billing and cloud sync are part of the long-term vision but are intentionally out of scope for the current MVP.

## Current status


## Documentation


## For testers (no developer tools needed)

Velora ships as a normal Windows installer. Testers do **not** need Node, Rust, or Visual Studio.

1. Run `Velora Poker_<version>_x64-setup.exe`.
2. It installs for the current user only, so Windows will not ask for administrator rights. Velora lands in `%LOCALAPPDATA%\Velora Poker` with a "Velora Poker" Start Menu shortcut.
3. Launch it from the Start Menu. On first run, Velora asks for your poker room, then auto-detects your PokerStars hand-history folder (including regional installs such as `PokerStars.PT`), then lets you pick a HUD.
4. Uninstall from Windows Settings → Installed apps → Velora Poker.

Notes:

- Windows SmartScreen will show "Windows protected your PC" because the installer is not code-signed. Choose **More info → Run anyway**. This is expected for now.
- Velora needs the Microsoft Edge WebView2 Runtime, which is already present on Windows 10/11. If it is missing, the installer downloads and installs it silently, so the machine needs an internet connection during installation only.
- Everything stays on the machine. Hands are parsed into `%APPDATA%\com.velora.poker\velora.db`; nothing is uploaded.
- If something looks wrong during play, open Settings → Diagnostics → **Copy Diagnostics** and paste the result into your bug report.

**Build the installer:**

```sh
npm run tauri build
```

The installer is written to `src-tauri/target/release/bundle/nsis/`.


## Setup (developers)

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
