//! Parser stress corpus: every simulator format, generated with fixed seeds,
//! through the real parser and import pipeline, plus fault injection over the
//! same hands.
//!
//! - Every format (`cash`, `zoom`, `mtt`, `spin`) parses with zero errors,
//!   imports exactly the generator manifest's hand count, per table, and a
//!   second import of the same corpus adds nothing.
//! - Every generated hand truncated at every line boundary, garbage lines and
//!   broken headers, all in one file between intact hands: nothing panics,
//!   every intact hand is stored exactly as a clean import stores it, and every
//!   bad hand is counted, never stored.
//! - A hand cut before its summary (the watcher reading a file PokerStars is
//!   still writing) never blocks the complete copy of the same hand.
//!
//! The generator runs through `node` into a temporary folder; databases are
//! temporary. Nothing touches the app's own data folder.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use rusqlite::Connection;
use serde_json::Value;
use velora_poker_lib::db;
use velora_poker_lib::import::{import_directory, import_file, import_text, validate, ImportSummary};
use velora_poker_lib::parser::{parse_hand_block, split_hands, HandHistoryParser, PokerStarsParser};

const SEED: &str = "parser-corpus";

/// `(format, --tables, --hands, stored variant)`.
const CORPUS: [(&str, u32, u32, &str); 4] = [
    ("cash", 3, 120, "cash"),
    ("zoom", 1, 300, "zoom_cash"),
    ("mtt", 2, 150, "tournament"),
    ("spin", 2, 150, "spin"),
];

/// Smaller corpus for fault injection: every hand is cut at every line.
const FAULT_CORPUS: [(&str, u32, u32); 4] = [("cash", 1, 25), ("zoom", 1, 25), ("mtt", 1, 25), ("spin", 1, 25)];

/// Lines that are not hand-history text. Inserted between hands, they become
/// trailing text of the block before them.
const GARBAGE: [&str; 8] = [
    "this line is not part of any hand history",
    "***",
    "PokerStars",
    "Seat 9: NobodyAtThisTable (1 in chips)",
    "NobodyAtThisTable: raises 10 to 20",
    "\u{1}\u{2}\u{7f} \u{0} control characters",
    "ÿþ€ – ☃ 名前 ü",
    "*** SUMMARY",
];

/// Header lines PokerStars never writes: each starts a block that must fail
/// to parse and be counted.
const BROKEN_HEADERS: [&str; 4] = [
    "PokerStars Hand #: Hold'em No Limit ($0.25/$0.50 USD) - 2026/09/12 20:00:18 ET",
    "PokerStars Hand #123456789012: Tournament #, garbage - Level ? (?/?) - never",
    "PokerStars Zoom Hand #999",
    "PokerStars Hand #252236300025:  Hold'em No Limit ($0.25/$0.50 USD)",
];

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().expect("repo root").to_path_buf()
}

/// Runs the generator into `out` and returns its manifest.
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

fn memory_db() -> Connection {
    db::open(Path::new(":memory:")).expect("open in-memory db")
}

fn count(conn: &Connection, sql: &str) -> i64 {
    conn.query_row(sql, [], |row| row.get(0)).unwrap()
}

fn row_counts(conn: &Connection) -> [i64; 3] {
    [
        count(conn, "SELECT COUNT(*) FROM hands"),
        count(conn, "SELECT COUNT(*) FROM player_hands"),
        count(conn, "SELECT COUNT(*) FROM actions"),
    ]
}

/// Every stored fact except `raw_text` and `imported_at`, one string per
/// hand, player row and action, sorted: two databases holding the same hands
/// give the same snapshot.
fn snapshot(conn: &Connection) -> Vec<String> {
    let queries = [
        "SELECT 'hand|' || hand_id || '|' || format || '|' || IFNULL(table_name,'') || '|' || IFNULL(game_type,'')
                || '|' || IFNULL(tournament_id,'') || '|' || IFNULL(buy_in,'') || '|' || IFNULL(level,'')
                || '|' || IFNULL(small_blind,'') || '|' || IFNULL(big_blind,'') || '|' || IFNULL(currency,'')
                || '|' || IFNULL(max_seats,'') || '|' || IFNULL(button_seat,'') || '|' || IFNULL(played_at,'')
                || '|' || IFNULL(variant,'')
           FROM hands",
        "SELECT 'player|' || h.hand_id || '|' || p.name || '|' || IFNULL(ph.seat,'') || '|' || IFNULL(ph.starting_stack,'')
                || '|' || IFNULL(ph.position,'') || '|' || ph.is_hero || '|' || ph.went_to_showdown
                || '|' || ph.won_at_showdown || '|' || IFNULL(ph.net_result,'') || '|' || IFNULL(ph.hole_cards,'')
                || '|' || IFNULL(ph.bounty,'')
           FROM player_hands ph JOIN hands h ON h.id = ph.hand_id JOIN players p ON p.id = ph.player_id",
        "SELECT 'action|' || h.hand_id || '|' || a.action_index || '|' || p.name || '|' || a.street || '|' || a.action_type
                || '|' || IFNULL(a.amount,'') || '|' || a.is_all_in
           FROM actions a JOIN hands h ON h.id = a.hand_id JOIN players p ON p.id = a.player_id",
    ];
    let mut rows = Vec::new();
    for sql in queries {
        let mut stmt = conn.prepare(sql).unwrap();
        let found = stmt.query_map([], |row| row.get::<_, String>(0)).unwrap();
        rows.extend(found.map(Result::unwrap));
    }
    rows.sort();
    rows
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|e| panic!("read {path:?}: {e}"))
}

