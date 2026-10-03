//! The hand-history simulator's tournament formats (`scripts/sim`, formats
//! `mtt` and `spin`) against the real import pipeline: every generated file
//! parses with zero errors, imports with no warning, and is stored the way
//! the real fixtures are (`real_bounty_tournament.txt`, `spin_three_max.txt`
//! and the `tournament_*` fixtures): format, variant, buy-in, level and seat
//! bounties. The short-stack profile lands in the engine's push/fold context
//! bucket and, in the `strategic-analysis` build, gets only push/fold reads.
//!
//! The generator runs through `node` into a temporary folder; the database
//! is a temporary file. Nothing touches the app's own data folder.

use std::path::{Path, PathBuf};
use std::process::Command;

use rusqlite::Connection;
use serde_json::Value;
use velora_poker_lib::db;
use velora_poker_lib::engine::context::initial_bounty;
use velora_poker_lib::engine::{load_hand, villain_context, EngineContext, FormatKey, StackBucket};
use velora_poker_lib::import::{import_directory, import_text};
use velora_poker_lib::parser::{GameVariant, HandFormat, HandHistoryParser, PokerStarsParser};

const MTT_TABLES: u32 = 4;
const MTT_HANDS: u32 = 400;
const SPIN_TABLES: u32 = 3;
const SPIN_HANDS: u32 = 300;

const REAL_BOUNTY: &str = include_str!("fixtures/real_bounty_tournament.txt");
const REAL_SPIN: &str = include_str!("fixtures/spin_three_max.txt");
const REAL_MTT: &str = include_str!("fixtures/tournament_showdown_allin.txt");

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
    assert!(output.status.success(), "generator failed: {}", String::from_utf8_lossy(&output.stderr));
    serde_json::from_slice(&output.stdout).expect("manifest JSON")
}

fn open_db(dir: &Path) -> Connection {
    let conn = db::open(&dir.join("velora.db")).expect("open temp db");
    conn.execute_batch("PRAGMA synchronous = OFF;").expect("pragma");
    conn
}

fn memory_db() -> Connection {
    db::open(Path::new(":memory:")).expect("open in-memory db")
}

fn player_id(conn: &Connection, name: &str) -> i64 {
    conn.query_row("SELECT id FROM players WHERE name = ?1", [name], |row| row.get(0))
        .unwrap_or_else(|_| panic!("player {name} not imported"))
}

