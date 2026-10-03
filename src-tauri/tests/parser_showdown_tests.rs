//! Stored hand facts the opponent engine reads (spec rows D01–D04,
//! `docs/specs/opponent-engine.md`): shown and mucked hole cards, seat-line
//! bounties, the game variant (including Zoom cash and spin-like 3-max), and
//! the reparse backfill that fills them on existing databases.

use rusqlite::types::Value;
use rusqlite::{params, Connection, OptionalExtension};
use velora_poker_lib::parser::{
    parse_hand_block, split_hands, GameVariant, HandFormat, ParsedHand,
};
use velora_poker_lib::{db, import};

const SHOWDOWN: &str = include_str!("fixtures/showdown_shows_and_mucks.txt");
const ZOOM_CASH: &str = include_str!("fixtures/zoom_cash.txt");
const SPIN: &str = include_str!("fixtures/spin_three_max.txt");
const BOUNTY_TOURNAMENT: &str = include_str!("fixtures/real_bounty_tournament.txt");

/// Every fixture that existed before the new facts.
const EXISTING_FIXTURES: [&str; 24] = [
    include_str!("fixtures/hand_3bet_showdown.txt"),
    include_str!("fixtures/hand_4bet.txt"),
    include_str!("fixtures/hand_4bet_pileup.txt"),
    include_str!("fixtures/hand_allin_preflop_folded.txt"),
    include_str!("fixtures/hand_allin_preflop_showdown.txt"),
    include_str!("fixtures/hand_cbet_fold.txt"),
    include_str!("fixtures/hand_cold_call.txt"),
    include_str!("fixtures/hand_cold_call_after_limp.txt"),
    include_str!("fixtures/hand_fold_to_3bet.txt"),
    include_str!("fixtures/hand_heads_up_steal.txt"),
    include_str!("fixtures/hand_isolation_raise_not_a_steal.txt"),
    include_str!("fixtures/hand_limp_behind_limp.txt"),
    include_str!("fixtures/hand_limped_multiway.txt"),
    include_str!("fixtures/hand_squeeze.txt"),
    include_str!("fixtures/hand_walk.txt"),
    include_str!("fixtures/real_cash_sitting_out.txt"),
    include_str!("fixtures/real_positions_by_table_size.txt"),
    include_str!("fixtures/real_seat_moved_out_of_hand.txt"),
    include_str!("fixtures/real_bounty_tournament.txt"),
    include_str!("fixtures/tournament_zoom_header.txt"),
    include_str!("fixtures/tournament_showdown_allin.txt"),
    include_str!("fixtures/tournament_allin_preflop_disconnect.txt"),
    include_str!("fixtures/tournament_uncontested_walk.txt"),
    include_str!("fixtures/onboarding_non_english_client.txt"),
];

fn setup_db() -> Connection {
    db::open(std::path::Path::new(":memory:")).expect("open db")
}

fn parse_all(text: &str) -> Vec<ParsedHand> {
    split_hands(text)
        .iter()
        .map(|block| parse_hand_block(block).expect("fixture hand should parse"))
        .collect()
}

fn seat_cards<'a>(hand: &'a ParsedHand, name: &str) -> Option<&'a str> {
    hand.seats
        .iter()
        .find(|s| s.player_name == name)
        .unwrap_or_else(|| panic!("{name} is not seated in hand {}", hand.hand_id))
        .hole_cards
        .as_deref()
}

/// `(hole_cards, bounty)` stored for `name` in hand `hand_id`.
fn stored_facts(conn: &Connection, hand_id: &str, name: &str) -> (Option<String>, Option<f64>) {
    conn.query_row(
        "SELECT ph.hole_cards, ph.bounty
           FROM player_hands ph
           JOIN hands h ON h.id = ph.hand_id
           JOIN players p ON p.id = ph.player_id
          WHERE h.hand_id = ?1 AND p.name = ?2",
        params![hand_id, name],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )
    .optional()
    .expect("query player_hands")
    .unwrap_or_else(|| panic!("no player_hands row for {name} in hand {hand_id}"))
}

fn stored_variant(conn: &Connection, hand_id: &str) -> Option<String> {
    conn.query_row(
        "SELECT variant FROM hands WHERE hand_id = ?1",
        params![hand_id],
        |row| row.get(0),
    )
    .expect("query hands.variant")
}

