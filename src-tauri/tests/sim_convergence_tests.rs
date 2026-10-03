//! The hand-history simulator (`scripts/sim`) against the real import
//! pipeline: generated cash and Zoom sessions must parse with zero errors,
//! re-import as pure duplicates, and converge to each villain profile's
//! target stats (`scripts/sim/profiles.json`). In the `strategic-analysis`
//! build each villain's top read and chip tag must be the profile's
//! expected one, and the tilting player must show `TILT` inside his tilt
//! episodes and (all but rarely) not before them.
//!
//! The generator runs through `node` into a temporary folder; the database
//! is a temporary file. Nothing touches the app's own data folder.

use std::path::{Path, PathBuf};
use std::process::Command;

use rusqlite::Connection;
use serde_json::Value;
use velora_poker_lib::db;
use velora_poker_lib::import::{import_directory, import_text};
use velora_poker_lib::parser::{split_hands, HandHistoryParser, PokerStarsParser};
use velora_poker_lib::stats::{compute_player_stats_with_opportunities, PlayerStats, PlayerStatsOpportunities};

/// Hands per cash table and cash tables: every villain sits at two or
/// three tables, so each one plays about 3,000 hands.
const CASH_TABLES: u32 = 6;
const CASH_HANDS_PER_TABLE: u32 = 1200;
const ZOOM_HANDS: u32 = 3000;
/// A stat is held to its tolerance only with at least this many
/// opportunities; below it the sampling error alone exceeds the tolerance.
const MIN_OPPORTUNITIES: i64 = 40;

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().expect("repo root").to_path_buf()
}

fn profiles() -> Value {
    let text = std::fs::read_to_string(repo_root().join("scripts/sim/profiles.json")).expect("read profiles.json");
    serde_json::from_str(&text).expect("profiles.json is JSON")
}

