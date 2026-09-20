use velora_poker_lib::description_rules::{self, ConfidenceTier, RuleResult};
use velora_poker_lib::stats::{PlayerStats, PlayerStatsOpportunities};

fn base_stats() -> PlayerStats {
    PlayerStats {
        vpip: None,
        pfr: None,
        three_bet: None,
        fold_to_three_bet: None,
        four_bet: None,
        fold_to_four_bet: None,
        rfi: None,
        limp: None,
        cold_call: None,
        squeeze: None,
        fold_to_squeeze: None,
        c_bet: None,
        fold_to_c_bet: None,
        aggression_factor: None,
        wtsd: None,
        wsd: None,
        steal_attempt: None,
        fold_to_steal: None,
    }
}

fn base_opp() -> PlayerStatsOpportunities {
    PlayerStatsOpportunities {
        hands: 0,
        three_bet_opportunities: 0,
        faced_3bet_opportunities: 0,
        four_bet_opportunities: 0,
        faced_4bet_opportunities: 0,
        rfi_limp_opportunities: 0,
        cold_call_opportunities: 0,
        squeeze_opportunities: 0,
        faced_squeeze_opportunities: 0,
        cbet_opportunities: 0,
        faced_cbet_opportunities: 0,
        saw_flop_hands: 0,
        went_to_showdown_hands: 0,
        postflop_calls: 0,
        postflop_bets_raises: 0,
        steal_attempt_opportunities: 0,
        fold_to_steal_opportunities: 0,
    }
}

fn find<'a>(results: &'a [RuleResult], rule_id: &str) -> Option<&'a RuleResult> {
    results.iter().find(|r| r.rule_id == rule_id)
}

const VERY_SELECTIVE: &str = "nitty-very-selective";
const LOOSE_PASSIVE: &str = "loose-passive";
const RARELY_3BETS: &str = "rarely-3bets";
const CBETS_ALMOST_ALWAYS: &str = "cbets-almost-always";
const CALLING_STATION: &str = "calling-station";
const PASSIVE_POSTFLOP: &str = "passive-postflop";

const LOOSE_PASSIVE_OBSERVATION: &str = "Loose-passive: calls wide, rarely raises.";
const LOOSE_PASSIVE_ADVICE: &str = "Value bet, don't bluff.";
const RARELY_3BETS_OBSERVATION: &str = "Rarely 3-bets; when he does, it's premium.";
const RARELY_3BETS_ADVICE: &str = "Respect his 3-bets.";
const CBETS_ALMOST_ALWAYS_OBSERVATION: &str = "C-bets nearly every flop, which is low information.";
const CBETS_ALMOST_ALWAYS_ADVICE: &str = "Float or raise him more.";
const CALLING_STATION_OBSERVATION: &str = "Reaches showdown often, wins little there.";
const CALLING_STATION_ADVICE: &str = "Value bet thin, don't bluff.";
const PASSIVE_POSTFLOP_OBSERVATION: &str = "Mostly calls postflop, rarely raises.";
const PASSIVE_POSTFLOP_ADVICE: &str = "Bet thin for value.";

const SQUEEZES_AGGRESSIVELY: &str = "squeezes-aggressively";
const RARELY_SQUEEZES: &str = "rarely-squeezes";
const FOLDS_A_LOT_TO_SQUEEZES: &str = "folds-a-lot-to-squeezes";

const SQUEEZES_AGGRESSIVELY_OBSERVATION: &str = "Squeezes often when there's a raise and a caller ahead of him, and his squeezes aren't always premium.";
const SQUEEZES_AGGRESSIVELY_ADVICE: &str = "Continue lighter into his squeezes.";
const RARELY_SQUEEZES_OBSERVATION: &str = "Almost never squeezes.";
const RARELY_SQUEEZES_ADVICE: &str =
    "Cold-call safely in front of him without fear of getting blown off the hand.";
const FOLDS_A_LOT_TO_SQUEEZES_OBSERVATION: &str =
    "Folds a lot when squeezed after someone else's cold call.";
const FOLDS_A_LOT_TO_SQUEEZES_ADVICE: &str = "Squeeze him light for profit.";

const FOUR_BETS_AGGRESSIVELY: &str = "4bets-aggressively";
const RARELY_4BETS: &str = "rarely-4bets";
const FOLDS_A_LOT_TO_4BETS: &str = "folds-a-lot-to-4bets";
const COLD_CALLS_TOO_MUCH: &str = "cold-calls-too-much";

const FOUR_BETS_AGGRESSIVELY_OBSERVATION: &str =
    "4-bets often when 3-bet, and his 4-bets aren't automatically premium.";
const FOUR_BETS_AGGRESSIVELY_ADVICE: &str = "Look for a spot to continue against his 4-bets.";
const RARELY_4BETS_OBSERVATION: &str =
    "Almost never 4-bets; when he does, it's about as strong as it gets.";
const RARELY_4BETS_ADVICE: &str = "Fold anything but the top of your range.";
const FOLDS_A_LOT_TO_4BETS_OBSERVATION: &str = "Gives up his 3-bets to a 4-bet often.";
const FOLDS_A_LOT_TO_4BETS_ADVICE: &str = "4-bet bluff him more.";
const COLD_CALLS_TOO_MUCH_OBSERVATION: &str =
    "Flats raises a lot instead of 3-betting or folding: a wide, hard-to-pin-down range.";
const COLD_CALLS_TOO_MUCH_ADVICE: &str = "Bet for value freely postflop.";

const STEALS_TOO_OFTEN: &str = "steals-too-often";
const RARELY_STEALS: &str = "rarely-steals";
const OVERFOLDS_TO_STEALS: &str = "overfolds-to-steals";
const DEFENDS_BLINDS_TOO_WIDE: &str = "defends-blinds-too-wide";

const STEALS_TOO_OFTEN_OBSERVATION: &str = "Opens light from the cutoff, button, or small blind.";
const STEALS_TOO_OFTEN_ADVICE: &str = "Defend your blinds wider and punish his late opens.";
const RARELY_STEALS_OBSERVATION: &str = "Rarely opens from late position; when he does, it's real.";
const RARELY_STEALS_ADVICE: &str = "Give it more respect than a normal open.";
const OVERFOLDS_TO_STEALS_OBSERVATION: &str = "Folds his blinds to a steal almost every time.";
const OVERFOLDS_TO_STEALS_ADVICE: &str = "Steal against him relentlessly.";
const DEFENDS_BLINDS_TOO_WIDE_OBSERVATION: &str = "Defends his blinds very wide against steals.";
const DEFENDS_BLINDS_TOO_WIDE_ADVICE: &str =
    "Don't bother stealing light against him; he'll fight back.";

const COLD_CALLS_WIDE_FOLDS_TO_CBETS: &str = "cold-calls-wide-folds-to-cbets";
const AGGRESSIVE_PREFLOP_FOLDS_TO_RERAISE: &str = "aggressive-preflop-folds-to-reraise";

const COLD_CALLS_WIDE_FOLDS_TO_CBETS_OBSERVATION: &str =
    "Cold-calls a wide range but folds too often to continuation bets.";
const COLD_CALLS_WIDE_FOLDS_TO_CBETS_ADVICE: &str =
    "Bet him off pots after he flats preflop; you don't need a real hand.";
const AGGRESSIVE_PREFLOP_FOLDS_TO_RERAISE_OBSERVATION: &str =
    "3-bets and squeezes with a wide range but gives up when re-raised back.";
const AGGRESSIVE_PREFLOP_FOLDS_TO_RERAISE_ADVICE: &str =
    "4-bet or re-raise his preflop aggression light; he folds too much.";

// --- confidence formula ---

#[test]
fn confidence_below_ten_opportunities_suppresses_the_rule_entirely() {
    let stats = PlayerStats {
        vpip: Some(10.0),
        ..base_stats()
    };
    let opp = PlayerStatsOpportunities {
        hands: 9,
        ..base_opp()
    };
    let out = description_rules::evaluate(&stats, &opp);
    assert!(
        find(&out, VERY_SELECTIVE).is_none(),
        "a rule with fewer than 10 opportunities must not appear at all, even as low confidence"
    );
}

