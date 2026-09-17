use velora_poker_lib::classification::{self, PlayerClassification};
use velora_poker_lib::db;
use velora_poker_lib::stats::PlayerStats;

fn stats(vpip: f64, pfr: f64, three_bet: f64) -> PlayerStats {
    PlayerStats {
        vpip: Some(vpip),
        pfr: Some(pfr),
        three_bet: Some(three_bet),
        fold_to_three_bet: Some(0.0),
        four_bet: Some(0.0),
        fold_to_four_bet: Some(0.0),
        rfi: Some(0.0),
        limp: Some(0.0),
        cold_call: Some(0.0),
        squeeze: Some(0.0),
        fold_to_squeeze: Some(0.0),
        c_bet: Some(0.0),
        fold_to_c_bet: Some(0.0),
        aggression_factor: Some(0.0),
        wtsd: Some(0.0),
        wsd: Some(0.0),
        steal_attempt: Some(0.0),
        fold_to_steal: Some(0.0),
    }
}

#[test]
fn below_min_hands_is_always_unknown() {
    let rules = classification::builtin_rules();
    // Stats that would otherwise be a clear maniac, but only 10 hands.
    let result = classification::classify(&rules, 10, &stats(60.0, 40.0, 20.0));
    assert_eq!(result.classification, PlayerClassification::Unknown);
}

#[test]
fn classifies_loose_passive() {
    let rules = classification::builtin_rules();
    let result = classification::classify(&rules, 100, &stats(45.0, 4.0, 1.0));
    assert_eq!(result.classification, PlayerClassification::LoosePassive);
}

#[test]
fn classifies_tight_aggressive() {
    let rules = classification::builtin_rules();
    let result = classification::classify(&rules, 100, &stats(19.0, 17.0, 11.0));
    assert_eq!(result.classification, PlayerClassification::TightAggressive);
}

#[test]
fn classifies_maniac() {
    let rules = classification::builtin_rules();
    let result = classification::classify(&rules, 100, &stats(55.0, 40.0, 18.0));
    assert_eq!(result.classification, PlayerClassification::Maniac);
}

#[test]
fn classifies_loose_aggressive() {
    let rules = classification::builtin_rules();
    let result = classification::classify(&rules, 100, &stats(35.0, 25.0, 8.0));
    assert_eq!(result.classification, PlayerClassification::LooseAggressive);
}

#[test]
fn falls_back_to_recreational() {
    let rules = classification::builtin_rules();
    // Moderate VPIP/PFR: doesn't match Maniac/LAG/LP/TAG, and isn't tight
    // enough (vpip <= 15 && pfr <= 10) to match Nitty/Rock either.
    let result = classification::classify(&rules, 100, &stats(22.0, 8.0, 1.0));
    assert_eq!(result.classification, PlayerClassification::Recreational);
}

/// The real gap Nitty/Rock (Phase 1) closes: a genuine nit — VPIP 10%, PFR
/// 5% — used to fall into Recreational for lack of a matching rule. These
/// are the exact stats `falls_back_to_recreational` used before this rule
/// was added.
#[test]
fn classifies_nitty_rock() {
    let rules = classification::builtin_rules();
    let result = classification::classify(&rules, 100, &stats(10.0, 5.0, 1.0));
    assert_eq!(result.classification, PlayerClassification::NittyRock);
}

#[test]
fn nitty_rock_does_not_match_just_over_its_pfr_boundary() {
    let rules = classification::builtin_rules();
    let result = classification::classify(&rules, 100, &stats(10.0, 10.1, 1.0));
    assert_ne!(result.classification, PlayerClassification::NittyRock);
}

/// PokerStars' HUD rules (per Hand2Note's own compliance manual) prohibit
/// automatic classification colors/labels; the `auto-classification` Cargo
/// feature gates whether `resolve_for_player` is allowed to surface them.
/// Without the feature, a player with no manual override must serialize as
/// the same neutral Unknown shape as a genuinely unclassified player — never
/// a leaked archetype — regardless of how clearly their stats match a rule.
#[test]
fn auto_classification_only_surfaces_with_feature_flag() {
    let conn = db::open(std::path::Path::new(":memory:")).expect("open db");
    let rules = classification::builtin_rules();
    let player_id = db::get_or_create_player(&conn, "PokerStars", "Maniac Mike").unwrap();

    let result = classification::resolve_for_player(
        &conn,
        &rules,
        player_id,
        100,
        &stats(55.0, 40.0, 18.0), // matches builtin-maniac unambiguously
    )
    .unwrap();

    assert!(!result.is_override);

    #[cfg(feature = "auto-classification")]
    {
        assert_eq!(result.classification, PlayerClassification::Maniac);
        assert!(result.available);
    }

    #[cfg(not(feature = "auto-classification"))]
    {
        // Distinct from a genuine Unknown (below min_hands / no rule
        // matched): `available: false` tells the frontend PROFILE section to
        // show "unavailable in this build" rather than a normal Unknown
        // badge, per Phase 1's flag-independence requirement.
        assert_eq!(result.classification, PlayerClassification::Unknown);
        assert_eq!(result.label, "Classification unavailable in this build");
        assert_eq!(result.color, "#6b7480");
        assert!(!result.available);
    }
}

/// A manual per-player override is always compliant and must render
/// identically whether or not `auto-classification` is compiled in.
#[test]
fn manual_override_renders_identically_regardless_of_feature_flag() {
    let conn = db::open(std::path::Path::new(":memory:")).expect("open db");
    let rules = classification::builtin_rules();
    let player_id = db::get_or_create_player(&conn, "PokerStars", "Custom Colored Villain").unwrap();
    db::set_player_color_override(&conn, player_id, "#ff00ff", Some("My Note")).unwrap();

    let result = classification::resolve_for_player(
        &conn,
        &rules,
        player_id,
        100,
        &stats(55.0, 40.0, 18.0), // would otherwise match builtin-maniac
    )
    .unwrap();

    assert!(result.is_override);
    assert_eq!(result.color, "#ff00ff");
    assert_eq!(result.label, "My Note");
    assert!(result.available);
}