/// Runs the generator into `out` and returns its manifest.
fn generate(out: &Path, format: &str, seed: &str, tables: u32, hands: u32) -> Value {
    let output = Command::new("node")
        .current_dir(repo_root())
        .arg("scripts/sim/generate.mjs")
        .args(["--out", out.to_str().expect("utf-8 path")])
        .args(["--format", format, "--seed", seed])
        .args(["--tables", &tables.to_string(), "--hands", &hands.to_string()])
        .args(["--pace", "0", "--manifest"])
        .output()
        .expect("run node scripts/sim/generate.mjs (is node on PATH?)");
    assert!(
        output.status.success(),
        "generator failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("manifest JSON")
}

fn open_db(dir: &Path) -> Connection {
    let conn = db::open(&dir.join("velora.db")).expect("open temp db");
    // Thousands of one-hand transactions: durability is not under test.
    conn.execute_batch("PRAGMA synchronous = OFF;").expect("pragma");
    conn
}

fn player_id(conn: &Connection, name: &str) -> i64 {
    conn.query_row("SELECT id FROM players WHERE name = ?1", [name], |row| row.get(0))
        .unwrap_or_else(|_| panic!("player {name} not imported"))
}

fn hand_count(conn: &Connection) -> i64 {
    conn.query_row("SELECT COUNT(*) FROM hands", [], |row| row.get(0)).expect("count hands")
}

/// Every generated file parses hand by hand with zero errors, and holds
/// the hands the manifest says it does.
fn assert_files_parse(out: &Path, manifest: &Value) -> Vec<(u64, String)> {
    let parser = PokerStarsParser;
    let mut blocks = Vec::new();
    let mut total = 0;
    for table in manifest["tables"].as_array().expect("tables") {
        let file = table["file"].as_str().expect("file name");
        let text = std::fs::read_to_string(out.join(file)).expect("read generated file");
        let parsed = parser.parse(&text);
        let errors: Vec<String> = parsed.iter().filter_map(|r| r.as_ref().err()).map(|e| format!("{e:?}")).collect();
        assert!(errors.is_empty(), "{file}: {} parse error(s): {:?}", errors.len(), &errors[..errors.len().min(3)]);
        assert_eq!(parsed.len() as u64, table["hands"].as_u64().unwrap(), "{file}: hand count");
        for hand in parsed.into_iter().map(Result::unwrap) {
            assert!(hand.hero_name.as_deref() == manifest["hero"].as_str(), "{file}: hero is the fixed screen name");
            assert!(hand.seats.iter().all(|s| s.position.is_some()), "{file}: every seat has a position");
        }
        for block in split_hands(&text) {
            let id: u64 = block["PokerStars ".len()..]
                .split('#')
                .nth(1)
                .and_then(|rest| rest.split(':').next())
                .and_then(|id| id.parse().ok())
                .expect("hand id in header");
            blocks.push((id, block));
        }
        total += table["hands"].as_u64().unwrap();
    }
    assert_eq!(total, manifest["hands"].as_u64().unwrap());
    blocks.sort_by_key(|(id, _)| *id);
    blocks
}

/// The whole folder through the real directory import, then again: the
/// second pass adds nothing.
fn assert_directory_import(conn: &mut Connection, out: &Path, expected_hands: u64, already: i64) {
    let first = import_directory(conn, out).expect("import directory");
    assert_eq!(first.hands_failed, 0, "parse failures");
    assert_eq!(first.hands_rejected_invalid, 0, "rejected by the integrity gate");
    assert_eq!(first.hands_with_warnings, 0, "hands with warnings");
    assert_eq!(first.hands_imported + already, expected_hands as i64);
    assert_eq!(first.hands_skipped_duplicate, already);
    assert_eq!(hand_count(conn), expected_hands as i64);

    let again = import_directory(conn, out).expect("re-import directory");
    assert_eq!(again.hands_imported, 0, "a duplicate re-import adds nothing");
    assert_eq!(again.hands_skipped_duplicate, expected_hands as i64);
    assert_eq!(hand_count(conn), expected_hands as i64);
}

fn stat_of(stats: &PlayerStats, opps: &PlayerStatsOpportunities, key: &str) -> (Option<f64>, i64) {
    match key {
        "vpip" => (stats.vpip, opps.hands),
        "pfr" => (stats.pfr, opps.hands),
        "threeBet" => (stats.three_bet, opps.three_bet_opportunities),
        "foldToThreeBet" => (stats.fold_to_three_bet, opps.faced_3bet_opportunities),
        "cBet" => (stats.c_bet, opps.cbet_opportunities),
        "foldToCBet" => (stats.fold_to_c_bet, opps.faced_cbet_opportunities),
        "wtsd" => (stats.wtsd, opps.saw_flop_hands),
        other => panic!("profiles.json names an unknown stat {other}"),
    }
}

/// The stated tolerance, widened to three standard errors of a rate of
/// `target` over `n` opportunities when the sample is too small for it.
fn allowed_error(tol: f64, target: f64, n: i64) -> f64 {
    let p = (target / 100.0).clamp(0.01, 0.99);
    tol.max(300.0 * (p * (1.0 - p) / n as f64).sqrt())
}

/// Every villain's imported stats within the profile tolerances. Returns
/// the failures (asserted by the caller) after printing the full table.
fn stat_failures(conn: &Connection, label: &str) -> Vec<String> {
    let mut failures = Vec::new();
    for profile in profiles()["profiles"].as_array().unwrap() {
        let id = profile["id"].as_str().unwrap();
        for name in profile["players"].as_array().unwrap() {
            let name = name.as_str().unwrap();
            let pid = player_id(conn, name);
            let (stats, opps) = compute_player_stats_with_opportunities(conn, pid).expect("stats");
            let mut row = format!("{label} {id:>6} {name:<16} hands {:>5}", opps.hands);
            for (key, spec) in profile["targets"].as_object().unwrap() {
                let target = spec["target"].as_f64().unwrap();
                let tol = spec["tol"].as_f64().unwrap();
                let (value, n) = stat_of(&stats, &opps, key);
                row += &format!(" | {key} {} ({n}) ~{target}", value.map_or("-".into(), |v| format!("{v:.1}")));
                if n < MIN_OPPORTUNITIES {
                    continue;
                }
                let value = value.expect("a stat with opportunities has a value");
                let allowed = allowed_error(tol, target, n);
                if (value - target).abs() > allowed {
                    failures.push(format!("{label} {name} ({id}) {key} = {value:.1} over {n}, target {target} ± {allowed:.1}"));
                }
            }
            eprintln!("{row}");
        }
    }
    failures
}

#[test]
fn generated_cash_session_imports_cleanly_and_converges_to_the_profiles() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let out = tmp.path().join("HandHistory");
    let manifest = generate(&out, "cash", "convergence", CASH_TABLES, CASH_HANDS_PER_TABLE);
    assert_eq!(manifest["hands"].as_u64(), Some((CASH_TABLES * CASH_HANDS_PER_TABLE) as u64));
    let blocks = assert_files_parse(&out, &manifest);
    let mut conn = open_db(tmp.path());

    // The tilting player, checked while the hands arrive (strategic-analysis
    // build): the real import, hand by hand in play order, up to each check.
    let mut imported = 0usize;
    let checks = tilt_checkpoints(&manifest);
    let mut tilt_results = Vec::new();
    for (hand_id, player, expect_tilt) in &checks {
        let upto = blocks.iter().position(|(id, _)| id > hand_id).unwrap_or(blocks.len());
        if upto > imported {
            let text: String = blocks[imported..upto].iter().map(|(_, b)| format!("{b}\n\n\n")).collect();
            let summary = import_text(&mut conn, &text).expect("import prefix");
            assert_eq!(summary.hands_failed + summary.hands_rejected_invalid, 0);
            imported = upto;
        }
        tilt_results.push((hand_id, player.clone(), *expect_tilt, engine_tag(&conn, player)));
    }

    assert_directory_import(&mut conn, &out, manifest["hands"].as_u64().unwrap(), imported as i64);

    let failures = stat_failures(&conn, "cash");
    if cfg!(feature = "strategic-analysis") {
        assert_tilt_checks(&tilt_results);
        assert_top_reads(&conn);
    }
    assert!(failures.is_empty(), "stats off target:\n{}", failures.join("\n"));
}