fn count(conn: &Connection, table: &str) -> i64 {
    conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
        row.get(0)
    })
    .expect("count rows")
}

/// Every row of `table`, every column, rendered exactly (`Value`'s `Debug`
/// keeps text bytes and the full `f64`), sorted so row order cannot matter.
fn snapshot(conn: &Connection, table: &str) -> Vec<String> {
    let mut stmt = conn
        .prepare(&format!("SELECT * FROM {table}"))
        .expect("prepare snapshot");
    let columns = stmt.column_count();
    let mut rows: Vec<String> = stmt
        .query_map([], |row| {
            let values: Vec<Value> = (0..columns)
                .map(|i| row.get::<_, Value>(i))
                .collect::<Result<_, _>>()?;
            Ok(format!("{values:?}"))
        })
        .expect("query snapshot")
        .collect::<Result<_, _>>()
        .expect("read snapshot");
    rows.sort();
    rows
}

// ---------------------------------------------------------------- D01 cards

#[test]
fn shown_and_mucked_cards_are_stored_per_player() {
    let hands = parse_all(SHOWDOWN);
    assert_eq!(hands.len(), 2);

    // Showdown: one player shows, the caller mucks (cards only in the summary).
    let showdown = &hands[0];
    assert_eq!(
        seat_cards(showdown, "Hero"),
        Some("9c 4h"),
        "hero's Dealt to cards"
    );
    assert_eq!(
        seat_cards(showdown, "Barrel_Bjorn"),
        Some("Ah Kh"),
        "shows / showed"
    );
    assert_eq!(
        seat_cards(showdown, "CallStation62"),
        Some("Kd Jc"),
        "mucked [..]"
    );
    for never_showed in ["quietnit", "Mr.Overbet", "punter_x"] {
        assert_eq!(
            seat_cards(showdown, never_showed),
            None,
            "{never_showed} never showed"
        );
    }

    // No showdown: the winner shows voluntarily after the pot is shipped.
    let voluntary = &hands[1];
    assert_eq!(seat_cards(voluntary, "Mr.Overbet"), Some("8c 7c"));
    assert_eq!(seat_cards(voluntary, "Hero"), Some("Qc 3d"));
    assert_eq!(
        seat_cards(voluntary, "Barrel_Bjorn"),
        None,
        "folded to the river bet"
    );

    // A spin all-in where both players show, and a Zoom cash muck.
    let spin = parse_all(SPIN);
    assert_eq!(seat_cards(&spin[1], "SpinCycle_9"), Some("Ah Kc"));
    assert_eq!(seat_cards(&spin[1], "Hero"), Some("6h 6d"));
    assert_eq!(seat_cards(&spin[1], "GrindHouse"), None);
    let zoom = parse_all(ZOOM_CASH);
    assert_eq!(seat_cards(&zoom[1], "lucky_pierre"), Some("8h 8s"));
    assert_eq!(seat_cards(&zoom[0], "lucky_pierre"), None);

    // Showing or mucking is not an action: 2 blinds + 6 preflop + 4 flop +
    // 2 turn + 2 river, and nothing for the show-down lines.
    assert_eq!(showdown.actions.len(), 16);

    // And all of it reaches player_hands.hole_cards.
    let mut conn = setup_db();
    import::import_text(&mut conn, SHOWDOWN).expect("import showdown fixture");
    assert_eq!(
        stored_facts(&conn, "262100000201", "Hero").0.as_deref(),
        Some("9c 4h")
    );
    assert_eq!(
        stored_facts(&conn, "262100000201", "Barrel_Bjorn")
            .0
            .as_deref(),
        Some("Ah Kh")
    );
    assert_eq!(
        stored_facts(&conn, "262100000201", "CallStation62")
            .0
            .as_deref(),
        Some("Kd Jc")
    );
    assert_eq!(stored_facts(&conn, "262100000201", "quietnit").0, None);
    assert_eq!(
        stored_facts(&conn, "262100000202", "Mr.Overbet")
            .0
            .as_deref(),
        Some("8c 7c")
    );
    assert_eq!(stored_facts(&conn, "262100000202", "Barrel_Bjorn").0, None);
}