#[test]
fn confidence_anchors_match_the_specified_points() {
    // 40 opportunities -> 20%, 100 -> 50%, exactly at the 10-floor -> 5%.
    for (opportunities, expected_confidence) in [(10, 5u8), (40, 20u8), (100, 50u8)] {
        let stats = PlayerStats {
            vpip: Some(10.0),
            ..base_stats()
        };
        let opp = PlayerStatsOpportunities {
            hands: opportunities,
            ..base_opp()
        };
        let out = description_rules::evaluate(&stats, &opp);
        let result = find(&out, VERY_SELECTIVE)
            .unwrap_or_else(|| panic!("expected a match at {opportunities} opportunities"));
        assert_eq!(
            result.confidence_pct,
            Some(expected_confidence),
            "opportunities={opportunities}"
        );
    }
}

#[test]
fn confidence_caps_at_100_percent_at_and_above_200_opportunities() {
    for opportunities in [200, 500, 10_000] {
        let stats = PlayerStats {
            vpip: Some(10.0),
            ..base_stats()
        };
        let opp = PlayerStatsOpportunities {
            hands: opportunities,
            ..base_opp()
        };
        let out = description_rules::evaluate(&stats, &opp);
        let result = find(&out, VERY_SELECTIVE).unwrap();
        assert_eq!(result.confidence_pct, Some(100), "opportunities={opportunities}");
        assert_eq!(result.confidence_tier, ConfidenceTier::High);
    }
}

// --- VPIP/PFR-based rule family ---

#[test]
fn loose_passive_matches_at_exactly_the_40_percent_vpip_boundary() {
    let stats = PlayerStats {
        vpip: Some(40.0),
        pfr: Some(10.0), // vpip - 10 = 30, 10 <= 30
        ..base_stats()
    };
    let opp = PlayerStatsOpportunities {
        hands: 100,
        ..base_opp()
    };
    let out = description_rules::evaluate(&stats, &opp);
    let result = find(&out, LOOSE_PASSIVE);
    assert!(result.is_some(), "VPIP=40% exactly must match Loose-passive");
    let result = result.unwrap();
    assert_eq!(result.observation, LOOSE_PASSIVE_OBSERVATION);
    assert_eq!(result.advice, LOOSE_PASSIVE_ADVICE);
}

#[test]
fn loose_passive_does_not_match_just_under_the_vpip_boundary() {
    let stats = PlayerStats {
        vpip: Some(39.9),
        pfr: Some(10.0),
        ..base_stats()
    };
    let opp = PlayerStatsOpportunities {
        hands: 100,
        ..base_opp()
    };
    let out = description_rules::evaluate(&stats, &opp);
    assert!(find(&out, LOOSE_PASSIVE).is_none(), "VPIP=39.9% must not match Loose-passive");
}

// --- 3-bet-based rule family ---

#[test]
fn rarely_3bets_matches_at_exactly_the_4_percent_boundary() {
    let stats = PlayerStats {
        three_bet: Some(4.0),
        ..base_stats()
    };
    let opp = PlayerStatsOpportunities {
        three_bet_opportunities: 100,
        ..base_opp()
    };
    let out = description_rules::evaluate(&stats, &opp);
    let result = find(&out, RARELY_3BETS);
    assert!(result.is_some());
    let result = result.unwrap();
    assert_eq!(result.observation, RARELY_3BETS_OBSERVATION);
    assert_eq!(result.advice, RARELY_3BETS_ADVICE);
}

#[test]
fn rarely_3bets_does_not_match_just_over_the_4_percent_boundary() {
    let stats = PlayerStats {
        three_bet: Some(4.1),
        ..base_stats()
    };
    let opp = PlayerStatsOpportunities {
        three_bet_opportunities: 100,
        ..base_opp()
    };
    let out = description_rules::evaluate(&stats, &opp);
    assert!(find(&out, RARELY_3BETS).is_none());
}

// --- c-bet-based rule family ---

#[test]
fn cbets_almost_always_matches_at_exactly_the_75_percent_boundary() {
    let stats = PlayerStats {
        c_bet: Some(75.0),
        ..base_stats()
    };
    let opp = PlayerStatsOpportunities {
        cbet_opportunities: 100,
        ..base_opp()
    };
    let out = description_rules::evaluate(&stats, &opp);
    let result = find(&out, CBETS_ALMOST_ALWAYS);
    assert!(result.is_some());
    let result = result.unwrap();
    assert_eq!(result.observation, CBETS_ALMOST_ALWAYS_OBSERVATION);
    assert_eq!(result.advice, CBETS_ALMOST_ALWAYS_ADVICE);
}

#[test]
fn cbets_almost_always_does_not_match_just_under_the_75_percent_boundary() {
    let stats = PlayerStats {
        c_bet: Some(74.9),
        ..base_stats()
    };
    let opp = PlayerStatsOpportunities {
        cbet_opportunities: 100,
        ..base_opp()
    };
    let out = description_rules::evaluate(&stats, &opp);
    assert!(find(&out, CBETS_ALMOST_ALWAYS).is_none());
}

// --- WTSD/W$SD-based rule family ---

#[test]
fn calling_station_matches_at_exactly_its_boundaries() {
    let stats = PlayerStats {
        wtsd: Some(30.0),
        wsd: Some(35.0),
        ..base_stats()
    };
    let opp = PlayerStatsOpportunities {
        went_to_showdown_hands: 100,
        saw_flop_hands: 300,
        ..base_opp()
    };
    let out = description_rules::evaluate(&stats, &opp);
    let result = find(&out, CALLING_STATION);
    assert!(result.is_some());
    let result = result.unwrap();
    assert_eq!(result.observation, CALLING_STATION_OBSERVATION);
    assert_eq!(result.advice, CALLING_STATION_ADVICE);
    assert_eq!(result.category, description_rules::RuleCategory::Tendency);
}

#[test]
fn calling_station_does_not_match_just_under_the_wtsd_boundary() {
    let stats = PlayerStats {
        wtsd: Some(29.9),
        wsd: Some(35.0),
        ..base_stats()
    };
    let opp = PlayerStatsOpportunities {
        went_to_showdown_hands: 100,
        ..base_opp()
    };
    let out = description_rules::evaluate(&stats, &opp);
    assert!(find(&out, CALLING_STATION).is_none());
}

// --- aggression-factor-based rule family ---

#[test]
fn passive_postflop_matches_at_exactly_the_1_0_af_boundary() {
    let stats = PlayerStats {
        aggression_factor: Some(1.0),
        ..base_stats()
    };
    let opp = PlayerStatsOpportunities {
        postflop_calls: 100,
        ..base_opp()
    };
    let out = description_rules::evaluate(&stats, &opp);
    let result = find(&out, PASSIVE_POSTFLOP);
    assert!(result.is_some());
    let result = result.unwrap();
    assert_eq!(result.observation, PASSIVE_POSTFLOP_OBSERVATION);
    assert_eq!(result.advice, PASSIVE_POSTFLOP_ADVICE);
}

#[test]
fn passive_postflop_does_not_match_just_over_the_1_0_af_boundary() {
    let stats = PlayerStats {
        aggression_factor: Some(1.01),
        ..base_stats()
    };
    let opp = PlayerStatsOpportunities {
        postflop_calls: 100,
        ..base_opp()
    };
    let out = description_rules::evaluate(&stats, &opp);
    assert!(find(&out, PASSIVE_POSTFLOP).is_none());
}

// --- squeeze/fold-to-squeeze-based rule family ---

#[test]
fn squeezes_aggressively_matches_at_exactly_the_12_percent_boundary() {
    let stats = PlayerStats {
        squeeze: Some(12.0),
        ..base_stats()
    };
    let opp = PlayerStatsOpportunities {
        squeeze_opportunities: 50,
        ..base_opp()
    };
    let out = description_rules::evaluate(&stats, &opp);
    let result = find(&out, SQUEEZES_AGGRESSIVELY);
    assert!(result.is_some(), "squeeze=12% exactly must match Squeezes aggressively");
    let result = result.unwrap();
    assert_eq!(result.observation, SQUEEZES_AGGRESSIVELY_OBSERVATION);
    assert_eq!(result.advice, SQUEEZES_AGGRESSIVELY_ADVICE);
    assert_eq!(result.category, description_rules::RuleCategory::Tendency);
    assert_eq!(result.evidence, vec![description_rules::Evidence {
        stat_name: "squeeze".to_string(),
        value: Some(12.0),
        opportunities: opp.squeeze_opportunities,
    }]);
}