fn strings(conn: &Connection, sql: &str) -> Vec<String> {
    conn.prepare(sql)
        .unwrap()
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

fn count(conn: &Connection, sql: &str) -> i64 {
    conn.query_row(sql, [], |row| row.get(0)).unwrap()
}

/// Every generated file parses hand by hand with zero errors: tournament
/// format and variant, the manifest's tournament and table, the fixed hero,
/// a position for every seat.
fn assert_files_parse(out: &Path, manifest: &Value, variant: GameVariant, max_seats: i64) {
    let parser = PokerStarsParser;
    let mut total = 0;
    for table in manifest["tables"].as_array().expect("tables") {
        let file = table["file"].as_str().expect("file name");
        let text = std::fs::read_to_string(out.join(file)).expect("read generated file");
        let parsed = parser.parse(&text);
        let errors: Vec<String> = parsed.iter().filter_map(|r| r.as_ref().err()).map(|e| format!("{e:?}")).collect();
        assert!(errors.is_empty(), "{file}: {} parse error(s): {:?}", errors.len(), &errors[..errors.len().min(3)]);
        assert_eq!(parsed.len() as u64, table["hands"].as_u64().unwrap(), "{file}: hand count");
        for hand in parsed.into_iter().map(Result::unwrap) {
            assert_eq!(hand.format, HandFormat::Tournament, "{file}");
            assert_eq!(hand.variant, variant, "{file}");
            assert_eq!(hand.max_seats, max_seats, "{file}");
            assert_eq!(hand.tournament_id.as_deref(), table["tournamentId"].as_str(), "{file}");
            assert_eq!(Some(hand.table_name.as_str()), table["name"].as_str(), "{file}");
            assert_eq!(hand.hero_name.as_deref(), manifest["hero"].as_str(), "{file}: hero");
            assert!(hand.seats.iter().all(|s| s.position.is_some()), "{file}: every seat has a position");
            assert!(hand.skipped_seats.is_empty(), "{file}: every seated player is dealt in");
        }
        total += table["hands"].as_u64().unwrap();
    }
    assert_eq!(total, manifest["hands"].as_u64().unwrap());
}

/// The whole folder through the real directory import, then again: no
/// failure, rejection or warning, and the second pass adds nothing.
fn assert_directory_import(conn: &mut Connection, out: &Path, expected: u64) {
    let first = import_directory(conn, out).expect("import directory");
    assert_eq!(first.hands_failed, 0, "parse failures");
    assert_eq!(first.hands_rejected_invalid, 0, "rejected by the integrity gate");
    assert_eq!(first.hands_with_warnings, 0, "hands with warnings");
    assert_eq!(first.hands_imported, expected as i64);
    let again = import_directory(conn, out).expect("re-import directory");
    assert_eq!(again.hands_imported, 0, "a duplicate re-import adds nothing");
    assert_eq!(again.hands_skipped_duplicate, expected as i64);
}

/// What a set of stored hands looks like: distinct `(format, variant,
/// currency, buy-in component count, has tournament id and level)`.
fn stored_shape(conn: &Connection) -> Vec<String> {
    strings(
        conn,
        "SELECT DISTINCT format || '|' || variant || '|' || currency || '|' ||
                (LENGTH(buy_in) - LENGTH(REPLACE(buy_in, '+', '')) + 1) || '|' ||
                (tournament_id IS NOT NULL) || '|' || (level IS NOT NULL)
         FROM hands ORDER BY 1",
    )
}

fn real(text: &str) -> Connection {
    let mut conn = memory_db();
    let summary = import_text(&mut conn, text).expect("import real fixture");
    assert_eq!(summary.hands_failed + summary.hands_rejected_invalid, 0);
    conn
}

#[test]
fn generated_mtt_is_stored_like_the_real_knockout_and_tournament_fixtures() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let out = tmp.path().join("HandHistory");
    let manifest = generate(&out, "mtt", "formats-mtt", MTT_TABLES, MTT_HANDS);
    assert_eq!(manifest["hands"].as_u64(), Some((MTT_TABLES * MTT_HANDS) as u64));
    assert_files_parse(&out, &manifest, GameVariant::Tournament, 9);
    let mut conn = open_db(tmp.path());
    assert_directory_import(&mut conn, &out, manifest["hands"].as_u64().unwrap());

    // The real fixtures: a regular 9-max tournament and a (Zoom) knockout.
    let real_mtt = real(REAL_MTT);
    let real_bounty = real(REAL_BOUNTY);
    assert_eq!(stored_shape(&real_mtt), ["tournament|tournament|CHIPS|2|1|1"]);
    assert_eq!(stored_shape(&real_bounty), ["tournament|zoom_tournament|CHIPS|3|1|1"]);
    // Generated: a regular tournament's format and variant, with the
    // knockout's three-part buy-in.
    assert_eq!(stored_shape(&conn), ["tournament|tournament|CHIPS|3|1|1"]);

    // Bounties: on every stored seat, like the real knockout, read from the
    // seat line in the buy-in's currency; they start at the buy-in's
    // bounty component and grow with knockouts.
    for c in [&real_bounty, &conn] {
        assert_eq!(count(c, "SELECT COUNT(*) FROM player_hands WHERE bounty IS NULL"), 0);
    }
    let buy_ins = strings(&conn, "SELECT DISTINCT buy_in FROM hands");
    assert_eq!(buy_ins, ["$5.00+$5.00+$1.00"]);
    assert_eq!(initial_bounty(&buy_ins[0]), Some(5.0));
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM player_hands WHERE bounty < 5.0"), 0);
    assert!(count(&conn, "SELECT COUNT(DISTINCT bounty) FROM player_hands") > 5, "bounties grow");
    let paid = manifest["tournaments"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["eliminations"].as_array().unwrap().len())
        .sum::<usize>();
    assert!(paid > 30, "knockouts paid: {paid}");

    // Levels rise with antes; tournaments come and go; players bust out.
    let levels = strings(&conn, "SELECT DISTINCT level FROM hands");
    assert!(levels.len() >= 8, "levels seen: {levels:?}");
    assert_eq!(
        count(&conn, "SELECT COUNT(DISTINCT tournament_id) FROM hands") as usize,
        manifest["tournaments"].as_array().unwrap().len()
    );
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM hands WHERE max_seats <> 9"), 0);
    let antes = count(&conn, "SELECT COUNT(*) FROM actions WHERE action_type = 'post_ante'");
    let seats = count(&conn, "SELECT COUNT(*) FROM player_hands");
    assert_eq!(antes, seats, "every dealt-in player antes");

    // The engine's context reads the same facts as from the real knockout.
    let hand_id: i64 = conn.query_row("SELECT MAX(id) FROM hands", [], |r| r.get(0)).unwrap();
    let hand = load_hand(&conn, hand_id).unwrap().expect("hand");
    let villain = hand.seats.iter().find(|s| !s.is_hero).expect("a villain");
    let ctx = villain_context(&hand, villain.player_id).expect("context");
    assert_eq!(ctx.format, FormatKey::Mtt);
    assert!(ctx.level.is_some() && ctx.stage.is_some());
    let bounty = ctx.bounty.expect("a knockout context");
    assert_eq!(bounty.currency.as_deref(), Some("USD"));
    assert_eq!(bounty.ratio, Some(((bounty.amount / 5.0) * 100.0).round() / 100.0));
}

