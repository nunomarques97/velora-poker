use velora_poker_lib::{db, import};

/// An older import (e.g. an original mock/test-data batch) with a
/// roster ("Villain", "Robot") that has nothing to do with the table played
/// most recently.
const OLD_MOCK_HAND: &str = include_str!("fixtures/hand_cbet_fold.txt");

/// A later hand at a different table with a different, overlapping-on-Hero
/// roster — this should be the only one the overlay renders.
const CURRENT_TABLE_HAND: &str = r#"PokerStars Hand #200000000099: Hold'em No Limit ($0.25/$0.50 USD) - 2026/08/20 22:00:00 ET
Table 'Atlas III' 3-max Seat #1 is the button
Seat 1: Hero ($48.50 in chips)
Seat 2: Regular1 ($49.75 in chips)
Seat 3: Regular2 ($50.00 in chips)
Regular1: posts small blind $0.25
Regular2: posts big blind $0.50
*** HOLE CARDS ***
Dealt to Hero [Qs Qh]
Hero: raises $1 to $1.50
Regular1: calls $1.25
Regular2: folds
*** FLOP *** [Qc 4d 9h]
Hero: bets $2
Regular1: folds
Uncalled bet ($0) returned to Hero
Hero collected $3.75 from pot
*** SUMMARY ***
Total pot $3.75 | Rake $0.25
Board [Qc 4d 9h]
Seat 1: Hero (button) collected ($3.75)
Seat 2: Regular1 (small blind) folded on the Flop
Seat 3: Regular2 (big blind) folded before Flop
"#;

/// Regression: an old sitting at a table *name* PokerStars later reuses —
/// finished well before the table window Velora is tracking now was ever
/// opened.
const OLD_SITTING_HAND: &str = r#"PokerStars Hand #400000000001: Hold'em No Limit ($0.25/$0.50 USD) - 2026/08/20 10:00:00 ET
Table 'Reused Table' 3-max Seat #1 is the button
Seat 1: Hero ($48.50 in chips)
Seat 2: OldOpponentA ($49.75 in chips)
Seat 3: OldOpponentB ($50.00 in chips)
OldOpponentA: posts small blind $0.25
OldOpponentB: posts big blind $0.50
*** HOLE CARDS ***
Dealt to Hero [Qs Qh]
Hero: raises $1 to $1.50
OldOpponentA: calls $1.25
OldOpponentB: folds
*** FLOP *** [Qc 4d 9h]
Hero: bets $2
OldOpponentA: folds
Uncalled bet ($0) returned to Hero
Hero collected $3.75 from pot
*** SUMMARY ***
Total pot $3.75 | Rake $0.25
Board [Qc 4d 9h]
Seat 1: Hero (button) collected ($3.75)
Seat 2: OldOpponentA (small blind) folded on the Flop
Seat 3: OldOpponentB (big blind) folded before Flop
"#;

/// Regression: the *same table name* reopened later, a fresh sitting
/// with a different roster — what the reopened table window actually shows.
const FRESH_SITTING_HAND: &str = r#"PokerStars Hand #400000000002: Hold'em No Limit ($0.25/$0.50 USD) - 2026/08/20 15:00:00 ET
Table 'Reused Table' 3-max Seat #1 is the button
Seat 1: Hero ($48.50 in chips)
Seat 2: NewOpponentA ($49.75 in chips)
Seat 3: NewOpponentB ($50.00 in chips)
NewOpponentA: posts small blind $0.25
NewOpponentB: posts big blind $0.50
*** HOLE CARDS ***
Dealt to Hero [Ac Ad]
Hero: raises $1 to $1.50
NewOpponentA: calls $1.25
NewOpponentB: folds
*** FLOP *** [2c 4d 9h]
Hero: bets $2
NewOpponentA: folds
Uncalled bet ($0) returned to Hero
Hero collected $3.75 from pot
*** SUMMARY ***
Total pot $3.75 | Rake $0.25
Board [2c 4d 9h]
Seat 1: Hero (button) collected ($3.75)
Seat 2: NewOpponentA (small blind) folded on the Flop
Seat 3: NewOpponentB (big blind) folded before Flop
"#;