#[test]
fn squeezes_aggressively_does_not_match_just_under_the_12_percent_boundary() {
    let stats = PlayerStats {
        squeeze: Some(11.9),
        ..base_stats()
    };
    let opp = PlayerStatsOpportunities {
        squeeze_opportunities: 50,
        ..base_opp()
    };
    let out = description_rules::evaluate(&stats, &opp);
    assert!(find(&out, SQUEEZES_AGGRESSIVELY).is_none(), "squeeze=11.9% must not match");
}

#[test]
fn rarely_squeezes_matches_at_exactly_the_4_percent_boundary() {
    let stats = PlayerStats {
        squeeze: Some(4.0),
        ..base_stats()
    };
    let opp = PlayerStatsOpportunities {
        squeeze_opportunities: 71, // a realistic sample size seen in real data
        ..base_opp()
    };
    let out = description_rules::evaluate(&stats, &opp);
    let result = find(&out, RARELY_SQUEEZES);
    assert!(result.is_some(), "squeeze=4% exactly must match Rarely squeezes");
    let result = result.unwrap();
    assert_eq!(result.observation, RARELY_SQUEEZES_OBSERVATION);
    assert_eq!(result.advice, RARELY_SQUEEZES_ADVICE);
    assert_eq!(result.category, description_rules::RuleCategory::Exploit);
}

#[test]
fn rarely_squeezes_does_not_match_just_over_the_4_percent_boundary() {
    let stats = PlayerStats {
        squeeze: Some(4.1),
        ..base_stats()
    };
    let opp = PlayerStatsOpportunities {
        squeeze_opportunities: 71,
        ..base_opp()
    };
    let out = description_rules::evaluate(&stats, &opp);
    assert!(find(&out, RARELY_SQUEEZES).is_none(), "squeeze=4.1% must not match");
}

#[test]
fn both_squeeze_rules_can_co_occur_with_a_direct_3bet_rule_on_the_same_stat_line() {
    // A player can be both "3-bets often" and "rarely squeezes" at once --
    // three_bet% and squeeze% are independent readings of different decision
    // points (a direct re-raise vs. a re-raise over a live caller).
    let stats = PlayerStats {
        three_bet: Some(10.0),
        squeeze: Some(4.0),
        ..base_stats()
    };
    let opp = PlayerStatsOpportunities {
        three_bet_opportunities: 100,
        squeeze_opportunities: 71,
        ..base_opp()
    };
    let out = description_rules::evaluate(&stats, &opp);
    assert!(find(&out, RARELY_3BETS).is_none(), "three_bet=10% is above the rarely-3bets ceiling");
    assert!(find(&out, "3bets-often").is_some());
    assert!(find(&out, RARELY_SQUEEZES).is_some());
}

#[test]
fn folds_a_lot_to_squeezes_matches_at_exactly_the_65_percent_boundary() {
    let stats = PlayerStats {
        fold_to_squeeze: Some(65.0),
        ..base_stats()
    };
    let opp = PlayerStatsOpportunities {
        faced_squeeze_opportunities: 20,
        ..base_opp()
    };
    let out = description_rules::evaluate(&stats, &opp);
    let result = find(&out, FOLDS_A_LOT_TO_SQUEEZES);
    assert!(result.is_some(), "fold_to_squeeze=65% exactly must match Folds a lot to squeezes");
    let result = result.unwrap();
    assert_eq!(result.observation, FOLDS_A_LOT_TO_SQUEEZES_OBSERVATION);
    assert_eq!(result.advice, FOLDS_A_LOT_TO_SQUEEZES_ADVICE);
    assert_eq!(result.category, description_rules::RuleCategory::Exploit);
    assert_eq!(result.evidence, vec![description_rules::Evidence {
        stat_name: "fold_to_squeeze".to_string(),
        value: Some(65.0),
        opportunities: opp.faced_squeeze_opportunities,
    }]);
}

#[test]
fn folds_a_lot_to_squeezes_does_not_match_just_under_the_65_percent_boundary() {
    let stats = PlayerStats {
        fold_to_squeeze: Some(64.9),
        ..base_stats()
    };
    let opp = PlayerStatsOpportunities {
        faced_squeeze_opportunities: 20,
        ..base_opp()
    };
    let out = description_rules::evaluate(&stats, &opp);
    assert!(
        find(&out, FOLDS_A_LOT_TO_SQUEEZES).is_none(),
        "fold_to_squeeze=64.9% must not match"
    );
}

/// Documents a real limitation found while calibrating against real data: a
/// player can show a genuine 100% fold-to-squeeze over 9 opportunities, but
/// that is below the 10-opportunity floor every rule enforces, so it
/// correctly produces no conclusion yet. Not a bug: the floor is doing
/// exactly what it is meant to ("insufficient data, not a fabricated
/// number"). This rule will start surfacing once enough hands accumulate.
#[test]
fn folds_a_lot_to_squeezes_correctly_shows_no_conclusion_below_the_confidence_floor() {
    let stats = PlayerStats {
        fold_to_squeeze: Some(100.0),
        ..base_stats()
    };
    let opp = PlayerStatsOpportunities {
        faced_squeeze_opportunities: 1,
        ..base_opp()
    };
    let out = description_rules::evaluate(&stats, &opp);
    assert!(
        find(&out, FOLDS_A_LOT_TO_SQUEEZES).is_none(),
        "a real 100% fold-to-squeeze with only 1 opportunity must not surface a conclusion"
    );
}

#[test]
fn evidence_matches_the_accumulator_for_a_squeeze_happy_reg() {
    let stats = PlayerStats {
        squeeze: Some(14.0),
        fold_to_squeeze: Some(70.0),
        ..base_stats()
    };
    let opp = PlayerStatsOpportunities {
        squeeze_opportunities: 50,
        faced_squeeze_opportunities: 20,
        ..base_opp()
    };
    let out = description_rules::evaluate(&stats, &opp);

    let squeezes_aggressively =
        find(&out, SQUEEZES_AGGRESSIVELY).expect("expected squeezes-aggressively match");
    assert_eq!(squeezes_aggressively.evidence[0].stat_name, "squeeze");
    assert_eq!(squeezes_aggressively.evidence[0].value, Some(14.0));
    assert_eq!(squeezes_aggressively.evidence[0].opportunities, opp.squeeze_opportunities);

    let folds_a_lot_to_squeezes =
        find(&out, FOLDS_A_LOT_TO_SQUEEZES).expect("expected folds-a-lot-to-squeezes match");
    assert_eq!(folds_a_lot_to_squeezes.evidence[0].stat_name, "fold_to_squeeze");
    assert_eq!(folds_a_lot_to_squeezes.evidence[0].value, Some(70.0));
    assert_eq!(
        folds_a_lot_to_squeezes.evidence[0].opportunities,
        opp.faced_squeeze_opportunities
    );

    assert!(
        find(&out, RARELY_SQUEEZES).is_none(),
        "squeeze=14% is above the rarely-squeezes ceiling, so it must not also match"
    );
}

// --- 4-bet/fold-to-4-bet/cold-call-based rule family ---

#[test]
fn four_bets_aggressively_matches_at_exactly_the_40_percent_boundary() {
    let stats = PlayerStats {
        four_bet: Some(40.0),
        ..base_stats()
    };
    let opp = PlayerStatsOpportunities {
        faced_3bet_opportunities: 60,
        ..base_opp()
    };
    let out = description_rules::evaluate(&stats, &opp);
    let result = find(&out, FOUR_BETS_AGGRESSIVELY);
    assert!(result.is_some(), "four_bet=40% exactly must match 4-bets aggressively");
    let result = result.unwrap();
    assert_eq!(result.observation, FOUR_BETS_AGGRESSIVELY_OBSERVATION);
    assert_eq!(result.advice, FOUR_BETS_AGGRESSIVELY_ADVICE);
    assert_eq!(result.category, description_rules::RuleCategory::Tendency);
    assert_eq!(result.evidence, vec![description_rules::Evidence {
        stat_name: "four_bet".to_string(),
        value: Some(40.0),
        opportunities: opp.faced_3bet_opportunities,
    }]);
}