#[test]
fn malformed_card_text_imports_with_null_cards() {
    let malformed = SHOWDOWN
        // A suit that does not exist, in both places Barrel_Bjorn's cards appear.
        .replace("shows [Ah Kh]", "shows [Ah Kx]")
        .replace("showed [Ah Kh]", "showed [Ah Kx]")
        // The same card twice.
        .replace("mucked [Kd Jc]", "mucked [Kd Kd]")
        // One card is not a hand; an unclosed bracket and non-ASCII junk never parse.
        .replace("Dealt to Hero [Qc 3d]", "Dealt to Hero [Qc]")
        .replace("Mr.Overbet: shows [8c 7c]", "Mr.Overbet: shows [8c 7♣");

    let hands = parse_all(&malformed);
    assert_eq!(hands.len(), 2, "malformed card text must not lose a hand");
    assert_eq!(seat_cards(&hands[0], "Barrel_Bjorn"), None);
    assert_eq!(seat_cards(&hands[0], "CallStation62"), None);
    assert_eq!(
        seat_cards(&hands[0], "Hero"),
        Some("9c 4h"),
        "a clean set elsewhere is unaffected"
    );
    assert_eq!(seat_cards(&hands[1], "Hero"), None);
    assert_eq!(seat_cards(&hands[1], "Mr.Overbet"), None);
    // The hero is still recognised from a one-card Dealt to line.
    assert_eq!(hands[1].hero_name.as_deref(), Some("Hero"));

    let mut conn = setup_db();
    let summary = import::import_text(&mut conn, &malformed).expect("import malformed");
    assert_eq!(summary.hands_imported, 2);
    assert_eq!(summary.hands_failed, 0);
    assert_eq!(stored_facts(&conn, "262100000201", "Barrel_Bjorn").0, None);
    assert_eq!(stored_facts(&conn, "262100000201", "CallStation62").0, None);
    assert_eq!(stored_facts(&conn, "262100000202", "Hero").0, None);
    assert_eq!(stored_facts(&conn, "262100000202", "Mr.Overbet").0, None);
}

#[test]
fn oversized_and_odd_card_sets_are_not_stored() {
    for (bad, why) in [
        ("[Ah Kh Qh Jh Th 9h]", "six cards"),
        ("[]", "empty"),
        ("[AhKh]", "no separator"),
        ("[ah kh]", "lower-case rank"),
        ("[10h Kh]", "ten written as 10"),
    ] {
        let text = SHOWDOWN.replace("[Ah Kh]", bad);
        let hand = parse_hand_block(&split_hands(&text)[0]).expect("hand still parses");
        assert_eq!(seat_cards(&hand, "Barrel_Bjorn"), None, "{why}: {bad}");
    }
}

// ---------------------------------------------------------------- D02 bounty

#[test]
fn seat_line_bounty_is_stored_per_player() {
    let hands = parse_all(BOUNTY_TOURNAMENT);
    let first = &hands[0];
    let bounty = |name: &str| {
        first
            .seats
            .iter()
            .find(|s| s.player_name == name)
            .unwrap()
            .bounty
    };
    assert_eq!(bounty("Player1"), Some(13.50));
    assert_eq!(bounty("Hero"), Some(13.50));
    assert_eq!(bounty("Player2"), Some(20.25));
    // A bounty seat that also says `is sitting out` keeps its bounty.
    let sitting_out = hands[1].seats.iter().find(|s| s.player_name == "Player1");
    assert_eq!(sitting_out.map(|s| s.bounty), Some(Some(13.50)));

    // Cash and non-bounty tournaments carry no bounty.
    for text in [SHOWDOWN, SPIN, ZOOM_CASH] {
        for hand in parse_all(text) {
            assert!(
                hand.seats.iter().all(|s| s.bounty.is_none()),
                "hand {}",
                hand.hand_id
            );
        }
    }

    let mut conn = setup_db();
    import::import_text(&mut conn, BOUNTY_TOURNAMENT).expect("import bounty fixture");
    import::import_text(&mut conn, SPIN).expect("import spin fixture");
    assert_eq!(
        stored_facts(&conn, "260993449710", "Player1").1,
        Some(13.50)
    );
    assert_eq!(
        stored_facts(&conn, "260993449710", "Player2").1,
        Some(20.25)
    );
    assert_eq!(stored_facts(&conn, "262000000101", "GrindHouse").1, None);
}