/// Multi-table scenario: a hand at "Table Alpha", 3-max.
const TABLE_ALPHA_HAND: &str = r#"PokerStars Hand #300000000001: Hold'em No Limit ($0.25/$0.50 USD) - 2026/08/20 22:00:00 ET
Table 'Table Alpha' 3-max Seat #1 is the button
Seat 1: Hero ($48.50 in chips)
Seat 2: Regular1 ($49.75 in chips)
Seat 3: Regular2 ($50.00 in chips)
Regular1: posts small blind $0.25
Regular2: posts big blind $0.50
*** HOLE CARDS ***
Dealt to Hero [Qs Qh]
Hero: raises $1 to $1.50
Regular1: calls $1.25
Regular2: folds
*** FLOP *** [Qc 4d 9h]
Hero: bets $2
Regular1: folds
Uncalled bet ($0) returned to Hero
Hero collected $3.75 from pot
*** SUMMARY ***
Total pot $3.75 | Rake $0.25
Board [Qc 4d 9h]
Seat 1: Hero (button) collected ($3.75)
Seat 2: Regular1 (small blind) folded on the Flop
Seat 3: Regular2 (big blind) folded before Flop
"#;

/// Multi-table scenario: a *different, simultaneously open* table
/// ("Table Beta", 6-max) whose hand finishes a few minutes *after* Table
/// Alpha's — simulating a user multi-tabling (several tournament tables open
/// at once). Before table-name scoping was added, completing this
/// hand would make Table Beta's roster the *only* one
/// `list_active_table_players_with_seats` could return, even for an overlay
/// still sitting on Table Alpha's window.
const TABLE_BETA_HAND: &str = r#"PokerStars Hand #300000000002: Hold'em No Limit ($0.25/$0.50 USD) - 2026/08/20 22:05:00 ET
Table 'Table Beta' 6-max Seat #1 is the button
Seat 1: Hero ($48.50 in chips)
Seat 2: Regular3 ($49.75 in chips)
Seat 3: Regular4 ($50.00 in chips)
Regular3: posts small blind $0.25
Regular4: posts big blind $0.50
*** HOLE CARDS ***
Dealt to Hero [Ac Ad]
Hero: raises $1 to $1.50
Regular3: calls $1.25
Regular4: folds
*** FLOP *** [2c 4d 9h]
Hero: bets $2
Regular3: folds
Uncalled bet ($0) returned to Hero
Hero collected $3.75 from pot
*** SUMMARY ***
Total pot $3.75 | Rake $0.25
Board [2c 4d 9h]
Seat 1: Hero (button) collected ($3.75)
Seat 2: Regular3 (small blind) folded on the Flop
Seat 3: Regular4 (big blind) folded before Flop
"#;

fn setup_db() -> rusqlite::Connection {
    db::open(std::path::Path::new(":memory:")).expect("open db")
}

#[test]
fn active_table_players_excludes_stale_import_history() {
    let mut conn = setup_db();
    import::import_text(&mut conn, OLD_MOCK_HAND).expect("import old mock hand");
    import::import_text(&mut conn, CURRENT_TABLE_HAND).expect("import current table hand");

    let all_players: Vec<String> = db::list_players(&conn)
        .unwrap()
        .into_iter()
        .map(|p| p.name)
        .collect();
    assert!(
        all_players.contains(&"Villain".to_string()) && all_players.contains(&"Robot".to_string()),
        "the full roster (Players view / HUD preview) must still include stale mock-data players"
    );

    let active = db::list_active_table_players(&conn).unwrap();
    let active_names: Vec<String> = active.iter().map(|p| p.name.clone()).collect();
    assert_eq!(
        active.len(),
        3,
        "the active table should only have the 3 seats from the most recently imported hand, got {active_names:?}"
    );
    assert!(active_names.contains(&"Hero".to_string()));
    assert!(active_names.contains(&"Regular1".to_string()));
    assert!(active_names.contains(&"Regular2".to_string()));
    assert!(
        !active_names.contains(&"Villain".to_string()) && !active_names.contains(&"Robot".to_string()),
        "stale mock-data players from an earlier import must never appear in the overlay's player list"
    );

    let hero = active.iter().find(|p| p.name == "Hero").expect("Hero present");
    assert_eq!(
        hero.hands, 2,
        "hand count shown on the card must stay the player's all-time total, not just 1 for the filtered hand"
    );
}

