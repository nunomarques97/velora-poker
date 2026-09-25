use rusqlite::{params, Connection};
use velora_poker_lib::{db, hud};

fn open_db() -> Connection {
    db::open(std::path::Path::new(":memory:")).expect("open db")
}

/// Puts a freshly opened database back into the shape of an install from
/// before the profiles were consolidated: the old builtin rows present, the
/// consolidation flag unset, and `active_id` active with `min_hands`.
fn legacy_install(active_id: &str, min_hands: i64) -> Connection {
    let conn = open_db();
    conn.execute(
        "DELETE FROM settings WHERE key = ?1",
        params![hud::PROFILES_CONSOLIDATED_FLAG],
    )
    .unwrap();
    // A pre-consolidation database never had the compact row.
    conn.execute("DELETE FROM hud_profiles WHERE id = 'compact'", []).unwrap();
    for (id, name, model) in [
        ("velora-hud", "Velora HUD", "velora_hud"),
        ("velora-classic", "Velora Classic", "velora_classic"),
        ("minimal", "Minimal", "minimal"),
        ("jivaro", "Jivaro", "jivaro"),
    ] {
        conn.execute(
            "INSERT OR IGNORE INTO hud_profiles (id, name, visual_model, stat_pages, min_hands, is_builtin, created_at)
             VALUES (?1, ?2, ?3, '[]', 25, 1, ?4)",
            params![id, name, model, db::now_iso()],
        )
        .unwrap();
    }
    if active_id == "jivaro-inspired" {
        // The oldest name of velora-hud; its row is renamed on start.
        conn.execute("DELETE FROM hud_profiles WHERE id = 'velora-hud'", []).unwrap();
        conn.execute(
            "INSERT INTO hud_profiles (id, name, visual_model, stat_pages, min_hands, is_builtin, created_at)
             VALUES ('jivaro-inspired', 'Jivaro-inspired', 'jivaro_inspired', '[]', 25, 1, ?1)",
            params![db::now_iso()],
        )
        .unwrap();
    }
    hud::set_profile_min_hands(&conn, active_id, min_hands).unwrap();
    hud::set_active_profile(&conn, active_id).unwrap();
    conn
}

fn profile_ids(conn: &Connection) -> Vec<String> {
    let mut ids: Vec<String> = hud::list_profiles(conn)
        .unwrap()
        .into_iter()
        .map(|p| p.id)
        .collect();
    ids.sort();
    ids
}

/// Exactly two builtin models remain: Compact (the default) and Badge.
#[test]
fn compact_and_badge_are_the_only_builtin_profiles() {
    let builtins = hud::builtin_profiles();
    let ids: Vec<&str> = builtins.iter().map(|p| p.id.as_str()).collect();
    assert_eq!(ids, vec!["compact", "badge"]);

    let compact = &builtins[0];
    assert_eq!(compact.name, "Compact");
    assert_eq!(compact.visual_model, "compact");
    let badge = &builtins[1];
    assert_eq!(badge.name, "Badge");
    assert_eq!(badge.visual_model, "badge");

    for profile in &builtins {
        assert!(profile.is_builtin);
        // Same gate on both: nothing is labelled below it.
        assert_eq!(profile.min_hands, 25);
        // Both carry the stat pages, so switching models never loses a page
        // configuration; Compact renders the first one.
        assert!(!profile.stat_pages.is_empty());
    }
}

/// A fresh install lands on Compact and never sees a retired model.
#[test]
fn a_new_database_starts_on_the_compact_model() {
    let conn = open_db();
    hud::seed_builtin_profiles(&conn).expect("seed");

    let active = hud::get_active_profile(&conn).expect("active profile");
    assert_eq!(active.id, "compact");
    assert_eq!(active.visual_model, "compact");
    assert_eq!(active.min_hands, 25);
    assert_eq!(profile_ids(&conn), vec!["badge", "compact"]);
}

/// Seeding is idempotent and never changes which profile is active.
#[test]
fn seeding_twice_changes_nothing() {
    let conn = open_db();
    hud::set_active_profile(&conn, "badge").unwrap();
    hud::seed_builtin_profiles(&conn).expect("first seed");
    hud::seed_builtin_profiles(&conn).expect("second seed");

    assert_eq!(profile_ids(&conn), vec!["badge", "compact"]);
    assert_eq!(hud::get_active_profile(&conn).unwrap().id, "badge");
}

