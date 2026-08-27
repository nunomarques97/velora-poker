use velora_poker_lib::{db, import, stats};

const SHOWDOWN_ALLIN: &str = include_str!("fixtures/tournament_showdown_allin.txt");
const UNCONTESTED_WALK: &str = include_str!("fixtures/tournament_uncontested_walk.txt");
const ALLIN_PREFLOP_DISCONNECT: &str =
    include_str!("fixtures/tournament_allin_preflop_disconnect.txt");
const ZOOM_HEADER: &str = include_str!("fixtures/tournament_zoom_header.txt");

const CASH_SHOWDOWN: &str = include_str!("fixtures/hand_3bet_showdown.txt");

fn setup_db() -> rusqlite::Connection {
    db::open(std::path::Path::new(":memory:")).expect("open db")
}

fn import_all_tournament_fixtures(conn: &mut rusqlite::Connection) {
    for text in [
        SHOWDOWN_ALLIN,
        UNCONTESTED_WALK,
        ALLIN_PREFLOP_DISCONNECT,
        ZOOM_HEADER,
    ] {
        import::import_text(conn, text).expect("import should succeed");
    }
}

fn player_id(conn: &rusqlite::Connection, name: &str) -> i64 {
    conn.query_row("SELECT id FROM players WHERE name = ?1", [name], |row| {
        row.get(0)
    })
    .unwrap_or_else(|_| panic!("player {name} not found"))
}

fn assert_close(actual: f64, expected: f64, label: &str) {
    assert!(
        (actual - expected).abs() < 0.05,
        "{label}: expected {expected}, got {actual}"
    );
}

#[test]
fn imports_tournament_hands_and_stores_format() {
    let mut conn = setup_db();
    import_all_tournament_fixtures(&mut conn);
    assert_eq!(db::count_hands(&conn).unwrap(), 4);

    let formats: Vec<String> = conn
        .prepare("SELECT format FROM hands ORDER BY format")
        .unwrap()
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert!(formats.iter().all(|f| f == "tournament"));

    let tournament_ids: Vec<Option<String>> = conn
        .prepare("SELECT tournament_id FROM hands ORDER BY id")
        .unwrap()
        .query_map([], |row| row.get::<_, Option<String>>(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert!(tournament_ids.iter().all(|t| t.is_some()));
}

#[test]
fn prevents_duplicate_tournament_hand_import() {
    let mut conn = setup_db();
    import_all_tournament_fixtures(&mut conn);
    assert_eq!(db::count_hands(&conn).unwrap(), 4);

    // Re-import the exact same fixtures (e.g. the watcher re-reading a file
    // after a benign filesystem event) must not create duplicates.
    let summary = import::import_text(&mut conn, SHOWDOWN_ALLIN).unwrap();
    assert_eq!(summary.hands_imported, 0);
    assert_eq!(summary.hands_skipped_duplicate, 1);
    assert_eq!(db::count_hands(&conn).unwrap(), 4);
}

#[test]
fn computes_stats_for_tournament_only_player() {
    let mut conn = setup_db();
    import_all_tournament_fixtures(&mut conn);

    let id = player_id(&conn, "TourneyHero");
    let s = stats::compute_player_stats(&conn, id).unwrap();

    assert_close(s.vpip, 50.0, "TourneyHero vpip");
    assert_close(s.pfr, 50.0, "TourneyHero pfr");
    assert_close(s.three_bet, 0.0, "TourneyHero three_bet");
    assert_close(s.fold_to_three_bet, 0.0, "TourneyHero fold_to_three_bet");
    assert_close(s.c_bet, 0.0, "TourneyHero c_bet");
    assert_close(s.fold_to_c_bet, 0.0, "TourneyHero fold_to_c_bet");
    assert_close(s.aggression_factor, 0.0, "TourneyHero aggression_factor");
    assert_close(s.wtsd, 100.0, "TourneyHero wtsd");
    assert_close(s.wsd, 0.0, "TourneyHero wsd");
}

#[test]
fn cash_and_tournament_hands_coexist_without_cross_contamination() {
    let mut conn = setup_db();
    import::import_text(&mut conn, CASH_SHOWDOWN).unwrap();
    import_all_tournament_fixtures(&mut conn);

    // 1 cash hand + 4 tournament hands, all distinct hand ids.
    assert_eq!(db::count_hands(&conn).unwrap(), 5);

    // A cash-only player has no tournament hands mixed in, and vice versa.
    let hero_id = player_id(&conn, "Hero");
    let hero_stats = stats::compute_player_stats(&conn, hero_id).unwrap();
    assert_close(hero_stats.vpip, 100.0, "hero vpip unaffected by tournaments");

    let TourneyHero_id = player_id(&conn, "TourneyHero");
    let TourneyHero_stats = stats::compute_player_stats(&conn, TourneyHero_id).unwrap();
    assert_close(TourneyHero_stats.vpip, 50.0, "TourneyHero vpip unaffected by cash hands");
}