#[test]
fn active_table_players_empty_when_no_hands_imported() {
    let conn = setup_db();
    assert!(db::list_active_table_players(&conn).unwrap().is_empty());
}

/// Regression: a cold-start backlog import walks the filesystem in
/// OS directory order, not chronological order, so files can be imported (and
/// therefore get their `hands.id` assigned) out of play order. The active
/// table must be resolved by `played_at`, not by insertion order — otherwise
/// whichever hand happened to be read last by the filesystem walk wins, which
/// can be an old hand while a chronologically later one already exists in the
/// DB. Here the later-played hand is imported FIRST (so it gets the lower
/// `id`) to prove `id DESC` alone would pick the wrong one.
#[test]
fn active_table_players_uses_play_order_not_insertion_order() {
    let mut conn = setup_db();
    import::import_text(&mut conn, CURRENT_TABLE_HAND).expect("import current table hand");
    import::import_text(&mut conn, OLD_MOCK_HAND).expect("import old mock hand");

    let active = db::list_active_table_players(&conn).unwrap();
    let active_names: Vec<String> = active.iter().map(|p| p.name.clone()).collect();
    assert_eq!(
        active.len(),
        3,
        "the active table should resolve to the later-played hand's 3 seats regardless of import order, got {active_names:?}"
    );
    assert!(active_names.contains(&"Regular1".to_string()));
    assert!(active_names.contains(&"Regular2".to_string()));
    assert!(
        !active_names.contains(&"Villain".to_string()) && !active_names.contains(&"Robot".to_string()),
        "an earlier-played hand imported after a later one must not win just because it was inserted last"
    );
}

/// Root-cause regression: without a table-name scope, a hand completing
/// on a *different* simultaneously-open table (Table Beta, played later)
/// hijacks the globally-latest-hand query away from Table Alpha — this is
/// the pre-scoping behavior, kept as a test so nobody "fixes" the fallback path
/// into silently ignoring `table_name` again.
#[test]
fn unscoped_active_table_players_follows_whichever_table_played_most_recently() {
    let mut conn = setup_db();
    import::import_text(&mut conn, TABLE_ALPHA_HAND).expect("import table alpha hand");
    import::import_text(&mut conn, TABLE_BETA_HAND).expect("import table beta hand");

    let active = db::list_active_table_players_with_seats(&conn, None, None).unwrap();
    let active_names: Vec<String> = active.iter().map(|p| p.name.clone()).collect();
    assert!(
        active_names.contains(&"Regular3".to_string()) && active_names.contains(&"Regular4".to_string()),
        "with no table scope, the globally most-recent hand (Table Beta) wins, got {active_names:?}"
    );
    assert!(
        !active_names.contains(&"Regular1".to_string()),
        "Table Alpha's players must not appear once a later hand exists on another table, got {active_names:?}"
    );
}

