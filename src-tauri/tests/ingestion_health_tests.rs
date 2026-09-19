use std::thread::sleep;
use std::time::Duration;

use velora_poker_lib::import::validate::{Problem, Severity};
use velora_poker_lib::{commands, db};

fn setup_db() -> rusqlite::Connection {
    db::open(std::path::Path::new(":memory:")).expect("open db")
}

/// A database that never saw a single import problem must report all-zero
/// health instead of failing or inventing a number.
#[test]
fn empty_database_reports_zero_ingestion_health_without_failing() {
    let conn = setup_db();

    let health = commands::ingestion_health(&conn).expect("ingestion health must not error");

    assert_eq!(health.hands_imported, 0);
    assert_eq!(health.hands_rejected, 0);
    assert_eq!(health.hands_with_warnings, 0);
    assert!(health.problems.is_empty());
    assert!(health.last_import_at.is_none());
}

/// A rejected hand and a warned hand, written the way the real import
/// pipeline writes them (`db::record_import_problems`), must show up counted
/// by severity, with a real `firstSeenAt`/`lastSeenAt` and a non-empty,
/// product-language explanation — never blank, never an in-memory number
/// that a restart could lose.
#[test]
fn a_recorded_reject_and_warn_appear_with_counts_and_explanations() {
    let conn = setup_db();

    let reject = vec![Problem {
        severity: Severity::Reject,
        code: "no_dealt_in_players",
        detail: "hand reject-1 parsed 0 seat lines".to_string(),
    }];
    let warn = vec![Problem {
        severity: Severity::Warn,
        code: "no_actions",
        detail: "hand warn-1 has no parsed actions".to_string(),
    }];
    db::record_import_problems(&conn, "reject-1", &reject).expect("record reject");
    db::record_import_problems(&conn, "warn-1", &warn).expect("record warn");

    let health = commands::ingestion_health(&conn).expect("ingestion health must not error");

    assert_eq!(health.hands_rejected, 1);
    assert_eq!(health.hands_with_warnings, 1);
    assert!(health.last_import_at.is_some());

    let reject_row = health
        .problems
        .iter()
        .find(|p| p.code == "no_dealt_in_players")
        .expect("the reject row must be present");
    assert_eq!(reject_row.severity, "reject");
    assert_eq!(reject_row.count, 1);
    assert!(!reject_row.first_seen_at.is_empty());
    assert!(!reject_row.last_seen_at.is_empty());
    assert!(
        !reject_row.explanation.is_empty(),
        "a known code must never explain itself with an empty string"
    );
    assert!(
        !reject_row.explanation.contains("parser") && !reject_row.explanation.contains("seat"),
        "the visible explanation must not leak parser jargon: {}",
        reject_row.explanation
    );

    let warn_row = health
        .problems
        .iter()
        .find(|p| p.code == "no_actions")
        .expect("the warn row must be present");
    assert_eq!(warn_row.severity, "warn");
    assert_eq!(warn_row.count, 1);
    assert!(!warn_row.explanation.is_empty());
}

/// Two occurrences of the same code, recorded a little apart in time, must
/// aggregate into one row whose count is 2 and whose `firstSeenAt` is no
/// later than its `lastSeenAt` — the whole point of extending
/// `import_problem_summary` with `MIN`/`MAX(detected_at)` instead of only
/// ever showing the latest occurrence.
#[test]
fn repeated_code_aggregates_count_and_brackets_first_and_last_seen() {
    let conn = setup_db();

    let problem = |detail: &str| {
        vec![Problem {
            severity: Severity::Reject,
            code: "missing_hand_id",
            detail: detail.to_string(),
        }]
    };
    db::record_import_problems(&conn, "reject-a", &problem("first occurrence")).expect("record 1");
    sleep(Duration::from_millis(5));
    db::record_import_problems(&conn, "reject-b", &problem("second occurrence")).expect("record 2");

    let health = commands::ingestion_health(&conn).expect("ingestion health must not error");
    assert_eq!(health.hands_rejected, 2);

    let row = health
        .problems
        .iter()
        .find(|p| p.code == "missing_hand_id")
        .expect("aggregated row must be present");
    assert_eq!(row.count, 2);
    assert!(
        row.first_seen_at <= row.last_seen_at,
        "first_seen_at ({}) must not be after last_seen_at ({})",
        row.first_seen_at,
        row.last_seen_at
    );
}

/// A code the explanation map has not caught up with must still fall into a
/// generic, honest sentence — never an empty string.
#[test]
fn unknown_code_falls_back_to_a_non_empty_generic_explanation() {
    let conn = setup_db();
    let problem = vec![Problem {
        severity: Severity::Warn,
        code: "some_future_check_not_in_the_map_yet",
        detail: "detail".to_string(),
    }];
    db::record_import_problems(&conn, "warn-unknown", &problem).expect("record");

    let health = commands::ingestion_health(&conn).expect("ingestion health must not error");
    let row = health
        .problems
        .iter()
        .find(|p| p.code == "some_future_check_not_in_the_map_yet")
        .expect("unknown-code row must still be present");
    assert!(!row.explanation.is_empty());
}

/// `get_app_version` must report the exact version Cargo built the binary
/// with (i.e. `src-tauri/Cargo.toml`'s `version`), and `features` must match
/// exactly the feature flags this test binary itself was compiled with — the
/// same `#[cfg(feature = ...)]` gates the app's own classification/
/// description-rules modules use, so a distributed (`--no-default-features`)
/// build reports an empty list.
#[test]
fn app_version_matches_cargo_toml_version_and_compiled_features() {
    let payload = commands::app_version();

    assert_eq!(payload.version, env!("CARGO_PKG_VERSION"));
    assert_eq!(payload.version, "0.1.0", "src-tauri/Cargo.toml's version field");

    let expected_features: Vec<String> = {
        #[allow(unused_mut)]
        let mut features = Vec::new();
        #[cfg(feature = "auto-classification")]
        features.push("auto-classification".to_string());
        #[cfg(feature = "strategic-analysis")]
        features.push("strategic-analysis".to_string());
        features
    };

    assert_eq!(payload.features, expected_features);
}
