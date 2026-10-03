//! The simulator's fake PokerStars tables (`scripts/sim/fake-tables.ps1`,
//! driven by `scripts/sim/e2e.mjs`) are found by the real table detection:
//! every window title they would use passes `table_track`'s title check and
//! parses to exactly the table name the importer stores for that table's
//! hands, for every generated format (cash 6-max, Zoom, MTT 9-max with
//! bounties, Spin & Go), and the window class they register is one
//! `table_track` enumerates.
//!
//! The generator and the title listing run through `node` into a temporary
//! folder; the database is in memory. Nothing touches the app's data folder
//! and no window is opened.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;

use rusqlite::Connection;
use serde_json::Value;
use velora_poker_lib::db;
use velora_poker_lib::import::import_directory;
use velora_poker_lib::table_track::{extract_table_name, is_table_title, POKERSTARS_TABLE_CLASS_CANDIDATES};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().expect("repo root").to_path_buf()
}

fn node(args: &[&str]) -> Vec<u8> {
    let output = Command::new("node")
        .current_dir(repo_root())
        .args(args)
        .output()
        .expect("run node (is node on PATH?)");
    assert!(output.status.success(), "node {args:?} failed: {}", String::from_utf8_lossy(&output.stderr));
    output.stdout
}

/// Generates a session into `out` and returns its manifest.
fn generate(out: &Path, format: &str, tables: u32, hands: u32) -> Value {
    let stdout = node(&[
        "scripts/sim/generate.mjs",
        "--out",
        out.to_str().expect("utf-8 path"),
        "--format",
        format,
        "--seed",
        "sim-titles",
        "--tables",
        &tables.to_string(),
        "--hands",
        &hands.to_string(),
        "--pace",
        "0",
        "--manifest",
    ]);
    serde_json::from_slice(&stdout).expect("manifest JSON")
}

/// The fake tables' windows for a folder: `[{ file, table, kind, maxSeats, title }]`.
fn titles(dir: &Path) -> Vec<Value> {
    let stdout = node(&["scripts/sim/fake-tables.mjs", "--titles", dir.to_str().expect("utf-8 path")]);
    let parsed: Value = serde_json::from_slice(&stdout).expect("titles JSON");
    parsed.as_array().expect("an array").clone()
}

fn stored_table_names(conn: &Connection) -> BTreeSet<String> {
    conn.prepare("SELECT DISTINCT table_name FROM hands")
        .unwrap()
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

fn str_of<'a>(value: &'a Value, key: &str) -> &'a str {
    value[key].as_str().unwrap_or_else(|| panic!("{key} missing in {value}"))
}

#[test]
fn every_fake_table_title_is_detected_and_names_the_imported_table() {
    for (format, tables, hands, max_seats) in [("cash", 4, 12, 6), ("zoom", 1, 30, 6), ("mtt", 3, 80, 9), ("spin", 3, 60, 3)] {
        let dir = tempfile::tempdir().expect("temp dir");
        let manifest = generate(dir.path(), format, tables, hands);
        let windows = titles(dir.path());
        assert_eq!(
            windows.len(),
            manifest["files"].as_array().expect("files").len(),
            "{format}: one fake table per hand-history file"
        );

        let mut conn = db::open(Path::new(":memory:")).expect("in-memory db");
        let summary = import_directory(&mut conn, dir.path()).expect("import");
        assert!(summary.hands_imported > 0, "{format}: hands imported");
        let stored = stored_table_names(&conn);

        let mut parsed = BTreeSet::new();
        for window in &windows {
            let title = str_of(window, "title");
            let table = str_of(window, "table");
            assert!(is_table_title(title), "{format}: table_track must accept {title:?}");
            let name = extract_table_name(title);
            assert_eq!(name.as_deref(), Some(table), "{format}: {title:?} must parse to {table:?}");
            assert_eq!(str_of(window, "kind"), format);
            assert_eq!(window["maxSeats"].as_u64(), Some(max_seats));
            parsed.insert(name.unwrap());
        }
        assert_eq!(parsed, stored, "{format}: titles name exactly the tables the importer stored");
    }
}

#[test]
fn the_fake_window_class_is_one_table_track_enumerates() {
    let script = std::fs::read_to_string(repo_root().join("scripts/sim/fake-tables.ps1")).expect("read fake-tables.ps1");
    let marker = "public const string ClassName = \"";
    let start = script.find(marker).expect("ClassName constant") + marker.len();
    let class = &script[start..start + script[start..].find('"').expect("closing quote")];
    assert!(
        POKERSTARS_TABLE_CLASS_CANDIDATES.iter().any(|c| c.eq_ignore_ascii_case(class)),
        "fake tables register {class:?}, table_track looks for {POKERSTARS_TABLE_CLASS_CANDIDATES:?}"
    );
}
