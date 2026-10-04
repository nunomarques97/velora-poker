# Simulator (dev-only)

Test tools for Velora. None of this is referenced by the app or shipped in its bundle. It never touches the real PokerStars client, and it never touches your real Velora database or settings.

| File | What it does |
|---|---|
| `generate.mjs` | Writes PokerStars-format hand histories for simulated villains (`profiles.json`) into a folder, at session pace |
| `fake-tables.ps1` | Fake PokerStars table windows (class `GLFW30`, PokerStars-like titles), with a mock felt and a left-click counter |
| `fake-tables.mjs` | Node side of the fake tables: starts the window program and sends it commands |
| `e2e.mjs` | End-to-end driver: runs the real app against the fake tables and the generator |
| `profiles.json` | Villain profiles with target stats and expected reads. The generator, the Rust convergence tests and the e2e expectations all use it |

Tests: `npm run test:sim` (Node), plus `sim_convergence_tests`, `sim_formats_tests` and `sim_titles_tests` in `src-tauri/tests` (`node scripts/cargo-test-msvc.mjs --test sim_titles_tests`).

## Hand-history generator

```sh
node scripts/sim/generate.mjs --out <folder> --format cash|zoom|mtt|spin --tables 4 --hands 200 --pace 2 --clock live
```

- `--hands` counts hands per cash table, in total for Zoom, or per tournament seat for `mtt` (9-max progressive knockout) and `spin` (3-max).
- `--pace` sets real seconds per hand at one table. `0` writes everything at once.
- `--clock live` stamps each hand with the wall clock. The HUD only treats a hand as "live" at a table if it was played after that table window opened.
- `--manifest` prints the tables, seating and tournaments as JSON.

The generator refuses to write into a PokerStars folder, the Velora data folder, a drive root, or a folder holding another player's hand histories.

## Fake tables

Windows PowerShell 5.1 with inline C# (`Add-Type`). Nothing to install.

```sh
node scripts/sim/fake-tables.mjs --probe                 # compile only, print the monitors; opens no window
node scripts/sim/fake-tables.mjs --titles <hh folder>    # the window title each table in the folder would get
```

Each window paints a felt with seat plates, hole cards, the board and Fold/Call/Raise buttons. They sit where the HUD's seat layout (`src/overlay/seatLayout.ts`) expects them. Every left click is counted per window in a JSON status file, which is how "the table stays clickable outside HUD elements" is measured. The program reads one JSON command per line on stdin: `open`, `move`, `retitle`, `close`, `closeAll`, `shot` (screen region to PNG, including the transparent overlay), `mouse`, `click`, `keys`, `show`, `list`, `status` and `quit`. See the header of `fake-tables.ps1`.

Titles follow the real client's shape, which `table_track::extract_table_name` parses:

- cash: `Session: 00:00 - Lesath II - No Limit Hold'em $0.25/$0.50 USD - Logged In as SimHero`
- MTT: `Session: 00:00 - Progressive KO $11.00 [9-Max] - 75/150 - Tournament 4100000001 Table 7 - Logged In as SimHero`

The Zoom and Spin & Go titles are assumed, not confirmed. `sim_titles_tests` checks every title against the app's own title rules and the importer's table names.

## End-to-end driver

**Always do a dry run first.** It checks the environment and prints the plan, without opening any window or launching the app:

```sh
node scripts/sim/e2e.mjs --dry-run
node scripts/sim/e2e.mjs --dry-run --features strategic-analysis
```

The dry run checks that vcvars, the MSVC compiler, cargo, npm and the Tauri CLI are present, and that the fake tables compile. It also checks the path guards. Then it prints the paths, the environment overrides, the eight table slots with their titles, and the command lines.

A real run **opens windows, moves the mouse and clicks**. Leave the computer alone while it runs:

```sh
node scripts/sim/e2e.mjs --features strategic-analysis --shots docs/specs/e2e
```

What it does:

