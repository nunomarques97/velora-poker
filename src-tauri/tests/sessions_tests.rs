use velora_poker_lib::{db, import, sessions};

fn setup_db() -> rusqlite::Connection {
    db::open(std::path::Path::new(":memory:")).expect("open db")
}

/// First hand of session 1 (cash, table A). Hero raises preflop, wins
/// uncontested; net results are hand-verified to sum to exactly zero (no
/// rake) so the expected Hero net result below is trustworthy, not a guess.
/// Hero: +$0.75, Villain: -$0.25, Robot: -$0.50.
const SESSION1_HAND1: &str = r#"PokerStars Hand #400000000001: Hold'em No Limit ($0.25/$0.50 USD) - 2026/08/25 21:00:00 ET
Table 'SessionTableA' 3-max Seat #1 is the button
Seat 1: Hero ($50.00 in chips)
Seat 2: Villain ($50.00 in chips)
Seat 3: Robot ($50.00 in chips)
Villain: posts small blind $0.25
Robot: posts big blind $0.50
*** HOLE CARDS ***
Dealt to Hero [Ah Ad]
Hero: raises $1.50 to $2
Villain: folds
Robot: folds
Uncalled bet ($1.50) returned to Hero
Hero collected $1.25 from pot
*** SUMMARY ***
Total pot $1.25 | Rake $0
Seat 1: Hero (button) collected ($1.25)
Seat 2: Villain (small blind) folded before Flop
Seat 3: Robot (big blind) folded before Flop
"#;

/// Second hand of session 1, 15 minutes after `SESSION1_HAND1` (inside the
/// 30-minute gap threshold) but at a *different* table (cash, table B) — this
/// is what proves a session can span multiple tables. Hero posts the small
/// blind and folds to a raise; net results again hand-verified to sum to
/// zero. Hero: -$0.25, Villain: +$0.75, Robot: -$0.50.
const SESSION1_HAND2: &str = r#"PokerStars Hand #400000000002: Hold'em No Limit ($0.25/$0.50 USD) - 2026/08/25 21:15:00 ET
Table 'SessionTableB' 3-max Seat #1 is the button
Seat 1: Villain ($50.00 in chips)
Seat 2: Hero ($50.00 in chips)
Seat 3: Robot ($50.00 in chips)
Hero: posts small blind $0.25
Robot: posts big blind $0.50
*** HOLE CARDS ***
Dealt to Hero [2c 7d]
Villain: raises $1 to $1.50
Hero: folds
Robot: folds
Uncalled bet ($1) returned to Villain
Villain collected $1.25 from pot
*** SUMMARY ***
Total pot $1.25 | Rake $0
Seat 1: Villain (button) collected ($1.25)
Seat 2: Hero (small blind) folded before Flop
Seat 3: Robot (big blind) folded before Flop
"#;

/// A tournament hand played 45 minutes after `SESSION1_HAND2` — past the
/// 30-minute gap threshold, so this must start a new session. Tournament
/// hand-history text has no buy-in/finish/payout, so even though Hero wins
/// chips here, this session's `net_result_cash` must stay `None`.
const SESSION2_TOURNAMENT_HAND: &str = r#"PokerStars Hand #400000000003: Tournament #900000001, $10+$1 Hold'em No Limit - Level I (10/20) - 2026/08/25 22:00:00 ET
Table '900000001 1' 6-max Seat #1 is the button
Seat 1: Hero (1500 in chips)
Seat 2: Villain (1500 in chips)
Villain: posts small blind 10
Hero: posts big blind 20
*** HOLE CARDS ***
Dealt to Hero [Ah Ad]
Villain: folds
Uncalled bet (0) returned to Hero
Hero collected 20 from pot
*** SUMMARY ***
Total pot 20 | Rake 0
Seat 1: Hero (big blind) collected (20)
Seat 2: Villain (small blind) folded before Flop
"#;

fn import_all(conn: &mut rusqlite::Connection) {
    for text in [SESSION1_HAND1, SESSION1_HAND2, SESSION2_TOURNAMENT_HAND] {
        import::import_text(conn, text).expect("import should succeed");
    }
}

