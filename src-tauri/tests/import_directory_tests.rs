use velora_poker_lib::{db, import};

const SHOWDOWN_HAND: &str = include_str!("fixtures/hand_3bet_showdown.txt");
const CBET_FOLD_HAND: &str = include_str!("fixtures/hand_cbet_fold.txt");

#[test]
fn imports_every_txt_file_in_a_directory_exactly_once() {
    let dir = tempfile::tempdir().expect("temp dir");
    std::fs::write(dir.path().join("session_one.txt"), SHOWDOWN_HAND).unwrap();
    std::fs::write(dir.path().join("session_two.txt"), CBET_FOLD_HAND).unwrap();
    // A non-hand-history file in the same folder must be ignored, not error.
    std::fs::write(dir.path().join("notes.md"), "not a hand history").unwrap();

    let mut conn = db::open(std::path::Path::new(":memory:")).expect("open db");

    let summary = import::import_directory(&mut conn, dir.path()).expect("import directory");
    assert_eq!(summary.hands_imported, 2);
    assert_eq!(db::count_hands(&conn).unwrap(), 2);

    // Simulating the watcher re-reading a file it already imported (e.g. on
    // a benign filesystem modify event) must not duplicate hands.
    let rescan = import::import_directory(&mut conn, dir.path()).expect("re-import directory");
    assert_eq!(rescan.hands_imported, 0);
    assert_eq!(rescan.hands_skipped_duplicate, 2);
    assert_eq!(db::count_hands(&conn).unwrap(), 2);
}

#[test]
fn settings_round_trip_through_the_database() {
    let conn = db::open(std::path::Path::new(":memory:")).expect("open db");
    assert_eq!(
        db::get_setting(&conn, "hand_history_dir").unwrap(),
        None
    );

    db::set_setting(&conn, "hand_history_dir", "C:\\HandHistory").unwrap();
    assert_eq!(
        db::get_setting(&conn, "hand_history_dir").unwrap(),
        Some("C:\\HandHistory".to_string())
    );

    db::set_setting(&conn, "hand_history_dir", "D:\\Other").unwrap();
    assert_eq!(
        db::get_setting(&conn, "hand_history_dir").unwrap(),
        Some("D:\\Other".to_string())
    );
}