#[test]
fn generated_zoom_session_imports_cleanly_and_converges_to_the_profiles() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let out = tmp.path().join("HandHistory");
    let manifest = generate(&out, "zoom", "convergence", 1, ZOOM_HANDS);
    assert_eq!(manifest["files"].as_array().map(Vec::len), Some(1), "one Zoom pool file");
    assert_files_parse(&out, &manifest);
    let mut conn = open_db(tmp.path());
    assert_directory_import(&mut conn, &out, ZOOM_HANDS as u64, 0);
    let variants: Vec<String> = conn
        .prepare("SELECT DISTINCT variant FROM hands")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(variants, ["zoom_cash"]);
    let failures = stat_failures(&conn, "zoom");
    assert!(failures.is_empty(), "stats off target:\n{}", failures.join("\n"));
}

#[test]
fn the_same_seed_writes_the_same_hands() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let a = generate(&tmp.path().join("a"), "cash", "same-seed", 2, 40);
    let b = generate(&tmp.path().join("b"), "cash", "same-seed", 2, 40);
    for (ta, tb) in a["tables"].as_array().unwrap().iter().zip(b["tables"].as_array().unwrap()) {
        let file = ta["file"].as_str().unwrap();
        assert_eq!(file, tb["file"].as_str().unwrap());
        let read = |dir: &str| std::fs::read(tmp.path().join(dir).join(file)).unwrap();
        assert_eq!(read("a"), read("b"), "{file} differs between two runs of one seed");
    }
}

// ------------------------------------------------------------ engine reads

/// `(hand id, player, expect TILT)`: for each tilt episode with a baseline
/// of at least 60 hands, the big-loss hand itself (his last 12 hands were
/// calm: no TILT) and his 12th hand on tilt (TILT), in play order.
fn tilt_checkpoints(manifest: &Value) -> Vec<(u64, String, bool)> {
    let mut checks = Vec::new();
    for episode in manifest["tiltEpisodes"].as_array().expect("tiltEpisodes") {
        let ids = episode["handIds"].as_array().unwrap();
        if episode["handsBefore"].as_u64().unwrap() < 60 || ids.len() < 12 {
            continue;
        }
        let player = episode["player"].as_str().unwrap().to_string();
        let id = |v: &Value| v.as_str().unwrap().parse::<u64>().unwrap();
        checks.push((id(&episode["bigLossHandId"]), player.clone(), false));
        checks.push((id(&ids[11]), player, true));
    }
    checks.sort();
    checks
}

/// What the overlay would show for a player, adapted to the latest hand at
/// his latest table: the chip tag, and his top read leaving out context
/// facts (`ctx.*`, such as `DEEP`), as `(tag, rule id)`.
#[derive(Debug, Clone, PartialEq)]
struct Seen {
    chip: Option<String>,
    read: Option<(Option<String>, String)>,
}

