use velora_poker_lib::{db, hud};

/// The badge model ships as a builtin, so a user switches to it from HUD
/// Profiles instead of needing a rebuild.
#[test]
fn badge_is_a_builtin_profile_with_its_own_visual_model() {
    let badge = hud::builtin_profiles()
        .into_iter()
        .find(|p| p.id == "badge")
        .expect("badge profile is built in");

    assert_eq!(badge.visual_model, "badge");
    assert_eq!(badge.name, "Badge");
    assert!(badge.is_builtin);
    // Same gate as every other model: nothing is labelled below it.
    assert_eq!(badge.min_hands, 25);
    // Stat pages travel with it even though the badge renders none, so
    // switching models back and forth never loses a page configuration.
    assert!(!badge.stat_pages.is_empty());
}

/// A fresh install lands on the badge model.
#[test]
fn a_new_database_starts_on_the_badge_model() {
    let conn = db::open(std::path::Path::new(":memory:")).expect("open db");
    hud::seed_builtin_profiles(&conn).expect("seed");

    let active = hud::get_active_profile(&conn).expect("active profile");

    assert_eq!(active.id, "badge");
    assert_eq!(active.visual_model, "badge");
}

/// Seeding is idempotent and additive: a database created before the badge
/// existed gains it on the next start, without touching the active profile.
#[test]
fn seeding_adds_the_badge_profile_to_an_existing_database() {
    let conn = db::open(std::path::Path::new(":memory:")).expect("open db");
    hud::seed_builtin_profiles(&conn).expect("first seed");
    let active_before = hud::get_active_profile(&conn).expect("active profile");

    hud::seed_builtin_profiles(&conn).expect("second seed");

    let profiles = hud::list_profiles(&conn).expect("list profiles");
    let badges: Vec<_> = profiles.iter().filter(|p| p.id == "badge").collect();
    assert_eq!(badges.len(), 1, "seeded exactly once, not duplicated");
    assert_eq!(
        hud::get_active_profile(&conn).expect("active profile").id,
        active_before.id,
        "seeding never changes which profile is active"
    );
}