#[test]
fn four_bets_aggressively_does_not_match_just_under_the_40_percent_boundary() {
    let stats = PlayerStats {
        four_bet: Some(39.9),
        ..base_stats()
    };
    let opp = PlayerStatsOpportunities {
        faced_3bet_opportunities: 60,
        ..base_opp()
    };
    let out = description_rules::evaluate(&stats, &opp);
    assert!(find(&out, FOUR_BETS_AGGRESSIVELY).is_none(), "four_bet=39.9% must not match");
}

#[test]
fn rarely_4bets_matches_at_exactly_the_10_percent_boundary() {
    let stats = PlayerStats {
        four_bet: Some(10.0),
        ..base_stats()
    };
    let opp = PlayerStatsOpportunities {
        faced_3bet_opportunities: 60,
        ..base_opp()
    };
    let out = description_rules::evaluate(&stats, &opp);
    let result = find(&out, RARELY_4BETS);
    assert!(result.is_some(), "four_bet=10% exactly must match Rarely 4-bets");
    let result = result.unwrap();
    assert_eq!(result.observation, RARELY_4BETS_OBSERVATION);
    assert_eq!(result.advice, RARELY_4BETS_ADVICE);
    assert_eq!(
        result.category,
        description_rules::RuleCategory::Tendency,
        "mirrors rarely-3bets, which is also a Tendency"
    );
}

#[test]
fn rarely_4bets_does_not_match_just_over_the_10_percent_boundary() {
    let stats = PlayerStats {
        four_bet: Some(10.1),
        ..base_stats()
    };
    let opp = PlayerStatsOpportunities {
        faced_3bet_opportunities: 60,
        ..base_opp()
    };
    let out = description_rules::evaluate(&stats, &opp);
    assert!(find(&out, RARELY_4BETS).is_none(), "four_bet=10.1% must not match");
}

#[test]
fn folds_a_lot_to_4bets_matches_at_exactly_the_60_percent_boundary() {
    let stats = PlayerStats {
        fold_to_four_bet: Some(60.0),
        ..base_stats()
    };
    let opp = PlayerStatsOpportunities {
        faced_4bet_opportunities: 15,
        ..base_opp()
    };
    let out = description_rules::evaluate(&stats, &opp);
    let result = find(&out, FOLDS_A_LOT_TO_4BETS);
    assert!(result.is_some(), "fold_to_four_bet=60% exactly must match Folds a lot to 4-bets");
    let result = result.unwrap();
    assert_eq!(result.observation, FOLDS_A_LOT_TO_4BETS_OBSERVATION);
    assert_eq!(result.advice, FOLDS_A_LOT_TO_4BETS_ADVICE);
    assert_eq!(result.category, description_rules::RuleCategory::Exploit);
    assert_eq!(result.evidence, vec![description_rules::Evidence {
        stat_name: "fold_to_four_bet".to_string(),
        value: Some(60.0),
        opportunities: opp.faced_4bet_opportunities,
    }]);
}

#[test]
fn folds_a_lot_to_4bets_does_not_match_just_under_the_60_percent_boundary() {
    let stats = PlayerStats {
        fold_to_four_bet: Some(59.9),
        ..base_stats()
    };
    let opp = PlayerStatsOpportunities {
        faced_4bet_opportunities: 15,
        ..base_opp()
    };
    let out = description_rules::evaluate(&stats, &opp);
    assert!(
        find(&out, FOLDS_A_LOT_TO_4BETS).is_none(),
        "fold_to_four_bet=59.9% must not match"
    );
}

#[test]
fn four_bets_aggressively_and_rarely_4bets_share_faced_3bet_opportunities_not_a_new_counter() {
    // Design choice: `four_bet` reuses `faced_3bet_opportunities` as its
    // confidence basis rather than a separate `four_bet_opportunities`
    // counter, because `four_bet` is just the response side of the same
    // `facing_3bet` event `fold_to_three_bet` already tracks. Confirm the
    // rule engine actually reads that field, not `four_bet_opportunities`.
    let stats = PlayerStats {
        four_bet: Some(40.0),
        ..base_stats()
    };
    let opp = PlayerStatsOpportunities {
        faced_3bet_opportunities: 60,
        four_bet_opportunities: 0, // deliberately left at 0 / unused
        ..base_opp()
    };
    let out = description_rules::evaluate(&stats, &opp);
    let result = find(&out, FOUR_BETS_AGGRESSIVELY).expect("expected a match");
    assert_eq!(result.confidence_pct, Some(30), "30% == 60/200, confirming faced_3bet_opportunities was used");
}

#[test]
fn cold_calls_too_much_matches_at_exactly_the_30_percent_boundary() {
    let stats = PlayerStats {
        cold_call: Some(30.0),
        ..base_stats()
    };
    let opp = PlayerStatsOpportunities {
        cold_call_opportunities: 80,
        ..base_opp()
    };
    let out = description_rules::evaluate(&stats, &opp);
    let result = find(&out, COLD_CALLS_TOO_MUCH);
    assert!(result.is_some(), "cold_call=30% exactly must match Cold-calls too much");
    let result = result.unwrap();
    assert_eq!(result.observation, COLD_CALLS_TOO_MUCH_OBSERVATION);
    assert_eq!(result.advice, COLD_CALLS_TOO_MUCH_ADVICE);
    assert_eq!(result.category, description_rules::RuleCategory::Tendency);
    assert_eq!(result.evidence, vec![description_rules::Evidence {
        stat_name: "cold_call".to_string(),
        value: Some(30.0),
        opportunities: opp.cold_call_opportunities,
    }]);
}

#[test]
fn cold_calls_too_much_does_not_match_just_under_the_30_percent_boundary() {
    let stats = PlayerStats {
        cold_call: Some(29.9),
        ..base_stats()
    };
    let opp = PlayerStatsOpportunities {
        cold_call_opportunities: 80,
        ..base_opp()
    };
    let out = description_rules::evaluate(&stats, &opp);
    assert!(find(&out, COLD_CALLS_TOO_MUCH).is_none(), "cold_call=29.9% must not match");
}

#[test]
fn steals_too_often_matches_at_exactly_the_45_percent_boundary() {
    let stats = PlayerStats {
        steal_attempt: Some(45.0),
        ..base_stats()
    };
    let opp = PlayerStatsOpportunities {
        steal_attempt_opportunities: 60,
        ..base_opp()
    };
    let out = description_rules::evaluate(&stats, &opp);
    let result = find(&out, STEALS_TOO_OFTEN);
    assert!(result.is_some(), "steal_attempt=45% exactly must match Steals too often");
    let result = result.unwrap();
    assert_eq!(result.observation, STEALS_TOO_OFTEN_OBSERVATION);
    assert_eq!(result.advice, STEALS_TOO_OFTEN_ADVICE);
    assert_eq!(result.category, description_rules::RuleCategory::Tendency);
    assert_eq!(result.evidence, vec![description_rules::Evidence {
        stat_name: "steal_attempt".to_string(),
        value: Some(45.0),
        opportunities: opp.steal_attempt_opportunities,
    }]);
}

#[test]
fn steals_too_often_does_not_match_just_under_the_45_percent_boundary() {
    let stats = PlayerStats {
        steal_attempt: Some(44.9),
        ..base_stats()
    };
    let opp = PlayerStatsOpportunities {
        steal_attempt_opportunities: 60,
        ..base_opp()
    };
    let out = description_rules::evaluate(&stats, &opp);
    assert!(find(&out, STEALS_TOO_OFTEN).is_none(), "steal_attempt=44.9% must not match");
}

#[test]
fn rarely_steals_matches_at_exactly_the_20_percent_boundary() {
    let stats = PlayerStats {
        steal_attempt: Some(20.0),
        ..base_stats()
    };
    let opp = PlayerStatsOpportunities {
        steal_attempt_opportunities: 60,
        ..base_opp()
    };
    let out = description_rules::evaluate(&stats, &opp);
    let result = find(&out, RARELY_STEALS);
    assert!(result.is_some(), "steal_attempt=20% exactly must match Rarely steals");
    let result = result.unwrap();
    assert_eq!(result.observation, RARELY_STEALS_OBSERVATION);
    assert_eq!(result.advice, RARELY_STEALS_ADVICE);
    assert_eq!(result.category, description_rules::RuleCategory::Tendency);
}

