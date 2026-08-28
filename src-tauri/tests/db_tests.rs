use velora_poker_lib::{db, import};

/// An older import (e.g. the user's original mock/test-data batch) with a
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

/// regression: a cold-start backlog import walks the filesystem in
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