/// The intact hand blocks of every file in `manifest`, in file order.
fn manifest_blocks(out: &Path, manifest: &Value) -> Vec<String> {
    let mut blocks = Vec::new();
    for table in manifest["tables"].as_array().expect("tables") {
        blocks.extend(split_hands(&read(&out.join(table["file"].as_str().expect("file")))));
    }
    blocks
}

fn assert_clean(summary: &ImportSummary, imported: i64) {
    assert_eq!(summary.hands_imported, imported, "{summary:?}");
    assert_eq!(summary.hands_failed, 0, "{summary:?}");
    assert_eq!(summary.hands_rejected_invalid, 0, "{summary:?}");
    assert_eq!(summary.hands_with_warnings, 0, "{summary:?}");
    assert_eq!(summary.hands_skipped_duplicate, 0, "{summary:?}");
}

/// Every format: zero parse errors and zero integrity problems file by file,
/// the manifest's hand count imported, the same count per table, and a
/// second import of the whole corpus that adds nothing.
#[test]
fn every_generated_format_imports_exactly_the_manifest() {
    let dir = tempfile::tempdir().expect("temp dir");
    let hh = dir.path().join("hh");
    let parser = PokerStarsParser;

    let mut expected_total = 0i64;
    let mut expected_tables: BTreeMap<String, i64> = BTreeMap::new();
    for (format, tables, hands, variant) in CORPUS {
        let out = hh.join(format);
        let manifest = generate(&out, format, tables, hands);
        let manifest_hands = manifest["hands"].as_i64().expect("manifest hand count");
        assert!(manifest_hands > 0, "{format}: generator wrote no hands");
        expected_total += manifest_hands;

        let mut file_total = 0i64;
        for table in manifest["tables"].as_array().expect("tables") {
            let file = table["file"].as_str().expect("file name");
            let parsed = parser.parse(&read(&out.join(file)));
            let errors: Vec<String> = parsed.iter().filter_map(|r| r.as_ref().err()).map(|e| e.to_string()).collect();
            assert!(errors.is_empty(), "{format}/{file}: {} parse error(s), first {:?}", errors.len(), errors.first());
            assert_eq!(parsed.len() as i64, table["hands"].as_i64().unwrap(), "{format}/{file}: parsed hand count");
            for hand in parsed.into_iter().map(Result::unwrap) {
                let problems = validate::check(&hand);
                assert!(problems.is_empty(), "{format} hand {}: {problems:?}", hand.hand_id);
            }
            file_total += table["hands"].as_i64().unwrap();

            let key = format!(
                "{variant}|{}|{}",
                table["tournamentId"].as_str().unwrap_or(""),
                table["name"].as_str().expect("table name")
            );
            *expected_tables.entry(key).or_default() += table["hands"].as_i64().unwrap();
        }
        assert_eq!(file_total, manifest_hands, "{format}: per-table hands add up to the manifest");
    }

    let mut conn = db::open(&dir.path().join("velora.db")).expect("open temp db");
    let first = import_directory(&mut conn, &hh).expect("import corpus");
    assert_clean(&first, expected_total);
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM hands"), expected_total);

    let stored_tables: BTreeMap<String, i64> = conn
        .prepare(
            "SELECT variant || '|' || IFNULL(tournament_id, '') || '|' || table_name, COUNT(*)
               FROM hands GROUP BY 1",
        )
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(stored_tables, expected_tables, "hands per table");

    let before = row_counts(&conn);
    let again = import_directory(&mut conn, &hh).expect("re-import corpus");
    assert_eq!(again.hands_imported, 0, "{again:?}");
    assert_eq!(again.hands_skipped_duplicate, expected_total, "{again:?}");
    assert_eq!(again.hands_failed + again.hands_rejected_invalid, 0, "{again:?}");
    assert_eq!(row_counts(&conn), before, "hands, player_hands, actions unchanged by the second import");
}

