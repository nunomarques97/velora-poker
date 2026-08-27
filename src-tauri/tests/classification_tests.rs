use velora_poker_lib::classification::{self, PlayerClassification};
use velora_poker_lib::stats::PlayerStats;

fn stats(vpip: f64, pfr: f64, three_bet: f64) -> PlayerStats {
    PlayerStats {
        vpip,
        pfr,
        three_bet,
        fold_to_three_bet: 0.0,
        c_bet: 0.0,
        fold_to_c_bet: 0.0,
        aggression_factor: 0.0,
        wtsd: 0.0,
        wsd: 0.0,
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
    // Tight and passive: doesn't match any specific bucket.
    let result = classification::classify(&rules, 100, &stats(10.0, 5.0, 1.0));
    assert_eq!(result.classification, PlayerClassification::Recreational);
}