#[test]
fn malformed_bounty_imports_with_null_bounty() {
    let first_hand = split_hands(BOUNTY_TOURNAMENT).remove(0);
    let long_digits = "9".repeat(400);
    for bad in [
        "€13.5.0 bounty".to_string(),
        "€ bounty".to_string(),
        "€abc bounty".to_string(),
        "€1e3 bounty".to_string(),
        "€-13.50 bounty".to_string(),
        format!("€{long_digits} bounty"),
    ] {
        let text = first_hand.replace("€20.25 bounty", &bad);
        let hand = parse_hand_block(&text).expect("hand with a malformed bounty still parses");
        let player2 = hand
            .seats
            .iter()
            .find(|s| s.player_name == "Player2")
            .expect("seated");
        assert_eq!(player2.bounty, None, "{bad}");
        assert_eq!(
            player2.starting_stack, 24875.0,
            "{bad}: the stack is unaffected"
        );
        assert_eq!(
            hand.seats
                .iter()
                .find(|s| s.player_name == "Hero")
                .unwrap()
                .bounty,
            Some(13.50),
            "{bad}: other seats keep their bounty"
        );

        let mut conn = setup_db();
        let summary = import::import_text(&mut conn, &text).expect("import");
        assert_eq!(summary.hands_imported, 1, "{bad}");
        assert_eq!(
            stored_facts(&conn, "260993449710", "Player2").1,
            None,
            "{bad}"
        );
    }
}

// ---------------------------------------------------------------- D03 variant