/// One file holding, for every generated hand: garbage lines, a broken
/// header, the hand cut at every line boundary, then the intact hand. Parsing
/// never panics, every intact hand is stored exactly as a clean import stores
/// it, and every bad block is counted as failed or rejected.
#[test]
fn fault_injected_file_keeps_every_intact_hand_and_counts_every_bad_one() {
    let dir = tempfile::tempdir().expect("temp dir");
    let mut intact: Vec<String> = Vec::new();
    for (format, tables, hands) in FAULT_CORPUS {
        let out = dir.path().join(format);
        let manifest = generate(&out, format, tables, hands);
        intact.extend(manifest_blocks(&out, &manifest));
    }

    let mut text = String::new();
    let mut bad_blocks = 0i64;
    for (i, block) in intact.iter().enumerate() {
        text.push_str(GARBAGE[i % GARBAGE.len()]);
        text.push('\n');
        text.push_str(BROKEN_HEADERS[i % BROKEN_HEADERS.len()]);
        text.push('\n');
        text.push_str(GARBAGE[(i + 3) % GARBAGE.len()]);
        text.push_str("\n\n");
        bad_blocks += 1;

        let lines: Vec<&str> = block.lines().collect();
        for cut in 1..lines.len() {
            text.push_str(&lines[..cut].join("\n"));
            text.push_str("\n\n\n");
            bad_blocks += 1;
        }
        text.push_str(block);
        text.push_str("\n\n\n");
    }

    // Parse every block on its own first, so a panic names its block instead
    // of aborting the import halfway.
    let blocks = split_hands(&text);
    assert_eq!(blocks.len() as i64, bad_blocks + intact.len() as i64, "block split");
    let panics: Vec<String> = blocks
        .iter()
        .filter(|block| {
            std::panic::catch_unwind(|| {
                if let Ok(hand) = parse_hand_block(block) {
                    validate::check(&hand);
                }
            })
            .is_err()
        })
        .map(|block| block.lines().take(2).collect::<Vec<_>>().join(" / "))
        .collect();
    assert!(panics.is_empty(), "{} block(s) panicked, first: {:?}", panics.len(), panics.first());

    let mut conn = memory_db();
    let summary = import_text(&mut conn, &text).expect("import fault file");
    assert_eq!(summary.hands_imported, intact.len() as i64, "every intact hand stored: {summary:?}");
    assert_eq!(summary.hands_skipped_duplicate, 0, "no bad copy was stored ahead of its intact hand: {summary:?}");
    assert_eq!(
        summary.hands_failed + summary.hands_rejected_invalid,
        bad_blocks,
        "every bad block counted: {summary:?}"
    );

    let mut clean = memory_db();
    let clean_summary = import_text(&mut clean, &intact.join("\n\n\n")).expect("clean import");
    assert_clean(&clean_summary, intact.len() as i64);
    assert_eq!(snapshot(&conn), snapshot(&clean), "intact hands stored exactly as a clean import stores them");
    assert_eq!(
        count(&conn, "SELECT COUNT(*) FROM import_problems"),
        0,
        "every hand id with a rejected copy was stored complete in the end, so no finding is left behind"
    );
}

/// The first generated cash file with a hand that reaches showdown: the file
/// text and the byte offsets of that hand's `*** SUMMARY ***` line, of its
/// first summary seat line and of its end.
fn showdown_file(dir: &Path) -> (String, String, usize, usize, usize) {
    let out = dir.join("cash");
    let manifest = generate(&out, "cash", 1, 40);
    let file = manifest["tables"][0]["file"].as_str().expect("file").to_string();
    let text = read(&out.join(&file));
    let blocks = split_hands(&text);
    let block = blocks
        .iter()
        .find(|b| b.contains("*** SHOW DOWN ***") && b.contains("Uncalled bet"))
        .or_else(|| blocks.iter().find(|b| b.contains("*** SHOW DOWN ***")))
        .expect("a showdown hand in 40 generated cash hands");
    // Offsets in the file itself, which has CRLF line ends.
    let header = block.lines().next().expect("header line");
    let start = text.find(header).expect("hand in file");
    let summary = start + text[start..].find("*** SUMMARY ***").expect("summary");
    let first_seat = summary + text[summary..].find("\nSeat ").expect("summary seat line") + 1;
    let end = text[summary..].find("\nPokerStars").map_or(text.len(), |i| summary + i);
    (file, text, summary, first_seat, end)
}

