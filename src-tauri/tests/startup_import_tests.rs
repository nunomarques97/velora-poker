//! Startup and watcher robustness (mock-session report, item 2 [orig 3]):
//! one file that cannot be read (locked, deleted or moved mid-scan) must not
//! abort the backlog import, a poisoned database lock must not silently stop
//! it, and the parser status must say what actually happened instead of
//! claiming "watching" before any watcher exists.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use velora_poker_lib::{db, import};

const SHOWDOWN_HAND: &str = include_str!("fixtures/hand_3bet_showdown.txt");
const CBET_FOLD_HAND: &str = include_str!("fixtures/hand_cbet_fold.txt");

#[test]
fn a_file_that_vanished_mid_scan_is_skipped_and_counted_not_fatal() {
    let dir = tempfile::tempdir().expect("temp dir");
    let first = dir.path().join("a_first.txt");
    let vanished = dir.path().join("b_vanished.txt");
    let last = dir.path().join("c_last.txt");
    std::fs::write(&first, SHOWDOWN_HAND).unwrap();
    std::fs::write(&last, CBET_FOLD_HAND).unwrap();

    let mut conn = db::open(std::path::Path::new(":memory:")).expect("open db");
    let paths: Vec<PathBuf> = vec![first, vanished, last];
    let summary = import::import_files(&mut conn, &paths);

    assert_eq!(summary.hands_imported, 2, "files after the bad one still import");
    assert_eq!(summary.files_failed, 1);
    assert_eq!(db::count_hands(&conn).unwrap(), 2);
}

#[cfg(windows)]
#[test]
fn a_file_locked_by_another_process_does_not_abort_the_directory_import() {
    use std::os::windows::fs::OpenOptionsExt;

    let dir = tempfile::tempdir().expect("temp dir");
    std::fs::write(dir.path().join("a_first.txt"), SHOWDOWN_HAND).unwrap();
    std::fs::write(dir.path().join("b_locked.txt"), "locked").unwrap();
    std::fs::write(dir.path().join("c_last.txt"), CBET_FOLD_HAND).unwrap();

    // share_mode(0): no other handle may read the file while this one is open,
    // the same sharing violation a writer holding the file exclusively causes.
    let _lock = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(dir.path().join("b_locked.txt"))
        .expect("open exclusively");

    let mut conn = db::open(std::path::Path::new(":memory:")).expect("open db");
    let summary = import::import_directory(&mut conn, dir.path()).expect("import directory");

    assert_eq!(summary.hands_imported, 2);
    assert_eq!(summary.files_failed, 1);
    assert_eq!(db::count_hands(&conn).unwrap(), 2);
}

#[test]
fn backlog_import_recovers_a_poisoned_database_lock() {
    let dir = tempfile::tempdir().expect("temp dir");
    std::fs::write(dir.path().join("session.txt"), SHOWDOWN_HAND).unwrap();

    let conn = Arc::new(Mutex::new(
        db::open(std::path::Path::new(":memory:")).expect("open db"),
    ));
    {
        let conn = Arc::clone(&conn);
        let _ = std::thread::spawn(move || {
            let _guard = conn.lock().unwrap();
            panic!("poison the lock on purpose");
        })
        .join();
    }
    assert!(conn.is_poisoned());

    let summary = import::run_backlog_import(&conn, dir.path());

    assert_eq!(summary.hands_imported, 1);
    let guard = conn.lock().unwrap_or_else(|p| p.into_inner());
    assert_eq!(db::count_hands(&guard).unwrap(), 1);
}

#[test]
fn status_is_watching_only_once_the_watcher_started() {
    let ok = import::ImportSummary::default();
    assert_eq!(import::status_after_startup(&ok, None), "watching");

    let failed = import::status_after_startup(&ok, Some("access denied"));
    assert!(failed.starts_with("error:"), "{failed}");
    assert!(failed.contains("access denied"), "{failed}");
}

#[test]
fn status_reports_skipped_files_while_still_watching() {
    let summary = import::ImportSummary {
        files_failed: 2,
        ..Default::default()
    };
    let status = import::status_after_startup(&summary, None);
    assert!(status.starts_with("watching"), "{status}");
    assert!(status.contains("2 files could not be read"), "{status}");

    // A watcher that failed to start wins: that is the HUD-dead case.
    let status = import::status_after_startup(&summary, Some("boom"));
    assert!(status.starts_with("error:"), "{status}");
}

#[test]
fn watcher_runtime_errors_turn_the_status_red_and_a_later_import_clears_it() {
    let status = import::status_for_watcher_error("buffer overflow");
    assert!(status.starts_with("error: watcher:"), "{status}");
    assert!(status.contains("buffer overflow"), "{status}");

    assert_eq!(
        import::status_after_watched_import(&status).as_deref(),
        Some("watching")
    );
    // Any other status is left alone by a successful watched import.
    assert_eq!(import::status_after_watched_import("watching"), None);
    assert_eq!(
        import::status_after_watched_import("error: folder not found"),
        None
    );
}
