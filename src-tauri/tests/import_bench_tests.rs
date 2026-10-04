//! Import and statistics benchmark over a fixed-seed generated corpus.
//!
//! `#[ignore]`d so the normal `cargo test` run stays fast. Run it in release:
//!
//! ```sh
//! node scripts/cargo-test-msvc.mjs --release --test import_bench_tests -- --ignored --nocapture
//! ```
//!
//! It generates the same corpus every time with `scripts/sim/generate.mjs`
//! (seed `velora-bench`: cash 6-max, Zoom, MTT 9-max knockout and Spin & Go,
//! 3,000 hands each) into a temporary folder, imports it into a temporary
//! database through the real directory import, then computes
//! `compute_player_stats` for every stored player. The numbers it prints are
//! the ones recorded under "Benchmarks" in `docs/specs/stats-audit-report.md`.
//! Nothing touches the app's own data folder.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use serde_json::Value;
use velora_poker_lib::db;
use velora_poker_lib::import::import_directory;
use velora_poker_lib::stats::compute_player_stats;

const SEED: &str = "velora-bench";

/// `(format, --tables, --hands)`: 3,000 hands per format.
const CORPUS: [(&str, u32, u32); 4] = [("cash", 6, 500), ("zoom", 1, 3000), ("mtt", 4, 750), ("spin", 4, 750)];

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().expect("repo root").to_path_buf()
}

fn generate(out: &Path, format: &str, tables: u32, hands: u32) -> Value {
    let output = Command::new("node")
        .current_dir(repo_root())
        .arg("scripts/sim/generate.mjs")
        .args(["--out", out.to_str().expect("utf-8 path")])
        .args(["--format", format, "--seed", SEED])
        .args(["--tables", &tables.to_string(), "--hands", &hands.to_string()])
        .args(["--pace", "0", "--manifest"])
        .output()
        .expect("run node scripts/sim/generate.mjs (is node on PATH?)");
    assert!(output.status.success(), "generator failed: {}", String::from_utf8_lossy(&output.stderr));
    serde_json::from_slice(&output.stdout).expect("manifest JSON")
}

#[test]
#[ignore = "benchmark: run in release with --ignored --nocapture"]
fn import_and_stats_benchmark() {
    let dir = tempfile::tempdir().expect("temp dir");
    let mut expected = 0i64;
    for (format, tables, hands) in CORPUS {
        let manifest = generate(&dir.path().join("hh").join(format), format, tables, hands);
        expected += manifest["hands"].as_i64().expect("manifest hand count");
    }

    let mut conn = db::open(&dir.path().join("velora.db")).expect("open temp db");

    let started = Instant::now();
    let summary = import_directory(&mut conn, &dir.path().join("hh")).expect("import corpus");
    let import_secs = started.elapsed().as_secs_f64();
    assert_eq!(summary.hands_imported, expected, "{summary:?}");

    let players: Vec<i64> = conn
        .prepare("SELECT id FROM players ORDER BY id")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    let started = Instant::now();
    for &player in &players {
        compute_player_stats(&conn, player).expect("compute stats");
    }
    let stats_secs = started.elapsed().as_secs_f64();

    println!("BENCH seed={SEED} hands={} players={}", summary.hands_imported, players.len());
    println!(
        "BENCH import_secs={import_secs:.3} hands_per_sec={:.0}",
        summary.hands_imported as f64 / import_secs
    );
    println!(
        "BENCH stats_secs={stats_secs:.3} ms_per_player={:.2}",
        stats_secs * 1000.0 / players.len().max(1) as f64
    );
}