/// Scoping by the tracked table's name keeps each table's own
/// roster stable regardless of what's happening on a different,
/// simultaneously-open table — the actual behavior `get_active_table_players`
/// now relies on via `table_track::active_table_name()`.
#[test]
fn scoped_active_table_players_ignores_hands_on_other_tables() {
    let mut conn = setup_db();
    import::import_text(&mut conn, TABLE_ALPHA_HAND).expect("import table alpha hand");
    import::import_text(&mut conn, TABLE_BETA_HAND).expect("import table beta hand");

    let alpha = db::list_active_table_players_with_seats(&conn, Some("Table Alpha"), None).unwrap();
    let alpha_names: Vec<String> = alpha.iter().map(|p| p.name.clone()).collect();
    assert_eq!(alpha.len(), 3, "got {alpha_names:?}");
    assert!(alpha_names.contains(&"Regular1".to_string()));
    assert!(alpha_names.contains(&"Regular2".to_string()));
    assert!(
        !alpha_names.contains(&"Regular3".to_string()) && !alpha_names.contains(&"Regular4".to_string()),
        "Table Beta's later hand must not leak into Table Alpha's scoped roster, got {alpha_names:?}"
    );

    let beta = db::list_active_table_players_with_seats(&conn, Some("Table Beta"), None).unwrap();
    let beta_names: Vec<String> = beta.iter().map(|p| p.name.clone()).collect();
    assert!(beta_names.contains(&"Regular3".to_string()));
    assert!(beta_names.contains(&"Regular4".to_string()));

    assert_eq!(
        db::active_table_max_players(&conn, Some("Table Alpha"), None).unwrap(),
        Some(3),
        "max-players must also be scoped to the tracked table, not the global latest hand"
    );
    assert_eq!(
        db::active_table_max_players(&conn, Some("Table Beta"), None).unwrap(),
        Some(6)
    );

    let alpha_hand = db::active_hand_info(&conn, Some("Table Alpha"), None)
        .unwrap()
        .expect("alpha hand");
    assert_eq!(alpha_hand.table_name.as_deref(), Some("Table Alpha"));
    let beta_hand = db::active_hand_info(&conn, Some("Table Beta"), None)
        .unwrap()
        .expect("beta hand");
    assert_eq!(beta_hand.table_name.as_deref(), Some("Table Beta"));
}

/// A tracked table with no hands imported yet (e.g. the user just opened
/// it) must render an empty roster rather than falling back to a different
/// table's players — silence is the correct, honest failure mode here per
/// the product's "never show wrong data" priority, not a bug to paper over.
#[test]
fn scoped_active_table_players_is_empty_for_a_table_with_no_hands_yet() {
    let mut conn = setup_db();
    import::import_text(&mut conn, TABLE_ALPHA_HAND).expect("import table alpha hand");

    let empty = db::list_active_table_players_with_seats(&conn, Some("Table Gamma"), None).unwrap();
    assert!(
        empty.is_empty(),
        "a table with no imported hands must show no players, not another table's roster"
    );
    assert_eq!(db::active_table_max_players(&conn, Some("Table Gamma"), None).unwrap(), None);
}

/// Regression: PokerStars reuses table *names* from a pool, so
/// `table_name = ?` alone cannot tell an old sitting apart from a fresh one.
/// Simulates the actual bug — the tracked table window was first seen at
/// 12:00, *after* the old sitting (10:00) finished, and no fresh hand has
/// been dealt yet at this reopened table. Without the `since` bound this old
/// hand would resolve as "the active hand" with full visual confidence.
#[test]
fn active_hand_excludes_a_sitting_older_than_the_tracked_tables_own_first_seen_at() {
    let mut conn = setup_db();
    import::import_text(&mut conn, OLD_SITTING_HAND).expect("import old sitting");

    let since = "2026-08-20T12:00:00";

    let players =
        db::list_active_table_players_with_seats(&conn, Some("Reused Table"), Some(since)).unwrap();
    let names: Vec<String> = players.iter().map(|p| p.name.clone()).collect();
    assert!(
        players.is_empty(),
        "a hand played before this table's own first_seen_at must not resolve as the active \
         hand, got {names:?}"
    );

    let hand = db::active_hand_info(&conn, Some("Reused Table"), Some(since)).unwrap();
    assert!(
        hand.is_none(),
        "a stale sitting must resolve the same way as no hand imported yet, got hand_id={:?}",
        hand.map(|h| h.hand_id)
    );
}

