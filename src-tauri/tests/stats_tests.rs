use velora_poker_lib::{db, import, stats};

const SHOWDOWN_HAND: &str = include_str!("fixtures/hand_3bet_showdown.txt");
const CBET_FOLD_HAND: &str = include_str!("fixtures/hand_cbet_fold.txt");
const LIMPED_HAND: &str = include_str!("fixtures/hand_limped_multiway.txt");
const FOLD_TO_3BET_HAND: &str = include_str!("fixtures/hand_fold_to_3bet.txt");

fn setup_db() -> rusqlite::Connection {
    // The special ":memory:" filename gives an isolated in-memory database
    // per connection while still going through the real schema/init path.
    db::open(std::path::Path::new(":memory:")).expect("open db")
}

fn import_all_fixtures(conn: &mut rusqlite::Connection) {
    for text in [SHOWDOWN_HAND, CBET_FOLD_HAND, LIMPED_HAND, FOLD_TO_3BET_HAND] {
        import::import_text(conn, text).expect("import should succeed");
    }
}

fn player_id(conn: &rusqlite::Connection, name: &str) -> i64 {
    conn.query_row(
        "SELECT id FROM players WHERE name = ?1",
        [name],
        |row| row.get(0),
    )
    .unwrap_or_else(|_| panic!("player {name} not found"))
}

fn assert_close(actual: f64, expected: f64, label: &str) {
    assert!(
        (actual - expected).abs() < 0.05,
        "{label}: expected {expected}, got {actual}"
    );
}

#[test]
fn imports_all_fixture_hands_exactly_once() {
    let mut conn = setup_db();
    import_all_fixtures(&mut conn);
    assert_eq!(db::count_hands(&conn).unwrap(), 4);

    // Re-importing the same content must be a no-op (idempotent on hand id).
    import_all_fixtures(&mut conn);
    assert_eq!(db::count_hands(&conn).unwrap(), 4);
}

#[test]
fn computes_hero_statistics() {
    let mut conn = setup_db();
    import_all_fixtures(&mut conn);

    let hero_id = player_id(&conn, "Hero");
    let s = stats::compute_player_stats(&conn, hero_id).unwrap();

    assert_close(s.vpip, 100.0, "hero vpip");
    assert_close(s.pfr, 75.0, "hero pfr");
    assert_close(s.three_bet, 0.0, "hero three_bet");
    assert_close(s.fold_to_three_bet, 50.0, "hero fold_to_three_bet");
    assert_close(s.c_bet, 100.0, "hero c_bet");
    assert_close(s.fold_to_c_bet, 0.0, "hero fold_to_c_bet");
    assert_close(s.aggression_factor, 1.0, "hero aggression_factor");
    assert_close(s.wtsd, 33.3, "hero wtsd");
    assert_close(s.wsd, 0.0, "hero wsd");
}

#[test]
fn computes_villain_statistics() {
    let mut conn = setup_db();
    import_all_fixtures(&mut conn);

    let villain_id = player_id(&conn, "Villain");
    let s = stats::compute_player_stats(&conn, villain_id).unwrap();

    assert_close(s.vpip, 50.0, "villain vpip");
    assert_close(s.pfr, 0.0, "villain pfr");
    assert_close(s.three_bet, 0.0, "villain three_bet");
    assert_close(s.fold_to_three_bet, 0.0, "villain fold_to_three_bet");
    assert_close(s.c_bet, 0.0, "villain c_bet");
    assert_close(s.fold_to_c_bet, 100.0, "villain fold_to_c_bet");
    assert_close(s.aggression_factor, 1.0, "villain aggression_factor");
    assert_close(s.wtsd, 0.0, "villain wtsd");
    assert_close(s.wsd, 0.0, "villain wsd");
}

#[test]
fn computes_robot_statistics() {
    let mut conn = setup_db();
    import_all_fixtures(&mut conn);

    let robot_id = player_id(&conn, "Robot");
    let s = stats::compute_player_stats(&conn, robot_id).unwrap();

    assert_close(s.vpip, 50.0, "robot vpip");
    assert_close(s.pfr, 50.0, "robot pfr");
    assert_close(s.three_bet, 66.7, "robot three_bet");
    assert_close(s.fold_to_three_bet, 0.0, "robot fold_to_three_bet");
    assert_close(s.c_bet, 100.0, "robot c_bet");
    assert_close(s.fold_to_c_bet, 0.0, "robot fold_to_c_bet");
    assert_close(s.aggression_factor, 3.0, "robot aggression_factor");
    assert_close(s.wtsd, 50.0, "robot wtsd");
    assert_close(s.wsd, 100.0, "robot wsd");
}

#[test]
fn player_with_no_hands_reports_zeroed_stats() {
    let conn = setup_db();
    // No import performed; any player id is guaranteed to have zero hands.
    let s = stats::compute_player_stats(&conn, 999).unwrap();
    assert_eq!(s.vpip, 0.0);
    assert_eq!(s.pfr, 0.0);
    assert_eq!(s.aggression_factor, 0.0);
}