#[test]
fn rarely_steals_does_not_match_just_over_the_20_percent_boundary() {
    let stats = PlayerStats {
        steal_attempt: Some(20.1),
        ..base_stats()
    };
    let opp = PlayerStatsOpportunities {
        steal_attempt_opportunities: 60,
        ..base_opp()
    };
    let out = description_rules::evaluate(&stats, &opp);
    assert!(find(&out, RARELY_STEALS).is_none(), "steal_attempt=20.1% must not match");
}

#[test]
fn overfolds_to_steals_matches_at_exactly_the_75_percent_boundary() {
    let stats = PlayerStats {
        fold_to_steal: Some(75.0),
        ..base_stats()
    };
    let opp = PlayerStatsOpportunities {
        fold_to_steal_opportunities: 40,
        ..base_opp()
    };
    let out = description_rules::evaluate(&stats, &opp);
    let result = find(&out, OVERFOLDS_TO_STEALS);
    assert!(result.is_some(), "fold_to_steal=75% exactly must match Overfolds to steals");
    let result = result.unwrap();
    assert_eq!(result.observation, OVERFOLDS_TO_STEALS_OBSERVATION);
    assert_eq!(result.advice, OVERFOLDS_TO_STEALS_ADVICE);
    assert_eq!(result.category, description_rules::RuleCategory::Exploit);
    assert_eq!(result.evidence, vec![description_rules::Evidence {
        stat_name: "fold_to_steal".to_string(),
        value: Some(75.0),
        opportunities: opp.fold_to_steal_opportunities,
    }]);
}

#[test]
fn overfolds_to_steals_does_not_match_just_under_the_75_percent_boundary() {
    let stats = PlayerStats {
        fold_to_steal: Some(74.9),
        ..base_stats()
    };
    let opp = PlayerStatsOpportunities {
        fold_to_steal_opportunities: 40,
        ..base_opp()
    };
    let out = description_rules::evaluate(&stats, &opp);
    assert!(find(&out, OVERFOLDS_TO_STEALS).is_none(), "fold_to_steal=74.9% must not match");
}

#[test]
fn defends_blinds_too_wide_matches_at_exactly_the_40_percent_boundary() {
    let stats = PlayerStats {
        fold_to_steal: Some(40.0),
        ..base_stats()
    };
    let opp = PlayerStatsOpportunities {
        fold_to_steal_opportunities: 40,
        ..base_opp()
    };
    let out = description_rules::evaluate(&stats, &opp);
    let result = find(&out, DEFENDS_BLINDS_TOO_WIDE);
    assert!(result.is_some(), "fold_to_steal=40% exactly must match Defends blinds too wide");
    let result = result.unwrap();
    assert_eq!(result.observation, DEFENDS_BLINDS_TOO_WIDE_OBSERVATION);
    assert_eq!(result.advice, DEFENDS_BLINDS_TOO_WIDE_ADVICE);
    assert_eq!(result.category, description_rules::RuleCategory::Exploit);
}

#[test]
fn defends_blinds_too_wide_does_not_match_just_over_the_40_percent_boundary() {
    let stats = PlayerStats {
        fold_to_steal: Some(40.1),
        ..base_stats()
    };
    let opp = PlayerStatsOpportunities {
        fold_to_steal_opportunities: 40,
        ..base_opp()
    };
    let out = description_rules::evaluate(&stats, &opp);
    assert!(
        find(&out, DEFENDS_BLINDS_TOO_WIDE).is_none(),
        "fold_to_steal=40.1% must not match"
    );
}

#[test]
fn steal_rules_correctly_show_no_conclusion_below_the_confidence_floor() {
    // Mirrors folds_a_lot_to_squeezes_correctly_shows_no_conclusion_below_the_confidence_floor:
    // a real, extreme reading with too few opportunities to trust must not
    // surface a conclusion (the shared confidence floor, not a special
    // exception for these two stats).
    let stats = PlayerStats {
        steal_attempt: Some(100.0),
        fold_to_steal: Some(100.0),
        ..base_stats()
    };
    let opp = PlayerStatsOpportunities {
        steal_attempt_opportunities: 1,
        fold_to_steal_opportunities: 1,
        ..base_opp()
    };
    let out = description_rules::evaluate(&stats, &opp);
    assert!(find(&out, STEALS_TOO_OFTEN).is_none(), "1 opportunity must not surface a conclusion");
    assert!(find(&out, OVERFOLDS_TO_STEALS).is_none(), "1 opportunity must not surface a conclusion");
}

#[test]
fn evidence_matches_the_accumulator_for_a_four_bet_happy_reg_who_also_cold_calls() {
    let stats = PlayerStats {
        four_bet: Some(45.0),
        fold_to_four_bet: Some(20.0),
        cold_call: Some(35.0),
        ..base_stats()
    };
    let opp = PlayerStatsOpportunities {
        faced_3bet_opportunities: 60,
        faced_4bet_opportunities: 15,
        cold_call_opportunities: 80,
        ..base_opp()
    };
    let out = description_rules::evaluate(&stats, &opp);

    let four_bets_aggressively =
        find(&out, FOUR_BETS_AGGRESSIVELY).expect("expected 4bets-aggressively match");
    assert_eq!(four_bets_aggressively.evidence[0].stat_name, "four_bet");
    assert_eq!(four_bets_aggressively.evidence[0].value, Some(45.0));
    assert_eq!(four_bets_aggressively.evidence[0].opportunities, opp.faced_3bet_opportunities);

    let cold_calls_too_much =
        find(&out, COLD_CALLS_TOO_MUCH).expect("expected cold-calls-too-much match");
    assert_eq!(cold_calls_too_much.evidence[0].stat_name, "cold_call");
    assert_eq!(cold_calls_too_much.evidence[0].value, Some(35.0));
    assert_eq!(cold_calls_too_much.evidence[0].opportunities, opp.cold_call_opportunities);

    assert!(
        find(&out, RARELY_4BETS).is_none(),
        "four_bet=45% is above the rarely-4bets ceiling, so it must not also match"
    );
    assert!(
        find(&out, FOLDS_A_LOT_TO_4BETS).is_none(),
        "fold_to_four_bet=20% is below the folds-a-lot-to-4bets floor"
    );
}

// --- composite/relational rule family ---

#[test]
fn cold_calls_wide_folds_to_cbets_matches_when_both_legs_qualify() {
    let stats = PlayerStats {
        cold_call: Some(30.0),
        fold_to_c_bet: Some(60.0),
        ..base_stats()
    };
    let opp = PlayerStatsOpportunities {
        cold_call_opportunities: 80,
        faced_cbet_opportunities: 55,
        ..base_opp()
    };
    let out = description_rules::evaluate(&stats, &opp);
    let result = find(&out, COLD_CALLS_WIDE_FOLDS_TO_CBETS);
    assert!(result.is_some(), "cold_call=30% and fold_to_c_bet=60% must match");
    let result = result.unwrap();
    assert_eq!(result.observation, COLD_CALLS_WIDE_FOLDS_TO_CBETS_OBSERVATION);
    assert_eq!(result.advice, COLD_CALLS_WIDE_FOLDS_TO_CBETS_ADVICE);
    assert_eq!(result.category, description_rules::RuleCategory::Exploit);
    assert_eq!(result.evidence, vec![
        description_rules::Evidence {
            stat_name: "cold_call".to_string(),
            value: Some(30.0),
            opportunities: opp.cold_call_opportunities,
        },
        description_rules::Evidence {
            stat_name: "fold_to_c_bet".to_string(),
            value: Some(60.0),
            opportunities: opp.faced_cbet_opportunities,
        },
    ]);
}

#[test]
fn cold_calls_wide_folds_to_cbets_does_not_match_when_only_the_cold_call_leg_qualifies() {
    let stats = PlayerStats {
        cold_call: Some(30.0),
        fold_to_c_bet: Some(59.9),
        ..base_stats()
    };
    let opp = PlayerStatsOpportunities {
        cold_call_opportunities: 80,
        faced_cbet_opportunities: 55,
        ..base_opp()
    };
    let out = description_rules::evaluate(&stats, &opp);
    assert!(
        find(&out, COLD_CALLS_WIDE_FOLDS_TO_CBETS).is_none(),
        "fold_to_c_bet=59.9% is below its leg's threshold, so the composite must not match"
    );
}