#[test]
fn zoom_cash_hands_are_parsed_and_tagged() {
    let blocks = split_hands(ZOOM_CASH);
    assert_eq!(
        blocks.len(),
        2,
        "each 'PokerStars Zoom Hand #' starts a block"
    );

    let hands = parse_all(ZOOM_CASH);
    let first = &hands[0];
    assert_eq!(first.hand_id, "254100000001");
    assert_eq!(first.format, HandFormat::Cash);
    assert_eq!(first.variant, GameVariant::ZoomCash);
    assert_eq!(first.table_name, "Halley");
    assert_eq!(first.max_seats, 6);
    assert_eq!(first.button_seat, 1);
    assert_eq!(first.small_blind, 0.05);
    assert_eq!(first.big_blind, 0.10);
    assert_eq!(first.seats.len(), 6);
    assert_eq!(first.hero_name.as_deref(), Some("Hero"));
    assert!(
        first.actions.len() >= 9,
        "blinds, folds, open, call, c-bet, fold"
    );
    assert!(hands.iter().all(|h| h.variant == GameVariant::ZoomCash));

    // Zoom and regular cash hands mixed in one file split and tag correctly.
    let mixed = format!("{}\n\n\n{}", SHOWDOWN, ZOOM_CASH);
    let variants: Vec<GameVariant> = parse_all(&mixed).iter().map(|h| h.variant).collect();
    assert_eq!(
        variants,
        [
            GameVariant::Cash,
            GameVariant::Cash,
            GameVariant::ZoomCash,
            GameVariant::ZoomCash
        ]
    );

    let mut conn = setup_db();
    let summary = import::import_text(&mut conn, ZOOM_CASH).expect("import zoom cash");
    assert_eq!(summary.hands_imported, 2);
    assert_eq!(
        stored_variant(&conn, "254100000001").as_deref(),
        Some("zoom_cash")
    );
    let format: String = conn
        .query_row(
            "SELECT format FROM hands WHERE hand_id = '254100000002'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(format, HandFormat::Cash.as_str());
}

#[test]
fn spin_like_three_max_is_tagged_spin() {
    let hands = parse_all(SPIN);
    assert!(hands.iter().all(|h| h.format == HandFormat::Tournament));
    assert!(hands.iter().all(|h| h.variant == GameVariant::Spin));

    // A 3-max Zoom tournament with bounties is not a spin.
    assert!(parse_all(BOUNTY_TOURNAMENT)
        .iter()
        .all(|h| h.variant == GameVariant::ZoomTournament));
    // Nor is a non-Zoom 3-max whose buy-in has a bounty component.
    let ko_three_max = BOUNTY_TOURNAMENT.replace("Zoom Tournament #", "Tournament #");
    assert!(parse_all(&ko_three_max)
        .iter()
        .all(|h| h.variant == GameVariant::Tournament));
    // Nor a spin-priced tournament at a 6-max table.
    let six_max = SPIN.replace("3-max", "6-max");
    assert!(parse_all(&six_max)
        .iter()
        .all(|h| h.variant == GameVariant::Tournament));

    let mut conn = setup_db();
    import::import_text(&mut conn, SPIN).expect("import spin");
    assert_eq!(
        stored_variant(&conn, "262000000102").as_deref(),
        Some("spin")
    );
}

#[test]
fn every_existing_fixture_is_tagged_with_its_variant() {
    // None of the older fixtures is a spin (no 3-max non-Zoom tournament), so
    // the header alone names each one's variant.
    let mut seen = std::collections::BTreeSet::new();
    for text in EXISTING_FIXTURES {
        for hand in parse_all(text) {
            let header = hand.raw_text.lines().next().unwrap_or_default();
            let expected = if header.contains("Zoom Tournament #") {
                "zoom_tournament"
            } else if header.contains("Tournament #") {
                "tournament"
            } else {
                "cash"
            };
            assert_eq!(hand.variant.as_str(), expected, "hand {}", hand.hand_id);
            seen.insert(expected);
        }
    }
    assert_eq!(
        seen.len(),
        3,
        "the fixtures cover cash, tournament and zoom_tournament"
    );
}

// ---------------------------------------------------------------- import

#[test]
fn reimporting_the_new_fixtures_adds_no_hands() {
    let mut conn = setup_db();
    for text in [SHOWDOWN, ZOOM_CASH, SPIN] {
        let first = import::import_text(&mut conn, text).expect("first import");
        assert_eq!(first.hands_imported, 2);
        let again = import::import_text(&mut conn, text).expect("re-import");
        assert_eq!(again.hands_imported, 0);
        assert_eq!(again.hands_skipped_duplicate, 2);
    }
    assert_eq!(count(&conn, "hands"), 6);
}

// ---------------------------------------------------------------- D04 backfill

/// Rows a user owns. The backfill must leave every one of them byte-identical.
const USER_TABLES: [&str; 6] = [
    "player_notes",
    "player_color_overrides",
    "seat_positions",
    "hud_positions",
    "hud_profiles",
    "settings",
];

fn user_rows(conn: &Connection) -> Vec<(String, Vec<String>)> {
    USER_TABLES
        .iter()
        .map(|t| (t.to_string(), snapshot(conn, t)))
        .collect()
}

#[test]
fn reparse_backfill_is_idempotent_and_preserves_user_rows() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("velora.db");

    // A database written by the previous version: no `variant` / `bounty`
    // columns, `hole_cards` never written, the backfill flag never set, and
    // user rows in every table a user owns.
    {
        let mut conn = db::open(&path).expect("open new db");
        for text in EXISTING_FIXTURES {
            import::import_text(&mut conn, text).expect("import existing fixture");
        }
        for text in [SHOWDOWN, ZOOM_CASH, SPIN] {
            import::import_text(&mut conn, text).expect("import new fixture");
        }
        conn.execute_batch(
            "ALTER TABLE hands DROP COLUMN variant;
             ALTER TABLE player_hands DROP COLUMN bounty;
             UPDATE player_hands SET hole_cards = NULL;",
        )
        .expect("downgrade to the previous schema");
        conn.execute(
            "DELETE FROM settings WHERE key = ?1",
            params![db::HAND_FACTS_BACKFILL_FLAG],
        )
        .expect("clear flag");

        let player = |name: &str| -> i64 {
            conn.query_row(
                "SELECT id FROM players WHERE name = ?1",
                params![name],
                |r| r.get(0),
            )
            .expect("player exists")
        };
        let (barrel, overbet) = (player("Barrel_Bjorn"), player("Mr.Overbet"));
        conn.execute(
            "INSERT INTO player_notes (player_id, note, updated_at) VALUES (?1, ?2, ?3)",
            params![
                barrel,
                "barrels 3 streets, \"never\" folds — ünicode",
                "2026-09-01T10:00:00.123Z"
            ],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO player_color_overrides (player_id, color, label, updated_at)
             VALUES (?1, '#e5484d', 'Whale', '2026-09-02T11:00:00Z')",
            params![overbet],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO hud_positions (player_id, x, y, updated_at)
             VALUES (?1, 0.123456789012345, 0.987654321, '2026-09-03T12:00:00Z')",
            params![barrel],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO seat_positions (max_players, frame, seat_key, x, y, updated_at)
             VALUES (6, 'hero', 2, 0.3333333333333333, 0.71, '2026-09-04T13:00:00Z'),
                    (9, 'absolute', 7, 0.5, 0.05, '2026-09-04T13:00:01Z')",
            [],
        )
        .unwrap();
        db::set_setting(&conn, "min_hands", "31").unwrap();
    }

    let before_user = {
        let conn = Connection::open(&path).expect("raw open");
        let rows = user_rows(&conn);
        assert!(
            rows.iter().all(|(_, r)| !r.is_empty()),
            "every user table has rows: {rows:?}"
        );
        rows
    };
    let hands_before;
    let players_before;
    {
        let conn = Connection::open(&path).expect("raw open");
        hands_before = count(&conn, "hands");
        players_before = count(&conn, "players");
    }

    // First start of the new version: columns added, facts backfilled once.
    let conn = db::open(&path).expect("reopen migrates and backfills");
    assert_eq!(
        db::get_setting(&conn, db::HAND_FACTS_BACKFILL_FLAG)
            .unwrap()
            .as_deref(),
        Some("true")
    );
    assert_eq!(
        stored_facts(&conn, "262100000201", "Barrel_Bjorn")
            .0
            .as_deref(),
        Some("Ah Kh")
    );
    assert_eq!(
        stored_facts(&conn, "262100000201", "CallStation62")
            .0
            .as_deref(),
        Some("Kd Jc")
    );
    assert_eq!(stored_facts(&conn, "262100000201", "quietnit").0, None);
    assert_eq!(
        stored_facts(&conn, "260993449710", "Player2").1,
        Some(20.25)
    );
    assert_eq!(
        stored_variant(&conn, "254100000001").as_deref(),
        Some("zoom_cash")
    );
    assert_eq!(
        stored_variant(&conn, "262000000101").as_deref(),
        Some("spin")
    );
    assert_eq!(
        stored_variant(&conn, "260993449710").as_deref(),
        Some("zoom_tournament")
    );
    let null_variants: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM hands WHERE variant IS NULL",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        null_variants, 0,
        "every stored hand reparses and gets a variant"
    );
    assert_eq!(
        count(&conn, "hands"),
        hands_before,
        "no hand added or removed"
    );
    assert_eq!(
        count(&conn, "players"),
        players_before,
        "no player added or removed"
    );

    // User rows are byte-identical; the only settings change is the flag row.
    let after_user = user_rows(&conn);
    for ((table, before), (_, after)) in before_user.iter().zip(after_user.iter()) {
        if table == "settings" {
            let without_flag: Vec<&String> = after
                .iter()
                .filter(|r| !r.contains(db::HAND_FACTS_BACKFILL_FLAG))
                .collect();
            assert_eq!(
                without_flag,
                before.iter().collect::<Vec<_>>(),
                "settings rows changed"
            );
            assert_eq!(after.len(), before.len() + 1, "only the flag row is added");
        } else {
            assert_eq!(after, before, "{table} rows changed");
        }
    }

    // A second run changes nothing: the body writes nothing...
    let hands_snapshot = snapshot(&conn, "hands");
    let player_hands_snapshot = snapshot(&conn, "player_hands");
    let report = db::run_hand_facts_backfill(&conn).expect("second run");
    assert_eq!(report.hands_examined, hands_before);
    assert_eq!(report.hands_reparse_failed, 0);
    assert_eq!(report.hands_variant_written, 0);
    assert_eq!(report.player_rows_written, 0);
    drop(conn);

    // ...and a further start skips it behind the flag.
    let conn = db::open(&path).expect("third open");
    assert_eq!(snapshot(&conn, "hands"), hands_snapshot);
    assert_eq!(snapshot(&conn, "player_hands"), player_hands_snapshot);
    assert_eq!(user_rows(&conn), after_user);
}
