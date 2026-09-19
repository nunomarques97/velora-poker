use velora_poker_lib::commands::onboarding_readiness;

const ENGLISH_WALK_HAND: &str = include_str!("fixtures/hand_walk.txt");
const NON_ENGLISH_CLIENT_HAND: &str = include_str!("fixtures/onboarding_non_english_client.txt");

/// A real, already-existing English-client fixture in a folder by itself:
/// the sample file must produce `parsedHandCount > 0` and `isEnglish: true`,
/// with a non-empty explanation and the sample file path recorded.
#[test]
fn english_fixture_folder_reports_parsed_hands_and_is_english() {
    let dir = tempfile::tempdir().expect("temp dir");
    std::fs::write(dir.path().join("session.txt"), ENGLISH_WALK_HAND).unwrap();

    let readiness = onboarding_readiness(Some(dir.path().to_string_lossy().to_string()), true);

    assert!(readiness.folder.exists);
    // `hand_file_count` is a pass-through of `settings::validate_hand_history_dir`
    // — this test asserts against its output, not a recomputed
    // count, so it only pins the lower bound the one file guarantees.
    assert!(readiness.folder.hand_file_count >= 1);
    assert_eq!(readiness.folder.parsed_hand_count, 1);
    assert!(!readiness.folder.message.is_empty());

    assert!(readiness.client_language.checked);
    assert!(readiness.client_language.is_english);
    assert!(!readiness.client_language.reason.is_empty());
    assert!(readiness
        .client_language
        .sample_file
        .as_deref()
        .unwrap()
        .ends_with("session.txt"));

    assert!(readiness.auto_center.enabled);
}

/// A file with a real PokerStars header/table/seat block (so it parses into
/// at least one hand) but whose action lines use no English verb at all —
/// the case this check exists for: never guess "English" off a checkbox, catch the
/// client's real language from the text itself.
#[test]
fn non_english_client_fixture_is_english_false_with_a_reason() {
    let dir = tempfile::tempdir().expect("temp dir");
    std::fs::write(dir.path().join("session.txt"), NON_ENGLISH_CLIENT_HAND).unwrap();

    let readiness = onboarding_readiness(Some(dir.path().to_string_lossy().to_string()), false);

    assert!(readiness.folder.exists);
    assert!(readiness.folder.parsed_hand_count > 0, "header/table/seats still parse fine");

    assert!(readiness.client_language.checked);
    assert!(!readiness.client_language.is_english);
    assert!(
        !readiness.client_language.reason.is_empty(),
        "a false verdict must always explain why"
    );
    assert!(readiness.client_language.reason.contains("English"));
}

/// An existing, empty folder: nothing to sample, so the language check must
/// report `checked: false` (never a guessed `false`) rather than pretending
/// to have tested anything, and the hand/file counts must stay at zero
/// rather than being invented.
#[test]
fn empty_folder_reports_unchecked_language_and_zero_counts() {
    let dir = tempfile::tempdir().expect("temp dir");

    let readiness = onboarding_readiness(Some(dir.path().to_string_lossy().to_string()), false);

    assert!(readiness.folder.exists);
    assert_eq!(readiness.folder.hand_file_count, 0);
    assert_eq!(readiness.folder.parsed_hand_count, 0);
    assert!(!readiness.folder.message.is_empty());

    assert!(!readiness.client_language.checked);
    assert!(!readiness.client_language.is_english);
    assert!(readiness.client_language.sample_file.is_none());
    assert!(!readiness.client_language.reason.is_empty());
}

/// A folder path that does not exist on disk at all must never panic and
/// must never report invented numbers — `exists: false`, zero counts, and
/// the language check left unchecked.
#[test]
fn nonexistent_folder_does_not_panic_and_reports_not_ready() {
    let dir = tempfile::tempdir().expect("temp dir");
    let missing = dir.path().join("this-folder-does-not-exist");

    let readiness = onboarding_readiness(Some(missing.to_string_lossy().to_string()), false);

    assert!(!readiness.folder.exists);
    assert_eq!(readiness.folder.hand_file_count, 0);
    assert_eq!(readiness.folder.parsed_hand_count, 0);
    assert!(!readiness.folder.message.is_empty());

    assert!(!readiness.client_language.checked);
    assert!(!readiness.client_language.is_english);
    assert!(!readiness.client_language.reason.is_empty());

    assert_eq!(readiness.folder.path.as_deref(), Some(missing.to_string_lossy()).as_deref());
}

/// A folder that exists and holds `.txt` files, but none of them nested under
/// a screen-name subdirectory this time — sanity check that the recursive
/// scan (same traversal as `import::collect_txt_files`) also works for the
/// plain, non-nested layout used by the other tests above, and that the
/// hand count really reflects hands, not files, when a folder holds more
/// than one file.
#[test]
fn folder_with_multiple_files_counts_hands_in_the_most_recent_file_only() {
    let dir = tempfile::tempdir().expect("temp dir");
    std::fs::write(dir.path().join("older.txt"), NON_ENGLISH_CLIENT_HAND).unwrap();
    std::fs::write(dir.path().join("newer.txt"), ENGLISH_WALK_HAND).unwrap();

    // Force a clear mtime ordering: "newer.txt" must be the most recently
    // modified file regardless of how fast this test runs. `File::set_modified`
    // is Rust stdlib (stable since 1.75) — no new dependency.
    let now = std::time::SystemTime::now();
    let older_time = now - std::time::Duration::from_secs(60);
    std::fs::OpenOptions::new()
        .write(true)
        .open(dir.path().join("older.txt"))
        .expect("open older.txt for writing")
        .set_modified(older_time)
        .expect("set older mtime");
    std::fs::OpenOptions::new()
        .write(true)
        .open(dir.path().join("newer.txt"))
        .expect("open newer.txt for writing")
        .set_modified(now)
        .expect("set newer mtime");

    let readiness = onboarding_readiness(Some(dir.path().to_string_lossy().to_string()), false);

    assert!(readiness.folder.hand_file_count >= 2);
    assert_eq!(readiness.folder.parsed_hand_count, 1);
    assert!(readiness.client_language.checked);
    assert!(readiness.client_language.is_english);
    assert!(readiness
        .client_language
        .sample_file
        .as_deref()
        .unwrap()
        .ends_with("newer.txt"));
}