#[test]
fn cold_calls_wide_folds_to_cbets_does_not_match_when_only_the_fold_to_cbet_leg_qualifies() {
    let stats = PlayerStats {
        cold_call: Some(29.9),
        fold_to_c_bet: Some(60.0),
        ..base_stats()
    };
    let opp = PlayerStatsOpportunities {
        cold_call_opportunities: 80,
        faced_cbet_opportunities: 55,
        ..base_opp()
    };
    let out = description_rules::evaluate(&stats, &opp);
    assert!(
        find(&out, COLD_CALLS_WIDE_FOLDS_TO_CBETS).is_none(),
        "cold_call=29.9% is below its leg's threshold, so the composite must not match"
    );
}

#[test]
fn cold_calls_wide_folds_to_cbets_confidence_basis_is_the_minimum_not_the_first_or_average() {
    let stats = PlayerStats {
        cold_call: Some(45.0),
        fold_to_c_bet: Some(70.0),
        ..base_stats()
    };
    let opp = PlayerStatsOpportunities {
        cold_call_opportunities: 200, // would be 100% confidence alone
        faced_cbet_opportunities: 20, // -> 10% confidence; the true minimum
        ..base_opp()
    };
    let out = description_rules::evaluate(&stats, &opp);
    let result = find(&out, COLD_CALLS_WIDE_FOLDS_TO_CBETS).expect("expected a match");
    assert_eq!(
        result.confidence_pct,
        Some(10),
        "basis must be min(200, 20)=20 -> 10%, not the first stat's 200 (100%) or the average (110 -> 55%)"
    );
}

#[test]
fn aggressive_preflop_folds_to_reraise_matches_on_the_3bet_leg_alone() {
    let stats = PlayerStats {
        three_bet: Some(10.0),
        fold_to_four_bet: Some(60.0),
        ..base_stats()
    };
    let opp = PlayerStatsOpportunities {
        three_bet_opportunities: 100,
        faced_4bet_opportunities: 15,
        ..base_opp()
    };
    let out = description_rules::evaluate(&stats, &opp);
    let result = find(&out, AGGRESSIVE_PREFLOP_FOLDS_TO_RERAISE);
    assert!(result.is_some(), "three_bet=10% and fold_to_four_bet=60% must match on the 3-bet leg alone");
    let result = result.unwrap();
    assert_eq!(result.observation, AGGRESSIVE_PREFLOP_FOLDS_TO_RERAISE_OBSERVATION);
    assert_eq!(result.advice, AGGRESSIVE_PREFLOP_FOLDS_TO_RERAISE_ADVICE);
    assert_eq!(result.category, description_rules::RuleCategory::Exploit);
    assert_eq!(result.evidence, vec![
        description_rules::Evidence {
            stat_name: "three_bet".to_string(),
            value: Some(10.0),
            opportunities: opp.three_bet_opportunities,
        },
        description_rules::Evidence {
            stat_name: "fold_to_four_bet".to_string(),
            value: Some(60.0),
            opportunities: opp.faced_4bet_opportunities,
        },
    ]);
}

#[test]
fn aggressive_preflop_folds_to_reraise_matches_on_the_squeeze_leg_alone() {
    let stats = PlayerStats {
        squeeze: Some(12.0),
        fold_to_squeeze: Some(65.0),
        ..base_stats()
    };
    let opp = PlayerStatsOpportunities {
        squeeze_opportunities: 50,
        faced_squeeze_opportunities: 20,
        ..base_opp()
    };
    let out = description_rules::evaluate(&stats, &opp);
    let result = find(&out, AGGRESSIVE_PREFLOP_FOLDS_TO_RERAISE);
    assert!(result.is_some(), "squeeze=12% and fold_to_squeeze=65% must match on the squeeze leg alone");
    let result = result.unwrap();
    assert_eq!(result.evidence, vec![
        description_rules::Evidence {
            stat_name: "squeeze".to_string(),
            value: Some(12.0),
            opportunities: opp.squeeze_opportunities,
        },
        description_rules::Evidence {
            stat_name: "fold_to_squeeze".to_string(),
            value: Some(65.0),
            opportunities: opp.faced_squeeze_opportunities,
        },
    ]);
}

#[test]
fn aggressive_preflop_folds_to_reraise_does_not_match_when_only_the_aggression_half_of_a_leg_qualifies() {
    // 3-bets often, but doesn't give up to a 4-bet -- not the leak this rule targets.
    let stats = PlayerStats {
        three_bet: Some(10.0),
        fold_to_four_bet: Some(20.0),
        squeeze: Some(3.0),
        fold_to_squeeze: Some(10.0),
        ..base_stats()
    };
    let opp = PlayerStatsOpportunities {
        three_bet_opportunities: 100,
        faced_4bet_opportunities: 15,
        squeeze_opportunities: 50,
        faced_squeeze_opportunities: 20,
        ..base_opp()
    };
    let out = description_rules::evaluate(&stats, &opp);
    assert!(
        find(&out, AGGRESSIVE_PREFLOP_FOLDS_TO_RERAISE).is_none(),
        "neither leg has both its aggression and fold-back condition satisfied"
    );
}

#[test]
fn aggressive_preflop_folds_to_reraise_produces_one_combined_result_when_both_legs_qualify() {
    let stats = PlayerStats {
        three_bet: Some(15.0),
        fold_to_four_bet: Some(70.0),
        squeeze: Some(14.0),
        fold_to_squeeze: Some(80.0),
        ..base_stats()
    };
    let opp = PlayerStatsOpportunities {
        three_bet_opportunities: 100,
        faced_4bet_opportunities: 15,
        squeeze_opportunities: 50,
        faced_squeeze_opportunities: 20,
        ..base_opp()
    };
    let out = description_rules::evaluate(&stats, &opp);
    let matches: Vec<_> = out
        .iter()
        .filter(|r| r.rule_id == AGGRESSIVE_PREFLOP_FOLDS_TO_RERAISE)
        .collect();
    assert_eq!(matches.len(), 1, "both legs qualifying must still produce exactly one result");
    let result = matches[0];
    assert_eq!(result.evidence.len(), 4, "evidence must cover both qualifying legs");
    let stat_names: Vec<&str> = result.evidence.iter().map(|e| e.stat_name.as_str()).collect();
    assert_eq!(
        stat_names,
        vec!["three_bet", "fold_to_four_bet", "squeeze", "fold_to_squeeze"]
    );
}

#[test]
fn aggressive_preflop_folds_to_reraise_confidence_basis_is_the_minimum_across_both_legs() {
    let stats = PlayerStats {
        three_bet: Some(15.0),
        fold_to_four_bet: Some(70.0), // opportunities below -> 40 -> 20%
        squeeze: Some(14.0),
        fold_to_squeeze: Some(80.0), // opportunities below -> 200 -> 100%, but not the true min
        ..base_stats()
    };
    let opp = PlayerStatsOpportunities {
        three_bet_opportunities: 200,
        faced_4bet_opportunities: 40, // the true minimum across all four counts
        squeeze_opportunities: 200,
        faced_squeeze_opportunities: 200,
        ..base_opp()
    };
    let out = description_rules::evaluate(&stats, &opp);
    let result = find(&out, AGGRESSIVE_PREFLOP_FOLDS_TO_RERAISE).expect("expected a match");
    assert_eq!(
        result.confidence_pct,
        Some(20),
        "basis must be min(200, 40, 200, 200)=40 -> 20%, the weakest of all four contributing counts"
    );
}

// --- shared behaviors ---

#[test]
fn multiple_matching_rules_are_all_returned_and_sorted_by_confidence_descending() {
    let stats = PlayerStats {
        vpip: Some(40.0),
        pfr: Some(10.0),
        three_bet: Some(4.0),
        ..base_stats()
    };
    let opp = PlayerStatsOpportunities {
        hands: 200,                  // -> 100% confidence
        three_bet_opportunities: 40, // -> 20% confidence
        ..base_opp()
    };
    let out = description_rules::evaluate(&stats, &opp);
    assert!(find(&out, LOOSE_PASSIVE).is_some());
    assert!(find(&out, RARELY_3BETS).is_some());
    // Sorted descending by confidence.
    for pair in out.windows(2) {
        assert!(pair[0].confidence_pct >= pair[1].confidence_pct);
    }
}