#[test]
fn contiguous_hands_across_tables_merge_into_one_session() {
    let mut conn = setup_db();
    import::import_text(&mut conn, SESSION1_HAND1).unwrap();
    import::import_text(&mut conn, SESSION1_HAND2).unwrap();

    let all = sessions::list_sessions(&conn).unwrap();
    assert_eq!(all.len(), 1, "two hands 15 minutes apart must be a single session");

    let session = &all[0];
    assert_eq!(session.hand_count, 2);
    assert_eq!(
        session.table_count, 2,
        "a session spanning two tables must report both, not collapse to one"
    );
    assert_eq!(session.start_at, "2026-08-25T21:00:00");
    assert_eq!(session.end_at, "2026-08-25T21:15:00");
    assert_eq!(session.duration_secs, 15 * 60);
}

#[test]
fn a_gap_over_30_minutes_splits_into_two_sessions() {
    let mut conn = setup_db();
    import_all(&mut conn);

    let all = sessions::list_sessions(&conn).unwrap();
    assert_eq!(
        all.len(),
        2,
        "a 45-minute gap between the cash hands and the tournament hand must start a new session"
    );
}

#[test]
fn cash_session_net_result_matches_hand_verified_expectation() {
    let mut conn = setup_db();
    import::import_text(&mut conn, SESSION1_HAND1).unwrap();
    import::import_text(&mut conn, SESSION1_HAND2).unwrap();

    let all = sessions::list_sessions(&conn).unwrap();
    let session = &all[0];

    assert!(session.has_cash);
    assert!(!session.has_tournament);
    let net = session.net_result_cash.expect("cash session must report a net result");
    assert!(
        (net - 0.50).abs() < 1e-9,
        "expected Hero's net result to be +0.75 - 0.25 = +0.50, got {net}"
    );
    assert_eq!(session.currency.as_deref(), Some("USD"));
}

#[test]
fn tournament_session_never_reports_a_fabricated_net_result() {
    let mut conn = setup_db();
    import::import_text(&mut conn, SESSION2_TOURNAMENT_HAND).unwrap();

    let all = sessions::list_sessions(&conn).unwrap();
    assert_eq!(all.len(), 1);
    let session = &all[0];

    assert!(session.has_tournament);
    assert!(!session.has_cash);
    assert_eq!(
        session.net_result_cash, None,
        "a tournament-only session must never show a net result, even though Hero won chips in it"
    );
    assert_eq!(session.currency, None);
}

#[test]
fn sessions_are_returned_most_recent_first() {
    let mut conn = setup_db();
    import_all(&mut conn);

    let all = sessions::list_sessions(&conn).unwrap();
    assert_eq!(all.len(), 2);
    assert!(
        all[0].start_at > all[1].start_at,
        "the tournament session (22:00) must come before the earlier cash session (21:00)"
    );
}

#[test]
fn no_sessions_today_returns_none_instead_of_a_zeroed_card() {
    let conn = setup_db();
    let today = chrono::NaiveDate::from_ymd_opt(2026, 8, 25).unwrap();
    assert_eq!(sessions::sessions_today(&conn, today).unwrap(), None);
}

#[test]
fn sessions_today_aggregates_only_sessions_that_started_today() {
    let mut conn = setup_db();
    import_all(&mut conn);

    let today = chrono::NaiveDate::from_ymd_opt(2026, 8, 25).unwrap();
    let summary = sessions::sessions_today(&conn, today)
        .unwrap()
        .expect("both sessions started today");

    assert_eq!(summary.session_count, 2);
    assert_eq!(summary.total_hands, 3);
    let net = summary
        .net_result_cash
        .expect("the cash session today must contribute a net result");
    assert!((net - 0.50).abs() < 1e-9);

    let other_day = chrono::NaiveDate::from_ymd_opt(2026, 8, 26).unwrap();
    assert_eq!(sessions::sessions_today(&conn, other_day).unwrap(), None);
}