/// Same reused-name setup, but with a genuinely fresh hand dealt *after*
/// `first_seen_at` — the fix must not also swallow current data, only the
/// stale sitting that came before it.
#[test]
fn active_hand_includes_a_sitting_newer_than_the_tracked_tables_own_first_seen_at() {
    let mut conn = setup_db();
    import::import_text(&mut conn, OLD_SITTING_HAND).expect("import old sitting");
    import::import_text(&mut conn, FRESH_SITTING_HAND).expect("import fresh sitting");

    let since = "2026-08-20T12:00:00";

    let players =
        db::list_active_table_players_with_seats(&conn, Some("Reused Table"), Some(since)).unwrap();
    let names: Vec<String> = players.iter().map(|p| p.name.clone()).collect();
    assert_eq!(players.len(), 3, "got {names:?}");
    assert!(names.contains(&"NewOpponentA".to_string()));
    assert!(names.contains(&"NewOpponentB".to_string()));
    assert!(
        !names.contains(&"OldOpponentA".to_string()) && !names.contains(&"OldOpponentB".to_string()),
        "the old sitting's players must never leak into the fresh sitting's roster, got {names:?}"
    );

    let hand = db::active_hand_info(&conn, Some("Reused Table"), Some(since))
        .unwrap()
        .expect("the fresh hand must resolve as the active hand");
    assert_eq!(hand.hand_id, "400000000002");
}

/// A fresh database seeds every table size from 2 to 10 seats, not just
/// the six measured 6-max rows — multi-table tournaments are typically
/// 9-max, so a 9-max table must never fall back to `OverlayApp`'s
/// stacking grid.
#[test]
fn nine_max_table_is_seeded_with_nine_distinct_seat_offsets() {
    let conn = setup_db();

    let nine_max = db::list_seat_templates(&conn, 9).unwrap();
    assert_eq!(nine_max.len(), 9, "expected all 9 offsets seeded, got {nine_max:?}");

    let mut offsets: Vec<i64> = nine_max.iter().map(|row| row.seat_offset).collect();
    offsets.sort_unstable();
    offsets.dedup();
    assert_eq!(
        offsets,
        (0..9).collect::<Vec<i64>>(),
        "every offset 0..9 must have its own distinct derived position"
    );
}

/// Seeding runs on every `db::open`, not just the first — a user who
/// dragged a seat on a derived (non-6-max) table size must keep that position
/// forever, exactly like the already-covered 6-max case.
#[test]
fn seeding_never_overwrites_a_users_dragged_position_on_a_derived_table_size() {
    let dir = tempfile::tempdir().expect("temp dir");
    let db_path = dir.path().join("velora.db");

    {
        let conn = db::open(&db_path).expect("first open seeds the derived layout");
        db::set_seat_template(&conn, 9, 2, 0.5, 0.5).expect("save the user's own drag");
    }

    // Reopening re-runs `seed_defaults`/`seed_builtin_seat_templates` on the
    // same database — the regression this guards is the derived-ellipse pass
    // clobbering a user's own row the same way the measured 6-max pass
    // already never does.
    let conn = db::open(&db_path).expect("second open re-seeds");
    assert_eq!(
        db::get_seat_template(&conn, 9, 2).unwrap(),
        Some((0.5, 0.5)),
        "a user-dragged position on a derived table size must survive reseeding"
    );
}