/// The watcher reads a file PokerStars is still writing. Whatever the cut
/// point inside the summary-less or summary-incomplete hand, that read never
/// stores the hand, and the read of the finished file stores it exactly as a
/// clean import does: showdown flags, net results and every action.
#[test]
fn a_hand_cut_before_its_summary_does_not_block_the_complete_copy() {
    let dir = tempfile::tempdir().expect("temp dir");
    let (file, text, summary_at, first_seat_at, end) = showdown_file(dir.path());

    let total = split_hands(&text).len() as i64;
    let mut clean = memory_db();
    assert_clean(&import_text(&mut clean, &text).expect("clean import"), total);

    // Cut before `*** SUMMARY ***`, right after it, and after its first seat line.
    let first_seat_end = first_seat_at + text[first_seat_at..].find('\n').expect("line end") + 1;
    for cut in [summary_at, first_seat_at, first_seat_end] {
        assert!(cut < end, "cut inside the hand");
        let live = dir.path().join("live");
        std::fs::create_dir_all(&live).unwrap();
        let path = live.join(&file);
        let mut conn = memory_db();

        std::fs::write(&path, &text[..cut]).unwrap();
        let partial = import_file(&mut conn, &path).expect("import partial file");
        assert_eq!(
            partial.hands_failed + partial.hands_rejected_invalid,
            1,
            "cut at byte {cut}: the incomplete hand is counted: {partial:?}"
        );

        std::fs::write(&path, &text).unwrap();
        let complete = import_file(&mut conn, &path).expect("import complete file");
        assert_eq!(
            partial.hands_imported + complete.hands_imported,
            total,
            "cut at byte {cut}: the complete copy is stored: {partial:?} then {complete:?}"
        );
        assert_eq!(complete.hands_failed + complete.hands_rejected_invalid, 0, "{complete:?}");

        assert_eq!(snapshot(&conn), snapshot(&clean), "cut at byte {cut}: same facts as a clean import");
        assert_eq!(
            count(&conn, "SELECT COUNT(*) FROM import_problems"),
            0,
            "cut at byte {cut}: the incomplete copy's findings are gone once the hand is stored"
        );
    }
}

/// Bytes that are not UTF-8 between two hands, and inside one player's name in
/// a third: the other hands of the file still import, the hand with the
/// undecodable name is counted and not stored under a mangled name.
#[test]
fn bytes_that_are_not_utf8_only_cost_the_hand_they_are_in() {
    let dir = tempfile::tempdir().expect("temp dir");
    let out = dir.path().join("cash");
    let manifest = generate(&out, "cash", 1, 3);
    let blocks = manifest_blocks(&out, &manifest);
    assert_eq!(blocks.len(), 3);

    let victim = blocks[2]
        .lines()
        .find_map(|l| l.strip_prefix("Seat 1: ").and_then(|r| r.split(" (").next()))
        .expect("seat 1 name")
        .to_string();
    let mut bytes: Vec<u8> = Vec::new();
    bytes.extend_from_slice(blocks[0].as_bytes());
    bytes.extend_from_slice(b"\n\xff\xfe\xc3 stray bytes\n\n\n");
    bytes.extend_from_slice(blocks[1].as_bytes());
    bytes.extend_from_slice(b"\n\n\n");
    let mangled = blocks[2].replace(&victim, "Mangled\u{1}NAME");
    for part in mangled.split("Mangled\u{1}NAME").enumerate() {
        if part.0 > 0 {
            bytes.extend_from_slice(b"Mangled\xffNAME");
        }
        bytes.extend_from_slice(part.1.as_bytes());
    }
    bytes.extend_from_slice(b"\n\n\n");

    let path = dir.path().join("mixed.txt");
    std::fs::write(&path, &bytes).unwrap();
    let mut conn = memory_db();
    let summary = import_file(&mut conn, &path).expect("a file with stray bytes still imports");
    assert_eq!(summary.hands_imported, 2, "{summary:?}");
    assert_eq!(summary.hands_failed + summary.hands_rejected_invalid, 1, "{summary:?}");
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM players WHERE name LIKE 'Mangled%'"), 0);

    let mut clean = memory_db();
    import_text(&mut clean, &blocks[..2].join("\n\n\n")).expect("clean import");
    assert_eq!(snapshot(&conn), snapshot(&clean), "the two intact hands are stored as a clean import stores them");
}