#[test]
fn no_stats_at_all_produces_an_empty_list() {
    let out = description_rules::evaluate(&base_stats(), &base_opp());
    assert!(out.is_empty());
}

// --- evidence accuracy (RuleResult.evidence must match the
// underlying accumulator exactly, independent of whatever basis a rule's own
// confidence math uses) ---

#[test]
fn evidence_matches_the_accumulator_for_a_loose_aggressive_reg() {
    let stats = PlayerStats {
        vpip: Some(42.0),
        pfr: Some(35.0),
        three_bet: Some(9.5),
        fold_to_three_bet: Some(45.0),
        four_bet: None,
        fold_to_four_bet: None,
        rfi: None,
        limp: None,
        cold_call: None,
        squeeze: None,
        fold_to_squeeze: None,
        c_bet: Some(68.0),
        fold_to_c_bet: Some(35.0),
        aggression_factor: Some(2.8),
        wtsd: Some(27.0),
        wsd: Some(48.0),
        steal_attempt: None,
        fold_to_steal: None,
    };
    let opp = PlayerStatsOpportunities {
        hands: 400,
        three_bet_opportunities: 150,
        faced_3bet_opportunities: 60,
        four_bet_opportunities: 0,
        faced_4bet_opportunities: 0,
        rfi_limp_opportunities: 0,
        cold_call_opportunities: 0,
        squeeze_opportunities: 0,
        faced_squeeze_opportunities: 0,
        cbet_opportunities: 200,
        faced_cbet_opportunities: 90,
        saw_flop_hands: 180,
        went_to_showdown_hands: 70,
        postflop_calls: 120,
        postflop_bets_raises: 260,
        steal_attempt_opportunities: 0,
        fold_to_steal_opportunities: 0,
    };
    let out = description_rules::evaluate(&stats, &opp);

    let loose_aggressive = find(&out, "loose-aggressive").expect("expected loose-aggressive match");
    assert_eq!(loose_aggressive.evidence.len(), 2);
    assert_eq!(loose_aggressive.evidence[0].stat_name, "vpip");
    assert_eq!(loose_aggressive.evidence[0].value, Some(42.0));
    assert_eq!(loose_aggressive.evidence[0].opportunities, opp.hands);
    assert_eq!(loose_aggressive.evidence[1].stat_name, "pfr");
    assert_eq!(loose_aggressive.evidence[1].value, Some(35.0));
    assert_eq!(loose_aggressive.evidence[1].opportunities, opp.hands);

    assert!(
        find(&out, LOOSE_PASSIVE).is_none(),
        "pfr=35 is above vpip-10=32, so Loose-passive must not also match"
    );

    let cbets_almost_always = find(&out, "cbets-almost-always");
    assert!(cbets_almost_always.is_none(), "68% c-bet is below the 75% threshold");
}

#[test]
fn evidence_matches_the_accumulator_for_a_nitty_rock() {
    let stats = PlayerStats {
        vpip: Some(12.0),
        pfr: Some(9.0),
        three_bet: Some(2.0),
        fold_to_three_bet: Some(80.0),
        four_bet: None,
        fold_to_four_bet: None,
        rfi: None,
        limp: None,
        cold_call: None,
        squeeze: None,
        fold_to_squeeze: None,
        c_bet: Some(30.0),
        fold_to_c_bet: Some(20.0),
        aggression_factor: Some(0.6),
        wtsd: Some(8.0),
        wsd: Some(60.0),
        steal_attempt: None,
        fold_to_steal: None,
    };
    let opp = PlayerStatsOpportunities {
        hands: 300,
        three_bet_opportunities: 80,
        faced_3bet_opportunities: 30,
        four_bet_opportunities: 0,
        faced_4bet_opportunities: 0,
        rfi_limp_opportunities: 0,
        cold_call_opportunities: 0,
        squeeze_opportunities: 0,
        faced_squeeze_opportunities: 0,
        cbet_opportunities: 60,
        faced_cbet_opportunities: 40,
        saw_flop_hands: 90,
        went_to_showdown_hands: 20,
        postflop_calls: 50,
        postflop_bets_raises: 30,
        steal_attempt_opportunities: 0,
        fold_to_steal_opportunities: 0,
    };
    let out = description_rules::evaluate(&stats, &opp);

    let very_selective = find(&out, VERY_SELECTIVE).expect("expected nitty-very-selective match");
    assert_eq!(very_selective.evidence, vec![description_rules::Evidence {
        stat_name: "vpip".to_string(),
        value: stats.vpip,
        opportunities: opp.hands,
    }]);

    let folds_to_3bet = find(&out, "folds-a-lot-to-3bets").expect("expected folds-a-lot-to-3bets match");
    assert_eq!(folds_to_3bet.evidence[0].opportunities, opp.faced_3bet_opportunities);
    assert_eq!(folds_to_3bet.evidence[0].value, stats.fold_to_three_bet);

    let never_gets_there = find(&out, "never-gets-there").expect("expected never-gets-there match");
    // wtsd's evidence opportunities is its true denominator (saw_flop_hands),
    // even though this rule's own confidence basis is went_to_showdown_hands.
    assert_eq!(never_gets_there.evidence[0].stat_name, "wtsd");
    assert_eq!(never_gets_there.evidence[0].opportunities, opp.saw_flop_hands);
    assert_ne!(opp.saw_flop_hands, opp.went_to_showdown_hands);
}

#[test]
fn evidence_matches_the_accumulator_for_a_calling_station() {
    let stats = PlayerStats {
        vpip: Some(38.0),
        pfr: Some(6.0),
        three_bet: Some(1.0),
        fold_to_three_bet: Some(70.0),
        four_bet: None,
        fold_to_four_bet: None,
        rfi: None,
        limp: None,
        cold_call: None,
        squeeze: None,
        fold_to_squeeze: None,
        c_bet: Some(45.0),
        fold_to_c_bet: Some(20.0),
        aggression_factor: Some(0.8),
        wtsd: Some(33.0),
        wsd: Some(28.0),
        steal_attempt: None,
        fold_to_steal: None,
    };
    let opp = PlayerStatsOpportunities {
        hands: 250,
        three_bet_opportunities: 70,
        faced_3bet_opportunities: 25,
        four_bet_opportunities: 0,
        faced_4bet_opportunities: 0,
        rfi_limp_opportunities: 0,
        cold_call_opportunities: 0,
        squeeze_opportunities: 0,
        faced_squeeze_opportunities: 0,
        cbet_opportunities: 80,
        faced_cbet_opportunities: 55,
        saw_flop_hands: 140,
        went_to_showdown_hands: 46,
        postflop_calls: 100,
        postflop_bets_raises: 40,
        steal_attempt_opportunities: 0,
        fold_to_steal_opportunities: 0,
    };
    let out = description_rules::evaluate(&stats, &opp);

    let calling_station = find(&out, CALLING_STATION).expect("expected calling-station match");
    assert_eq!(calling_station.evidence[0].stat_name, "wtsd");
    assert_eq!(calling_station.evidence[0].value, stats.wtsd);
    assert_eq!(calling_station.evidence[0].opportunities, opp.saw_flop_hands);
    assert_eq!(calling_station.evidence[1].stat_name, "wsd");
    assert_eq!(calling_station.evidence[1].value, stats.wsd);
    assert_eq!(calling_station.evidence[1].opportunities, opp.went_to_showdown_hands);
    assert_eq!(calling_station.category, description_rules::RuleCategory::Tendency);

    let passive_postflop = find(&out, PASSIVE_POSTFLOP).expect("expected passive-postflop match");
    assert_eq!(passive_postflop.evidence[0].opportunities, opp.postflop_calls);
}

// --- observation/advice split: every rule must produce both non-empty,
// and neither field may still carry an em dash as an observation/advice
// separator (that job now belongs to having two fields, not punctuation). ---