1. Creates a temporary root (or uses `--root`, if that folder is empty or was created by the driver).
2. Seeds a database there through the app's own migrations (`cargo run --example sim_seed`): onboarding complete, PokerStars, hand-history folder `<root>/HandHistory`.
3. Opens the fake tables. By default these are 4 cash 6-max and 4 MTT 9-max, each kind at 483x359 and 800x570. Cash windows open before their first hand. An MTT window opens when its tournament file appears and closes when the hero's tournament ends.
4. Launches `npm run tauri dev` under the VS2022 BuildTools environment. It sets `APPDATA`, `LOCALAPPDATA`, `WEBVIEW2_USER_DATA_FOLDER` and `TEMP`/`TMP` inside the root, then minimizes the main window (`--keep-main` leaves it up).
5. Starts the generators on the live clock.
6. Every `--shot-every` hands, it takes a desktop screenshot and one per table. It hovers a villain chip and screenshots the hover, then clicks the felt outside HUD elements and reads that table's click counter. It also lists the overlay windows. Once per run, it toggles the side panel with Ctrl+Alt+P.
7. Writes `<root>/e2e-summary.json` (events, rounds, clicks, overlay windows). Then it closes every fake window, the generators and the app. The same cleanup runs on an error or Ctrl+C.

Options: `--cash n`, `--mtt n`, `--hands n` (per table or tournament seat, default 60), `--backlog n` (cash hands imported before launch), `--pace s`, `--seed s`, `--shot-every n`, `--launch-timeout s`, `--root dir`, `--shots dir`, `--observe`, `--inspect port`.

**Locked session.** A real run refuses to start while Windows is locked: the lock screen covers every window and takes the mouse and keyboard. The dry run warns about it. `--observe` runs the session without mouse, keyboard or screen captures and only lists the windows.

**`--inspect port`** reads what the real app shows without using the screen, so it also works on a locked session. The app's own temporary environment gets `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS` with a DevTools port on `127.0.0.1` (never set for your normal app). Every round, `lib/cdp.mjs` does the following:

- Reads each overlay's chips from its DOM: text and accessible name, which includes the tag, the top read and its sample.
- Renders each table window with its HUD on top. The fake table draws itself (`print`, PrintWindow) and the overlay page captures itself.
- Hovers a tagged villain's chip and checks that the hover card closes when the pointer leaves.
- Once per run, opens that player's drawer.
- From the second round, shows the side panel (the same `show_side_panel` command as HUD Profiles → Open side panel), reads it and searches a seated villain.

The pointer events go to the page, not to the desktop, so this does not prove that the OS hands clicks through to the table. Only an unlocked run checks that, with its click counter.

```sh
node scripts/sim/e2e.mjs --observe --inspect 9333 --features strategic-analysis --shots <folder>
```

**Reads vs profiles after a run.** `src-tauri/examples/sim_reads.rs` opens a run's database read-only. For each simulated villain and each format he played, it prints one JSON line: his chip tag and his ranked reads at his latest hand of that format, the profile's expected reads, and whether the first tendency read is one of them:

```sh
cd src-tauri
cargo run --example sim_reads --features strategic-analysis -- <root>/AppData/Roaming/com.velora.poker/velora.db ../scripts/sim/profiles.json
```

Run it under the VS2022 environment, like the tests (see `lib/msvc.mjs`).

Guards (`lib/e2e-guard.mjs`, tested in `test/e2e-guard.test.mjs`):

- The driver refuses to run if the root, the database, the data folders or the hand-history folder resolve to the real Velora folder (`%APPDATA%\com.velora.poker`) or to a PokerStars folder. That covers paths read from your real profile and from the environment, and paths reached through junctions.
- Every data path must be inside the root.
- Every file the driver writes is checked against the root and the screenshot folder.
- The seeder refuses an existing database.
- Cargo's build output (`src-tauri/target`) is the normal dev build, as with any `npm run tauri dev`.

Chip hover points come from the seat layout. They are estimates: check them against the screenshots of the first run.
