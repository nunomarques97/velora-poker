//! The import-time integrity gate, and the repair migration.

use velora_poker_lib::import::validate::{self, Severity};
use velora_poker_lib::parser::{parse_hand_block, split_hands};
use velora_poker_lib::{db, import};

const CASH_SITTING_OUT: &str = include_str!("fixtures/real_cash_sitting_out.txt");
const BOUNTY_TOURNAMENT: &str = include_str!("fixtures/real_bounty_tournament.txt");
const SEAT_MOVED_OUT_OF_HAND: &str = include_str!("fixtures/real_seat_moved_out_of_hand.txt");
const POSITIONS_BY_TABLE_SIZE: &str = include_str!("fixtures/real_positions_by_table_size.txt");

const ALL_REAL: [&str; 4] = [
    CASH_SITTING_OUT,
    BOUNTY_TOURNAMENT,
    SEAT_MOVED_OUT_OF_HAND,
    POSITIONS_BY_TABLE_SIZE,
];

fn setup_db() -> rusqlite::Connection {
    db::open(std::path::Path::new(":memory:")).expect("open db")
}

/// The safety property that makes rejecting hands acceptable at all: the gate
/// must not refuse a single real hand from the bundled real-hand fixtures.
#[test]
fn the_gate_rejects_none_of_the_real_hands() {
    let mut checked = 0;
    for text in ALL_REAL {
        for block in split_hands(text) {
            let hand = parse_hand_block(&block).expect("fixture should parse");
            let problems = validate::check(&hand);
            let rejects: Vec<_> = problems
                .iter()
                .filter(|p| p.severity == Severity::Reject)
                .collect();
            assert!(
                rejects.is_empty(),
                "hand {} was rejected by {:?}",
                hand.hand_id,
                rejects
            );
            assert!(validate::is_storable(&problems));
            checked += 1;
        }
    }
    assert_eq!(checked, 12, "all four fixture files must be covered");
}

/// The bounty failure had two symptoms the gate now catches directly: a hand
/// with no players, and actions by somebody with no seat. Simulated by feeding
/// the gate a hand whose seats have been emptied, which is exactly the state the
/// old parser produced.
#[test]
fn a_hand_that_stored_no_players_is_rejected() {
    let block = &split_hands(BOUNTY_TOURNAMENT)[0];
    let mut hand = parse_hand_block(block).expect("parse");
    assert!(!hand.actions.is_empty());

    hand.seats.clear();

    let problems = validate::check(&hand);
    assert!(!validate::is_storable(&problems));
    let codes: Vec<&str> = problems.iter().map(|p| p.code).collect();
    assert!(codes.contains(&"no_dealt_in_players"), "got {codes:?}");
    assert!(codes.contains(&"action_by_unseated_player"), "got {codes:?}");
}

#[test]
fn a_duplicated_seat_is_rejected() {
    let block = &split_hands(POSITIONS_BY_TABLE_SIZE)[1];
    let mut hand = parse_hand_block(block).expect("parse");
    let duplicate = hand.seats[0].clone();
    hand.seats.push(duplicate);

    let codes: Vec<&str> = validate::check(&hand).iter().map(|p| p.code).collect();
    assert!(codes.contains(&"duplicate_seat_number"), "got {codes:?}");
    assert!(codes.contains(&"duplicate_player_in_hand"), "got {codes:?}");
}

#[test]
fn a_button_seat_outside_the_table_is_rejected() {
    let block = &split_hands(POSITIONS_BY_TABLE_SIZE)[0];
    let mut hand = parse_hand_block(block).expect("parse");
    hand.button_seat = hand.max_seats + 5;

    let codes: Vec<&str> = validate::check(&hand).iter().map(|p| p.code).collect();
    assert!(codes.contains(&"button_seat_out_of_range"), "got {codes:?}");
}