#[cfg(feature = "strategic-analysis")]
fn engine_tag(conn: &Connection, name: &str) -> Option<Seen> {
    use velora_poker_lib::engine::{latest_table_hand, villain_context, EngineCache};
    let pid = player_id(conn, name);
    let table: String = conn
        .query_row(
            "SELECT h.table_name FROM hands h JOIN player_hands ph ON ph.hand_id = h.id
             WHERE ph.player_id = ?1 ORDER BY h.played_at DESC, h.id DESC LIMIT 1",
            [pid],
            |row| row.get(0),
        )
        .expect("latest table");
    let hand = latest_table_hand(conn, &table).expect("latest hand").expect("a hand");
    let context = villain_context(&hand, pid);
    let payload = EngineCache::new().payload(conn, pid, context).expect("engine payload");
    let top: Vec<String> = payload.reads.iter().take(4).map(|r| format!("{} {:.2}", r.rule_id, r.score)).collect();
    eprintln!("{name}: tag {:?}, reads {top:?}", payload.tag.as_ref().map(|t| &t.text));
    let read = payload
        .reads
        .iter()
        .find(|r| !r.rule_id.starts_with("ctx."))
        .map(|r| (r.tag.clone(), r.rule_id.clone()));
    Some(Seen { chip: payload.tag.map(|t| t.text), read })
}

#[cfg(not(feature = "strategic-analysis"))]
fn engine_tag(_conn: &Connection, _name: &str) -> Option<Seen> {
    None
}

#[allow(dead_code)]
fn assert_tilt_checks(results: &[(&u64, String, bool, Option<Seen>)]) {
    let profiles = profiles();
    let tilter = profiles["profiles"].as_array().unwrap().iter().find(|p| p["id"] == "tilter").expect("tilter profile");
    let expected = tilter["expected"]["reads"][0]["tag"].as_str().unwrap();
    let on_tilt: Vec<_> = results.iter().filter(|r| r.2).collect();
    let calm: Vec<_> = results.iter().filter(|r| !r.2).collect();
    assert!(on_tilt.len() >= 3, "too few tilt episodes to check: {}", on_tilt.len());
    let hits = on_tilt.iter().filter(|r| r.3.as_ref().is_some_and(|seen| seen.chip.as_deref() == Some(expected))).count();
    let false_alarms: Vec<_> = calm.iter().filter(|r| r.3.as_ref().is_some_and(|seen| seen.chip.as_deref() == Some(expected))).collect();
    eprintln!("tilt: {hits}/{} episodes tagged {expected}; {} false alarm(s) before them", on_tilt.len(), false_alarms.len());
    // The engine's test is statistical (z >= 2.5 over 12 hands): most, not
    // necessarily every, episode clears it. The other way round, a calm
    // 12-hand window that happens to run hot on the big-loss hand (which
    // is itself a played hand) flags him about once in 30: at most one
    // check in ten may.
    assert!(hits * 5 >= on_tilt.len() * 4, "TILT in only {hits} of {} tilt episodes", on_tilt.len());
    assert!(false_alarms.len() * 10 <= calm.len(), "TILT before the big loss too often: {false_alarms:?}");
}

/// Each villain's top read (context facts aside) is one of his profile's
/// expected reads, with its chip tag.
#[allow(dead_code)]
fn assert_top_reads(conn: &Connection) {
    let mut wrong = Vec::new();
    for profile in profiles()["profiles"].as_array().unwrap() {
        let expected = &profile["expected"];
        if expected["during"].is_string() {
            continue; // Checked during his episodes.
        }
        let reads: Vec<(Option<String>, String)> = expected["reads"]
            .as_array()
            .expect("expected.reads")
            .iter()
            .map(|r| (r["tag"].as_str().map(String::from), r["ruleId"].as_str().unwrap().to_string()))
            .collect();
        for name in profile["players"].as_array().unwrap() {
            let name = name.as_str().unwrap();
            let seen = engine_tag(conn, name).expect("engine payload");
            if !seen.read.as_ref().is_some_and(|read| reads.contains(read)) {
                wrong.push(format!("{name}: expected one of {reads:?}, got {:?}", seen.read));
            }
        }
    }
    assert!(wrong.is_empty(), "top reads off profile:\n{}", wrong.join("\n"));
}
