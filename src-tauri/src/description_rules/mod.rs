use serde::Serialize;

use crate::stats::{PlayerStats, PlayerStatsOpportunities};

/// TENDENCY describes an observed frequency/style pattern (what the player
/// does); EXPLOIT pairs a specific, actionable weakness with the counter to
/// play against it. A "Calling Station" tendency and a "Recreational"
/// classification are different layers and can both be true of the same
/// player at once — this enum never competes with `classification`'s single
/// best-match archetype.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum RuleCategory {
    Tendency,
    Exploit,
}

/// UI-facing confidence band. Bands split the 0..100 `confidence_pct` range
/// produced by `confidence_tier` below: High from 100 opportunities up
/// (>=50%), Medium from 40 (>=20%), Low from the 10-opportunity floor,
/// InsufficientData below that floor (no conclusion is shown at all). The
/// bands track common poker-tracker convention, not re-derived math.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ConfidenceTier {
    High,
    Medium,
    Low,
    InsufficientData,
}

/// One stat that fed a `RuleResult`'s conclusion, carrying the stat's own
/// true opportunity count from `PlayerStatsOpportunities` (e.g. `wtsd`'s is
/// `saw_flop_hands`, not whatever basis a particular rule's confidence math
/// happens to use) — kept independent of `RuleResult::confidence_pct` so
/// evidence can be verified against the accumulator directly, regardless of
/// which basis a given rule's formula picks.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Evidence {
    pub stat_name: String,
    pub value: Option<f64>,
    pub opportunities: i64,
}

fn evidence(stat_name: &str, value: Option<f64>, opportunities: i64) -> Evidence {
    Evidence {
        stat_name: stat_name.to_string(),
        value,
        opportunities,
    }
}

/// Structured replacement for the old bare `{ text, confidence }` pair.
/// `confidence_pct`/`confidence_tier` are per-conclusion, never
/// a single global player score — a player can be shown as both a strong
/// Tendency and a low-confidence Exploit at once.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuleResult {
    pub rule_id: String,
    pub category: RuleCategory,
    /// What the opponent does. No advice, no imperative verbs addressed to
    /// the reader — that belongs in `advice`.
    pub observation: String,
    /// What the reader should do about it. Imperative, addressed to the
    /// reader — never a restatement of `observation`.
    pub advice: String,
    pub confidence_pct: Option<u8>,
    pub confidence_tier: ConfidenceTier,
    pub evidence: Vec<Evidence>,
}

/// `min(100, round(opportunities / 200 * 100))` — the exact formula every
/// rule used before this refactor, now encapsulated behind one function
/// instead of being inlined at 22 call sites, so it can be revised later
/// without touching every rule. `opportunities` is always the rule's own
/// named confidence-basis count, never `hands` as a fallback — the caller
/// passes whichever field/`min()` the rule's definition names. Below 10
/// opportunities there isn't enough data to show a conclusion at all.
pub fn confidence_tier(opportunities: i64) -> (Option<u8>, ConfidenceTier) {
    if opportunities < 10 {
        return (None, ConfidenceTier::InsufficientData);
    }
    let pct = (opportunities as f64 / 200.0 * 100.0).round().min(100.0) as u8;
    let tier = if pct >= 50 {
        ConfidenceTier::High
    } else if pct >= 20 {
        ConfidenceTier::Medium
    } else {
        ConfidenceTier::Low
    };
    (Some(pct), tier)
}

#[allow(clippy::too_many_arguments)]
fn push(
    out: &mut Vec<RuleResult>,
    matched: bool,
    rule_id: &str,
    category: RuleCategory,
    opportunities: i64,
    observation: &str,
    advice: &str,
    evidence: Vec<Evidence>,
) {
    if !matched {
        return;
    }
    let (confidence_pct, confidence_tier) = confidence_tier(opportunities);
    if confidence_pct.is_none() {
        return;
    }
    out.push(RuleResult {
        rule_id: rule_id.to_string(),
        category,
        observation: observation.to_string(),
        advice: advice.to_string(),
        confidence_pct,
        confidence_tier,
        evidence,
    });
}

