use velora_poker_lib::classification;
use velora_poker_lib::db;
use velora_poker_lib::description_rules;
use velora_poker_lib::stats::{PlayerStats, PlayerStatsOpportunities};

#[cfg(feature = "auto-classification")]
use velora_poker_lib::classification::PlayerClassification;

fn maniac_stats() -> PlayerStats {
    PlayerStats {
        vpip: Some(55.0),
        pfr: Some(40.0),
        three_bet: Some(18.0),
        fold_to_three_bet: Some(20.0),
        four_bet: Some(6.0),
        fold_to_four_bet: Some(25.0),
        rfi: Some(30.0),
        limp: Some(5.0),
        cold_call: Some(15.0),
        squeeze: Some(8.0),
        fold_to_squeeze: Some(45.0),
        c_bet: Some(80.0),
        fold_to_c_bet: Some(20.0),
        aggression_factor: Some(4.0),
        wtsd: Some(35.0),
        wsd: Some(30.0),
        steal_attempt: Some(35.0),
        fold_to_steal: Some(50.0),
    }
}

fn opportunities() -> PlayerStatsOpportunities {
    PlayerStatsOpportunities {
        hands: 300,
        three_bet_opportunities: 100,
        faced_3bet_opportunities: 80,
        four_bet_opportunities: 40,
        faced_4bet_opportunities: 30,
        rfi_limp_opportunities: 120,
        cold_call_opportunities: 60,
        squeeze_opportunities: 25,
        faced_squeeze_opportunities: 20,
        cbet_opportunities: 150,
        faced_cbet_opportunities: 100,
        saw_flop_hands: 200,
        went_to_showdown_hands: 70,
        postflop_calls: 60,
        postflop_bets_raises: 240,
        steal_attempt_opportunities: 90,
        fold_to_steal_opportunities: 50,
    }
}

/// Real incident this guards against: a
/// build compiled with `strategic-analysis` but *without*
/// `auto-classification` must still compute and render TENDENCIES/EXPLOITS/
/// CONFIDENCE normally from raw stats — that path must be completely
/// unaffected by whether auto-classification is present. Only the PROFILE
/// section (driven by `classification::resolve_for_player`) may change
/// shape when `auto-classification` is off, via `ClassificationResult.available`.
///
/// Structured the same way as `auto_classification_only_surfaces_with_feature_flag`
/// in `classification_tests.rs`: one test, two `#[cfg(feature = ...)]`
/// branches, each asserting the shape for that build's actual feature set —
/// not a claim that both were compiled and run in the same process.
#[test]
fn rule_results_are_unaffected_by_the_auto_classification_flag() {
    let stats = maniac_stats();
    let opp = opportunities();

    // `description_rules::evaluate` never inspects `auto-classification` —
    // it isn't `#[cfg]`-gated on that flag at all, so its rule matches are
    // identical regardless of which classification flag this build has.
    let rule_results = description_rules::evaluate(&stats, &opp);
    assert!(
        rule_results.iter().any(|r| r.rule_id == "loose-aggressive")
            || rule_results.iter().any(|r| r.rule_id == "3bets-often"),
        "rule engine should still find matches for a maniac-shaped stat line \
         regardless of the auto-classification feature"
    );

    let conn = db::open(std::path::Path::new(":memory:")).expect("open db");
    let rules = classification::builtin_rules();
    let player_id = db::get_or_create_player(&conn, "PokerStars", "Flag Independence Test").unwrap();
    let classification_result =
        classification::resolve_for_player(&conn, &rules, player_id, 300, &stats).unwrap();

    #[cfg(feature = "auto-classification")]
    {
        assert_eq!(classification_result.classification, PlayerClassification::Maniac);
        assert!(classification_result.available);
    }

    #[cfg(not(feature = "auto-classification"))]
    {
        assert!(!classification_result.available);
        assert_eq!(classification_result.label, "Classification unavailable in this build");
    }

    // Whichever branch above ran, the rule engine's output above was
    // computed identically either way -- this is the independence claim:
    // TENDENCIES/EXPLOITS/CONFIDENCE never consulted `classification_result`
    // or the `auto-classification` feature at all.
    assert!(!rule_results.is_empty());
}