/// Every retired builtin id moves its user to Compact, carrying the
/// sample-size gate they chose, and the retired rows are gone for good.
#[test]
fn each_retired_active_profile_moves_to_compact_with_its_min_hands() {
    for (i, retired) in hud::RETIRED_PROFILE_IDS.iter().enumerate() {
        let min_hands = 40 + i as i64;
        let conn = legacy_install(retired, min_hands);

        hud::seed_builtin_profiles(&conn).expect("seed");

        let active = hud::get_active_profile(&conn).unwrap();
        assert_eq!(active.id, "compact", "{retired}");
        assert_eq!(active.visual_model, "compact", "{retired}");
        assert_eq!(active.min_hands, min_hands, "{retired}: min_hands carried over");
        assert_eq!(profile_ids(&conn), vec!["badge", "compact"], "{retired}: retired rows removed");
        assert_eq!(
            db::get_setting(&conn, hud::PROFILES_CONSOLIDATED_FLAG).unwrap().as_deref(),
            Some("true")
        );
    }
}

/// Running the migration again — or starting the app again — changes
/// nothing: a later min_hands edit and a later switch to Badge both stick,
/// and no retired row is seeded back.
#[test]
fn the_profile_migration_is_idempotent() {
    let conn = legacy_install("velora-classic", 50);
    hud::seed_builtin_profiles(&conn).unwrap();
    let after_first = hud::list_profiles(&conn).unwrap();

    hud::consolidate_profiles_to_compact(&conn).unwrap();
    hud::seed_builtin_profiles(&conn).unwrap();
    let after_second = hud::list_profiles(&conn).unwrap();
    assert_eq!(
        after_first.iter().map(|p| (&p.id, p.min_hands)).collect::<Vec<_>>(),
        after_second.iter().map(|p| (&p.id, p.min_hands)).collect::<Vec<_>>(),
    );

    hud::set_profile_min_hands(&conn, "compact", 30).unwrap();
    hud::set_active_profile(&conn, "badge").unwrap();
    hud::seed_builtin_profiles(&conn).unwrap();
    assert_eq!(hud::get_active_profile(&conn).unwrap().id, "badge");
    assert_eq!(hud::get_profile(&conn, "compact").unwrap().unwrap().min_hands, 30);
    assert_eq!(profile_ids(&conn), vec!["badge", "compact"]);
}

/// A Badge user keeps Badge and its own gate; only the retired rows go.
#[test]
fn a_badge_user_is_left_on_badge() {
    let conn = legacy_install("badge", 35);
    hud::seed_builtin_profiles(&conn).unwrap();

    let active = hud::get_active_profile(&conn).unwrap();
    assert_eq!(active.id, "badge");
    assert_eq!(active.min_hands, 35);
    assert_eq!(hud::get_profile(&conn, "compact").unwrap().unwrap().min_hands, 25);
    assert_eq!(profile_ids(&conn), vec!["badge", "compact"]);
}

/// A user's own profile on a retired model keeps its id, name and gate but
/// renders as Compact; one on a surviving model is untouched.
#[test]
fn custom_profiles_on_a_retired_model_are_remapped_to_compact() {
    let conn = legacy_install("velora-hud", 25);
    for (id, model) in [("mine-jivaro", "jivaro"), ("mine-minimal", "minimal"), ("mine-badge", "badge")] {
        conn.execute(
            "INSERT INTO hud_profiles (id, name, visual_model, stat_pages, min_hands, is_builtin, created_at)
             VALUES (?1, ?1, ?2, '[]', 60, 0, ?3)",
            params![id, model, db::now_iso()],
        )
        .unwrap();
    }
    hud::set_active_profile(&conn, "mine-jivaro").unwrap();

    hud::seed_builtin_profiles(&conn).unwrap();

    let active = hud::get_active_profile(&conn).unwrap();
    assert_eq!(active.id, "mine-jivaro", "a custom active profile stays active");
    assert_eq!(active.visual_model, "compact");
    assert_eq!(active.min_hands, 60);
    assert_eq!(hud::get_profile(&conn, "mine-minimal").unwrap().unwrap().visual_model, "compact");
    assert_eq!(hud::get_profile(&conn, "mine-badge").unwrap().unwrap().visual_model, "badge");
    assert_eq!(
        profile_ids(&conn),
        vec!["badge", "compact", "mine-badge", "mine-jivaro", "mine-minimal"]
    );
}