/// A rejected hand must leave a trace: no `hands` row, but a recorded finding.
#[test]
fn a_rejected_hand_is_not_stored_silently() {
    let mut conn = setup_db();
    // A structurally impossible hand: the header parses, the seat block does
    // not survive, so nobody is dealt in.
    let broken = "PokerStars Hand #999999999:  Hold'em No Limit (€0.01/€0.02 EUR) - 2026/09/12 10:00:00 WET [2026/09/12 05:00:00 ET]\n\
                  Table 'Nowhere' 6-max Seat #2 is the button\n\
                  *** HOLE CARDS ***\n\
                  *** SUMMARY ***\n\
                  Total pot €0 | Rake €0\n";

    let summary = import::import_text(&mut conn, broken).expect("import runs");
    assert_eq!(summary.hands_imported, 0);
    assert_eq!(summary.hands_rejected_invalid, 1);
    assert_eq!(db::count_hands(&conn).unwrap(), 0);

    let problems = db::import_problem_summary(&conn).expect("summary");
    assert!(
        problems
            .iter()
            .any(|(sev, code, _, _, _, _)| sev == "reject" && code == "no_dealt_in_players"),
        "the rejection must be recorded where diagnostics can read it: {problems:?}"
    );
}

/// End-to-end: importing the real bounty fixture stores players, positions and
/// no orphan actions — the exact counters the diagnostics report shows.
#[test]
fn importing_real_hands_leaves_every_integrity_counter_at_zero() {
    let mut conn = setup_db();
    for text in ALL_REAL {
        import::import_text(&mut conn, text).expect("import");
    }

    let (no_players, orphan_actions, phantoms, no_position) =
        db::integrity_counters(&conn).expect("counters");
    assert_eq!(no_players, 0, "every hand must store its players");
    assert_eq!(orphan_actions, 0, "no action may lack a player row");
    assert_eq!(phantoms, 0, "no player may exist without a hand");
    assert_eq!(no_position, 0, "every player row must carry a position");
}

/// Seats that were not dealt in are counted, not silently dropped.
#[test]
fn seats_not_dealt_in_are_counted_in_the_import_summary() {
    let mut conn = setup_db();
    let summary = import::import_text(&mut conn, SEAT_MOVED_OUT_OF_HAND).expect("import");
    assert_eq!(summary.hands_imported, 1);
    assert_eq!(summary.seats_not_dealt_in, 1);
}

/// The repair migration must converge and then do nothing, so it is safe on a
/// database that has already been migrated.
#[test]
fn the_repair_migration_is_idempotent() {
    let mut conn = setup_db();
    for text in ALL_REAL {
        import::import_text(&mut conn, text).expect("import");
    }
    let hands_before = db::count_hands(&conn).unwrap();
    let rows_before: i64 = conn
        .query_row("SELECT COUNT(*) FROM player_hands", [], |r| r.get(0))
        .unwrap();

    let first = db::run_ingestion_repair(&conn).expect("first repair");
    let second = db::run_ingestion_repair(&conn).expect("second repair");

    // Freshly imported data is already correct, so the repair changes nothing.
    assert_eq!(first.player_rows_inserted, 0);
    assert_eq!(first.player_rows_deleted, 0);
    assert_eq!(second.player_rows_inserted, 0);
    assert_eq!(second.player_rows_deleted, 0);
    assert_eq!(first.hands_reparse_failed, 0);

    assert_eq!(db::count_hands(&conn).unwrap(), hands_before, "must not duplicate hands");
    let rows_after: i64 = conn
        .query_row("SELECT COUNT(*) FROM player_hands", [], |r| r.get(0))
        .unwrap();
    assert_eq!(rows_after, rows_before);
}

/// The repair must actually repair: starting from the corrupted shape the old
/// parser produced (a hand with actions but no player rows), it reconstructs the
/// player rows from `raw_text` and clears the orphan actions.
#[test]
fn the_repair_reconstructs_players_from_stored_raw_text() {
    let mut conn = setup_db();
    import::import_text(&mut conn, BOUNTY_TOURNAMENT).expect("import");

    // Recreate the old corruption: drop the player rows, keep hand and actions.
    conn.execute("DELETE FROM player_hands", []).unwrap();
    let (no_players, orphans, _, _) = db::integrity_counters(&conn).unwrap();
    assert_eq!(no_players, 2, "both bounty hands now look like the old defect");
    assert!(orphans > 0);

    let report = db::run_ingestion_repair(&conn).expect("repair");
    assert_eq!(report.player_rows_inserted, 6, "3 players x 2 hands");

    let (no_players, orphans, phantoms, no_position) = db::integrity_counters(&conn).unwrap();
    assert_eq!((no_players, orphans, phantoms, no_position), (0, 0, 0, 0));
}