/// Evaluates every description rule against a player's stats. Batches 1 and
/// 2 are the original 22 single-stat rules. Batch 3 adds rules for the
/// derived stats: `squeeze`/`fold_to_squeeze` (the latter mirroring
/// `fold_to_three_bet`'s threshold/style), `four_bet`/`rarely-4bets`/
/// `fold_to_four_bet`/`cold_call`, the steal rules (`steals-too-often`/
/// `rarely-steals`/`overfolds-to-steals`/`defends-blinds-too-wide`), and the
/// first two *composite* rules — `cold-calls-wide-folds-to-cbets` and
/// `aggressive-preflop-folds-to-reraise` — each combining two
/// already-computed stats into one read instead of thresholding a single
/// stat. Composite rules reuse their component rules' existing thresholds
/// verbatim (never a new calibration) and use the minimum of their
/// contributing opportunity counts as the confidence basis, since a
/// composite read is only as strong as its weaker leg. The rest of batch 3
/// (donk_bet, check_raise, and the other derived stats) is not implemented
/// yet — do not add a rule here that needs a stat not already in
/// `PlayerStats`/`PlayerStatsOpportunities`.
///
/// Every trigger, confidence basis, and observation/advice string below is
/// deliberate — the wording was specifically reviewed against the hard rule
/// against inferring villain range composition from frequency alone, so do
/// not reword any text field even for style.
pub fn evaluate(stats: &PlayerStats, opp: &PlayerStatsOpportunities) -> Vec<RuleResult> {
    use RuleCategory::{Exploit, Tendency};

    let mut out = Vec::new();

    // --- batch 1 ---

    if let (Some(vpip), Some(pfr)) = (stats.vpip, stats.pfr) {
        push(
            &mut out,
            vpip >= 40.0 && pfr <= vpip - 10.0,
            "loose-passive",
            Tendency,
            opp.hands,
            "Loose-passive: calls wide, rarely raises.",
            "Value bet, don't bluff.",
            vec![
                evidence("vpip", stats.vpip, opp.hands),
                evidence("pfr", stats.pfr, opp.hands),
            ],
        );
        push(
            &mut out,
            vpip >= 40.0 && pfr >= 28.0,
            "loose-aggressive",
            Tendency,
            opp.hands,
            "Loose-aggressive: wide range, bets/raises often.",
            "Give less credit, call wider.",
            vec![
                evidence("vpip", stats.vpip, opp.hands),
                evidence("pfr", stats.pfr, opp.hands),
            ],
        );
        push(
            &mut out,
            vpip >= 20.0 && (vpip - pfr) <= 3.0,
            "raises-face-up",
            Tendency,
            opp.hands,
            "Barely limps or flats preflop: if he's in the pot, he's usually raising.",
            "Read his preflop strength easily from that raise-or-fold pattern.",
            vec![
                evidence("vpip", stats.vpip, opp.hands),
                evidence("pfr", stats.pfr, opp.hands),
            ],
        );
    }
    if let Some(vpip) = stats.vpip {
        push(
            &mut out,
            vpip <= 18.0,
            "nitty-very-selective",
            Tendency,
            opp.hands,
            "Very selective preflop.",
            "Respect his raises and avoid marginal spots.",
            vec![evidence("vpip", stats.vpip, opp.hands)],
        );
    }

    if let Some(f3b) = stats.fold_to_three_bet {
        push(
            &mut out,
            f3b >= 65.0,
            "folds-a-lot-to-3bets",
            Exploit,
            opp.faced_3bet_opportunities,
            "Folds a lot to 3-bets.",
            "3-bet him wider.",
            vec![evidence(
                "fold_to_three_bet",
                stats.fold_to_three_bet,
                opp.faced_3bet_opportunities,
            )],
        );
        push(
            &mut out,
            f3b <= 30.0,
            "doesnt-fold-to-3bets",
            Exploit,
            opp.faced_3bet_opportunities,
            "Doesn't fold to 3-bets.",
            "Only 3-bet him for value.",
            vec![evidence(
                "fold_to_three_bet",
                stats.fold_to_three_bet,
                opp.faced_3bet_opportunities,
            )],
        );
    }

    if let Some(tb) = stats.three_bet {
        push(
            &mut out,
            tb >= 10.0,
            "3bets-often",
            Tendency,
            opp.three_bet_opportunities,
            "3-bets often, with a wide range.",
            "Don't overfold to his 3-bets.",
            vec![evidence("three_bet", stats.three_bet, opp.three_bet_opportunities)],
        );
        push(
            &mut out,
            tb <= 4.0,
            "rarely-3bets",
            Tendency,
            opp.three_bet_opportunities,
            "Rarely 3-bets; when he does, it's premium.",
            "Respect his 3-bets.",
            vec![evidence("three_bet", stats.three_bet, opp.three_bet_opportunities)],
        );
    }

    if let (Some(tb), Some(f3b)) = (stats.three_bet, stats.fold_to_three_bet) {
        push(
            &mut out,
            tb >= 9.0 && f3b <= 40.0,
            "combative-preflop",
            Tendency,
            opp.three_bet_opportunities.min(opp.faced_3bet_opportunities),
            "3-bets a lot and doesn't fold to 3-bets.",
            "Expect a big preflop pot if you get involved with him.",
            vec![
                evidence("three_bet", stats.three_bet, opp.three_bet_opportunities),
                evidence(
                    "fold_to_three_bet",
                    stats.fold_to_three_bet,
                    opp.faced_3bet_opportunities,
                ),
            ],
        );
    }

    if let Some(fcb) = stats.fold_to_c_bet {
        push(
            &mut out,
            fcb >= 60.0,
            "folds-a-lot-to-cbets",
            Exploit,
            opp.faced_cbet_opportunities,
            "Folds a lot to continuation bets.",
            "C-bet him often.",
            vec![evidence(
                "fold_to_c_bet",
                stats.fold_to_c_bet,
                opp.faced_cbet_opportunities,
            )],
        );
        push(
            &mut out,
            fcb <= 30.0,
            "doesnt-fold-to-cbets",
            Exploit,
            opp.faced_cbet_opportunities,
            "Doesn't fold to c-bets.",
            "Don't bluff him without a follow-up plan.",
            vec![evidence(
                "fold_to_c_bet",
                stats.fold_to_c_bet,
                opp.faced_cbet_opportunities,
            )],
        );

        if let Some(pfr) = stats.pfr {
            push(
                &mut out,
                pfr >= 22.0 && fcb >= 55.0,
                "preflop-raiser-postflop-pushover",
                Exploit,
                opp.faced_cbet_opportunities,
                "Raises a lot preflop but gives up easily to a c-bet.",
                "Apply pressure back at him postflop.",
                vec![
                    evidence("pfr", stats.pfr, opp.hands),
                    evidence(
                        "fold_to_c_bet",
                        stats.fold_to_c_bet,
                        opp.faced_cbet_opportunities,
                    ),
                ],
            );
        }
    }

    if let Some(cb) = stats.c_bet {
        push(
            &mut out,
            cb >= 75.0,
            "cbets-almost-always",
            Tendency,
            opp.cbet_opportunities,
            "C-bets nearly every flop, which is low information.",
            "Float or raise him more.",
            vec![evidence("c_bet", stats.c_bet, opp.cbet_opportunities)],
        );
        push(
            &mut out,
            cb <= 40.0,
            "cbets-selectively",
            Tendency,
            opp.cbet_opportunities,
            "Only c-bets a minority of flops.",
            "Respect the c-bets he makes.",
            vec![evidence("c_bet", stats.c_bet, opp.cbet_opportunities)],
        );

        if let Some(af) = stats.aggression_factor {
            push(
                &mut out,
                cb >= 55.0 && af <= 1.3,
                "bets-flop-wont-back-it-up",
                Exploit,
                opp.cbet_opportunities.min(opp.postflop_calls),
                "C-bets the flop often but is passive the rest of the way.",
                "Float him and apply turn/river aggression.",
                vec![
                    evidence("c_bet", stats.c_bet, opp.cbet_opportunities),
                    evidence("aggression_factor", stats.aggression_factor, opp.postflop_calls),
                ],
            );
        }
    }

    if let (Some(wtsd), Some(wsd)) = (stats.wtsd, stats.wsd) {
        push(
            &mut out,
            wtsd >= 30.0 && wsd <= 35.0,
            "calling-station",
            Tendency,
            opp.went_to_showdown_hands,
            "Reaches showdown often, wins little there.",
            "Value bet thin, don't bluff.",
            vec![
                evidence("wtsd", stats.wtsd, opp.saw_flop_hands),
                evidence("wsd", stats.wsd, opp.went_to_showdown_hands),
            ],
        );
        push(
            &mut out,
            wtsd >= 25.0 && wsd >= 55.0,
            "strong-at-showdown",
            Tendency,
            opp.went_to_showdown_hands,
            "Wins most showdowns he reaches.",
            "Don't bluff-catch rivers light against him.",
            vec![
                evidence("wtsd", stats.wtsd, opp.saw_flop_hands),
                evidence("wsd", stats.wsd, opp.went_to_showdown_hands),
            ],
        );

        if let Some(af) = stats.aggression_factor {
            push(
                &mut out,
                wtsd >= 20.0 && wsd <= 40.0 && af >= 2.0,
                "aggressive-but-light-at-showdown",
                Exploit,
                opp.went_to_showdown_hands,
                "Aggressive but light when he shows down; his big bets don't always mean a big hand.",
                "Look him up more.",
                vec![
                    evidence("wtsd", stats.wtsd, opp.saw_flop_hands),
                    evidence("wsd", stats.wsd, opp.went_to_showdown_hands),
                    evidence("aggression_factor", stats.aggression_factor, opp.postflop_calls),
                ],
            );
        }
    }

    if let Some(wtsd) = stats.wtsd {
        push(
            &mut out,
            wtsd <= 12.0,
            "never-gets-there",
            Tendency,
            opp.went_to_showdown_hands,
            "Rarely reaches showdown at all.",
            "Bet him off pots liberally.",
            vec![evidence("wtsd", stats.wtsd, opp.saw_flop_hands)],
        );

        if let Some(af) = stats.aggression_factor {
            push(
                &mut out,
                af >= 3.0 && wtsd <= 15.0,
                "pressure-folder",
                Exploit,
                opp.went_to_showdown_hands,
                "Applies a lot of postflop pressure but rarely reaches showdown.",
                "Call down lighter against him.",
                vec![
                    evidence("aggression_factor", stats.aggression_factor, opp.postflop_calls),
                    evidence("wtsd", stats.wtsd, opp.saw_flop_hands),
                ],
            );
        }
    }

    if let Some(af) = stats.aggression_factor {
        push(
            &mut out,
            af <= 1.0,
            "passive-postflop",
            Tendency,
            opp.postflop_calls,
            "Mostly calls postflop, rarely raises.",
            "Bet thin for value.",
            vec![evidence("aggression_factor", stats.aggression_factor, opp.postflop_calls)],
        );
        push(
            &mut out,
            af >= 3.5,
            "aggressive-postflop",
            Tendency,
            opp.postflop_bets_raises,
            "Bets and raises heavily postflop relative to calling.",
            "Don't overfold to the pressure.",
            vec![evidence("aggression_factor", stats.aggression_factor, opp.postflop_calls)],
        );
    }

    // --- batch 3 (partial): squeeze, fold_to_squeeze ---

    if let Some(sq) = stats.squeeze {
        push(
            &mut out,
            sq >= 12.0,
            "squeezes-aggressively",
            Tendency,
            opp.squeeze_opportunities,
            "Squeezes often when there's a raise and a caller ahead of him, and his squeezes aren't always premium.",
            "Continue lighter into his squeezes.",
            vec![evidence("squeeze", stats.squeeze, opp.squeeze_opportunities)],
        );
        push(
            &mut out,
            sq <= 4.0,
            "rarely-squeezes",
            Exploit,
            opp.squeeze_opportunities,
            "Almost never squeezes.",
            "Cold-call safely in front of him without fear of getting blown off the hand.",
            vec![evidence("squeeze", stats.squeeze, opp.squeeze_opportunities)],
        );
    }

    if let Some(fsq) = stats.fold_to_squeeze {
        push(
            &mut out,
            fsq >= 65.0,
            "folds-a-lot-to-squeezes",
            Exploit,
            opp.faced_squeeze_opportunities,
            "Folds a lot when squeezed after someone else's cold call.",
            "Squeeze him light for profit.",
            vec![evidence(
                "fold_to_squeeze",
                stats.fold_to_squeeze,
                opp.faced_squeeze_opportunities,
            )],
        );
    }

    // --- batch 3 (continued): four_bet, fold_to_four_bet, cold_call ---
    //
    // `four_bet`'s confidence basis is `faced_3bet_opportunities`, not a
    // separate `four_bet_opportunities` counter, by design: `four_bet` reuses the exact `facing_3bet` event already computed for
    // `fold_to_three_bet`, so its opportunity population *is*
    // `faced_3bet_opportunities`. Not a bug, not changed here.
    if let Some(fb) = stats.four_bet {
        push(
            &mut out,
            fb >= 40.0,
            "4bets-aggressively",
            Tendency,
            opp.faced_3bet_opportunities,
            "4-bets often when 3-bet, and his 4-bets aren't automatically premium.",
            "Look for a spot to continue against his 4-bets.",
            vec![evidence("four_bet", stats.four_bet, opp.faced_3bet_opportunities)],
        );
        push(
            &mut out,
            fb <= 10.0,
            "rarely-4bets",
            Tendency,
            opp.faced_3bet_opportunities,
            "Almost never 4-bets; when he does, it's about as strong as it gets.",
            "Fold anything but the top of your range.",
            vec![evidence("four_bet", stats.four_bet, opp.faced_3bet_opportunities)],
        );
    }

    if let Some(f4b) = stats.fold_to_four_bet {
        push(
            &mut out,
            f4b >= 60.0,
            "folds-a-lot-to-4bets",
            Exploit,
            opp.faced_4bet_opportunities,
            "Gives up his 3-bets to a 4-bet often.",
            "4-bet bluff him more.",
            vec![evidence(
                "fold_to_four_bet",
                stats.fold_to_four_bet,
                opp.faced_4bet_opportunities,
            )],
        );
    }

    if let Some(cc) = stats.cold_call {
        push(
            &mut out,
            cc >= 30.0,
            "cold-calls-too-much",
            Tendency,
            opp.cold_call_opportunities,
            "Flats raises a lot instead of 3-betting or folding: a wide, hard-to-pin-down range.",
            "Bet for value freely postflop.",
            vec![evidence("cold_call", stats.cold_call, opp.cold_call_opportunities)],
        );
    }

    // --- batch 3 (continued): steal_attempt, fold_to_steal. Condition/
    // basis/text are fixed by the rule definitions ("Steals too often"/"Rarely steals"/"Overfolds to steals"/"Defends
    // blinds too wide") — no new calibration, the stats and their
    // opportunity denominators already exist in PlayerStats/
    // PlayerStatsOpportunities, this only wires them into the rule engine.

    if let Some(sa) = stats.steal_attempt {
        push(
            &mut out,
            sa >= 45.0,
            "steals-too-often",
            Tendency,
            opp.steal_attempt_opportunities,
            "Opens light from the cutoff, button, or small blind.",
            "Defend your blinds wider and punish his late opens.",
            vec![evidence(
                "steal_attempt",
                stats.steal_attempt,
                opp.steal_attempt_opportunities,
            )],
        );
        push(
            &mut out,
            sa <= 20.0,
            "rarely-steals",
            Tendency,
            opp.steal_attempt_opportunities,
            "Rarely opens from late position; when he does, it's real.",
            "Give it more respect than a normal open.",
            vec![evidence(
                "steal_attempt",
                stats.steal_attempt,
                opp.steal_attempt_opportunities,
            )],
        );
    }

    if let Some(fts) = stats.fold_to_steal {
        push(
            &mut out,
            fts >= 75.0,
            "overfolds-to-steals",
            Exploit,
            opp.fold_to_steal_opportunities,
            "Folds his blinds to a steal almost every time.",
            "Steal against him relentlessly.",
            vec![evidence(
                "fold_to_steal",
                stats.fold_to_steal,
                opp.fold_to_steal_opportunities,
            )],
        );
        push(
            &mut out,
            fts <= 40.0,
            "defends-blinds-too-wide",
            Exploit,
            opp.fold_to_steal_opportunities,
            "Defends his blinds very wide against steals.",
            "Don't bother stealing light against him; he'll fight back.",
            vec![evidence(
                "fold_to_steal",
                stats.fold_to_steal,
                opp.fold_to_steal_opportunities,
            )],
        );
    }

    // --- batch 3 (composite): relational rules combining two stats. A
    // composite rule is only as strong as its weaker leg, so its
    // confidence basis is the *minimum* of its contributing opportunity
    // counts, never the first stat's or an average — same principle
    // `combative-preflop` and `bets-flop-wont-back-it-up` already use above.

    if let (Some(cc), Some(fcb)) = (stats.cold_call, stats.fold_to_c_bet) {
        push(
            &mut out,
            cc >= 30.0 && fcb >= 60.0,
            "cold-calls-wide-folds-to-cbets",
            Exploit,
            opp.cold_call_opportunities.min(opp.faced_cbet_opportunities),
            "Cold-calls a wide range but folds too often to continuation bets.",
            "Bet him off pots after he flats preflop; you don't need a real hand.",
            vec![
                evidence("cold_call", stats.cold_call, opp.cold_call_opportunities),
                evidence("fold_to_c_bet", stats.fold_to_c_bet, opp.faced_cbet_opportunities),
            ],
        );
    }

    // Two independent aggression-then-fold legs (3-bet/4-bet-fold,
    // squeeze/squeeze-fold), reusing each leg's own existing trigger
    // threshold verbatim rather than inventing a new one. Either leg alone is
    // enough to fire; if both qualify, this still produces one RuleResult
    // (not two), carrying evidence for every leg that qualified and a
    // confidence basis that is the minimum opportunity count across all of
    // them.
    let three_bet_fold_leg = stats.three_bet.zip(stats.fold_to_four_bet).map_or(
        false,
        |(tb, f4b)| tb >= 10.0 && f4b >= 60.0,
    );
    let squeeze_fold_leg = stats.squeeze.zip(stats.fold_to_squeeze).map_or(
        false,
        |(sq, fsq)| sq >= 12.0 && fsq >= 65.0,
    );
    if three_bet_fold_leg || squeeze_fold_leg {
        let mut composite_evidence = Vec::new();
        let mut leg_opportunities = Vec::new();
        if three_bet_fold_leg {
            composite_evidence.push(evidence("three_bet", stats.three_bet, opp.three_bet_opportunities));
            composite_evidence.push(evidence(
                "fold_to_four_bet",
                stats.fold_to_four_bet,
                opp.faced_4bet_opportunities,
            ));
            leg_opportunities.push(opp.three_bet_opportunities.min(opp.faced_4bet_opportunities));
        }
        if squeeze_fold_leg {
            composite_evidence.push(evidence("squeeze", stats.squeeze, opp.squeeze_opportunities));
            composite_evidence.push(evidence(
                "fold_to_squeeze",
                stats.fold_to_squeeze,
                opp.faced_squeeze_opportunities,
            ));
            leg_opportunities.push(opp.squeeze_opportunities.min(opp.faced_squeeze_opportunities));
        }
        let basis = leg_opportunities.into_iter().min().unwrap();
        push(
            &mut out,
            true,
            "aggressive-preflop-folds-to-reraise",
            Exploit,
            basis,
            "3-bets and squeezes with a wide range but gives up when re-raised back.",
            "4-bet or re-raise his preflop aggression light; he folds too much.",
            composite_evidence,
        );
    }

    out.sort_by(|a, b| b.confidence_pct.cmp(&a.confidence_pct));
    out
}

/// Resolves the rule results the HUD popup should show: the full rule engine
/// when compiled with `strategic-analysis` (an opt-in build feature),
/// otherwise an empty list — same flag-off shape as
/// `classification::resolve_for_player`. This function never inspects the
/// `auto-classification` feature, so TENDENCIES/EXPLOITS/CONFIDENCE compute identically whether or not
/// automatic classification is compiled into this build.
pub fn descriptions_for_player(
    stats: &PlayerStats,
    opportunities: &PlayerStatsOpportunities,
) -> Vec<RuleResult> {
    #[cfg(feature = "strategic-analysis")]
    {
        evaluate(stats, opportunities)
    }
    #[cfg(not(feature = "strategic-analysis"))]
    {
        let _ = (stats, opportunities);
        Vec::new()
    }
}