/// All 35 rule ids `evaluate` can currently produce (the module doc names 22
/// single-stat rules from batches 1-2 plus 13 more from batch 3, including
/// composites -- 35 total, not 31: this test's own tally, not a guess).
/// Several rule families threshold the *same* stat in mutually exclusive
/// directions (e.g. `three_bet <= 4.0` for `rarely-3bets` vs.
/// `three_bet >= 10.0` for `3bets-often`), so no single `PlayerStats` value
/// can fire all of them at once. This test instead runs five independent,
/// hand-picked scenarios and takes the union of rule ids fired across all of
/// them, which this list confirms covers every rule -- full coverage, just
/// not in one `evaluate` call.
const ALL_RULE_IDS: &[&str] = &[
    "loose-passive",
    "loose-aggressive",
    "raises-face-up",
    "nitty-very-selective",
    "folds-a-lot-to-3bets",
    "doesnt-fold-to-3bets",
    "3bets-often",
    "rarely-3bets",
    "combative-preflop",
    "folds-a-lot-to-cbets",
    "doesnt-fold-to-cbets",
    "preflop-raiser-postflop-pushover",
    "cbets-almost-always",
    "cbets-selectively",
    "bets-flop-wont-back-it-up",
    "calling-station",
    "strong-at-showdown",
    "aggressive-but-light-at-showdown",
    "never-gets-there",
    "pressure-folder",
    "passive-postflop",
    "aggressive-postflop",
    "squeezes-aggressively",
    "rarely-squeezes",
    "folds-a-lot-to-squeezes",
    "4bets-aggressively",
    "rarely-4bets",
    "folds-a-lot-to-4bets",
    "cold-calls-too-much",
    "steals-too-often",
    "rarely-steals",
    "overfolds-to-steals",
    "defends-blinds-too-wide",
    "cold-calls-wide-folds-to-cbets",
    "aggressive-preflop-folds-to-reraise",
];

#[test]
fn every_rule_produces_a_non_empty_observation_and_advice_with_no_dash_separator() {
    use std::collections::HashSet;

    // Scenario 1: the "wide/aggressive" side of every family that can be hit
    // together without contradicting itself on a shared stat.
    let stats1 = PlayerStats {
        vpip: Some(45.0),
        pfr: Some(30.0),
        three_bet: Some(15.0),
        fold_to_three_bet: Some(35.0),
        four_bet: Some(45.0),
        fold_to_four_bet: Some(70.0),
        rfi: None,
        limp: None,
        cold_call: Some(35.0),
        squeeze: Some(15.0),
        fold_to_squeeze: Some(70.0),
        c_bet: Some(80.0),
        fold_to_c_bet: Some(65.0),
        aggression_factor: Some(1.2),
        wtsd: Some(35.0),
        wsd: Some(30.0),
        steal_attempt: Some(50.0),
        fold_to_steal: Some(80.0),
    };
    let opp1 = PlayerStatsOpportunities {
        hands: 200,
        three_bet_opportunities: 200,
        faced_3bet_opportunities: 200,
        four_bet_opportunities: 0,
        faced_4bet_opportunities: 200,
        rfi_limp_opportunities: 0,
        cold_call_opportunities: 200,
        squeeze_opportunities: 200,
        faced_squeeze_opportunities: 200,
        cbet_opportunities: 200,
        faced_cbet_opportunities: 200,
        saw_flop_hands: 300,
        went_to_showdown_hands: 200,
        postflop_calls: 200,
        postflop_bets_raises: 200,
        steal_attempt_opportunities: 200,
        fold_to_steal_opportunities: 200,
    };

    // Scenario 2: the "tight/passive" side of the same families, plus the
    // "rarely" variants that scenario 1's thresholds exclude.
    let stats2 = PlayerStats {
        vpip: Some(15.0),
        pfr: Some(8.0),
        three_bet: Some(2.0),
        fold_to_three_bet: Some(80.0),
        four_bet: Some(8.0),
        fold_to_four_bet: Some(20.0),
        rfi: None,
        limp: None,
        cold_call: None,
        squeeze: Some(3.0),
        fold_to_squeeze: Some(20.0),
        c_bet: Some(30.0),
        fold_to_c_bet: Some(20.0),
        aggression_factor: Some(0.9),
        wtsd: Some(10.0),
        wsd: Some(60.0),
        steal_attempt: Some(15.0),
        fold_to_steal: Some(35.0),
    };
    let opp2 = PlayerStatsOpportunities {
        hands: 200,
        three_bet_opportunities: 200,
        faced_3bet_opportunities: 200,
        four_bet_opportunities: 0,
        faced_4bet_opportunities: 200,
        rfi_limp_opportunities: 0,
        cold_call_opportunities: 0,
        squeeze_opportunities: 200,
        faced_squeeze_opportunities: 200,
        cbet_opportunities: 200,
        faced_cbet_opportunities: 200,
        saw_flop_hands: 300,
        went_to_showdown_hands: 200,
        postflop_calls: 200,
        postflop_bets_raises: 200,
        steal_attempt_opportunities: 200,
        fold_to_steal_opportunities: 200,
    };

    // Scenario 3: raises-face-up, doesnt-fold-to-3bets and strong-at-showdown
    // each need a stat value scenarios 1-2's thresholds don't hit.
    let stats3 = PlayerStats {
        vpip: Some(22.0),
        pfr: Some(20.0),
        three_bet: Some(6.0),
        fold_to_three_bet: Some(25.0),
        four_bet: None,
        fold_to_four_bet: None,
        rfi: None,
        limp: None,
        cold_call: None,
        squeeze: None,
        fold_to_squeeze: None,
        c_bet: None,
        fold_to_c_bet: None,
        aggression_factor: None,
        wtsd: Some(30.0),
        wsd: Some(60.0),
        steal_attempt: None,
        fold_to_steal: None,
    };
    let opp3 = PlayerStatsOpportunities {
        hands: 200,
        three_bet_opportunities: 200,
        faced_3bet_opportunities: 200,
        went_to_showdown_hands: 200,
        saw_flop_hands: 300,
        ..base_opp()
    };

    // Scenario 4: aggressive-but-light-at-showdown needs wsd <= 40, which
    // contradicts scenario 3's strong-at-showdown (wsd >= 55) on the same field.
    let stats4 = PlayerStats {
        wtsd: Some(25.0),
        wsd: Some(30.0),
        aggression_factor: Some(2.5),
        ..base_stats()
    };
    let opp4 = PlayerStatsOpportunities {
        went_to_showdown_hands: 200,
        saw_flop_hands: 300,
        ..base_opp()
    };

    // Scenario 5: pressure-folder and aggressive-postflop both need a high
    // aggression_factor, but with wtsd low enough to also stay clear of
    // calling-station/strong-at-showdown (which need wsd, left None here).
    let stats5 = PlayerStats {
        wtsd: Some(10.0),
        aggression_factor: Some(4.0),
        ..base_stats()
    };
    let opp5 = PlayerStatsOpportunities {
        went_to_showdown_hands: 200,
        saw_flop_hands: 300,
        postflop_calls: 200,
        postflop_bets_raises: 200,
        ..base_opp()
    };

    let mut seen_ids: HashSet<String> = HashSet::new();
    let mut all_results = Vec::new();
    for (stats, opp) in [
        (&stats1, &opp1),
        (&stats2, &opp2),
        (&stats3, &opp3),
        (&stats4, &opp4),
        (&stats5, &opp5),
    ] {
        let out = description_rules::evaluate(stats, opp);
        for result in out {
            seen_ids.insert(result.rule_id.clone());
            all_results.push(result);
        }
    }

    for result in &all_results {
        assert!(
            !result.observation.is_empty(),
            "rule {} produced an empty observation",
            result.rule_id
        );
        assert!(
            !result.advice.is_empty(),
            "rule {} produced an empty advice",
            result.rule_id
        );
        assert!(
            !result.observation.contains(" — "),
            "rule {} observation still carries an em-dash separator: {:?}",
            result.rule_id,
            result.observation
        );
        assert!(
            !result.advice.contains(" — "),
            "rule {} advice still carries an em-dash separator: {:?}",
            result.rule_id,
            result.advice
        );
    }

    for expected_id in ALL_RULE_IDS {
        assert!(
            seen_ids.contains(*expected_id),
            "rule {expected_id} was not fired by any of the five scenarios -- coverage gap"
        );
    }
    assert_eq!(
        seen_ids.len(),
        ALL_RULE_IDS.len(),
        "the five scenarios fired a rule id not present in ALL_RULE_IDS -- update the list"
    );
}