#[test]
fn generated_spin_is_stored_like_the_real_spin_fixture() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let out = tmp.path().join("HandHistory");
    let manifest = generate(&out, "spin", "formats-spin", SPIN_TABLES, SPIN_HANDS);
    assert_files_parse(&out, &manifest, GameVariant::Spin, 3);
    let mut conn = open_db(tmp.path());
    assert_directory_import(&mut conn, &out, manifest["hands"].as_u64().unwrap());

    let real_spin = real(REAL_SPIN);
    assert_eq!(stored_shape(&real_spin), ["tournament|spin|CHIPS|2|1|1"]);
    assert_eq!(stored_shape(&conn), stored_shape(&real_spin));
    assert_eq!(strings(&conn, "SELECT DISTINCT buy_in FROM hands"), strings(&real_spin, "SELECT DISTINCT buy_in FROM hands"));
    for c in [&real_spin, &conn] {
        assert_eq!(count(c, "SELECT COUNT(*) FROM player_hands WHERE bounty IS NOT NULL"), 0, "no bounty in a Spin");
        assert_eq!(count(c, "SELECT COUNT(*) FROM actions WHERE action_type = 'post_ante'"), 0, "no ante in a Spin");
    }
    // Hyper: 500 chips, and the blinds climb within each Spin.
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM hands WHERE max_seats <> 3"), 0);
    assert!(strings(&conn, "SELECT DISTINCT level FROM hands").len() >= 6);
    assert!(count(&conn, "SELECT COUNT(*) FROM hands h WHERE (SELECT COUNT(*) FROM player_hands p WHERE p.hand_id = h.id) = 2") > 50, "heads-up");
    let spins = manifest["tournaments"].as_array().unwrap();
    assert!(spins.len() > 6, "{} Spins", spins.len());
    assert_eq!(count(&conn, "SELECT COUNT(DISTINCT tournament_id) FROM hands") as usize, spins.len());

    let hand_id: i64 = conn.query_row("SELECT MIN(id) FROM hands", [], |r| r.get(0)).unwrap();
    let hand = load_hand(&conn, hand_id).unwrap().expect("hand");
    let villain = hand.seats.iter().find(|s| !s.is_hero).expect("a villain");
    let ctx = villain_context(&hand, villain.player_id).expect("context");
    assert_eq!(ctx.format, FormatKey::Spin);
    assert_eq!(ctx.effective_stack_bb, Some(25.0), "a hyper Spin starts at 25bb");
    assert!(ctx.bounty.is_none());
}