/// The on-screen bound for seeded seat positions, *and* the one place where
/// two requirements on those positions contradict each other, resolved here
/// in code.
///
/// A seat card is anchored at its top-left corner, so a position too close to
/// an edge draws part of the card outside the overlay window. Hence two
/// bounds, deliberately different:
///
/// - **Hard bound — every size 2..=10, 6-max included:** `0.0 < x,y < 1.0`.
///   Outside this a card is born off the overlay entirely. Nothing may ever
///   be seeded there, measured or derived.
/// - **Safe margin `0.02..=0.92`** — the intended margin
///   "for every size 2..10". Every *derived* position satisfies it by
///   construction (`db::ellipse_seat_template` clamps to it). Exactly one
///   seeded position does not, and cannot be made to: the hand-measured
///   `BUILTIN_SEAT_TEMPLATES` row `(6, 1)` is `x = 0.01080431`, and
///   the six measured rows must stay byte-for-byte identical. The margin and
///   the measured rows cannot both hold literally for 6-max; the measured
///   rows win, because those six points are the live-measured
///   calibration every derived layout is fitted against, and a real
///   client does render a seat there.
///
/// So the safe margin is asserted as a guarantee of the *generator*, and the
/// single measured exception is asserted explicitly instead of being
/// skipped: if a future change ever adds a second position outside the
/// margin, or moves this one, this test fails and the conflict comes back to
/// a person rather than shipping silently.
#[test]
fn every_table_size_stays_on_screen_and_only_the_measured_six_max_row_leaves_the_safe_margin() {
    let conn = setup_db();

    const SAFE_MIN: f64 = 0.02;
    const SAFE_MAX: f64 = 0.92;
    let mut outside_safe_margin: Vec<(i64, i64, f64, f64)> = Vec::new();

    for max_players in 2..=10i64 {
        let rows = db::list_seat_templates(&conn, max_players).unwrap();
        assert_eq!(
            rows.len(),
            max_players as usize,
            "{max_players}-max must have one row per seat"
        );
        for row in rows {
            assert!(
                row.x > 0.0 && row.x < 1.0 && row.y > 0.0 && row.y < 1.0,
                "{max_players}-max offset {}: ({}, {}) would place a card off the overlay \
                 window entirely",
                row.seat_offset,
                row.x,
                row.y
            );
            if !(SAFE_MIN..=SAFE_MAX).contains(&row.x)
                || !(SAFE_MIN..=SAFE_MAX).contains(&row.y)
            {
                outside_safe_margin.push((max_players, row.seat_offset, row.x, row.y));
            }
        }
    }

    assert_eq!(
        outside_safe_margin,
        vec![(6, 1, 0.01080431, 0.49984758)],
        "the hand-measured 6-max row (6, 1) is the only seeded position allowed outside \
         the {SAFE_MIN}..={SAFE_MAX} safe margin (the measured rows are frozen \
         byte-for-byte); every derived position must be inside it"
    );
}

/// A reused table name reopening before a fresh hand deals
/// must not resolve `max_seats` from an earlier sitting either — the same
/// recency floor `active_hand_info`/`list_active_table_players_with_seats`
/// already apply. Without `since`, this would still return the old sitting's
/// `3`, which is merely correct by coincidence here (both sittings share the
/// same size); the real-world failure this bound prevents is a name reused at
/// a *different* max-players format, so the important assertion is that the
/// pre-first-hand answer is `None`, exactly like "no hand imported yet".
#[test]
fn active_table_max_players_excludes_a_sitting_older_than_the_tracked_tables_own_first_seen_at() {
    let mut conn = setup_db();
    import::import_text(&mut conn, OLD_SITTING_HAND).expect("import old sitting");

    let since = "2026-08-20T12:00:00";

    assert_eq!(
        db::active_table_max_players(&conn, Some("Reused Table"), Some(since)).unwrap(),
        None,
        "a stale sitting's max_seats must not leak into a table that has not dealt a fresh hand yet"
    );

    // Unbounded, the same query still resolves the stale sitting — proves the
    // assertion above is actually exercising the `since` floor, not just
    // reflecting an empty table.
    assert_eq!(
        db::active_table_max_players(&conn, Some("Reused Table"), None).unwrap(),
        Some(3)
    );
}