#[test]
fn short_stack_profile_lands_in_the_push_fold_bucket_with_only_push_fold_reads() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let out = tmp.path().join("HandHistory");
    let manifest = generate(&out, "mtt", "formats-short", MTT_TABLES, MTT_HANDS);
    let mut conn = open_db(tmp.path());
    assert_directory_import(&mut conn, &out, manifest["hands"].as_u64().unwrap());

    let profiles = profiles();
    let short = profiles["profiles"].as_array().unwrap().iter().find(|p| p["id"] == "short").expect("short profile");
    for name in short["players"].as_array().unwrap() {
        let name = name.as_str().unwrap();
        let pid = player_id(&conn, name);
        let ids: Vec<i64> = conn
            .prepare(
                "SELECT h.id FROM hands h JOIN player_hands ph ON ph.hand_id = h.id
                 WHERE ph.player_id = ?1 ORDER BY h.played_at, h.id",
            )
            .unwrap()
            .query_map([pid], |row| row.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        // The context of each of his hands, as the overlay builds it from
        // the latest completed hand at his table.
        let contexts: Vec<_> = ids
            .iter()
            .map(|id| villain_context(&load_hand(&conn, *id).unwrap().unwrap(), pid).expect("context"))
            .collect();
        let push_fold: Vec<_> = contexts.iter().filter(|c| c.stack_bucket == Some(StackBucket::PushFold)).collect();
        eprintln!("{name}: {} of {} hands at push/fold depth", push_fold.len(), contexts.len());
        assert!(contexts.len() > 100, "{name} plays: {} hands", contexts.len());
        assert!(push_fold.len() * 5 >= contexts.len() * 2, "{name} at push/fold depth in {} of {} hands", push_fold.len(), contexts.len());
        // His latest push/fold hand with a stack worth shoving (5bb or more).
        let ctx = (*push_fold
            .iter()
            .rev()
            .find(|c| c.effective_stack_bb.is_some_and(|eff| eff >= 5.0))
            .expect("a push/fold hand of 5bb or more"))
        .clone();
        assert_eq!(ctx.format, FormatKey::Mtt);
        assert!(ctx.effective_stack_bb.is_some_and(|eff| eff <= 15.0));
        assert!(ctx.bounty.is_some(), "a knockout seat");
        assert_push_fold_reads(&conn, name, pid, ctx, &short["tournament"]["expected"]);
    }
}

#[cfg(feature = "strategic-analysis")]
fn assert_push_fold_reads(conn: &Connection, name: &str, pid: i64, ctx: EngineContext, expected: &Value) {
    use velora_poker_lib::engine::{EngineCache, Family};
    let eff = ctx.effective_stack_bb.unwrap();
    let payload = EngineCache::new().payload(conn, pid, Some(ctx)).expect("engine payload");
    let reads: Vec<String> = payload.reads.iter().map(|r| format!("{} ({:?}) {:.2}", r.rule_id, r.family, r.score)).collect();
    eprintln!("{name} at {eff}bb: tag {:?}, reads {reads:?}", payload.tag.as_ref().map(|t| &t.text));
    // Section 5 of the spec: at push/fold depth only stack, short-stack,
    // bounty, recency and showdown reads apply.
    for read in &payload.reads {
        let allowed = matches!(read.family, Family::Stack | Family::Bounty | Family::Recency | Family::Showdown)
            || read.rule_id == "ctx.short_stack";
        assert!(allowed, "{name}: {} ({:?}) at push/fold depth", read.rule_id, read.family);
    }
    let short = payload.reads.iter().find(|r| r.rule_id == "ctx.short_stack").expect("the short-stack fact");
    assert_eq!(short.tag.as_deref(), Some(format!("{}bb", eff.floor() as u32).as_str()));
    // His top read past the context facts is the profile's push/fold read.
    assert_eq!(expected["context"], "push_fold");
    let wanted: Vec<(Option<String>, String)> = expected["reads"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| (r["tag"].as_str().map(String::from), r["ruleId"].as_str().unwrap().to_string()))
        .collect();
    let top = payload.reads.iter().find(|r| !r.rule_id.starts_with("ctx.")).map(|r| (r.tag.clone(), r.rule_id.clone()));
    assert!(top.as_ref().is_some_and(|t| wanted.contains(t)), "{name}: top read {top:?}, expected one of {wanted:?}");
}

#[cfg(not(feature = "strategic-analysis"))]
fn assert_push_fold_reads(_conn: &Connection, _name: &str, _pid: i64, _ctx: EngineContext, _expected: &Value) {}
