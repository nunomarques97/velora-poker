//! Opponent engine, task T7: the scenario rules of section 8 of
//! `docs/specs/opponent-engine.md`, their context adaptation (section 5) and
//! the between-hands text guards (section 1).
//!
//! Rule boundaries are tested on hand-built inputs: a stat sits at the first
//! count whose *shrunk* value crosses the spec threshold, and one count
//! short of it. Where a test pins a boundary by hand, the arithmetic is in
//! its comment. The last tests run the whole path on generated PokerStars
//! hands imported like a file.

mod common;

use common::{import, player_id, Act, Table, HERO};
use velora_poker_lib::description_rules::{ConfidenceTier, RuleCategory};
use velora_poker_lib::engine::context::{BountyContext, SeatRelation};
use velora_poker_lib::engine::pooling::shrink;
use velora_poker_lib::engine::rules::{
    observation_template, Variant, BB_VS_SB_OVERDEFEND, FOLD_TO_STEAL_LOW, FOLD_TO_STEAL_SB_HIGH,
    H2H_OPPOSITES, HOVER_CARD_ADVICE_BUDGET, JAM_DEPTH_MAX_BB, KO_COVERING_HIGH, KO_COVER_GAP, LATE_CALLS_SHOVES,
    LATE_SHOVES_WIDE, LIMP_HIGH, POSTFLOP_OPPOSITES, PREFLOP_OPPOSITES, REFINED_OPPOSITES, SEAT_SUFFIX, SHOWDOWN_OPPOSITES,
    STAGE_EARLY_LIMP, TOURNAMENT_OPPOSITES,
};
use velora_poker_lib::engine::{
    evaluate_rules, latest_table_hand, player_rule_input, rule_def, stat_spec, templates,
    villain_context, EngineContext, EngineRuleResult, Family, FormFlag, FormatKey, H2hKey,
    ReadSample, RecentForm, RuleInput, SampleUnit, ShowdownTally, Side, SizingTell, StackBucket, Stage, StatKey,
    TemplateKind, RULES,
};
use velora_poker_lib::engine::{H2hStat, ShrunkStat, SizeBucket};

use StatKey::*;

// ------------------------------------------------------------------ helpers

fn prior(format: FormatKey, key: StatKey) -> f64 {
    stat_spec(key).builtin_prior(format)
}

fn stat(format: FormatKey, key: StatKey, hits: u32, n: u32) -> ShrunkStat {
    ShrunkStat::new(key, hits, n, prior(format, key))
}

fn input(format: FormatKey, stats: &[(StatKey, u32, u32)]) -> RuleInput {
    let mut input = RuleInput { format: Some(format), ..RuleInput::default() };
    for (key, hits, n) in stats {
        input.stats.insert(*key, stat(format, *key, *hits, *n));
    }
    input
}

fn cash(stats: &[(StatKey, u32, u32)]) -> RuleInput {
    input(FormatKey::Cash, stats)
}

fn read<'a>(reads: &'a [EngineRuleResult], id: &str) -> Option<&'a EngineRuleResult> {
    reads.iter().find(|r| r.rule_id == id)
}

fn fires(input: &RuleInput, id: &str) -> bool {
    read(&evaluate_rules(input), id).is_some()
}

fn ids(reads: &[EngineRuleResult]) -> Vec<&str> {
    reads.iter().map(|r| r.rule_id.as_str()).collect()
}

fn shrunk_at(format: FormatKey, key: StatKey, hits: u32, n: u32) -> f64 {
    shrink(f64::from(hits), f64::from(n), stat_spec(key).k, prior(format, key))
}

/// Smallest hit count whose shrunk value reaches `prior + delta`.
fn first_above(format: FormatKey, key: StatKey, n: u32, delta: f64) -> u32 {
    (0..=n)
        .find(|h| shrunk_at(format, key, *h, n) >= prior(format, key) + delta - 1e-9)
        .unwrap_or_else(|| panic!("{key:?}: no count of {n} reaches prior + {delta}"))
}

/// Largest hit count whose shrunk value is at most `prior − delta`.
fn last_below(format: FormatKey, key: StatKey, n: u32, delta: f64) -> u32 {
    (0..=n)
        .rev()
        .find(|h| shrunk_at(format, key, *h, n) <= prior(format, key) - delta + 1e-9)
        .unwrap_or_else(|| panic!("{key:?}: no count of {n} reaches prior - {delta}"))
}

/// The rule fires at the first count reaching `prior + delta` and not one
/// count below it. `base` adds stats the rule needs besides `key`.
fn assert_above(format: FormatKey, id: &str, key: StatKey, n: u32, delta: f64, base: &[(StatKey, u32, u32)]) -> u32 {
    let h = first_above(format, key, n, delta);
    assert!(h > 0, "{id}: boundary must sit above zero hits");
    let with = |hits| {
        let mut stats = base.to_vec();
        stats.push((key, hits, n));
        input(format, &stats)
    };
    assert!(fires(&with(h), id), "{id} must fire at {h}/{n}");
    assert!(!fires(&with(h - 1), id), "{id} must not fire at {}/{n}", h - 1);
    h
}

/// The rule fires at the last count at or under `prior − delta` and not one
/// count above it.
fn assert_below(format: FormatKey, id: &str, key: StatKey, n: u32, delta: f64, base: &[(StatKey, u32, u32)]) -> u32 {
    let h = last_below(format, key, n, delta);
    assert!(h < n, "{id}: boundary must sit below all hits");
    let with = |hits| {
        let mut stats = base.to_vec();
        stats.push((key, hits, n));
        input(format, &stats)
    };
    assert!(fires(&with(h), id), "{id} must fire at {h}/{n}");
    assert!(!fires(&with(h + 1), id), "{id} must not fire at {}/{n}", h + 1);
    h
}

fn context(format: FormatKey, eff: f64) -> EngineContext {
    let tournament = matches!(format, FormatKey::Mtt | FormatKey::Spin);
    EngineContext {
        source_hand_id: "300000001".into(),
        variant: Some(if tournament { "tournament" } else { "cash" }.into()),
        format,
        effective_stack_bb: Some(eff),
        stack_bucket: Some(StackBucket::of(eff)),
        stage: tournament.then_some(Stage::Middle),
        level: tournament.then_some(5),
        avg_stack_bb: Some(30.0),
        bounty: None,
        seat: None,
    }
}

fn seat(distance: u32, players: u32) -> SeatRelation {
    let side = if f64::from(distance) <= (f64::from(players) - 1.0) / 2.0 { Side::Left } else { Side::Right };
    SeatRelation {
        distance,
        side,
        acts_after_hero: side == Side::Left,
        direct_left: distance == 1,
        direct_right: distance == players - 1,
    }
}

// --------------------------------------------------------- profile (P12)

#[test]
fn profile_rules_fire_at_threshold_and_not_below() {
    // NIT: cash prior .27, k 20; n = 60: (6 + 5.4)/80 = .1425 ≤ .15 fires,
    // (7 + 5.4)/80 = .155 does not.
    assert!(fires(&cash(&[(Vpip, 6, 60), (Pfr, 5, 60)]), "pf.profile.nit"));
    assert!(!fires(&cash(&[(Vpip, 7, 60), (Pfr, 5, 60)]), "pf.profile.nit"));

    // Loose-passive: VPIP .40 needs (h + 5.4)/80 ≥ .40 → h = 27; PFR well under.
    let lp = cash(&[(Vpip, 27, 60), (Pfr, 3, 60), (Wtsd, 5, 30)]);
    assert!(fires(&lp, "pf.profile.loose_passive"));
    assert!(!fires(&lp, "pf.profile.station"), "WTSD .19 is no station");
    assert!(!fires(&cash(&[(Vpip, 26, 60), (Pfr, 3, 60)]), "pf.profile.loose_passive"));
    // PFR too close to VPIP: not passive.
    assert!(!fires(&cash(&[(Vpip, 27, 60), (Pfr, 20, 60)]), "pf.profile.loose_passive"));

    // Station: the same plus WTSD ≥ .32 (cash prior .27, k 20; n = 40:
    // (17 + 5.4)/60 = .373). The station read replaces loose-passive.
    let station = cash(&[(Vpip, 27, 60), (Pfr, 3, 60), (Wtsd, 17, 40)]);
    let reads = evaluate_rules(&station);
    assert!(read(&reads, "pf.profile.station").is_some());
    assert!(read(&reads, "pf.profile.loose_passive").is_none());
    let h = first_above(FormatKey::Cash, Wtsd, 40, 0.32 - prior(FormatKey::Cash, Wtsd));
    assert!(!fires(&cash(&[(Vpip, 27, 60), (Pfr, 3, 60), (Wtsd, h - 1, 40)]), "pf.profile.station"));

    // LAG: VPIP ≥ .32 and PFR ≥ .25 (PFR prior .20, k 20; n = 60:
    // (15 + 4)/80 = .2375 no, (16 + 4)/80 = .25 yes).
    assert!(fires(&cash(&[(Vpip, 30, 60), (Pfr, 16, 60)]), "pf.profile.lag"));
    assert!(!fires(&cash(&[(Vpip, 30, 60), (Pfr, 15, 60)]), "pf.profile.lag"));
    // VPIP (h + 5.4)/80 ≥ .32 → h = 21; 20 is short.
    assert!(fires(&cash(&[(Vpip, 21, 60), (Pfr, 16, 60)]), "pf.profile.lag"));
    assert!(!fires(&cash(&[(Vpip, 20, 60), (Pfr, 16, 60)]), "pf.profile.lag"));
}

#[test]
fn profile_small_sample_extreme_is_shrunk_below_threshold() {
    // 0 of 15 hands is a raw VPIP of 0%, but (0 + 5.4)/35 = .154 > .15.
    let tight = cash(&[(Vpip, 0, 15), (Pfr, 0, 15)]);
    assert!(evaluate_rules(&tight).is_empty(), "{:?}", ids(&evaluate_rules(&tight)));
    // Under the 15-hand floor nothing shows, however extreme.
    assert!(evaluate_rules(&cash(&[(Vpip, 14, 14), (Pfr, 0, 14)])).is_empty());
    // A big sample at the same raw rate does fire.
    assert!(fires(&cash(&[(Vpip, 0, 60), (Pfr, 0, 60)]), "pf.profile.nit"));
}

#[test]
fn profile_thresholds_follow_format_priors() {
    // Pro review (D107): the profile reads are deltas from the format's
    // priors; with the cash priors they are D106's absolute numbers (the
    // cash boundaries above are unchanged).
    let spin = FormatKey::Spin;
    // Spin & Go, VPIP prior .38, PFR .25: 27/60 VPIP and 3/60 PFR shrink to
    // (27 + 7.6)/80 = .4325 and (3 + 5)/80 = .10. D106's absolute .40 read
    // that as loose-passive; in a 3-max hyper it is an ordinary player.
    let ordinary = [(Vpip, 27, 60), (Pfr, 3, 60), (Wtsd, 5, 30)];
    assert!(fires(&cash(&ordinary), "pf.profile.loose_passive"));
    assert!(!fires(&input(spin, &ordinary), "pf.profile.loose_passive"));
    // Spin loose: VPIP ≥ .38 + .13 = .51 → (h + 7.6)/80 ≥ .51 → h = 34
    // (.52); 33 is .5075.
    assert!(fires(&input(spin, &[(Vpip, 34, 60), (Pfr, 3, 60), (Wtsd, 5, 30)]), "pf.profile.loose_passive"));
    assert!(!fires(&input(spin, &[(Vpip, 33, 60), (Pfr, 3, 60), (Wtsd, 5, 30)]), "pf.profile.loose_passive"));
    // Spin station: WTSD prior .30 → ≥ .35: (h + 6)/50 → h = 12 fires, 11 not.
    assert!(fires(&input(spin, &[(Vpip, 34, 60), (Pfr, 3, 60), (Wtsd, 12, 30)]), "pf.profile.station"));
    assert!(!fires(&input(spin, &[(Vpip, 34, 60), (Pfr, 3, 60), (Wtsd, 11, 30)]), "pf.profile.station"));

    // Spin nit: VPIP ≤ .38 − .12 = .26: 13/60 → (13 + 7.6)/80 = .2575 fires,
    // 14/60 → .27 does not. In cash 13/60 is .23, nowhere near a nit.
    assert!(fires(&input(spin, &[(Vpip, 13, 60), (Pfr, 5, 60)]), "pf.profile.nit"));
    assert!(!fires(&input(spin, &[(Vpip, 14, 60), (Pfr, 5, 60)]), "pf.profile.nit"));
    assert!(!fires(&cash(&[(Vpip, 13, 60), (Pfr, 5, 60)]), "pf.profile.nit"));
    // Zoom (prior .23) is tighter than cash: nit ≤ .11. 6/60 is a cash nit
    // ((6 + 5.4)/80 = .1425) but not a Zoom one ((6 + 4.6)/80 = .1325).
    assert!(fires(&cash(&[(Vpip, 6, 60), (Pfr, 5, 60)]), "pf.profile.nit"));
    assert!(!fires(&input(FormatKey::Zoom, &[(Vpip, 6, 60), (Pfr, 5, 60)]), "pf.profile.nit"));
    assert!(fires(&input(FormatKey::Zoom, &[(Vpip, 4, 60), (Pfr, 3, 60)]), "pf.profile.nit"));

    // MTT LAG: VPIP ≥ .24 + .05 = .29, PFR ≥ .17 + .05 = .22. 19/60 and
    // 15/60 → .2975 and .23 fire; 18/60 → .285 and 14/60 → .2175 do not.
    let mtt = FormatKey::Mtt;
    assert!(fires(&input(mtt, &[(Vpip, 19, 60), (Pfr, 15, 60)]), "pf.profile.lag"));
    assert!(!fires(&input(mtt, &[(Vpip, 18, 60), (Pfr, 15, 60)]), "pf.profile.lag"));
    assert!(!fires(&input(mtt, &[(Vpip, 19, 60), (Pfr, 14, 60)]), "pf.profile.lag"));
    // The same counts in cash are under the cash LAG line (.32).
    assert!(!fires(&cash(&[(Vpip, 19, 60), (Pfr, 15, 60)]), "pf.profile.lag"));
}

// ------------------------------------------------- opens and steals (P01, P02)

#[test]
fn rfi_rules_fire_at_threshold_and_not_below() {
    let c = FormatKey::Cash;
    assert_below(c, "pf.rfi.tight_early", RfiEp, 40, 0.06, &[]);
    assert_below(c, "pf.rfi.tight_early", RfiMp, 40, 0.06, &[]);
    assert_above(c, "pf.rfi.loose_early", RfiEp, 40, 0.08, &[]);
    assert_above(c, "pf.rfi.loose_early", RfiMp, 40, 0.08, &[]);
    assert_above(c, "pf.rfi.loose_late", RfiCo, 40, 0.12, &[]);
    assert_above(c, "pf.rfi.loose_late", RfiBtn, 40, 0.12, &[]);
    assert_below(c, "pf.rfi.tight_late", RfiBtn, 40, 0.12, &[]);
    // Under the 8-opportunity floor: 7 of 7 EP opens shows nothing.
    assert!(!fires(&cash(&[(RfiEp, 7, 7)]), "pf.rfi.loose_early"));
    // The rate shown is the raw one with its count; the evidence carries
    // raw, count and shrunk value.
    let reads = evaluate_rules(&cash(&[(RfiEp, 20, 40)]));
    let r = read(&reads, "pf.rfi.loose_early").unwrap();
    assert_eq!(r.observation, "Opens 50% (20/40) from early and middle seats.");
    assert_eq!(r.evidence[0].stat_name, "rfi_ep");
    assert_eq!(r.evidence[0].value, Some(50.0));
    assert_eq!(r.evidence[0].hits, 20);
    assert_eq!(r.evidence[0].opportunities, 40);
    // (20 + 15 × .16)/55 = .4073.
    assert_eq!(r.evidence[0].shrunk, Some(40.7));
}

#[test]
fn steal_rules_fire_at_threshold_and_not_below() {
    let c = FormatKey::Cash;
    assert_above(c, "pf.steal.high", Steal, 40, 0.12, &[]);
    assert_below(c, "pf.steal.low", Steal, 40, 0.12, &[]);
    // Spin & Go has its own prior (.48): the same 25/40 that fires in cash
    // ((25 + 5.25)/55 = .55 vs .47) does not in a Spin ((25 + 7.2)/55 = .5855
    // vs .60).
    assert!(fires(&cash(&[(Steal, 25, 40)]), "pf.steal.high"));
    assert!(!fires(&input(FormatKey::Spin, &[(Steal, 25, 40)]), "pf.steal.high"));
}

#[test]
fn sb_open_rule_fires_at_threshold_and_not_below() {
    // Pro review (D107, P13): small-blind raise-first-in against the big
    // blind. Cash prior .36, k 15: (h + 5.4)/55 ≥ .51 → 23 of 40.
    let c = FormatKey::Cash;
    assert_eq!(assert_above(c, "pf.rfi_sb.wide", RfiSb, 40, 0.15, &[]), 23);
    let reads = evaluate_rules(&cash(&[(RfiSb, 24, 40)]));
    let r = read(&reads, "pf.rfi_sb.wide").unwrap();
    assert_eq!(r.observation, "Opens 60% (24/40) of his small blinds when folded to.");
    assert_eq!(r.advice, "3-bet his small-blind opens with 77+, A8s+, KTs+ and ATo+.");
    assert_eq!((r.tag.as_deref(), r.scenario_id.as_str(), r.category), (Some("SB+"), "P13", RuleCategory::Exploit));
    // Spin & Go's SB prior is .45: the same 23/40 is (23 + 6.75)/55 = .54,
    // under .60.
    assert!(!fires(&input(FormatKey::Spin, &[(RfiSb, 23, 40)]), "pf.rfi_sb.wide"));
    // Under the 8-opportunity floor nothing shows.
    assert!(!fires(&cash(&[(RfiSb, 7, 7)]), "pf.rfi_sb.wide"));
    // He opens in front of the hero: the seat suffix is for the right side.
    let mut v = cash(&[(RfiSb, 30, 40)]);
    let mut ctx = context(c, 100.0);
    ctx.seat = Some(seat(5, 6));
    v.context = Some(ctx);
    assert_eq!(
        read(&evaluate_rules(&v), "pf.rfi_sb.wide").unwrap().advice,
        "3-bet his small-blind opens with 77+, A8s+, KTs+ and ATo+ (directly on your right)."
    );
}

// ------------------------------------------------ blind defence (P03, P04)

/// The four formats in the order of the per-format delta arrays.
const FORMATS: [FormatKey; 4] = [FormatKey::Mtt, FormatKey::Cash, FormatKey::Zoom, FormatKey::Spin];

#[test]
fn fold_to_steal_rules_fire_at_threshold_and_not_below() {
    // Audit 2026-10 (section 17): the small blind's leg needs prior + .15
    // (a small blind folding ~85% to a button open is close to correct),
    // the big blind's stays at + .12; "defends" needs fold ≤ prior − .25 in
    // MTT and − .20 elsewhere (with antes, wide big-blind defence is right).
    for (format, low) in FORMATS.into_iter().zip(FOLD_TO_STEAL_LOW) {
        assert_above(format, "pf.fold_to_steal.high", FoldToStealSb, 60, FOLD_TO_STEAL_SB_HIGH, &[]);
        assert_above(format, "pf.fold_to_steal.high", FoldToStealBb, 30, 0.12, &[]);
        assert_below(format, "pf.fold_to_steal.low", FoldToStealBb, 30, low, &[]);
    }
    assert_eq!(FOLD_TO_STEAL_LOW, [0.25, 0.20, 0.20, 0.20]);
    assert_eq!(FOLD_TO_STEAL_SB_HIGH, 0.15);
    // Cash small blind (prior .78, k 12): 29 of 30 folds shrink to
    // (29 + 9.36)/42 = .913, which fired under D106 (≥ .90) and is under .93
    // now; 30 of 30 is .937.
    assert!(!fires(&cash(&[(FoldToStealSb, 29, 30)]), "pf.fold_to_steal.high"));
    assert!(fires(&cash(&[(FoldToStealSb, 30, 30)]), "pf.fold_to_steal.high"));
    // MTT big blind (prior .55, k 12): 9 of 30 folds (a 70% defence, correct
    // with antes) shrink to (9 + 6.6)/42 = .371; D106 (≤ .40) called that
    // "defends", the audit needs ≤ .30: 5 of 30 shrinks to (5 + 6.6)/42 =
    // .276 and fires.
    assert!(!fires(&input(FormatKey::Mtt, &[(FoldToStealBb, 9, 30)]), "pf.fold_to_steal.low"));
    assert!(fires(&input(FormatKey::Mtt, &[(FoldToStealBb, 5, 30)]), "pf.fold_to_steal.low"));
    let c = FormatKey::Cash;
    let h = last_below(c, FoldToStealBb, 30, FOLD_TO_STEAL_LOW[1]);
    // "Defends" shows the complement of the fold rate.
    let reads = evaluate_rules(&cash(&[(FoldToStealBb, h, 30)]));
    let r = read(&reads, "pf.fold_to_steal.low").unwrap();
    let defends = 30 - h;
    assert_eq!(
        r.observation,
        format!("Defends his big blind against {:.0}% ({defends}/30) of steals.", f64::from(defends) / 30.0 * 100.0)
    );
}

#[test]
fn bb_vs_sb_rules_fire_at_threshold_and_not_below() {
    // Audit 2026-10 (section 17): "overdefends" needs prior + .25 in MTT and
    // + .20 elsewhere; the overfold line is unchanged.
    for (format, over) in FORMATS.into_iter().zip(BB_VS_SB_OVERDEFEND) {
        assert_below(format, "pf.bb_vs_sb.overfold", BbDefendVsSb, 30, 0.15, &[]);
        assert_above(format, "pf.bb_vs_sb.overdefend", BbDefendVsSb, 30, over, &[]);
    }
    assert_eq!(BB_VS_SB_OVERDEFEND, [0.25, 0.20, 0.20, 0.20]);
    // Cash (prior .52, k 12): 22 of 30 defences (73%) shrink to
    // (22 + 6.24)/42 = .672, which fired under D106 (≥ .67); a defence near
    // 70% against small-blind opens is close to correct, so the audit needs
    // ≥ .72: 24 of 30 is exactly .72.
    assert!(!fires(&cash(&[(BbDefendVsSb, 22, 30)]), "pf.bb_vs_sb.overdefend"));
    assert!(!fires(&cash(&[(BbDefendVsSb, 23, 30)]), "pf.bb_vs_sb.overdefend"));
    assert!(fires(&cash(&[(BbDefendVsSb, 24, 30)]), "pf.bb_vs_sb.overdefend"));
}

// ----------------------------------------- 3-bets, 4-bets, squeezes (P05–P08)

#[test]
fn three_bet_rules_fire_at_threshold_and_not_below() {
    let c = FormatKey::Cash;
    assert_above(c, "pf.3bet.high_ip", ThreeBetIp, 60, 0.05, &[]);
    assert_above(c, "pf.3bet.high_oop", ThreeBetOop, 60, 0.05, &[]);
    assert_below(c, "pf.3bet.low", ThreeBetIp, 100, 0.03, &[(ThreeBetOop, 0, 100)]);
    // max(ip, oop): one frequent side blocks the "rarely 3-bets" read.
    assert!(!fires(&cash(&[(ThreeBetIp, 0, 100), (ThreeBetOop, 20, 100)]), "pf.3bet.low"));
    // Small-sample extreme: 2 of 12 is a raw 17% against an 8% prior, but
    // k = 25 keeps it at (2 + 2)/37 = .108, under .13.
    assert!(!fires(&cash(&[(ThreeBetIp, 2, 12)]), "pf.3bet.high_ip"));
}

#[test]
fn fold_to_three_bet_rules_fire_at_threshold_and_not_below() {
    let c = FormatKey::Cash;
    // IP: prior .55, k 15, n = 30: (22 + 8.25)/45 = .672 ≥ .67 fires,
    // (21 + 8.25)/45 = .650 does not.
    assert_eq!(assert_above(c, "pf.fold_to_3bet.high", FoldTo3betIp, 30, 0.12, &[]), 22);
    assert_above(c, "pf.fold_to_3bet.high", FoldTo3betOop, 30, 0.12, &[]);
    assert_below(c, "pf.fold_to_3bet.low", FoldTo3betOop, 30, 0.15, &[]);
    let reads = evaluate_rules(&cash(&[(FoldTo3betIp, 5, 30)]));
    let r = read(&reads, "pf.fold_to_3bet.low").unwrap();
    assert_eq!(r.observation, "Continues against 83% (25/30) of 3-bets.");
}

#[test]
fn four_bet_rules_fire_at_threshold_and_not_below() {
    let c = FormatKey::Cash;
    assert_above(c, "pf.4bet.high", FourBet, 40, 0.06, &[]);
    assert_above(c, "pf.fold_to_4bet.high", FoldTo4bet, 20, 0.20, &[]);
    // Small-sample extreme: 6 of 6 folds to 4-bets is (6 + 5)/16 = .6875,
    // still under .70.
    assert!(!fires(&cash(&[(FoldTo4bet, 6, 6)]), "pf.fold_to_4bet.high"));
}

#[test]
fn squeeze_rule_fires_at_threshold_and_not_below() {
    assert_above(FormatKey::Cash, "pf.squeeze.high", Squeeze, 40, 0.05, &[]);
}

// --------------------------------------------- limps and cold calls (P09–P11)

#[test]
fn limp_rules_fire_at_threshold_and_not_below() {
    // Audit 2026-10 (section 17): Spin & Go needs prior + .12 (the limp stat
    // counts the small blind, and limping it is part of solid 3-max play);
    // the other formats keep + .08.
    for (format, delta) in FORMATS.into_iter().zip(LIMP_HIGH) {
        assert_above(format, "pf.limp.high", Limp, 50, delta, &[]);
    }
    assert_eq!(LIMP_HIGH, [0.08, 0.08, 0.08, 0.12]);
    // Spin (prior .12, k 20): 14 of 50 limps shrink to (14 + 2.4)/70 = .234,
    // which fired under D106 (≥ .20) and is under .24 now; 15 of 50 is .249.
    assert!(!fires(&input(FormatKey::Spin, &[(Limp, 14, 50)]), "pf.limp.high"));
    assert!(fires(&input(FormatKey::Spin, &[(Limp, 15, 50)]), "pf.limp.high"));
    assert_above(FormatKey::Cash, "pf.iso.high", IsoRaise, 20, 0.15, &[]);
}

#[test]
fn limp_followup_rules_fire_at_threshold_and_not_below() {
    let c = FormatKey::Cash;
    assert_above(c, "pf.limp_fold.high", LimpFold, 20, 0.20, &[]);
    // Limp-reraise: shrunk ≥ .15 and at least two seen. k 8, prior .05;
    // n = 10: (2 + .4)/18 = .133 no, (3 + .4)/18 = .189 yes.
    assert!(!fires(&cash(&[(LimpReraise, 2, 10)]), "pf.limp_reraise.seen"));
    assert!(fires(&cash(&[(LimpReraise, 3, 10)]), "pf.limp_reraise.seen"));
    // n = 5: 2 of 5 gives (2 + .4)/13 = .185 — fires with two seen; one of
    // five never does.
    assert!(fires(&cash(&[(LimpReraise, 2, 5)]), "pf.limp_reraise.seen"));
    assert!(!fires(&cash(&[(LimpReraise, 1, 5)]), "pf.limp_reraise.seen"));
    let reads = evaluate_rules(&cash(&[(LimpReraise, 3, 10)]));
    assert_eq!(read(&reads, "pf.limp_reraise.seen").unwrap().observation, "Has limp-reraised 3 of 10 times after limping.");
}

#[test]
fn limp_call_rule_fires_at_threshold_and_not_below() {
    // Pro review (D107, P14): the sticky limper. Prior .40, k 8:
    // (h + 3.2)/28 ≥ .60 → 14 of 20.
    let c = FormatKey::Cash;
    assert_eq!(assert_above(c, "pf.limp_call.high", LimpCall, 20, 0.20, &[]), 14);
    let reads = evaluate_rules(&cash(&[(LimpCall, 14, 20)]));
    let r = read(&reads, "pf.limp_call.high").unwrap();
    assert_eq!(r.observation, "Calls 70% (14/20) of raises after limping.");
    assert_eq!(r.tag.as_deref(), Some("LPC"));
    assert_eq!(r.scenario_id, "P14");
    // Small-sample extreme: 4 of 4 is under the 5-opportunity floor, and 5
    // of 5 shrinks to (5 + 3.2)/13 = .63, which does fire: five calls out of
    // five raises is already a sticky limper.
    assert!(!fires(&cash(&[(LimpCall, 4, 4)]), "pf.limp_call.high"));
    assert!(fires(&cash(&[(LimpCall, 5, 5)]), "pf.limp_call.high"));
    // A limp-folder and a limp-caller are different reads of the same spot.
    assert!(!fires(&cash(&[(LimpCall, 14, 20)]), "pf.limp_fold.high"));
}

#[test]
fn cold_call_rule_fires_at_threshold_and_not_below() {
    assert_above(FormatKey::Cash, "pf.cold_call.high", ColdCall, 60, 0.07, &[]);
}

// ---------------------------------------------- stack depth (S01–S03)

#[test]
fn open_shove_rules_fire_at_threshold_and_not_below() {
    let m = FormatKey::Mtt;
    assert_above(m, "stk.open_shove.wide", OpenShove, 30, 0.15, &[]);
    assert_below(m, "stk.open_shove.tight", OpenShove, 30, 0.10, &[]);
}

#[test]
fn call_vs_shove_rules_fire_at_threshold_and_not_below() {
    let m = FormatKey::Mtt;
    assert_above(m, "stk.call_shove.wide", CallVsShove, 30, 0.12, &[]);
    assert_below(m, "stk.call_shove.tight", CallVsShove, 30, 0.10, &[]);
    // Small-sample extreme: 0 of 6 calls is (0 + 2.5)/16 = .156, above .15.
    assert!(!fires(&input(m, &[(CallVsShove, 0, 6)]), "stk.call_shove.tight"));
}

#[test]
fn reshove_rules_fire_at_threshold_and_not_below() {
    let m = FormatKey::Mtt;
    assert_above(m, "stk.reshove.high", Reshove, 30, 0.08, &[]);
    assert_below(m, "stk.reshove.low", Reshove, 30, 0.06, &[]);
    // Small-sample extreme: 0 of 6 reshoves is (0 + 1)/16 = .0625 > .04.
    assert!(!fires(&input(m, &[(Reshove, 0, 6)]), "stk.reshove.low"));
}

// ------------------------------------------------ context adaptation (S04, S05)

/// A villain whose every family fires without context.
fn busy_villain(format: FormatKey) -> RuleInput {
    let mut input = input(
        format,
        &[
            (Vpip, 40, 100),
            (Pfr, 30, 100),
            (ThreeBetIp, 8, 40),
            (FoldTo3betOop, 25, 30),
            (Squeeze, 10, 40),
            (CbetFlop, 38, 40),
            (FoldToCbetFlop, 30, 40),
            (OpenShove, 15, 30),
            (Reshove, 5, 12),
            (CallVsShove, 15, 30),
        ],
    );
    input.h2h.push(h2h(H2hKey::FoldToHeroCbet, 12, 12, 0.45));
    input
}

#[test]
fn short_stack_villain_gets_only_push_fold_reads() {
    let free = evaluate_rules(&busy_villain(FormatKey::Mtt));
    for id in ["pf.3bet.high_ip", "pf.fold_to_3bet.high", "post.cbet_flop.high", "h2h.fold_to_hero_cbet.high"] {
        assert!(read(&free, id).is_some(), "{id} fires without a short stack");
    }

    let mut short = busy_villain(FormatKey::Mtt);
    short.context = Some(context(FormatKey::Mtt, 12.4));
    let reads = evaluate_rules(&short);
    for r in &reads {
        assert!(
            matches!(r.family, Family::Stack | Family::Bounty | Family::Recency | Family::Showdown)
                || r.rule_id == "ctx.short_stack"
                || r.rule_id.starts_with("stage.late."),
            "{} is not a push/fold read",
            r.rule_id
        );
    }
    let stack = read(&reads, "ctx.short_stack").expect("short-stack read");
    assert_eq!(stack.tag.as_deref(), Some("12bb"));
    assert_eq!(stack.observation, "Sits on 12.4bb effective after the last hand.");
    assert!(read(&reads, "stk.open_shove.wide").is_some());
    assert!(read(&reads, "stk.call_shove.wide").is_some());
    for id in ["pf.3bet.high_ip", "pf.squeeze.high", "post.cbet_flop.high", "post.fold_cbet_flop.high"] {
        assert!(read(&reads, id).is_none(), "{id} must not show at 12bb");
    }

    // 15bb is still push/fold; 15.1bb is not. (At 15–25bb his 3-bet read
    // yields to his reshove read when both fire, section 7.2, so the
    // fold-to-3-bet read shows the preflop reads are back.)
    short.context = Some(context(FormatKey::Mtt, 15.0));
    assert!(read(&evaluate_rules(&short), "pf.fold_to_3bet.high").is_none());
    short.context = Some(context(FormatKey::Mtt, 15.1));
    let reads = evaluate_rules(&short);
    assert!(read(&reads, "pf.fold_to_3bet.high").is_some());
    assert!(read(&reads, "ctx.short_stack").is_none());
}

#[test]
fn reshove_family_takes_priority_at_20bb() {
    // Reshove 5/12 in an MTT: shrunk (5 + 1)/22 = .273, d = 2.88,
    // confidence 12/22 → score 1.57. 3-bet IP 8/40: (8 + 1.75)/65 = .15,
    // d = 2.67, confidence 40/65 → score 1.64. Section 7.2: ×1.5 for the
    // reshove and call-shove reads at 15–25bb, ×1.25 for the reshove read
    // at 25–40bb, ×1 above; at 15–25bb the 3-bet read yields to the reshove
    // read (DEPTH_OPPOSITES).
    let mut v = input(FormatKey::Mtt, &[(Reshove, 5, 12), (ThreeBetIp, 8, 40), (CallVsShove, 15, 30)]);
    v.context = Some(context(FormatKey::Mtt, 60.0));
    let mid = evaluate_rules(&v);
    v.context = Some(context(FormatKey::Mtt, 40.0));
    let jam = evaluate_rules(&v);
    v.context = Some(context(FormatKey::Mtt, 20.0));
    let reshove = evaluate_rules(&v);

    let score = |reads: &[EngineRuleResult], id: &str| read(reads, id).unwrap().score;
    assert!(score(&mid, "pf.3bet.high_ip") > score(&mid, "stk.reshove.high"));
    assert!(score(&jam, "stk.reshove.high") > score(&jam, "pf.3bet.high_ip"));
    assert!(read(&reshove, "pf.3bet.high_ip").is_none(), "{:?}", ids(&reshove));
    for id in ["stk.reshove.high", "stk.call_shove.wide"] {
        let ratio = score(&reshove, id) / score(&mid, id);
        assert!((ratio - 1.5).abs() < 1e-9, "{id}: ×{ratio}");
    }
    assert!((score(&jam, "stk.reshove.high") / score(&mid, "stk.reshove.high") - 1.25).abs() < 1e-9);
    assert_eq!(score(&jam, "stk.call_shove.wide"), score(&mid, "stk.call_shove.wide"));
    assert_eq!(score(&jam, "pf.3bet.high_ip"), score(&mid, "pf.3bet.high_ip"));
}

#[test]
fn deep_stack_read_fires_above_150bb_only() {
    let mut v = cash(&[]);
    v.context = Some(context(FormatKey::Cash, 150.0));
    assert!(!fires(&v, "ctx.deep_stack"));
    v.context = Some(context(FormatKey::Cash, 150.1));
    let reads = evaluate_rules(&v);
    let deep = read(&reads, "ctx.deep_stack").expect("deep read");
    assert_eq!(deep.tag.as_deref(), Some("DEEP"));
    assert_eq!(deep.observation, "Sits on 150.1bb effective after the last hand.");
    // A cash short stack is push/fold-only but gets no tournament stack read.
    v.context = Some(context(FormatKey::Cash, 12.0));
    assert!(!fires(&v, "ctx.short_stack"));
}

// --------------------------------------------- stage and bounty (T01, K01)

#[test]
fn late_stage_turns_fold_to_steal_into_shove_advice() {
    let m = FormatKey::Mtt;
    let mut v = input(m, &[(FoldToStealBb, 24, 30)]);
    v.late_fold_to_steal_bb = Some(stat(m, FoldToStealBb, 16, 20));
    v.context = Some(context(m, 30.0));

    // Middle stage: the tournament steal advice, no late-stage read.
    let reads = evaluate_rules(&v);
    assert_eq!(
        read(&reads, "pf.fold_to_steal.high").unwrap().advice,
        "Min-raise any two suited from late seats into his blinds."
    );
    assert!(read(&reads, "stage.late.overfolds_blinds").is_none());

    v.context.as_mut().unwrap().stage = Some(Stage::Late);
    let reads = evaluate_rules(&v);
    let steal = read(&reads, "pf.fold_to_steal.high").unwrap();
    assert_eq!(steal.advice, "Min-raise any two from the button into his blinds.");
    assert!(steal.adapted_by.contains(&"stage.late"));
    let late = read(&reads, "stage.late.overfolds_blinds").expect("late-stage read");
    // Over late-stage hands only: 16/20, prior .55, k 12: (16 + 6.6)/32 = .706 ≥ .65.
    assert_eq!(late.observation, "Folds his big blind to 80% (16/20) of steals late in the tournament.");
    assert_eq!(late.advice, "Min-raise any two from the button or small blind into his big blind.");
    // Under the threshold: (13 + 6.6)/32 = .6125 < .65.
    v.late_fold_to_steal_bb = Some(stat(m, FoldToStealBb, 13, 20));
    assert!(!fires(&v, "stage.late.overfolds_blinds"));

    // Early stage: limping read over early-level hands only.
    v.context.as_mut().unwrap().stage = Some(Stage::Early);
    v.early_limp = Some(stat(m, Limp, 8, 40));
    // (8 + 1.2)/60 = .153 ≥ .12 fires; (5 + 1.2)/60 = .103 does not.
    assert!(fires(&v, "stage.early.passive"));
    v.early_limp = Some(stat(m, Limp, 5, 40));
    assert!(!fires(&v, "stage.early.passive"));
}

#[test]
fn big_bounty_villain_gets_wider_call_advice() {
    let m = FormatKey::Mtt;
    let mut v = input(m, &[(CallVsShove, 15, 30)]);
    v.ko_call_vs_shove = Some(stat(m, CallVsShove, 14, 20));
    let mut ctx = context(m, 40.0);
    ctx.bounty = Some(BountyContext { amount: 27.0, currency: Some("EUR".into()), ratio: Some(2.0), hero_covers: Some(true) });
    v.context = Some(ctx.clone());

    let reads = evaluate_rules(&v);
    let ko = read(&reads, "ko.big_bounty").expect("big bounty read");
    assert_eq!(ko.observation, "Carries a 2× bounty (€27.00).");
    assert_eq!(ko.advice, "When you cover him, call his shoves with any pair, any ace and K9s+.");
    assert_eq!(ko.tag.as_deref(), Some("KO"));
    // KO: call-vs-shove advice switches to the bounty variant.
    assert_eq!(
        read(&reads, "stk.call_shove.wide").unwrap().advice,
        "Shove only pairs, aces and broadways; tighter still when he covers you."
    );
    // Over KO hands: (14 + 2.5)/30 = .55 ≥ .40. The hero covers him, so he
    // cannot win the hero's bounty.
    let hunter = read(&reads, "ko.hunts_bounties").expect("bounty hunter read");
    assert_eq!(hunter.advice, "While you cover him, shove pairs, aces and broadways into him.");

    // The hero does not cover him: no bounty to win, call by range.
    ctx.bounty.as_mut().unwrap().hero_covers = Some(false);
    v.context = Some(ctx.clone());
    let reads = evaluate_rules(&v);
    assert_eq!(
        read(&reads, "ko.big_bounty").unwrap().advice,
        "You don't cover him: ignore his bounty and call his shoves with 55+, A9+, KQ."
    );
    assert_eq!(read(&reads, "ko.hunts_bounties").unwrap().advice, "When he covers you, shove into him only with 77+, AT+ and KQ.");

    // Under 2× the initial bounty: no big-bounty read.
    ctx.bounty.as_mut().unwrap().ratio = Some(1.99);
    v.context = Some(ctx.clone());
    assert!(!fires(&v, "ko.big_bounty"));
    // No bounty at all: plain call-vs-shove advice.
    ctx.bounty = None;
    v.context = Some(ctx);
    assert_eq!(
        read(&evaluate_rules(&v), "stk.call_shove.wide").unwrap().advice,
        "Shove only pairs, aces and broadways into him; no K5o or 65s-type shoves."
    );
    // ko.hunts_bounties needs its KO-hand sample: 5 opportunities is short.
    v.ko_call_vs_shove = Some(stat(m, CallVsShove, 5, 5));
    assert!(!fires(&v, "ko.hunts_bounties"));
}

#[test]
fn late_stage_blind_read_stays_eligible_at_push_fold() {
    // Section 5 (audit 2026-10): late-stage blind battles are played at
    // 15bb or less, so an overfolding big blind is the read that matters
    // most there. The stage read stays eligible at push/fold with shove
    // wording; the preflop steal read does not.
    let m = FormatKey::Mtt;
    let mut v = input(m, &[(FoldToStealBb, 24, 30)]);
    v.late_fold_to_steal_bb = Some(stat(m, FoldToStealBb, 16, 20));
    let mut ctx = context(m, 12.0);
    ctx.stage = Some(Stage::Late);
    v.context = Some(ctx.clone());
    let reads = evaluate_rules(&v);
    let late = read(&reads, "stage.late.overfolds_blinds").expect("late-stage read at 12bb");
    assert_eq!(late.advice, "Shove any two from the button or small blind into his big blind at 10bb or less.");
    assert!(late.adapted_by.contains(&"stack.push_fold"), "{:?}", late.adapted_by);
    assert!(read(&reads, "pf.fold_to_steal.high").is_none(), "preflop reads stay out at push/fold");
    // Under its threshold it still does not fire at push/fold.
    v.late_fold_to_steal_bb = Some(stat(m, FoldToStealBb, 13, 20));
    assert!(!fires(&v, "stage.late.overfolds_blinds"));
    // Above push/fold: the min-raise text.
    v.late_fold_to_steal_bb = Some(stat(m, FoldToStealBb, 16, 20));
    ctx.effective_stack_bb = Some(15.1);
    ctx.stack_bucket = Some(StackBucket::of(15.1));
    v.context = Some(ctx.clone());
    let late = read(&evaluate_rules(&v), "stage.late.overfolds_blinds").cloned().expect("late-stage read at 15.1bb");
    assert_eq!(late.advice, "Min-raise any two from the button or small blind into his big blind.");
    assert!(!late.adapted_by.contains(&"stack.push_fold"));

    // The early-stage limp read is a preflop limp read: not at push/fold.
    let mut v = input(m, &[]);
    v.early_limp = Some(stat(m, Limp, 8, 40));
    let mut ctx = context(m, 12.0);
    ctx.stage = Some(Stage::Early);
    v.context = Some(ctx.clone());
    assert!(!fires(&v, "stage.early.passive"));
    ctx.effective_stack_bb = Some(30.0);
    ctx.stack_bucket = Some(StackBucket::of(30.0));
    v.context = Some(ctx);
    assert!(fires(&v, "stage.early.passive"));
}

#[test]
fn early_stage_limp_threshold_follows_format() {
    // A Spin & Go's small blind limps by design (limp prior .12), so the
    // early-stage limp read needs the same + .12 as `pf.limp.high` there
    // (section 17.3); MTT keeps + .06. Spin, k 20, prior .12, n 40:
    // 9/40 → (9 + 2.4)/60 = .19 (≥ .18, fired before) does not fire;
    // 14/40 → (14 + 2.4)/60 = .273 (≥ .24) fires.
    let s = FormatKey::Spin;
    let mut v = input(s, &[]);
    let mut ctx = context(s, 22.0);
    ctx.stage = Some(Stage::Early);
    v.context = Some(ctx);
    v.early_limp = Some(stat(s, Limp, 9, 40));
    assert!(!fires(&v, "stage.early.passive"), "Spin 9/40 must not fire");
    let h = first_above(s, Limp, 40, STAGE_EARLY_LIMP[3]);
    v.early_limp = Some(stat(s, Limp, h, 40));
    assert!(fires(&v, "stage.early.passive"), "Spin {h}/40 fires");
    v.early_limp = Some(stat(s, Limp, h - 1, 40));
    assert!(!fires(&v, "stage.early.passive"), "Spin {}/40 does not", h - 1);

    let m = FormatKey::Mtt;
    let mut v = input(m, &[]);
    let mut ctx = context(m, 30.0);
    ctx.stage = Some(Stage::Early);
    v.context = Some(ctx);
    let h = first_above(m, Limp, 40, STAGE_EARLY_LIMP[0]);
    v.early_limp = Some(stat(m, Limp, h, 40));
    assert!(fires(&v, "stage.early.passive"), "MTT {h}/40 fires");
    v.early_limp = Some(stat(m, Limp, h - 1, 40));
    assert!(!fires(&v, "stage.early.passive"), "MTT {}/40 does not", h - 1);
    assert_eq!(STAGE_EARLY_LIMP, [0.06, 0.06, 0.06, 0.12]);
}

// ------------------------------- late-stage push/fold and bounty calls (T02, K02)

/// An MTT context at `eff` in the given stage.
fn mtt_stage(eff: f64, stage: Stage) -> EngineContext {
    let mut ctx = context(FormatKey::Mtt, eff);
    ctx.stage = Some(stage);
    ctx
}

#[test]
fn late_stage_shove_reads_fire_at_threshold_and_not_below() {
    // Row T02: late-stage MTT samples against the format prior. Call vs
    // shove (prior .25, k 10) needs + .10: 12/30 → (12 + 2.5)/40 = .3625
    // fires, 11/30 → .3375 does not. Open shove (prior .20, k 10) needs
    // + .15: 12/30 → (12 + 2)/40 = .35 fires, 11/30 → .325 does not.
    let m = FormatKey::Mtt;
    assert_eq!((LATE_CALLS_SHOVES, LATE_SHOVES_WIDE), (0.10, 0.15));
    for (id, key, delta) in [
        ("stage.late.calls_shoves", CallVsShove, LATE_CALLS_SHOVES),
        ("stage.late.shoves_wide", OpenShove, LATE_SHOVES_WIDE),
    ] {
        let late = |hits: u32, n: u32, ctx: Option<EngineContext>, format: FormatKey| {
            let mut v = input(format, &[]);
            let sample = Some(stat(format, key, hits, n));
            match key {
                CallVsShove => v.late_call_vs_shove = sample,
                _ => v.late_open_shove = sample,
            }
            v.context = ctx;
            v
        };
        let h = first_above(m, key, 30, delta);
        assert_eq!(h, 12, "{id} boundary");
        // Eligible at push/fold depth (12bb) and above it (30bb).
        for eff in [12.0, 30.0] {
            assert!(fires(&late(h, 30, Some(mtt_stage(eff, Stage::Late)), m), id), "{id} fires at {h}/30, {eff}bb");
            assert!(!fires(&late(h - 1, 30, Some(mtt_stage(eff, Stage::Late)), m), id), "{id} not at {}/30", h - 1);
        }
        // n_min 6: 5 of 5 is not shown; 6 of 6 is (shrunk .53 and .50).
        assert!(!fires(&late(5, 5, Some(mtt_stage(12.0, Stage::Late)), m), id), "{id} below n_min");
        assert!(fires(&late(6, 6, Some(mtt_stage(12.0, Stage::Late)), m), id), "{id} at n_min");
        // Only at the late stage, only in an MTT, only with a table context.
        assert!(!fires(&late(20, 30, Some(mtt_stage(12.0, Stage::Middle)), m), id), "{id} middle stage");
        assert!(!fires(&late(20, 30, None, m), id), "{id} without context");
        let mut spin = context(FormatKey::Spin, 8.0);
        spin.stage = Some(Stage::Late);
        assert!(!fires(&late(20, 30, Some(spin), FormatKey::Spin), id), "{id} in a Spin & Go");
    }

    let mut v = input(m, &[]);
    v.late_call_vs_shove = Some(stat(m, CallVsShove, 12, 30));
    v.late_open_shove = Some(stat(m, OpenShove, 12, 30));
    v.context = Some(mtt_stage(12.0, Stage::Late));
    let reads = evaluate_rules(&v);
    let calls = read(&reads, "stage.late.calls_shoves").unwrap();
    assert_eq!(calls.observation, "Calls 40% (12/30) of all-in shoves late in the tournament.");
    assert_eq!(calls.advice, "Late, shove into him for value only: 55+, A8+, KJ+; no K5o or 65s-type shoves.");
    assert_eq!(calls.tag.as_deref(), Some("ICM-"));
    assert_eq!(calls.family, Family::Stage);
    let shoves = read(&reads, "stage.late.shoves_wide").unwrap();
    assert_eq!(shoves.observation, "Open-shoves 40% (12/30) when folded to late in the tournament.");
    assert_eq!(shoves.advice, "Call his late shoves with 22+, A2+, K9s+ if you cover him; 66+, A9+, KQ if not.");
    assert_eq!(shoves.tag.as_deref(), Some("SHV"));
    assert_eq!(shoves.evidence[0].stat_name, "open_shove");

    // The late call read ranks like the call-vs-shove family it refines:
    // ×1.5 at 15–25bb.
    let score_at = |eff| {
        let mut v = v.clone();
        v.context = Some(mtt_stage(eff, Stage::Late));
        read(&evaluate_rules(&v), "stage.late.calls_shoves").unwrap().score
    };
    assert!((score_at(20.0) / score_at(30.0) - 1.5).abs() < 1e-9);
}

#[test]
fn bounty_driven_calling_needs_both_cover_legs_and_the_gap() {
    // Row K02, MTT prior .25, k 10. Shoves he covers 14/20 → (14 + 2.5)/30
    // = .55; shoves that cover him 2/20 → .15: .55 ≥ .40 and the gap .40 ≥
    // .15, so the read fires.
    let m = FormatKey::Mtt;
    assert_eq!((KO_COVERING_HIGH, KO_COVER_GAP), (0.15, 0.15));
    let bounty = |covering: (u32, u32), covered: (u32, u32)| {
        let mut v = input(m, &[]);
        v.ko_call_covering = Some(stat(m, CallVsShove, covering.0, covering.1));
        v.ko_call_covered = Some(stat(m, CallVsShove, covered.0, covered.1));
        v
    };
    let reads = evaluate_rules(&bounty((14, 20), (2, 20)));
    let r = read(&reads, "ko.calls_when_covering").expect("bounty-driven read");
    assert_eq!(r.observation, "Calls 70% (14/20) of shoves he covers in bounty games, 10% (2/20) when covered.");
    assert_eq!(r.advice, "When he covers you, shove only 77+, AT+ and KQ; when you cover him, any pair or ace.");
    assert_eq!(r.tag.as_deref(), Some("KO+"));
    assert_eq!(r.family, Family::Bounty);
    assert_eq!(r.confidence_pct, Some(67), "20 / (20 + 10) on both legs");
    assert_eq!(r.evidence.len(), 2);

    // The covering leg: + .15 over the prior. 10/20 → .4167 fires, 9/20 →
    // .383 does not.
    let h = first_above(m, CallVsShove, 20, KO_COVERING_HIGH);
    assert_eq!(h, 10);
    assert!(fires(&bounty((h, 20), (2, 20)), "ko.calls_when_covering"));
    assert!(!fires(&bounty((h - 1, 20), (2, 20)), "ko.calls_when_covering"));
    // The gap: with 14/20 covering (.55), 9/20 covered (.383, gap .167)
    // fires and 10/20 (.4167, gap .133) does not: he calls wide whatever
    // the cover, which is `ko.hunts_bounties`, not a bounty-driven split.
    assert!(fires(&bounty((14, 20), (9, 20)), "ko.calls_when_covering"));
    assert!(!fires(&bounty((14, 20), (10, 20)), "ko.calls_when_covering"));
    // n_min 6 on each leg.
    assert!(!fires(&bounty((14, 20), (0, 5)), "ko.calls_when_covering"), "covered leg below n_min");
    assert!(!fires(&bounty((5, 5), (0, 20)), "ko.calls_when_covering"), "covering leg below n_min");
    assert!(fires(&bounty((6, 6), (0, 6)), "ko.calls_when_covering"), "both legs at n_min");
    // A missing leg never fires it.
    let mut v = bounty((14, 20), (2, 20));
    v.ko_call_covered = None;
    assert!(!fires(&v, "ko.calls_when_covering"));

    // The cover of the latest hand picks the half that applies; it stays
    // eligible at push/fold.
    let mut v = bounty((14, 20), (2, 20));
    let mut ctx = mtt_stage(12.0, Stage::Middle);
    for (covers, advice) in [
        (Some(true), "While you cover him, shove any pair, any ace and broadways into him."),
        (Some(false), "While he covers you, shove into him only with 77+, AT+ and KQ."),
        (None, "When he covers you, shove only 77+, AT+ and KQ; when you cover him, any pair or ace."),
    ] {
        ctx.bounty = Some(BountyContext { amount: 10.0, currency: Some("USD".into()), ratio: Some(1.0), hero_covers: covers });
        v.context = Some(ctx.clone());
        let reads = evaluate_rules(&v);
        assert_eq!(read(&reads, "ko.calls_when_covering").expect("at 12bb").advice, advice);
    }
}

#[test]
fn tournament_reads_replace_their_general_reads() {
    // TOURNAMENT_OPPOSITES: the late-stage read counts the same stat over
    // the stage the context is in, and the cover split explains his bounty
    // and late calls, so the first read of each pair stays.
    let m = FormatKey::Mtt;
    let villain = |general: &[(StatKey, u32, u32)]| {
        let mut v = input(m, general);
        v.late_call_vs_shove = Some(stat(m, CallVsShove, 12, 30));
        v.late_open_shove = Some(stat(m, OpenShove, 12, 30));
        v
    };
    // Calls 15/30 → .4375 (wide), 2/30 → .1125 (tight); shoves 15/30 →
    // .425 (wide), 1/30 → .075 (tight).
    for general in [[(CallVsShove, 15, 30), (OpenShove, 15, 30)], [(CallVsShove, 2, 30), (OpenShove, 1, 30)]] {
        let mut v = villain(&general);
        v.context = Some(mtt_stage(12.0, Stage::Middle));
        let middle = evaluate_rules(&v);
        let generals: Vec<&str> = ids(&middle).into_iter().filter(|id| id.starts_with("stk.")).collect();
        assert_eq!(generals.len(), 2, "both general reads fire at the middle stage: {generals:?}");
        v.context = Some(mtt_stage(12.0, Stage::Late));
        let late = evaluate_rules(&v);
        for general in generals {
            let (specific, _) = TOURNAMENT_OPPOSITES.iter().find(|(_, g)| *g == general).expect("declared pair");
            assert!(read(&late, specific).is_some(), "{specific} fires late");
            assert!(read(&late, general).is_none(), "{general} goes when {specific} fires");
        }
    }

    // The cover split against the general bounty read and the late calls.
    let mut v = villain(&[]);
    v.ko_call_vs_shove = Some(stat(m, CallVsShove, 14, 20));
    v.ko_call_covering = Some(stat(m, CallVsShove, 14, 20));
    v.ko_call_covered = Some(stat(m, CallVsShove, 2, 20));
    v.context = Some(mtt_stage(12.0, Stage::Late));
    let reads = evaluate_rules(&v);
    assert!(read(&reads, "ko.calls_when_covering").is_some());
    assert!(read(&reads, "ko.hunts_bounties").is_none());
    assert!(read(&reads, "stage.late.calls_shoves").is_none());
    assert!(read(&reads, "stage.late.shoves_wide").is_some(), "a shove read is not a call read");
    // Without the split, the general bounty read and the late call read stay.
    v.ko_call_covered = Some(stat(m, CallVsShove, 12, 20));
    let reads = evaluate_rules(&v);
    assert!(read(&reads, "ko.calls_when_covering").is_none());
    assert!(read(&reads, "ko.hunts_bounties").is_some());
    assert!(read(&reads, "stage.late.calls_shoves").is_some());
    for (a, b) in TOURNAMENT_OPPOSITES {
        assert!(RULES.iter().any(|r| r.id == a) && RULES.iter().any(|r| r.id == b), "{a} / {b}");
    }
}

// ------------------------------------------------------- seat relation (C01)

#[test]
fn seat_relation_reflected_in_advice() {
    let mut v = cash(&[(ThreeBetIp, 8, 40), (FoldTo3betIp, 25, 30), (CbetFlop, 38, 40)]);
    let mut ctx = context(FormatKey::Cash, 100.0);
    ctx.seat = Some(seat(1, 6));
    v.context = Some(ctx.clone());
    let reads = evaluate_rules(&v);
    // A frequent 3-bettor directly on the left answers every open.
    let three = read(&reads, "pf.3bet.high_ip").unwrap();
    assert_eq!(three.advice, "4-bet his 3-bets with QQ+, AK and A5s-A4s as bluffs (directly on your left).");
    assert!(three.adapted_by.contains(&"ctx.seat_relation"));
    // His fold-to-3-bet matters when he opens in front of the hero.
    assert_eq!(read(&reads, "pf.fold_to_3bet.high").unwrap().advice, "3-bet his opens light with A2s-A5s, K9s+ and 65s+.");
    // Postflop reads never carry the seat.
    assert_eq!(read(&reads, "post.cbet_flop.high").unwrap().advice, "Raise his flop c-bets with draws and gutshots; call them with any pair.");

    ctx.seat = Some(seat(4, 6));
    v.context = Some(ctx.clone());
    let reads = evaluate_rules(&v);
    assert_eq!(read(&reads, "pf.3bet.high_ip").unwrap().advice, "4-bet his 3-bets with QQ+, AK and A5s-A4s as bluffs.");
    assert_eq!(
        read(&reads, "pf.fold_to_3bet.high").unwrap().advice,
        "3-bet his opens light with A2s-A5s, K9s+ and 65s+ (on your right)."
    );
    ctx.seat = Some(seat(5, 6));
    v.context = Some(ctx);
    assert_eq!(
        read(&evaluate_rules(&v), "pf.fold_to_3bet.high").unwrap().advice,
        "3-bet his opens light with A2s-A5s, K9s+ and 65s+ (directly on your right)."
    );
    // The seat relation is a suffix, never a read of its own.
    assert!(!fires(&v, "ctx.seat_relation"));
}

#[test]
fn zoom_context_never_appends_a_seat_suffix() {
    // Section 5 (audit 2026-10): Zoom reseats players every hand, so a seat
    // relation in a Zoom context (cash or tournament) never reaches the
    // advice, even when a caller supplies one.
    let stats = [(ThreeBetIp, 8, 40), (FoldTo3betIp, 25, 30)];
    for (format, variant) in [(FormatKey::Zoom, "zoom_cash"), (FormatKey::Mtt, "zoom_tournament")] {
        let mut v = input(format, &stats);
        let mut ctx = context(format, 100.0);
        ctx.variant = Some(variant.into());
        ctx.seat = Some(seat(1, 6));
        v.context = Some(ctx.clone());
        let reads = evaluate_rules(&v);
        let three = read(&reads, "pf.3bet.high_ip").unwrap();
        assert!(!three.advice.contains("your left"), "{variant}: {}", three.advice);
        assert!(!three.adapted_by.contains(&"ctx.seat_relation"), "{variant}");
        ctx.seat = Some(seat(5, 6));
        v.context = Some(ctx);
        let fold = read(&evaluate_rules(&v), "pf.fold_to_3bet.high").cloned().unwrap();
        assert!(!fold.advice.contains("your right"), "{variant}: {}", fold.advice);
    }
    // The same villain at a fixed-seat table keeps the suffix.
    let mut v = input(FormatKey::Mtt, &stats);
    let mut ctx = context(FormatKey::Mtt, 100.0);
    ctx.seat = Some(seat(1, 6));
    v.context = Some(ctx);
    assert!(read(&evaluate_rules(&v), "pf.3bet.high_ip").unwrap().advice.ends_with("(directly on your left)."));
}

// ------------------------------------- preflop advice audit (section 17)

/// Stats on which one preflop rule fires in cash and in MTT alike.
fn preflop_firing(id: &str) -> Vec<(StatKey, u32, u32)> {
    match id {
        "pf.profile.nit" => vec![(Vpip, 2, 100), (Pfr, 2, 100)],
        "pf.profile.station" => vec![(Vpip, 60, 100), (Pfr, 5, 100), (Wtsd, 30, 60)],
        "pf.profile.loose_passive" => vec![(Vpip, 60, 100), (Pfr, 5, 100), (Wtsd, 10, 60)],
        "pf.profile.lag" => vec![(Vpip, 45, 100), (Pfr, 38, 100)],
        "pf.rfi.tight_early" => vec![(RfiEp, 0, 40)],
        "pf.rfi.loose_early" => vec![(RfiEp, 30, 40)],
        "pf.rfi.loose_late" => vec![(RfiBtn, 36, 40)],
        "pf.rfi.tight_late" => vec![(RfiBtn, 2, 40)],
        "pf.steal.high" => vec![(Steal, 36, 40)],
        "pf.steal.low" => vec![(Steal, 2, 40)],
        "pf.rfi_sb.wide" => vec![(RfiSb, 36, 40)],
        "pf.fold_to_steal.high" => vec![(FoldToStealBb, 30, 30)],
        "pf.fold_to_steal.low" => vec![(FoldToStealBb, 0, 30)],
        "pf.bb_vs_sb.overfold" => vec![(BbDefendVsSb, 0, 30)],
        "pf.bb_vs_sb.overdefend" => vec![(BbDefendVsSb, 30, 30)],
        "pf.3bet.high_ip" => vec![(ThreeBetIp, 20, 60)],
        "pf.3bet.high_oop" => vec![(ThreeBetOop, 20, 60)],
        "pf.3bet.low" => vec![(ThreeBetIp, 0, 100), (ThreeBetOop, 0, 100)],
        "pf.fold_to_3bet.high" => vec![(FoldTo3betIp, 30, 30)],
        "pf.fold_to_3bet.low" => vec![(FoldTo3betIp, 0, 30)],
        "pf.4bet.high" => vec![(FourBet, 15, 40)],
        "pf.fold_to_4bet.high" => vec![(FoldTo4bet, 20, 20)],
        "pf.squeeze.high" => vec![(Squeeze, 15, 40)],
        "pf.limp.high" => vec![(Limp, 25, 50)],
        "pf.iso.high" => vec![(IsoRaise, 20, 20)],
        "pf.limp_fold.high" => vec![(LimpFold, 20, 20)],
        "pf.limp_reraise.seen" => vec![(LimpReraise, 5, 10)],
        "pf.limp_call.high" => vec![(LimpCall, 20, 20)],
        "pf.cold_call.high" => vec![(ColdCall, 20, 60)],
        _ => panic!("no firing stats for {id}"),
    }
}

/// Every preflop advice as shown, per context: cash at 100bb (the default
/// text), MTT at 100bb in the middle stage (tournament sizing), cash at 30bb
/// (jam depth) and MTT at 30bb in the late stage (late stage first, then jam
/// depth, then tournament).
const PREFLOP_ADVICE: [(&str, [&str; 4]); 29] = [
    ("pf.profile.nit", [
        "Steal his blinds with any two suited; fold AJo and KQo to his opens.",
        "Min-raise his blinds with any two suited; fold AJo and KQo to his opens.",
        "Steal his blinds with any two suited; fold AJo and KQo to his opens.",
        "Min-raise any two into his blinds; call his shoves with TT+ and AQ+ only.",
    ]),
    ("pf.profile.station", ["Value bet top pair on all three streets; cut your river bluffs."; 4]),
    ("pf.profile.loose_passive", ["Isolate his limps with broadways and pairs; value bet top pair thin."; 4]),
    ("pf.profile.lag", [
        "3-bet him for value with 99+, AJs+, KQs and AQo; call down with top pair.",
        "3-bet him for value with 99+, AJs+, KQs and AQo; call down with top pair.",
        "Reshove 88+, AT+ and KQ over his opens; call down with top pair.",
        "Reshove 88+, AT+ and KQ over his opens; call down with top pair.",
    ]),
    ("pf.rfi.tight_early", ["Fold AJo, KQo and ATs to his early opens."; 4]),
    ("pf.rfi.loose_early", [
        "3-bet his early opens with 99+, AJs+, KQs and AQo.",
        "3-bet his early opens with 99+, AJs+, KQs and AQo.",
        "Reshove 99+ and AJ+ over his early opens.",
        "Reshove 99+ and AJ+ over his early opens.",
    ]),
    ("pf.rfi.loose_late", [
        "3-bet his late opens wider for value: 88+, ATs+, KJs+, AQo.",
        "3-bet his late opens wider for value: 88+, ATs+, KJs+, AQo.",
        "Reshove 66+, A9+ and KJ+ over his late opens.",
        "Reshove 66+, A9+ and KJ+ over his late opens.",
    ]),
    ("pf.rfi.tight_late", ["Defend tighter: fold KTo, QJo and A9o to his late opens."; 4]),
    ("pf.steal.high", [
        "Defend your big blind with any suited king vs his steals.",
        "Defend your big blind with any suited king vs his steals.",
        "Reshove 55+, A7+ and K9s+ over his steals from the blinds.",
        "Reshove 55+, A7+ and K9s+ over his steals from the blinds.",
    ]),
    ("pf.steal.low", ["From the blinds, fold offsuit Kx and Qx to his steals."; 4]),
    ("pf.rfi_sb.wide", [
        "3-bet his small-blind opens with 77+, A8s+, KTs+ and ATo+.",
        "3-bet his small-blind opens with 77+, A8s+, KTs+ and ATo+.",
        "Reshove 66+, A8+ and KJ+ over his small-blind opens.",
        "Reshove 66+, A8+ and KJ+ over his small-blind opens.",
    ]),
    ("pf.fold_to_steal.high", [
        "Steal any two suited from the button into his blinds.",
        "Min-raise any two suited from late seats into his blinds.",
        "Steal any two suited from the button into his blinds.",
        "Min-raise any two from the button into his blinds.",
    ]),
    ("pf.fold_to_steal.low", [
        "Cut K8o and Q8o from your steals; open 3x for value.",
        "Cut K8o and Q8o from your steals; open 2.5x for value.",
        "Cut K8o and Q8o from your steals; open 3x for value.",
        "Cut K8o and Q8o from your steals; open 2.5x for value.",
    ]),
    ("pf.bb_vs_sb.overfold", [
        "Raise any two from the small blind into him at 2.5x.",
        "Min-raise any two from the small blind into him.",
        "Raise any two from the small blind into him at 2.5x.",
        "Min-raise or shove any two from the small blind into him.",
    ]),
    ("pf.bb_vs_sb.overdefend", [
        "Cut K5o-type hands from your small-blind opens; raise 3.5x.",
        "Limp K5o-type hands from the small blind; raise 3x for value.",
        "Cut K5o-type hands from your small-blind opens; raise 3.5x.",
        "Limp K5o-type hands from the small blind; raise 3x for value.",
    ]),
    ("pf.3bet.high_ip", [
        "4-bet his 3-bets with QQ+, AK and A5s-A4s as bluffs.",
        "4-bet his 3-bets with QQ+, AK and A5s-A4s as bluffs.",
        "Jam over his 3-bets with 99+, AQ+ and AJs.",
        "Jam over his 3-bets with 99+, AQ+ and AJs.",
    ]),
    ("pf.3bet.high_oop", [
        "Call his 3-bets in position with 77-JJ, AQ, AJs and KQs.",
        "Call his 3-bets in position with 77-JJ, AQ, AJs and KQs.",
        "Jam 88+, AJ+ and KQs over his blind 3-bets.",
        "Jam 88+, AJ+ and KQs over his blind 3-bets.",
    ]),
    ("pf.3bet.low", ["Fold JJ and AQ to his 3-bets; open more suited hands."; 4]),
    ("pf.fold_to_3bet.high", [
        "3-bet his opens light with A2s-A5s, K9s+ and 65s+.",
        "3-bet his opens light with A2s-A5s, K9s+ and 65s+.",
        "Reshove A2s+, K9s+ and 55+ over his opens.",
        "Reshove A2s+, K9s+ and 55+ over his opens.",
    ]),
    ("pf.fold_to_3bet.low", [
        "3-bet his opens at 4x, for value only: no 65s-type bluffs.",
        "3-bet his opens at 4x, for value only: no 65s-type bluffs.",
        "Reshove his opens for value only; no A2s-A5s jams.",
        "Reshove his opens for value only; no A2s-A5s jams.",
    ]),
    ("pf.4bet.high", [
        "Jam TT+ and AQ+ over his 4-bets; fold your light 3-bets.",
        "Jam TT+ and AQ+ over his 4-bets; fold your light 3-bets.",
        "Jam, don't 3-bet small; call his 4-bet jam with 99+ and AQ+.",
        "Jam, don't 3-bet small; call his 4-bet jam with 99+ and AQ+.",
    ]),
    ("pf.fold_to_4bet.high", [
        "4-bet bluff his 3-bets with A2s-A5s, sized 2.2x.",
        "4-bet bluff his 3-bets with A2s-A5s, sized 2.2x.",
        "4-bet jam over his 3-bets with A2s+ and K9s+.",
        "4-bet jam over his 3-bets with A2s+ and K9s+.",
    ]),
    ("pf.squeeze.high", [
        "4-bet his squeezes with TT+ and AK; flat less with KJo.",
        "4-bet his squeezes with TT+ and AK; flat less with KJo.",
        "Jam 99+ and AQ+ over his squeezes; flat less with KJo.",
        "Jam 99+ and AQ+ over his squeezes; flat less with KJo.",
    ]),
    ("pf.limp.high", ["Isolate his limps with pairs, suited aces and broadways."; 4]),
    ("pf.iso.high", ["Raise or fold over limpers; limp-reraise QQ+ and AK."; 4]),
    ("pf.limp_fold.high", ["Isolate his limps to 3x with any two suited cards."; 4]),
    ("pf.limp_reraise.seen", ["Fold all but QQ+ and AK to his limp-reraise."; 4]),
    ("pf.limp_call.high", [
        "Isolate his limps to 5x with broadways and pairs; bluff less.",
        "Isolate his limps to 4x with broadways and pairs; bluff less.",
        "Isolate his limps to 5x with broadways and pairs; bluff less.",
        "Isolate his limps to 4x with broadways and pairs; bluff less.",
    ]),
    ("pf.cold_call.high", [
        "Open 3x for value; squeeze his flats with 99+, AQ+ and A5s.",
        "Open 2.5x for value; squeeze his flats with 99+, AQ+ and A5s.",
        "Squeeze-jam 88+ and AJ+ when he flats an open.",
        "Squeeze-jam 88+ and AJ+ when he flats an open.",
    ]),
];

/// The four contexts of [`PREFLOP_ADVICE`], in its column order.
fn preflop_contexts() -> [(FormatKey, EngineContext); 4] {
    let mut mtt = context(FormatKey::Mtt, 100.0);
    mtt.stage = Some(Stage::Middle);
    let mut late = context(FormatKey::Mtt, 30.0);
    late.stage = Some(Stage::Late);
    [
        (FormatKey::Cash, context(FormatKey::Cash, 100.0)),
        (FormatKey::Mtt, mtt),
        (FormatKey::Cash, context(FormatKey::Cash, 30.0)),
        (FormatKey::Mtt, late),
    ]
}

fn preflop_advice(id: &str, format: FormatKey, ctx: Option<EngineContext>) -> String {
    let mut v = input(format, &preflop_firing(id));
    v.context = ctx;
    read(&evaluate_rules(&v), id).unwrap_or_else(|| panic!("{id} must fire in {format:?}")).advice.clone()
}

#[test]
fn preflop_advice_follows_format_and_stack_depth() {
    // Every preflop rule is pinned, and every text it can show is asserted.
    let preflop: Vec<&str> = RULES.iter().filter(|r| r.family == Family::Preflop).map(|r| r.id).collect();
    let pinned: Vec<&str> = PREFLOP_ADVICE.iter().map(|(id, _)| *id).collect();
    assert_eq!(pinned, preflop);
    let contexts = preflop_contexts();
    for (id, expected) in PREFLOP_ADVICE {
        for ((format, ctx), want) in contexts.iter().zip(expected) {
            assert_eq!(preflop_advice(id, *format, Some(ctx.clone())), want, "{id} in {format:?} at {:?}bb", ctx.effective_stack_bb);
        }
        let def = rule_def(id);
        for (variant, text) in def.variants {
            assert!(expected.contains(text), "{id}: the {variant:?} text is never shown by the pinned contexts");
        }
    }
}

#[test]
fn preflop_variants_switch_at_their_boundaries() {
    // Jam depth is 15 < eff ≤ 40 (section 5); at 15bb or less preflop reads
    // do not show at all.
    assert_eq!(JAM_DEPTH_MAX_BB, 40.0);
    let id = "pf.fold_to_3bet.high";
    let jam = "Reshove A2s+, K9s+ and 55+ over his opens.";
    let sized = "3-bet his opens light with A2s-A5s, K9s+ and 65s+.";
    for (eff, want) in [(15.1, jam), (25.0, jam), (40.0, jam), (40.1, sized), (60.0, sized)] {
        assert_eq!(preflop_advice(id, FormatKey::Cash, Some(context(FormatKey::Cash, eff))), want, "{eff}bb");
    }
    let mut v = input(FormatKey::Cash, &preflop_firing(id));
    v.context = Some(context(FormatKey::Cash, 15.0));
    assert!(!fires(&v, id), "push/fold suppresses preflop reads");
    // Jam depth records why the text changed.
    v.context = Some(context(FormatKey::Cash, 30.0));
    assert!(read(&evaluate_rules(&v), id).unwrap().adapted_by.contains(&"stack.jam_depth"));

    // The tournament text follows the format of the stats, with or without
    // a table: MTT and Spin & Go, never cash or Zoom.
    let id = "pf.cold_call.high";
    let cash_text = "Open 3x for value; squeeze his flats with 99+, AQ+ and A5s.";
    let tournament_text = "Open 2.5x for value; squeeze his flats with 99+, AQ+ and A5s.";
    for (format, want) in
        [(FormatKey::Cash, cash_text), (FormatKey::Zoom, cash_text), (FormatKey::Mtt, tournament_text), (FormatKey::Spin, tournament_text)]
    {
        assert_eq!(preflop_advice(id, format, None), want, "{format:?} without a table");
        assert_eq!(preflop_advice(id, format, Some(context(format, 100.0))), want, "{format:?} at 100bb");
    }
    let mut v = input(FormatKey::Spin, &preflop_firing(id));
    v.context = Some(context(FormatKey::Spin, 100.0));
    assert!(read(&evaluate_rules(&v), id).unwrap().adapted_by.contains(&"format.tournament"));
    // Precedence: jam depth before the tournament text.
    assert_eq!(
        preflop_advice(id, FormatKey::Mtt, Some(context(FormatKey::Mtt, 30.0))),
        "Squeeze-jam 88+ and AJ+ when he flats an open."
    );
    // Every variant list is written in precedence order: late stage, jam
    // depth, tournament (the first match wins).
    let rank = |v: &Variant| match v {
        Variant::LateStage => 0,
        Variant::JamDepth => 1,
        Variant::Tournament => 2,
        _ => 3,
    };
    for def in RULES.iter().filter(|r| r.family == Family::Preflop) {
        let order: Vec<u8> = def.variants.iter().map(|(v, _)| rank(v)).collect();
        assert!(order.windows(2).all(|w| w[0] < w[1]), "{} variants out of order", def.id);
    }
}

#[test]
fn opposite_preflop_reads_never_fire_together() {
    // Audit 2026-10 (section 17): "a or b" reads fire on either leg, so one
    // villain could get both sides of a pair: "Fold AJo, KQo and ATs to his
    // early opens" next to "3-bet his early opens with 99+…". Only the read
    // with the higher score is kept.
    let ids = |input: &RuleInput| -> Vec<String> {
        evaluate_rules(input).iter().map(|r| r.rule_id.clone()).filter(|id| id.starts_with("pf.")).collect()
    };
    // Tight UTG (0/40: .044, deviation 2.3) and loose HJ (30/40: .603,
    // deviation 6.5): the loose read wins.
    let both = cash(&[(RfiEp, 0, 40), (RfiMp, 30, 40)]);
    assert_eq!(ids(&both), ["pf.rfi.loose_early"]);
    // Tight cutoff (2/40: .113, deviation 2.1) and loose button (36/40:
    // .769, deviation 3.5).
    let both = cash(&[(RfiCo, 2, 40), (RfiBtn, 36, 40)]);
    assert_eq!(ids(&both), ["pf.rfi.loose_late"]);
    // Tight cutoff and a wide steal (36/40: .711, deviation 4.0).
    let both = cash(&[(RfiCo, 2, 40), (Steal, 36, 40)]);
    assert_eq!(ids(&both), ["pf.steal.high"]);
    // A wide button (36/40) and a rare steal (2/40: .136, deviation 2.4).
    let both = cash(&[(RfiBtn, 36, 40), (Steal, 2, 40)]);
    assert_eq!(ids(&both), ["pf.rfi.loose_late"]);
    // Folds his small blind (30/30: .937, deviation 1.6) but defends his big
    // blind (0/30: .171, deviation 3.6): "steal any two" and "cut K8o from
    // your steals" cannot both stand; the big blind's read wins.
    let both = cash(&[(FoldToStealSb, 30, 30), (FoldToStealBb, 0, 30)]);
    assert_eq!(ids(&both), ["pf.fold_to_steal.low"]);
    // The other side wins when it is the stronger read: tight UTG over 100
    // opens (.021, deviation 2.8) and a hijack just over the line (13/40:
    // .294, deviation 1.4).
    let both = cash(&[(RfiEp, 0, 100), (RfiMp, 13, 40)]);
    assert!(fires(&cash(&[(RfiMp, 13, 40)]), "pf.rfi.loose_early"));
    assert_eq!(ids(&both), ["pf.rfi.tight_early"]);
    // Each side alone still fires.
    assert!(fires(&cash(&[(RfiEp, 0, 40)]), "pf.rfi.tight_early"));
    assert!(fires(&cash(&[(FoldToStealSb, 30, 30)]), "pf.fold_to_steal.high"));
    // Every declared pair names two preflop rules.
    for (a, b) in PREFLOP_OPPOSITES {
        assert!(rule_def(a).family == Family::Preflop && rule_def(b).family == Family::Preflop, "{a} / {b}");
    }
}

#[test]
fn reads_from_different_stats_give_advice_that_fits_together() {
    // Review of audit-preflop: a reg who folds (30 + 8.25)/55 = .695 of his
    // in-position 3-bets and 4-bets (14 + 1.4)/80 = .19 gets both reads.
    // That is one consistent plan, not a contradiction: 3-bet him light and
    // fold the light 3-bets to his 4-bet, jam the value hands over it. So
    // both reads stay, but the 4-bet read must not tell the hero to drop the
    // light 3-bets the fold-to-3-bet read recommends.
    for (format, depth, fold_text, four_bet_text) in [
        (
            FormatKey::Cash,
            100.0,
            "3-bet his opens light with A2s-A5s, K9s+ and 65s+.",
            "Jam TT+ and AQ+ over his 4-bets; fold your light 3-bets.",
        ),
        (
            FormatKey::Cash,
            30.0,
            "Reshove A2s+, K9s+ and 55+ over his opens.",
            "Jam, don't 3-bet small; call his 4-bet jam with 99+ and AQ+.",
        ),
    ] {
        let mut v = input(format, &[(FoldTo3betIp, 30, 40), (FourBet, 14, 60)]);
        v.context = Some(context(format, depth));
        let reads = evaluate_rules(&v);
        let fold = &read(&reads, "pf.fold_to_3bet.high").expect("fold to 3-bet fires").advice;
        let four = &read(&reads, "pf.4bet.high").expect("4-bet fires").advice;
        for word in ["skip", "only", "A5s", "A2s"] {
            assert!(!four.contains(word), "{depth}bb: 4-bet advice drops the light 3-bets ({word}): {four}");
        }
        assert_eq!(fold, fold_text);
        assert_eq!(four, four_bet_text);
    }

    // A limp-folder who limp-reraised 7 of 40 iso raises: limp-fold (33 +
    // 4.4)/48 = .78 and limp-reraise (7 + .4)/48 = .154 both fire. His fold
    // rate is measured over the same iso raises, reraises included, so
    // isolating wide stays right; the limp-reraise read must not narrow the
    // iso range ("only 88+" against "any two suited"), only say what to do
    // when he reraises.
    let reads = evaluate_rules(&cash(&[(LimpFold, 33, 40), (LimpReraise, 7, 40)]));
    let iso = &read(&reads, "pf.limp_fold.high").expect("limp-fold fires").advice;
    let lrr = &read(&reads, "pf.limp_reraise.seen").expect("limp-reraise fires").advice;
    for word in ["Isolate", "only", "88+"] {
        assert!(!lrr.contains(word), "limp-reraise advice narrows the iso range ({word}): {lrr}");
    }
    assert_eq!(iso, "Isolate his limps to 3x with any two suited cards.");
    assert_eq!(lrr, "Fold all but QQ+ and AK to his limp-reraise.");

    // Second review: a wide opener who rarely folds to 3-bets (a LAG reg)
    // got "3-bet his late opens light: A2s-A5s…" next to "3-bet him only
    // with TT+ and AQ+". A pro 3-bets him wider for value and drops the
    // light 3-bets, so the opener reads name value ranges only and the
    // continue read removes bluffs without capping the value range.
    let cap = regex::Regex::new(r"(?:[2-9TJQKA]{2}\+|\bonly with\b)").unwrap();
    let continues = (FoldTo3betIp, 0, 30);
    for (opens, id, at_100, at_30) in [
        (
            (RfiBtn, 36, 40),
            "pf.rfi.loose_late",
            "3-bet his late opens wider for value: 88+, ATs+, KJs+, AQo.",
            "Reshove 66+, A9+ and KJ+ over his late opens.",
        ),
        (
            (RfiSb, 36, 40),
            "pf.rfi_sb.wide",
            "3-bet his small-blind opens with 77+, A8s+, KTs+ and ATo+.",
            "Reshove 66+, A8+ and KJ+ over his small-blind opens.",
        ),
        (
            (RfiEp, 30, 40),
            "pf.rfi.loose_early",
            "3-bet his early opens with 99+, AJs+, KQs and AQo.",
            "Reshove 99+ and AJ+ over his early opens.",
        ),
    ] {
        for (depth, want, c3b_want) in [
            (100.0, at_100, "3-bet his opens at 4x, for value only: no 65s-type bluffs."),
            (30.0, at_30, "Reshove his opens for value only; no A2s-A5s jams."),
        ] {
            let mut v = cash(&[opens, continues]);
            v.context = Some(context(FormatKey::Cash, depth));
            let reads = evaluate_rules(&v);
            let wide = &read(&reads, id).unwrap_or_else(|| panic!("{id} fires")).advice;
            let c3b = &read(&reads, "pf.fold_to_3bet.low").expect("fold to 3-bet low fires").advice;
            for word in ["light", "A2s", "A5s"] {
                assert!(!wide.contains(word), "{id} at {depth}bb bluffs a villain who continues ({word}): {wide}");
            }
            assert!(!cap.is_match(c3b), "{depth}bb: the continue read caps the value range: {c3b}");
            assert_eq!((wide.as_str(), c3b.as_str()), (want, c3b_want), "{id} at {depth}bb");
        }
    }
    // The LAG profile names the same value range as the early opener read.
    let lag = cash(&[(Vpip, 45, 100), (Pfr, 38, 100), continues]);
    let reads = evaluate_rules(&lag);
    assert_eq!(
        read(&reads, "pf.profile.lag").expect("lag fires").advice,
        "3-bet him for value with 99+, AJs+, KQs and AQo; call down with top pair."
    );
    assert!(!cap.is_match(&read(&reads, "pf.fold_to_3bet.low").expect("continues").advice));

    // A tight late opener who folds to 3-bets: "3-bet QQ+ and AK only" next
    // to "3-bet his opens light". The tight-opener read says what to fold;
    // the light 3-bets come from the fold-to-3-bet read.
    let reads = evaluate_rules(&cash(&[(RfiBtn, 2, 40), (FoldTo3betIp, 30, 30)]));
    let tight = &read(&reads, "pf.rfi.tight_late").expect("tight late fires").advice;
    assert!(read(&reads, "pf.fold_to_3bet.high").is_some());
    assert!(!tight.contains("3-bet") && !tight.contains("only"), "{tight}");
    assert_eq!(tight, "Defend tighter: fold KTo, QJo and A9o to his late opens.");

    // A big blind who rarely 3-bets but defends wide: "open wider in front
    // of him" next to "cut K8o and Q8o from your steals" and "open your
    // small blind tighter". Against a flatter, open more suited hands and cut
    // offsuit trash.
    let reads = evaluate_rules(&cash(&[
        (ThreeBetIp, 0, 100),
        (ThreeBetOop, 0, 100),
        (FoldToStealBb, 0, 30),
        (BbDefendVsSb, 30, 30),
    ]));
    let low = &read(&reads, "pf.3bet.low").expect("3-bet low fires").advice;
    let defends = &read(&reads, "pf.fold_to_steal.low").expect("defends fires").advice;
    let over = &read(&reads, "pf.bb_vs_sb.overdefend").expect("overdefend fires").advice;
    assert!(!low.contains("wider"), "{low}");
    assert!(!over.contains("tighter"), "{over}");
    assert_eq!(low, "Fold JJ and AQ to his 3-bets; open more suited hands.");
    assert_eq!(defends, "Cut K8o and Q8o from your steals; open 3x for value.");
    assert_eq!(over, "Cut K5o-type hands from your small-blind opens; raise 3.5x.");
}

// ---------------------------------------------------------- postflop (F01–F12)

#[test]
fn cbet_flop_rules_fire_at_threshold_and_not_below() {
    let c = FormatKey::Cash;
    assert_above(c, "post.cbet_flop.high", CbetFlop, 40, 0.15, &[]);
    assert_below(c, "post.cbet_flop.low", CbetFlop, 40, 0.15, &[]);
    // Small-sample extreme: 8 of 8 c-bets is (8 + 8.7)/23 = .726 < .73.
    assert!(!fires(&cash(&[(CbetFlop, 8, 8)]), "post.cbet_flop.high"));
}

#[test]
fn fold_to_cbet_flop_rules_fire_at_threshold_and_not_below() {
    let c = FormatKey::Cash;
    assert_above(c, "post.fold_cbet_flop.high", FoldToCbetFlop, 40, 0.12, &[]);
    assert_below(c, "post.fold_cbet_flop.low", FoldToCbetFlop, 40, 0.15, &[]);
}

#[test]
fn cbet_turn_rules_fire_at_threshold_and_not_below() {
    let c = FormatKey::Cash;
    assert_above(c, "post.cbet_turn.high", CbetTurn, 30, 0.15, &[]);
    assert_below(c, "post.cbet_turn.giveup", CbetTurn, 30, 0.15, &[]);
}

#[test]
fn cbet_river_rules_fire_at_threshold_and_not_below() {
    let c = FormatKey::Cash;
    assert_above(c, "post.cbet_river.high", CbetRiver, 30, 0.15, &[]);
    assert_below(c, "post.cbet_river.low", CbetRiver, 30, 0.15, &[]);
}

#[test]
fn fold_to_barrel_rules_fire_at_threshold_and_not_below() {
    let c = FormatKey::Cash;
    assert_above(c, "post.fold_cbet_turn.high", FoldToCbetTurn, 30, 0.15, &[]);
    assert_below(c, "post.fold_cbet_river.low", FoldToCbetRiver, 30, 0.15, &[]);
}

#[test]
fn fold_to_river_barrel_rule_fires_at_threshold_and_not_below() {
    // Pro review (D107, F14): calls flop and turn, then folds the river.
    // Prior .48, k 10: (h + 4.8)/40 ≥ .63 → 21 of 30.
    let c = FormatKey::Cash;
    assert_eq!(assert_above(c, "post.fold_cbet_river.high", FoldToCbetRiver, 30, 0.15, &[]), 21);
    let reads = evaluate_rules(&cash(&[(FoldToCbetRiver, 21, 30)]));
    let r = read(&reads, "post.fold_cbet_river.high").unwrap();
    assert_eq!(r.observation, "Folds to 70% (21/30) of river barrels after calling the turn.");
    assert_eq!(r.advice, "Fire third barrels on the river at 3/4 pot with missed draws and ace blockers.");
    assert_eq!((r.tag.as_deref(), r.scenario_id.as_str()), (Some("RVF"), "F14"));
    // The opposite read of the same stat never fires with it.
    assert!(read(&reads, "post.fold_cbet_river.low").is_none());
    // Small-sample extreme: 6 of 6 river folds is (6 + 4.8)/16 = .675, which
    // fires; 5 of 5 is under the 6-opportunity floor.
    assert!(fires(&cash(&[(FoldToCbetRiver, 6, 6)]), "post.fold_cbet_river.high"));
    assert!(!fires(&cash(&[(FoldToCbetRiver, 5, 5)]), "post.fold_cbet_river.high"));
}

#[test]
fn pro_review_reads_respect_push_fold_and_sample_floors() {
    // Every D107 read is a preflop or postflop read: at 15bb or less in a
    // tournament only push/fold reads show, and none fires under its floor.
    let m = FormatKey::Mtt;
    let busy = [(RfiSb, 30, 40), (LimpCall, 15, 20), (FoldToCbetRiver, 25, 30), (OpenShove, 15, 30)];
    let free = evaluate_rules(&input(m, &busy));
    for id in ["pf.rfi_sb.wide", "pf.limp_call.high", "post.fold_cbet_river.high"] {
        assert!(read(&free, id).is_some(), "{id} fires with deep stacks");
    }
    let mut short = input(m, &busy);
    short.context = Some(context(m, 12.0));
    let reads = evaluate_rules(&short);
    for id in ["pf.rfi_sb.wide", "pf.limp_call.high", "post.fold_cbet_river.high"] {
        assert!(read(&reads, id).is_none(), "{id} must not show at 12bb");
    }
    assert!(read(&reads, "stk.open_shove.wide").is_some());
    for (key, floor) in [(RfiSb, 8), (LimpCall, 5), (FoldToCbetRiver, 6)] {
        assert_eq!(stat_spec(key).n_min, floor, "{key:?}");
        let under = cash(&[(key, floor - 1, floor - 1)]);
        assert!(evaluate_rules(&under).is_empty(), "{key:?} under its floor: {:?}", ids(&evaluate_rules(&under)));
    }
}

#[test]
fn delayed_cbet_rule_fires_at_threshold_and_not_below() {
    assert_above(FormatKey::Cash, "post.delayed_cbet.high", DelayedCbet, 30, 0.15, &[]);
}

#[test]
fn check_raise_rules_fire_at_threshold_and_not_below() {
    let c = FormatKey::Cash;
    assert_above(c, "post.check_raise.high", CheckRaiseFlop, 60, 0.06, &[]);
    // Never check-raises: shrunk ≤ .02 with n ≥ 20. Prior .09, k 20; 0 of
    // 80 is 1.8/100 = .018; 0 of 60 is 1.8/80 = .0225 (short).
    assert!(fires(&cash(&[(CheckRaiseFlop, 0, 80)]), "post.check_raise.never"));
    assert!(!fires(&cash(&[(CheckRaiseFlop, 0, 60)]), "post.check_raise.never"));
    assert!(!fires(&cash(&[(CheckRaiseFlop, 1, 80)]), "post.check_raise.never"));
    // Small-sample extreme: 0 of 19 (raw 0%) never fires.
    assert!(!fires(&cash(&[(CheckRaiseFlop, 0, 19)]), "post.check_raise.never"));
    let reads = evaluate_rules(&cash(&[(CheckRaiseFlop, 0, 80)]));
    assert_eq!(read(&reads, "post.check_raise.never").unwrap().observation, "Has check-raised 0 of 80 flop bets after checking.");
}

#[test]
fn donk_rule_fires_at_threshold_and_not_below() {
    assert_above(FormatKey::Cash, "post.donk.high", DonkFlop, 60, 0.06, &[]);
}

#[test]
fn float_rule_fires_at_threshold_and_not_below() {
    assert_above(FormatKey::Cash, "post.float.high", FloatFlop, 30, 0.15, &[]);
}

#[test]
fn probe_rule_fires_at_threshold_and_not_below() {
    assert_above(FormatKey::Cash, "post.probe.high", ProbeTurn, 30, 0.15, &[]);
}

#[test]
fn river_rules_fire_at_threshold_and_not_below() {
    let c = FormatKey::Cash;
    assert_above(c, "post.river_bet.high", RiverBet, 40, 0.12, &[]);
    assert_below(c, "post.river_bet.low", RiverBet, 40, 0.12, &[]);
    assert_above(c, "post.river_raise.high", RiverRaise, 60, 0.06, &[]);
}

#[test]
fn showdown_rules_fire_at_threshold_and_not_below() {
    let c = FormatKey::Cash;
    // Station: WTSD ≥ prior + .10 with W$SD ≤ prior − .06.
    let wsd_low = last_below(c, Wsd, 40, 0.06);
    assert_above(c, "post.station", Wtsd, 60, 0.10, &[(Wsd, wsd_low, 40)]);
    assert!(!fires(&cash(&[(Wtsd, 40, 60), (Wsd, wsd_low + 1, 40)]), "post.station"));
    let reads = evaluate_rules(&cash(&[(Wtsd, 30, 60), (Wsd, 10, 40)]));
    assert_eq!(
        read(&reads, "post.station").unwrap().observation,
        "Goes to showdown 50% (30/60) and wins there only 25%."
    );
    // W$SD strong needs WTSD at or under its prior.
    let wtsd_low = last_below(c, Wtsd, 60, 0.0);
    assert_above(c, "post.wsd_strong", Wsd, 40, 0.08, &[(Wtsd, wtsd_low, 60)]);
    assert!(!fires(&cash(&[(Wtsd, wtsd_low + 1, 60), (Wsd, 40, 40)]), "post.wsd_strong"));
    // Audit 2026-10: the low-WWSF read also needs WTSD at or under its prior.
    assert_below(c, "post.wwsf_low", Wwsf, 60, 0.08, &[(Wtsd, wtsd_low, 60)]);
    assert_above(c, "post.wwsf_high", Wwsf, 60, 0.08, &[]);
}

#[test]
fn preflop_all_in_creates_no_postflop_rule_reads() {
    // Rules see only what extraction counted: no postflop opportunity means
    // no postflop read, whatever the preflop numbers.
    let reads = evaluate_rules(&cash(&[(Vpip, 50, 60), (Pfr, 40, 60)]));
    assert!(reads.iter().all(|r| r.family != Family::Postflop));
}

// ------------------------------------------- postflop audit (section 17.6)

/// Stats on which a postflop rule fires in 6-max cash (shrunk values in the
/// comments are against the cash priors of section 4.3).
fn postflop_firing(id: &str) -> Vec<(StatKey, u32, u32)> {
    match id {
        // (36 + 8.7)/55 = .81 ≥ .73; (8 + 8.7)/55 = .30 ≤ .43
        "post.cbet_flop.high" => vec![(CbetFlop, 36, 40)],
        "post.cbet_flop.low" => vec![(CbetFlop, 8, 40)],
        // (30 + 6.75)/55 = .67 ≥ .57; (5 + 6.75)/55 = .21 ≤ .30
        "post.fold_cbet_flop.high" => vec![(FoldToCbetFlop, 30, 40)],
        "post.fold_cbet_flop.low" => vec![(FoldToCbetFlop, 5, 40)],
        "post.cbet_turn.high" => vec![(CbetTurn, 25, 30)],
        "post.cbet_turn.giveup" => vec![(CbetTurn, 4, 30)],
        "post.fold_cbet_turn.high" => vec![(FoldToCbetTurn, 25, 30)],
        "post.cbet_river.high" => vec![(CbetRiver, 28, 30)],
        "post.cbet_river.low" => vec![(CbetRiver, 3, 30)],
        "post.fold_cbet_river.low" => vec![(FoldToCbetRiver, 2, 30)],
        "post.fold_cbet_river.high" => vec![(FoldToCbetRiver, 25, 30)],
        "post.delayed_cbet.high" => vec![(DelayedCbet, 25, 30)],
        "post.check_raise.high" => vec![(CheckRaiseFlop, 15, 60)],
        "post.check_raise.never" => vec![(CheckRaiseFlop, 0, 80)],
        "post.donk.high" => vec![(DonkFlop, 12, 60)],
        "post.float.high" => vec![(FloatFlop, 25, 30)],
        "post.probe.high" => vec![(ProbeTurn, 25, 30)],
        "post.river_bet.high" => vec![(RiverBet, 30, 40)],
        "post.river_bet.low" => vec![(RiverBet, 2, 40)],
        "post.river_raise.high" => vec![(RiverRaise, 15, 60)],
        // WTSD (30 + 5.4)/80 = .44 ≥ .37; W$SD (10 + 7.5)/55 = .32 ≤ .44
        "post.station" => vec![(Wtsd, 30, 60), (Wsd, 10, 40)],
        // W$SD (30 + 7.5)/55 = .68 ≥ .58; WTSD (10 + 5.4)/80 = .19 ≤ .27
        "post.wsd_strong" => vec![(Wsd, 30, 40), (Wtsd, 10, 60)],
        // WWSF (15 + 9.2)/80 = .30 ≤ .38; WTSD (12 + 5.4)/80 = .22 ≤ .27
        "post.wwsf_low" => vec![(Wwsf, 15, 60), (Wtsd, 12, 60)],
        "post.wwsf_high" => vec![(Wwsf, 45, 60)],
        // (10 + 1.2)/30 = .37 ≥ .24; (12 + 2)/30 = .47 ≥ .35
        "post.river_sizing.polar" => vec![(RiverBetPolar, 10, 20)],
        "post.river_sizing.small" => vec![(RiverBetSmall, 12, 20)],
        // (12 + 3.6)/23 = .68 ≥ .60
        "post.fold_river_raise.high" => vec![(FoldToRiverRaise, 12, 15)],
        // FCB (30 + 6.75)/55 = .67 ≥ .55; WTSD (10 + 5.4)/80 = .19 ≤ .22
        "post.fit_or_fold" => vec![(FoldToCbetFlop, 30, 40), (Wtsd, 10, 60)],
        other => panic!("no postflop firing input for {other}"),
    }
}

/// Every postflop advice after the 2026-10 audit (section 17.6): street,
/// sizing or hand classes, and never the same note as another read.
const POSTFLOP_ADVICE: [(&str, &str); 28] = [
    ("post.cbet_flop.high", "Raise his flop c-bets with draws and gutshots; call them with any pair."),
    ("post.cbet_flop.low", "Fold bottom pair to his flop c-bets; bet 1/2 pot when he checks the flop."),
    ("post.fold_cbet_flop.high", "C-bet him at 1/3 pot with your whole range on dry flops."),
    ("post.fold_cbet_flop.low", "C-bet flops at 2/3 pot with top pair+ and draws; check back overcards."),
    ("post.cbet_turn.high", "Call his turn barrels with second pair+; float flops with pairs, not gutshots."),
    ("post.cbet_turn.giveup", "Float his flop c-bets in position with overcards; bet 2/3 pot if he checks the turn."),
    ("post.fold_cbet_turn.high", "Barrel the turn at 2/3 pot with draws and overcards after he calls the flop."),
    ("post.cbet_river.high", "Bluff-catch his river barrels with top pair and good second pairs."),
    ("post.cbet_river.low", "Fold one pair to his river barrels; call his turn barrels with draws."),
    ("post.fold_cbet_river.low", "Size river value bets up to pot with two pair or better after two calls."),
    ("post.fold_cbet_river.high", "Fire third barrels on the river at 3/4 pot with missed draws and ace blockers."),
    ("post.delayed_cbet.high", "Call his delayed turn bets with any pair; raise them with draws."),
    ("post.check_raise.high", "Call his flop check-raises with top pair and open-enders; never fold overpairs."),
    ("post.check_raise.never", "C-bet second pair on the flop for thin value; fold one pair to his rare check-raise."),
    ("post.donk.high", "Raise his flop leads to 3x with top pair, draws and overcards."),
    ("post.float.high", "Check-raise his turn stabs with top pair+ and draws after he calls your c-bet."),
    ("post.probe.high", "Call his turn probes with second pair+; raise them with flush draws."),
    ("post.river_bet.high", "Check top pair to him on the river; call his river bets with second pair+."),
    ("post.river_bet.low", "Fold one pair to his river bets; call them with two pair or better."),
    ("post.river_raise.high", "Check second pair on the river; call his river raises with top pair or better."),
    ("post.station", "Value bet middle pair on the river; bet pot with two pair or better."),
    ("post.wsd_strong", "Value bet top pair on flop and turn only; bet rivers with two pair or better."),
    ("post.wwsf_low", "Stab 1/2 pot on flops and turns he checks; barrel draws twice."),
    ("post.wwsf_high", "Bluff him less on turns and rivers; call his flop bets with any pair."),
    ("post.river_sizing.polar", "Call his river overbets with top pair+ and fold second pair; raise only with sets+."),
    ("post.river_sizing.small", "Raise his small river bets to 3x with two pair+ and some missed draws; call any pair."),
    ("post.fold_river_raise.high", "Raise his river bets 2.5x with missed draws; call, never raise, with one pair."),
    ("post.fit_or_fold", "C-bet any two at 1/3 pot on the flop; barrel the turn only with draws or top pair+."),
];

#[test]
fn postflop_advice_texts_are_pinned() {
    let postflop: Vec<&str> = RULES.iter().filter(|r| r.family == Family::Postflop).map(|r| r.id).collect();
    assert_eq!(postflop, POSTFLOP_ADVICE.iter().map(|(id, _)| *id).collect::<Vec<_>>(), "one pinned text per rule, in catalogue order");
    for (id, advice) in POSTFLOP_ADVICE {
        let reads = evaluate_rules(&cash(&postflop_firing(id)));
        let r = read(&reads, id).unwrap_or_else(|| panic!("{id} must fire on {:?}: {:?}", postflop_firing(id), ids(&reads)));
        assert_eq!(r.advice, advice, "{id}");
        // Postflop reads have no variants and no seat side: the text is the
        // same in every format and context where they show.
        assert!(rule_def(id).variants.is_empty() && rule_def(id).seat.is_none(), "{id}");
    }
    // Observations that described a narrower spot than the stat counts.
    let reads = evaluate_rules(&cash(&[(CheckRaiseFlop, 15, 60), (FloatFlop, 25, 30)]));
    assert_eq!(read(&reads, "post.check_raise.high").unwrap().observation, "Check-raises 25% (15/60) of flop bets he faces after checking.");
    assert_eq!(read(&reads, "post.float.high").unwrap().observation, "Stabs 83% (25/30) of turns checked to him after calling the flop in position.");
}

#[test]
fn postflop_thresholds_hold_in_every_format() {
    // Section 17.4 states each postflop threshold in absolute terms per
    // format; the deltas are the same in every format and the priors differ.
    let single: [(&str, StatKey, bool, f64, u32); 23] = [
        ("post.cbet_flop.high", CbetFlop, true, 0.15, 40),
        ("post.cbet_flop.low", CbetFlop, false, 0.15, 40),
        ("post.fold_cbet_flop.high", FoldToCbetFlop, true, 0.12, 40),
        ("post.fold_cbet_flop.low", FoldToCbetFlop, false, 0.15, 40),
        ("post.cbet_turn.high", CbetTurn, true, 0.15, 30),
        ("post.cbet_turn.giveup", CbetTurn, false, 0.15, 30),
        ("post.fold_cbet_turn.high", FoldToCbetTurn, true, 0.15, 30),
        ("post.cbet_river.high", CbetRiver, true, 0.15, 30),
        ("post.cbet_river.low", CbetRiver, false, 0.15, 30),
        ("post.fold_cbet_river.low", FoldToCbetRiver, false, 0.15, 30),
        ("post.fold_cbet_river.high", FoldToCbetRiver, true, 0.15, 30),
        ("post.delayed_cbet.high", DelayedCbet, true, 0.15, 30),
        ("post.check_raise.high", CheckRaiseFlop, true, 0.06, 60),
        ("post.donk.high", DonkFlop, true, 0.06, 60),
        ("post.float.high", FloatFlop, true, 0.15, 30),
        ("post.probe.high", ProbeTurn, true, 0.15, 30),
        ("post.river_bet.high", RiverBet, true, 0.12, 40),
        ("post.river_bet.low", RiverBet, false, 0.12, 40),
        ("post.river_raise.high", RiverRaise, true, 0.06, 60),
        ("post.wwsf_high", Wwsf, true, 0.08, 60),
        ("post.river_sizing.polar", RiverBetPolar, true, 0.12, 40),
        ("post.river_sizing.small", RiverBetSmall, true, 0.15, 40),
        ("post.fold_river_raise.high", FoldToRiverRaise, true, 0.15, 30),
    ];
    for format in [FormatKey::Mtt, FormatKey::Cash, FormatKey::Zoom, FormatKey::Spin] {
        for (id, key, above, delta, n) in single {
            if above {
                assert_above(format, id, key, n, delta, &[]);
            } else {
                assert_below(format, id, key, n, delta, &[]);
            }
        }
        let wsd_low = last_below(format, Wsd, 40, 0.06);
        assert_above(format, "post.station", Wtsd, 60, 0.10, &[(Wsd, wsd_low, 40)]);
        let wtsd_low = last_below(format, Wtsd, 60, 0.0);
        assert_above(format, "post.wsd_strong", Wsd, 40, 0.08, &[(Wtsd, wtsd_low, 60)]);
        assert_below(format, "post.wwsf_low", Wwsf, 60, 0.08, &[(Wtsd, wtsd_low, 60)]);
        // Fit-or-fold: FCB ≥ prior + .10 with WTSD ≤ prior − .05, each
        // boundary checked with the other leg just passing.
        let wtsd_fof = last_below(format, Wtsd, 60, 0.05);
        let fcb_fof = assert_above(format, "post.fit_or_fold", FoldToCbetFlop, 40, 0.10, &[(Wtsd, wtsd_fof, 60)]);
        assert_below(format, "post.fit_or_fold", Wtsd, 60, 0.05, &[(FoldToCbetFlop, fcb_fof, 40)]);
    }
}

#[test]
fn wwsf_low_needs_showdown_frequency_at_or_under_prior() {
    // A station wins few pots after the flop because he calls down and
    // loses, not because he gives up: "stab and barrel him" loses money
    // against him. The low-WWSF read needs WTSD at or under its prior.
    let c = FormatKey::Cash;
    let station = cash(&postflop_firing("post.station").into_iter().chain([(Wwsf, 15, 60)]).collect::<Vec<_>>());
    assert!(fires(&station, "post.station"));
    assert!(!fires(&station, "post.wwsf_low"), "{:?}", ids(&evaluate_rules(&station)));
    // Boundary on WTSD: at the last count at or under the prior it fires,
    // one count above it does not; without a WTSD sample it never fires.
    let wtsd_at = last_below(c, Wtsd, 60, 0.0);
    assert!(fires(&cash(&[(Wwsf, 15, 60), (Wtsd, wtsd_at, 60)]), "post.wwsf_low"));
    assert!(!fires(&cash(&[(Wwsf, 15, 60), (Wtsd, wtsd_at + 1, 60)]), "post.wwsf_low"));
    assert!(!fires(&cash(&[(Wwsf, 15, 60)]), "post.wwsf_low"));
    assert!(!fires(&cash(&[(Wwsf, 15, 60), (Wtsd, 5, 9)]), "post.wwsf_low"), "WTSD under its floor");
}

/// Postflop reads whose advice points opposite ways (section 17.6).
const POSTFLOP_PAIRS: [(&str, &str); 6] = [
    ("post.wwsf_high", "post.fold_cbet_turn.high"),
    ("post.wwsf_high", "post.fold_cbet_river.high"),
    ("post.river_bet.low", "post.cbet_river.high"),
    ("post.river_bet.high", "post.cbet_river.low"),
    ("post.station", "post.river_raise.high"),
    ("post.fit_or_fold", "post.fold_cbet_turn.high"),
];

#[test]
fn opposite_postflop_reads_never_fire_together() {
    // "Bluff him less on turns and rivers" next to "Barrel the turn" or
    // "Fire third barrels"; "Fold one pair to his river bets" next to
    // "Bluff-catch his river barrels"; "Value bet middle pair on the river"
    // next to "Check second pair on the river".
    assert_eq!(POSTFLOP_OPPOSITES, POSTFLOP_PAIRS);
    for (a, b) in POSTFLOP_PAIRS {
        let stats: Vec<_> = postflop_firing(a).into_iter().chain(postflop_firing(b)).collect();
        let alone = |id: &str| {
            let reads = evaluate_rules(&cash(&postflop_firing(id)));
            read(&reads, id).unwrap_or_else(|| panic!("{id} fires alone")).score
        };
        let (sa, sb) = (alone(a), alone(b));
        let reads = evaluate_rules(&cash(&stats));
        let (ra, rb) = (read(&reads, a).is_some(), read(&reads, b).is_some());
        assert!(ra != rb, "{a} and {b} fired together (or neither): {:?}", ids(&reads));
        let kept = if ra { a } else { b };
        assert_eq!(kept, if sa >= sb { a } else { b }, "the higher score stays ({sa:.3} vs {sb:.3})");
    }
}

#[test]
fn reads_from_different_postflop_stats_give_advice_that_fits_together() {
    // A station: the preflop profile already says "Value bet top pair on all
    // three streets; cut your river bluffs." The postflop station and river
    // non-folder reads must add a lever (thinner river value, sizing), not
    // repeat "stop bluffing".
    let fish = cash(&[(Vpip, 30, 60), (Pfr, 5, 60), (Wtsd, 30, 60), (Wsd, 10, 40), (FoldToCbetRiver, 2, 30)]);
    let reads = evaluate_rules(&fish);
    let profile = read(&reads, "pf.profile.station").expect("profile station fires").advice.clone();
    for id in ["post.station", "post.fold_cbet_river.low"] {
        let advice = &read(&reads, id).unwrap_or_else(|| panic!("{id} fires: {:?}", ids(&reads))).advice;
        assert!(advice.contains("river"), "{id} names the street: {advice}");
        assert!(!advice.to_lowercase().contains("bluff"), "{id} repeats the profile's bluff note: {advice}");
        assert!(!advice.contains("top pair"), "{id} repeats the profile's top-pair note ({profile}): {advice}");
    }
    // A nit postflop: rarely bets the river and wins most showdowns he
    // reaches. Only one of the two notes may be about calling his river
    // bets; the other says what the hero bets for value.
    let nit = cash(&[(RiverBet, 2, 40), (Wsd, 30, 40), (Wtsd, 10, 60)]);
    let reads = evaluate_rules(&nit);
    let about_his_bets: Vec<&str> = ["post.river_bet.low", "post.wsd_strong"]
        .into_iter()
        .filter(|id| read(&reads, id).unwrap_or_else(|| panic!("{id} fires: {:?}", ids(&reads))).advice.contains("his river bets"))
        .collect();
    assert_eq!(about_his_bets, ["post.river_bet.low"], "{:?}", reads.iter().map(|r| &r.advice).collect::<Vec<_>>());
}

#[test]
fn postflop_advice_names_street_and_lever() {
    // Section 17.6: every postflop note names the street it is about and a
    // size or hand class. The guard rejects the texts the audit replaced.
    let street = regex::Regex::new(r"(?i)\b(?:flops?|turns?|rivers?)\b").unwrap();
    for old in [
        "Double-barrel him more.",
        "Raise his donk bets more.",
        "Bluff him less postflop.",
        "C-bet him often with small sizes.",
        "Value bet thin and stop bluffing him.",
    ] {
        assert!(!street.is_match(old), "the street guard must reject: {old}");
    }
    for old in ["Give his river barrels credit.", "Fold more to his river bets.", "Check-raise more turns against him."] {
        assert!(concrete_levers(old).is_empty(), "the lever guard must reject: {old}");
    }
    for def in RULES.iter().filter(|r| r.family == Family::Postflop) {
        for text in std::iter::once(def.advice).chain(def.variants.iter().map(|(_, t)| *t)) {
            assert!(street.is_match(text), "{} names no street: {text}", def.id);
            assert!(!concrete_levers(text).is_empty(), "{} names no size or hand class: {text}", def.id);
        }
    }
}

// ------------------------------------- postflop gaps (F15–F17, section 17.9)

#[test]
fn river_sizing_rules_fire_at_threshold_and_not_below() {
    // Shares of his river bets by size (section 9.2). Cash: polar prior .12,
    // k 10: (h + 1.2)/40 ≥ .24 → 9 of 30; small prior .20: (h + 2)/40 ≥ .35
    // → 12 of 30. Every format: postflop_thresholds_hold_in_every_format.
    let c = FormatKey::Cash;
    assert_eq!(assert_above(c, "post.river_sizing.polar", RiverBetPolar, 30, 0.12, &[]), 9);
    assert_eq!(assert_above(c, "post.river_sizing.small", RiverBetSmall, 30, 0.15, &[]), 12);
    let reads = evaluate_rules(&cash(&[(RiverBetPolar, 9, 30)]));
    let r = read(&reads, "post.river_sizing.polar").unwrap();
    assert_eq!(r.observation, "Sizes 30% (9/30) of his river bets as overbets or all-ins.");
    assert_eq!((r.tag.as_deref(), r.scenario_id.as_str(), r.family), (Some("OVB"), "F15", Family::Postflop));
    let reads = evaluate_rules(&cash(&[(RiverBetSmall, 13, 30)]));
    let r = read(&reads, "post.river_sizing.small").unwrap();
    assert_eq!(r.observation, "Sizes 43% (13/30) of his river bets under 40% of the pot.");
    assert_eq!((r.tag.as_deref(), r.scenario_id.as_str()), (Some("SRB"), "F15"));
    // A villain who mixes both sizes gets both notes: they price different
    // bets, so neither drops the other.
    let both = evaluate_rules(&cash(&[(RiverBetPolar, 12, 30), (RiverBetSmall, 15, 30)]));
    assert!(read(&both, "post.river_sizing.polar").is_some() && read(&both, "post.river_sizing.small").is_some());
    // Small-sample extreme: 7 of 7 is under the 8-bet floor; 8 of 8 fires.
    assert_eq!(stat_spec(RiverBetPolar).n_min, 8);
    assert!(evaluate_rules(&cash(&[(RiverBetPolar, 7, 7), (RiverBetSmall, 7, 7)])).is_empty());
    assert!(fires(&cash(&[(RiverBetPolar, 8, 8)]), "post.river_sizing.polar"));
}

#[test]
fn fold_to_river_raise_rule_fires_at_threshold_and_not_below() {
    // Prior .45, k 8: (h + 3.6)/28 ≥ .60 → 14 of 20.
    let c = FormatKey::Cash;
    assert_eq!(assert_above(c, "post.fold_river_raise.high", FoldToRiverRaise, 20, 0.15, &[]), 14);
    let reads = evaluate_rules(&cash(&[(FoldToRiverRaise, 14, 20)]));
    let r = read(&reads, "post.fold_river_raise.high").unwrap();
    assert_eq!(r.observation, "Folds 70% (14/20) of his river bets when raised.");
    assert_eq!((r.tag.as_deref(), r.scenario_id.as_str()), (Some("FRR"), "F16"));
    // Floor 5: 4 of 4 never fires; 5 of 5 is (5 + 3.6)/13 = .66 and fires.
    assert_eq!(stat_spec(FoldToRiverRaise).n_min, 5);
    assert!(!fires(&cash(&[(FoldToRiverRaise, 4, 4)]), "post.fold_river_raise.high"));
    assert!(fires(&cash(&[(FoldToRiverRaise, 5, 5)]), "post.fold_river_raise.high"));
}

#[test]
fn fit_or_fold_needs_both_legs_and_replaces_the_flop_overfold_read() {
    let reads = evaluate_rules(&cash(&postflop_firing("post.fit_or_fold")));
    let r = read(&reads, "post.fit_or_fold").unwrap();
    assert_eq!(r.observation, "Folds to 75% (30/40) of flop c-bets and reaches showdown only 17%.");
    assert_eq!((r.tag.as_deref(), r.scenario_id.as_str()), (Some("FOF"), "F17"));
    // As sure as the weaker leg: 40 c-bets faced (k 15) against 60 flops
    // seen (k 20).
    let fcb = stat(FormatKey::Cash, FoldToCbetFlop, 30, 40);
    let wtsd = stat(FormatKey::Cash, Wtsd, 10, 60);
    assert!(fcb.confidence < wtsd.confidence);
    assert_eq!(r.confidence, fcb.confidence);
    assert_eq!(r.evidence.len(), 2);
    // The general flop overfold read on the same stat yields to it.
    assert!(read(&reads, "post.fold_cbet_flop.high").is_none(), "{:?}", ids(&reads));
    // Either leg alone is no fit-or-fold: a flop overfolder who reaches
    // showdown at the prior keeps the general read; a low WTSD alone fires
    // nothing about the flop.
    let at_prior = cash(&[(FoldToCbetFlop, 30, 40), (Wtsd, 16, 60)]);
    assert!(!fires(&at_prior, "post.fit_or_fold"));
    assert!(fires(&at_prior, "post.fold_cbet_flop.high"));
    assert!(!fires(&cash(&[(FoldToCbetFlop, 30, 40)]), "post.fit_or_fold"));
    assert!(!fires(&cash(&[(Wtsd, 10, 60)]), "post.fit_or_fold"));
    // WTSD under its 10-hand floor is no leg.
    assert!(!fires(&cash(&[(FoldToCbetFlop, 30, 40), (Wtsd, 0, 9)]), "post.fit_or_fold"));
}

#[test]
fn new_postflop_reads_are_suppressed_at_push_fold() {
    // Rows F15–F17 are postflop reads: at 15bb or less in a tournament only
    // push/fold reads show (section 5).
    let ids_new = ["post.river_sizing.polar", "post.river_sizing.small", "post.fold_river_raise.high", "post.fit_or_fold"];
    let stats = [
        (RiverBetPolar, 16, 30),
        (RiverBetSmall, 14, 30),
        (FoldToRiverRaise, 14, 20),
        (FoldToCbetFlop, 30, 40),
        (Wtsd, 10, 60),
    ];
    for format in [FormatKey::Mtt, FormatKey::Spin] {
        let mut deep = input(format, &stats);
        deep.context = Some(context(format, 40.0));
        let reads = evaluate_rules(&deep);
        for id in ids_new {
            assert!(read(&reads, id).is_some(), "{format:?}: {id} fires at 40bb: {:?}", ids(&reads));
        }
        let mut short = input(format, &stats);
        short.context = Some(context(format, 12.0));
        let reads = evaluate_rules(&short);
        assert!(reads.iter().all(|r| r.family != Family::Postflop), "{format:?} at 12bb: {:?}", ids(&reads));
    }
}

/// A postflop read and the general read it refines (rows F15, F17). The
/// refining read always stays.
const REFINED_PAIRS: [(&str, &str); 5] = [
    ("post.river_sizing.polar", "post.river_bet.high"),
    ("post.river_sizing.polar", "post.cbet_river.high"),
    ("post.river_sizing.small", "post.river_bet.low"),
    ("post.river_sizing.small", "post.cbet_river.low"),
    ("post.fit_or_fold", "post.fold_cbet_flop.high"),
];

#[test]
fn sizing_and_fit_or_fold_reads_replace_the_general_read() {
    // "Call his river bets with second pair+" next to "fold second pair to
    // his overbets"; "Fold one pair to his river bets" next to "call any
    // pair" against his small ones. The size decides the price, so the
    // sizing read wins whatever the scores.
    assert_eq!(REFINED_OPPOSITES, REFINED_PAIRS);
    for (specific, general) in REFINED_PAIRS {
        assert!(fires(&cash(&postflop_firing(general)), general), "{general} fires alone");
        let stats: Vec<_> = postflop_firing(specific).into_iter().chain(postflop_firing(general)).collect();
        let reads = evaluate_rules(&cash(&stats));
        assert!(read(&reads, specific).is_some(), "{specific} stays: {:?}", ids(&reads));
        assert!(read(&reads, general).is_none(), "{general} fired next to {specific}: {:?}", ids(&reads));
    }
}

// ----------------------------------------------------- head-to-head (H01–H03)

fn h2h(key: H2hKey, hits: u32, n: u32, villain: f64) -> H2hStat {
    let k = stat_spec(key.basis().0[0]).k;
    H2hStat {
        key,
        hits,
        opportunities: n,
        raw: (n > 0).then(|| f64::from(hits) / f64::from(n)),
        villain_shrunk: villain,
        shrunk: shrink(f64::from(hits), f64::from(n), k, villain),
    }
}

fn h2h_input(stat: H2hStat) -> RuleInput {
    let mut input = cash(&[]);
    input.h2h.push(stat);
    input
}

#[test]
fn h2h_rules_fire_only_at_rule_sample() {
    // 3-bets the hero's opens: k 25, toward his own .08. n = 10:
    // (4 + 2)/35 = .171 ≥ .13 fires; (3 + 2)/35 = .143 fires too; (2 + 2)/35
    // = .114 does not — 20% raw against the hero is still shrunk under.
    let id = "h2h.3bet_vs_hero.high";
    assert!(fires(&h2h_input(h2h(H2hKey::ThreeBetVsHeroOpen, 3, 10, 0.08)), id));
    assert!(!fires(&h2h_input(h2h(H2hKey::ThreeBetVsHeroOpen, 2, 10, 0.08)), id));
    // Nine opportunities: never, however extreme.
    assert!(!fires(&h2h_input(h2h(H2hKey::ThreeBetVsHeroOpen, 9, 9, 0.08)), id));

    let reads = evaluate_rules(&h2h_input(h2h(H2hKey::ThreeBetVsHeroOpen, 4, 11, 0.08)));
    let r = read(&reads, id).unwrap();
    assert_eq!(r.observation, "3-bets your opens 36% (4/11).");
    assert_eq!(r.family, Family::H2h);
    assert_eq!(r.tag.as_deref(), Some("H2H"));
    assert_eq!(r.evidence[0].stat_name, "three_bet_vs_hero_open");
    // Confidence on the head-to-head count: 11/(11 + 25) = 31%.
    assert_eq!(r.confidence_pct, Some(31));
    // Head-to-head reads rank ×1.25.
    assert!((r.score - r.deviation * r.confidence * 1.25).abs() < 1e-12);

    for (key, id, delta) in [
        (H2hKey::FoldToHeroThreeBet, "h2h.fold_to_hero_3bet.high", 0.12),
        (H2hKey::FoldToHeroCbet, "h2h.fold_to_hero_cbet.high", 0.12),
        (H2hKey::StealVsHero, "h2h.steal_vs_hero.high", 0.12),
        (H2hKey::DefendVsHeroSteal, "h2h.defend_vs_hero_steal.high", 0.15),
    ] {
        let villain = 0.40;
        let n = 20;
        let k = stat_spec(key.basis().0[0]).k;
        let h = (0..=n)
            .find(|h| shrink(f64::from(*h), f64::from(n), k, villain) >= villain + delta - 1e-9)
            .unwrap();
        assert!(fires(&h2h_input(h2h(key, h, n, villain)), id), "{id} at {h}/{n}");
        assert!(!fires(&h2h_input(h2h(key, h - 1, n, villain)), id), "{id} at {}/{n}", h - 1);
        // Same rate on nine opportunities: below the rule sample.
        assert!(!fires(&h2h_input(h2h(key, 9, 9, villain)), id), "{id} at 9/9");
    }
}

// ------------------------------------------- showdown-backed tells (M04, M05)

fn tell(bucket: SizeBucket, value: u32, bluff: u32, neither: u32) -> SizingTell {
    SizingTell { bucket, value, bluff, neither, n: value + bluff + neither }
}

#[test]
fn sizing_tell_rules_need_four_and_cite_counts() {
    let mut v = cash(&[]);
    v.sizing = vec![tell(SizeBucket::Overbet, 3, 0, 1)];
    let reads = evaluate_rules(&v);
    let r = read(&reads, "sd.tell.big_is_value").expect("3 of 4 overbets shown as value");
    assert_eq!(r.observation, "Showed value in 3 of 4 overbet-sized bets.");
    assert_eq!(r.advice, "Fold weak top pair and worse to his overbet-sized bets; call with two pair or better.");
    assert_eq!(r.evidence[0].hits, 3);
    assert_eq!(r.evidence[0].opportunities, 4);
    assert_eq!(r.evidence[0].shrunk, None);
    // 4/(4 + 4) = 50%.
    assert_eq!(r.confidence_pct, Some(50));
    // Three shown, all value: under the rule sample.
    v.sizing = vec![tell(SizeBucket::Overbet, 3, 0, 0)];
    assert!(evaluate_rules(&v).is_empty());
    // 2 of 4 value is under 75%.
    v.sizing = vec![tell(SizeBucket::Large, 2, 1, 1)];
    assert!(!fires(&v, "sd.tell.big_is_value"));

    v.sizing = vec![tell(SizeBucket::AllIn, 1, 2, 1)];
    let reads = evaluate_rules(&v);
    assert_eq!(read(&reads, "sd.tell.big_is_bluff").unwrap().observation, "Was bluffing in 2 of 4 shown all-in bets.");
    v.sizing = vec![tell(SizeBucket::AllIn, 2, 1, 1)];
    assert!(!fires(&v, "sd.tell.big_is_bluff"));

    v.sizing = vec![tell(SizeBucket::Small, 3, 1, 0), tell(SizeBucket::Large, 0, 0, 5)];
    let reads = evaluate_rules(&v);
    assert_eq!(read(&reads, "sd.tell.small_is_value").unwrap().observation, "Showed value in 3 of 4 small bets.");
    assert!(read(&reads, "sd.tell.big_is_value").is_none());
    // Small bets are never read as "big" tells.
    v.sizing = vec![tell(SizeBucket::Medium, 0, 4, 0)];
    assert!(!fires(&v, "sd.tell.big_is_bluff"));
}

#[test]
fn shows_bluffs_rule_cites_counts_and_needs_six() {
    let mut v = cash(&[]);
    v.river_showdowns = ShowdownTally { value: 3, bluff: 3, neither: 0, n: 6 };
    let reads = evaluate_rules(&v);
    let r = read(&reads, "sd.shows_bluffs").expect("3 of 6 river bets were bluffs");
    assert_eq!(r.observation, "Was bluffing in 3 of 6 river bets that reached showdown.");
    assert_eq!(r.tag.as_deref(), Some("SVB"));
    // Five, all bluffs: under the sample.
    v.river_showdowns = ShowdownTally { value: 0, bluff: 5, neither: 0, n: 5 };
    assert!(!fires(&v, "sd.shows_bluffs"));
    // 3 of 8 is under 40%.
    v.river_showdowns = ShowdownTally { value: 5, bluff: 3, neither: 0, n: 8 };
    assert!(!fires(&v, "sd.shows_bluffs"));
    // 4 of 10 is exactly 40%.
    v.river_showdowns = ShowdownTally { value: 6, bluff: 4, neither: 0, n: 10 };
    assert!(fires(&v, "sd.shows_bluffs"));
}

// ------------------------------------------------------- recent form (R02)

fn form(window: u32, hits: u32, baseline: u32, flag: Option<FormFlag>, z: f64) -> RecentForm {
    RecentForm {
        window,
        window_hits: hits,
        baseline_opportunities: baseline,
        window_vpip_pct: f64::from(hits) / f64::from(window) * 100.0,
        baseline_vpip_pct: 25.0,
        z,
        flag,
        after_big_loss: flag == Some(FormFlag::Tilt),
    }
}

#[test]
fn recent_form_rules_fire_only_with_full_samples() {
    let mut v = cash(&[]);
    v.recent_form = Some(form(12, 9, 49, Some(FormFlag::Tilt), 3.33));
    let reads = evaluate_rules(&v);
    let tilt = read(&reads, "rec.tilt").expect("tilt read");
    assert_eq!(tilt.observation, "Played 9 of his last 12 hands after a big lost pot (usual 25%).");
    assert_eq!(tilt.tag.as_deref(), Some("TILT"));
    assert!((tilt.score - 3.33).abs() < 1e-9, "tilt score is min(|z|, 5)");

    v.recent_form = Some(form(12, 8, 49, Some(FormFlag::Looser), 2.67));
    let looser = evaluate_rules(&v);
    assert!((read(&looser, "rec.looser").unwrap().score - 2.67 * 0.8).abs() < 1e-9);
    v.recent_form = Some(form(12, 0, 60, Some(FormFlag::Tighter), -2.9));
    assert_eq!(
        read(&evaluate_rules(&v), "rec.tighter").unwrap().observation,
        "Played only 0 of his last 12 hands (usual 25%)."
    );
    // Ordinary variance has no flag and no read.
    v.recent_form = Some(form(12, 5, 49, None, 1.3));
    assert!(evaluate_rules(&v).is_empty());
    // Window or baseline under the spec sample: nothing, even with a flag.
    v.recent_form = Some(form(9, 9, 49, Some(FormFlag::Tilt), 4.0));
    assert!(evaluate_rules(&v).is_empty());
    v.recent_form = Some(form(12, 9, 39, Some(FormFlag::Tilt), 4.0));
    assert!(evaluate_rules(&v).is_empty());
}

// ------------------- head-to-head, showdown and recency audit (section 17.7)

/// One input on which an `h2h.*`, `sd.*` or `rec.*` rule fires, in any
/// format: the head-to-head stats are shrunk toward the villain's own value,
/// the showdown and form reads carry their own samples.
fn h2h_sd_rec_firing(format: FormatKey, id: &str) -> RuleInput {
    let mut v = input(format, &[]);
    match id {
        // (6 + 25 × .08)/37 = .22 ≥ .08 + .05.
        "h2h.3bet_vs_hero.high" => v.h2h.push(h2h(H2hKey::ThreeBetVsHeroOpen, 6, 12, 0.08)),
        "h2h.fold_to_hero_3bet.high" => v.h2h.push(h2h(H2hKey::FoldToHeroThreeBet, 18, 20, 0.40)),
        "h2h.fold_to_hero_cbet.high" => v.h2h.push(h2h(H2hKey::FoldToHeroCbet, 18, 20, 0.40)),
        "h2h.steal_vs_hero.high" => v.h2h.push(h2h(H2hKey::StealVsHero, 18, 20, 0.40)),
        "h2h.defend_vs_hero_steal.high" => v.h2h.push(h2h(H2hKey::DefendVsHeroSteal, 18, 20, 0.40)),
        "sd.tell.big_is_value" => v.sizing = vec![tell(SizeBucket::Overbet, 4, 0, 0)],
        "sd.tell.big_is_bluff" => v.sizing = vec![tell(SizeBucket::AllIn, 1, 3, 0)],
        "sd.tell.small_is_value" => v.sizing = vec![tell(SizeBucket::Small, 4, 0, 0)],
        "sd.shows_bluffs" => v.river_showdowns = ShowdownTally { value: 3, bluff: 3, neither: 0, n: 6 },
        "rec.tilt" => v.recent_form = Some(form(12, 9, 49, Some(FormFlag::Tilt), 3.3)),
        "rec.looser" => v.recent_form = Some(form(12, 8, 49, Some(FormFlag::Looser), 2.67)),
        "rec.tighter" => v.recent_form = Some(form(12, 0, 60, Some(FormFlag::Tighter), -2.9)),
        other => panic!("no firing input for {other}"),
    }
    v
}

/// Every head-to-head, showdown and recency advice as shown after the audit
/// (section 17.7), in three contexts: cash at 100bb, MTT at 30bb (jam depth)
/// and MTT at 12bb (push/fold, where head-to-head reads are not evaluated:
/// an empty text).
const H2H_SD_REC_ADVICE: [(&str, [&str; 3]); 12] = [
    ("h2h.3bet_vs_hero.high", [
        "Cut KTo-type opens in front of him; 4-bet his 3-bets with QQ+, AK and A5s-A4s.",
        "Jam 99+, AJ+ and KQs over his 3-bets of your opens.",
        "",
    ]),
    ("h2h.fold_to_hero_3bet.high", [
        "3-bet his opens light at 3x in position: A2s-A5s, K9s+ and 65s+.",
        "Reshove A2s+, K9s+, 55+ and QJs over his opens.",
        "",
    ]),
    ("h2h.fold_to_hero_cbet.high", [
        "C-bet him with any two at 1/3 pot on the flop, wet boards included.",
        "C-bet him with any two at 1/3 pot on the flop, wet boards included.",
        "",
    ]),
    ("h2h.steal_vs_hero.high", [
        "3-bet his steals from your blinds with 77+, A8s+, KTs+, ATo+ and A5s.",
        "From your blinds, reshove 44+, A5+, K9s+ and QJs over his steals.",
        "",
    ]),
    ("h2h.defend_vs_hero_steal.high", [
        "Steal into him only with pairs, suited aces and broadways; value bet top pair.",
        "Steal into him only with pairs, suited aces and broadways; value bet top pair.",
        "",
    ]),
    ("sd.tell.big_is_value", [
        "Fold weak top pair and worse to his overbet-sized bets; call with two pair or better.",
        "Fold weak top pair and worse to his overbet-sized bets; call with two pair or better.",
        "Fold weak top pair and worse to his overbet-sized bets; call with two pair or better.",
    ]),
    ("sd.tell.big_is_bluff", [
        "Call his all-in bets down with second pair or better; never fold top pair.",
        "Call his all-in bets down with second pair or better; never fold top pair.",
        "Call his all-in bets down with second pair or better; never fold top pair.",
    ]),
    ("sd.tell.small_is_value", [
        "Fold second pair or worse to his small bets; raise two pair or better.",
        "Fold second pair or worse to his small bets; raise two pair or better.",
        "Fold second pair or worse to his small bets; raise two pair or better.",
    ]),
    ("sd.shows_bluffs", [
        "Call his river bets with any pair; check top pair to him to let him bluff.",
        "Call his river bets with any pair; check top pair to him to let him bluff.",
        "Call his river bets with any pair; check top pair to him to let him bluff.",
    ]),
    ("rec.tilt", [
        "While this lasts, value bet top pair three streets; call his bets with second pair.",
        "While this lasts, value bet top pair three streets; call his bets with second pair.",
        "While this lasts, call his shoves with 22+, A7+ and KTs+; shove for value only.",
    ]),
    ("rec.looser", [
        "While this lasts, 3-bet his opens with 99+, AJ+ and KQ; value bet top pair thin.",
        "While this lasts, reshove 88+, AT+ and KQ over his opens.",
        "While this lasts, call his shoves with 33+, A8+ and KTs+.",
    ]),
    ("rec.tighter", [
        "While this lasts, fold AJo and KQo to his opens; steal his blinds with any suited.",
        "While this lasts, fold AJo and KQo to his opens; steal his blinds with any suited.",
        "While this lasts, call his shoves only with 88+ and AJ+; shove any ace into him.",
    ]),
];

fn h2h_sd_rec_contexts() -> [(FormatKey, EngineContext); 3] {
    [
        (FormatKey::Cash, context(FormatKey::Cash, 100.0)),
        (FormatKey::Mtt, context(FormatKey::Mtt, 30.0)),
        (FormatKey::Mtt, context(FormatKey::Mtt, 12.0)),
    ]
}

#[test]
fn h2h_showdown_and_recency_advice_texts_are_pinned() {
    let audited: Vec<&str> = RULES
        .iter()
        .filter(|r| matches!(r.family, Family::H2h | Family::Showdown | Family::Recency))
        .map(|r| r.id)
        .collect();
    assert_eq!(audited, H2H_SD_REC_ADVICE.map(|(id, _)| id).to_vec(), "every audited rule is pinned");
    for (id, texts) in H2H_SD_REC_ADVICE {
        let mut variants_seen = std::collections::BTreeSet::new();
        for ((format, ctx), want) in h2h_sd_rec_contexts().into_iter().zip(texts) {
            let mut v = h2h_sd_rec_firing(format, id);
            v.context = Some(ctx.clone());
            let reads = evaluate_rules(&v);
            match read(&reads, id) {
                Some(r) => {
                    assert_eq!(r.advice, want, "{id} at {:.0}bb {format:?}", ctx.effective_stack_bb.unwrap());
                    variants_seen.insert(r.advice.clone());
                }
                None => assert_eq!(want, "", "{id} must fire at {:.0}bb {format:?}", ctx.effective_stack_bb.unwrap()),
            }
        }
        // Every variant text is reachable from one of the three contexts.
        let def = rule_def(id);
        for (_, text) in def.variants {
            let rendered = text.replace("{bucket}", if id == "sd.tell.big_is_bluff" { "all-in" } else { "overbet-sized" });
            assert!(variants_seen.contains(&rendered), "{id} variant not reached: {text}");
        }
    }
}

#[test]
fn recent_form_advice_is_scoped_to_the_window() {
    // Section 17.7: a recent-form read holds while the window lasts, so
    // every text says so and names what to change (hand classes or sizes).
    for def in RULES.iter().filter(|r| r.family == Family::Recency) {
        for text in std::iter::once(def.advice).chain(def.variants.iter().map(|(_, t)| *t)) {
            assert!(text.starts_with("While this lasts, "), "{} is not scoped to the window: {text}", def.id);
            assert!(!concrete_levers(text).is_empty(), "{} names no lever: {text}", def.id);
        }
    }
    // The old texts named no lever ("wider", "more credit").
    for old in [
        "Value bet wider and give his aggression less credit while this lasts.",
        "Isolate and value bet him wider while this lasts.",
        "Give his recent entries more credit.",
    ] {
        assert!(concrete_levers(old).is_empty(), "{old}: {:?}", concrete_levers(old));
    }
}

/// Head-to-head reads and the general read on another stat that points the
/// opposite way (section 17.7). The head-to-head read always stays.
const H2H_PAIRS: [(&str, &str); 5] = [
    ("h2h.3bet_vs_hero.high", "pf.3bet.low"),
    ("h2h.fold_to_hero_3bet.high", "pf.fold_to_3bet.low"),
    ("h2h.fold_to_hero_cbet.high", "post.fold_cbet_flop.low"),
    ("h2h.steal_vs_hero.high", "pf.steal.low"),
    ("h2h.defend_vs_hero_steal.high", "pf.fold_to_steal.high"),
];

fn general_firing(id: &str) -> Vec<(StatKey, u32, u32)> {
    if id.starts_with("post.") { postflop_firing(id) } else { preflop_firing(id) }
}

#[test]
fn opposite_h2h_reads_keep_the_head_to_head_read() {
    // "Fold JJ and AQ to his 3-bets" next to "4-bet his 3-bets with QQ+"
    // when he 3-bets the hero's opens far more than anyone else's; "C-bet
    // flops at 2/3 pot with top pair+ … check back overcards" next to
    // "C-bet him with any two at 1/3 pot" when he folds to the hero's
    // c-bets. The head-to-head stat is already shrunk toward his overall
    // value, so when it still fires it is the evidence about the hero's own
    // spots and it stays, whatever the scores.
    assert_eq!(H2H_OPPOSITES, H2H_PAIRS);
    let mut general_scored_higher = 0;
    for (h2h_id, general) in H2H_PAIRS {
        let alone_h2h = evaluate_rules(&h2h_sd_rec_firing(FormatKey::Cash, h2h_id));
        let alone_general = evaluate_rules(&cash(&general_firing(general)));
        let sh = read(&alone_h2h, h2h_id).unwrap_or_else(|| panic!("{h2h_id} fires alone")).score;
        let sg = read(&alone_general, general).unwrap_or_else(|| panic!("{general} fires alone")).score;
        if sg > sh {
            general_scored_higher += 1;
        }
        let mut both = h2h_sd_rec_firing(FormatKey::Cash, h2h_id);
        for (key, hits, n) in general_firing(general) {
            both.stats.insert(key, stat(FormatKey::Cash, key, hits, n));
        }
        let reads = evaluate_rules(&both);
        assert!(read(&reads, h2h_id).is_some(), "{h2h_id} stays: {:?}", ids(&reads));
        assert!(read(&reads, general).is_none(), "{general} fired next to {h2h_id}: {:?}", ids(&reads));
    }
    assert!(general_scored_higher > 0, "at least one pair proves the head-to-head read wins on evidence, not score");
}

/// Showdown reads and the frequency read that points the opposite way
/// (section 17.7). The showdown read always stays.
const SHOWDOWN_PAIRS: [(&str, &str); 6] = [
    ("sd.shows_bluffs", "post.river_bet.low"),
    ("sd.shows_bluffs", "post.cbet_river.low"),
    ("sd.shows_bluffs", "post.river_sizing.polar"),
    ("sd.tell.big_is_value", "post.river_sizing.polar"),
    ("sd.tell.big_is_bluff", "post.river_sizing.polar"),
    ("sd.tell.small_is_value", "post.river_sizing.small"),
];

#[test]
fn shown_river_bluffs_overrule_river_frequency_reads() {
    // "Fold one pair to his river bets" infers strength from a low betting
    // frequency, which only proves a narrow range (section 8.1); 3 bluffs in
    // 6 river bets that reached showdown are counts of what he bet. The
    // shown bluffs win, whatever the scores.
    assert_eq!(SHOWDOWN_OPPOSITES, SHOWDOWN_PAIRS);
    for (sd, frequency) in SHOWDOWN_PAIRS {
        let mut v = h2h_sd_rec_firing(FormatKey::Cash, sd);
        for (key, hits, n) in postflop_firing(frequency) {
            v.stats.insert(key, stat(FormatKey::Cash, key, hits, n));
        }
        let alone = evaluate_rules(&cash(&postflop_firing(frequency)));
        assert!(read(&alone, frequency).is_some(), "{frequency} fires alone");
        let reads = evaluate_rules(&v);
        assert!(read(&reads, sd).is_some(), "{sd} stays: {:?}", ids(&reads));
        assert!(read(&reads, frequency).is_none(), "{frequency} fired next to {sd}: {:?}", ids(&reads));
    }
}

#[test]
fn head_to_head_and_general_reads_on_one_villain_fit_together() {
    // The same leak against everyone and, more strongly, against the hero:
    // both reads fire, and the head-to-head note adds a lever instead of
    // repeating the general one (17.7), at 100bb and at jam depth.
    for (h2h_id, general) in [
        ("h2h.3bet_vs_hero.high", "pf.3bet.high_ip"),
        ("h2h.fold_to_hero_3bet.high", "pf.fold_to_3bet.high"),
        ("h2h.fold_to_hero_cbet.high", "post.fold_cbet_flop.high"),
        ("h2h.steal_vs_hero.high", "pf.steal.high"),
        ("h2h.defend_vs_hero_steal.high", "pf.fold_to_steal.low"),
    ] {
        for eff in [100.0, 30.0] {
            let mut v = h2h_sd_rec_firing(FormatKey::Cash, h2h_id);
            for (key, hits, n) in general_firing(general) {
                v.stats.insert(key, stat(FormatKey::Cash, key, hits, n));
            }
            v.context = Some(context(FormatKey::Cash, eff));
            let reads = evaluate_rules(&v);
            let a = &read(&reads, h2h_id).unwrap_or_else(|| panic!("{h2h_id} fires at {eff}bb")).advice;
            let b = &read(&reads, general).unwrap_or_else(|| panic!("{general} fires at {eff}bb")).advice;
            assert_ne!(a, b, "{h2h_id} repeats {general} at {eff}bb");
        }
    }
    // A villain who folds to the hero's 3-bets and 4-bets light: "fold your
    // light 3-bets" to his 4-bet fits the head-to-head plan (3-bet light,
    // sized small) instead of contradicting it.
    let mut v = h2h_sd_rec_firing(FormatKey::Cash, "h2h.fold_to_hero_3bet.high");
    v.stats.insert(FourBet, stat(FormatKey::Cash, FourBet, 15, 40));
    let reads = evaluate_rules(&v);
    assert!(read(&reads, "pf.4bet.high").is_some() && read(&reads, "h2h.fold_to_hero_3bet.high").is_some());
    assert!(!read(&reads, "h2h.fold_to_hero_3bet.high").unwrap().advice.contains("4-bet"));
}

#[test]
fn showdown_tells_keep_their_counts_and_size_scope() {
    // Section 17.7: a showdown read is the only one allowed to speak about
    // value or bluffs, so its observation keeps citing the counts, and every
    // tell advice is scoped to the size (or street) the counts are about.
    for (id, bucket) in [
        ("sd.tell.big_is_value", "overbet-sized"),
        ("sd.tell.big_is_bluff", "all-in"),
        ("sd.tell.small_is_value", "small"),
        ("sd.shows_bluffs", "river"),
    ] {
        let reads = evaluate_rules(&h2h_sd_rec_firing(FormatKey::Cash, id));
        let r = read(&reads, id).unwrap_or_else(|| panic!("{id} fires"));
        assert!(r.observation.contains(" of "), "{id} observation cites counts: {}", r.observation);
        assert!(r.observation.contains(bucket) && r.advice.contains(bucket), "{id} names {bucket}: {} / {}", r.observation, r.advice);
        let def = rule_def(id);
        if id.starts_with("sd.tell.") {
            assert!(def.advice.contains("{bucket}"), "{id} advice is scoped to the bucket");
        }
        assert!(!concrete_levers(def.advice).is_empty(), "{id} names a hand class: {}", def.advice);
    }
}

#[test]
fn rule_quality_matrix_lists_every_rule_once() {
    // Section 17.4 has one row per rule id in RULES, plus the effective-stack
    // derivation behind the stack tag. docs/ is local-only, so a checkout
    // without it skips (check-engine-spec checks the same).
    let spec_path = concat!(env!("CARGO_MANIFEST_DIR"), "/../docs/specs/opponent-engine.md");
    let Ok(spec) = std::fs::read_to_string(spec_path) else {
        eprintln!("skipping the matrix check: {spec_path} is not present");
        return;
    };
    let start = spec.find("### 17.4").expect("section 17.4");
    let end = start + spec[start..].find("\n### 17.5").expect("section 17.5 after 17.4");
    let rows: Vec<&str> = spec[start..end]
        .lines()
        .filter_map(|l| l.strip_prefix("| `").and_then(|l| l.split('`').next()))
        .collect();
    for def in RULES {
        let count = rows.iter().filter(|r| **r == def.id).count();
        assert_eq!(count, 1, "{} has {count} rows in the 17.4 matrix", def.id);
    }
    for row in &rows {
        assert!(RULES.iter().any(|r| r.id == *row) || *row == "ctx.effective_stack", "17.4 row {row} is no rule");
    }
}

// ------------------------------------------- read sample (section 13, chip)

fn sample_of(r: &EngineRuleResult) -> Option<(u32, SampleUnit)> {
    r.sample.map(|s| (s.count, s.unit))
}

#[test]
fn every_read_carries_the_sample_its_confidence_rests_on() {
    // One stat: its own opportunities.
    let reads = evaluate_rules(&cash(&[(FoldTo3betIp, 24, 30)]));
    let r = read(&reads, "pf.fold_to_3bet.high").unwrap();
    assert_eq!(sample_of(r), Some((30, SampleUnit::Opportunities)));

    // VPIP-based profile reads count hands (one VPIP decision per hand).
    let reads = evaluate_rules(&cash(&[(Vpip, 6, 60), (Pfr, 5, 60)]));
    assert_eq!(sample_of(read(&reads, "pf.profile.nit").unwrap()), Some((60, SampleUnit::Hands)));

    // Several legs: the weakest one, the leg the confidence came from.
    // Station: VPIP 60 hands, PFR 60 hands, WTSD 40 opportunities.
    let station = cash(&[(Vpip, 27, 60), (Pfr, 3, 60), (Wtsd, 17, 40)]);
    let r = read(&evaluate_rules(&station), "pf.profile.station").cloned().unwrap();
    let legs = [stat(FormatKey::Cash, Vpip, 27, 60), stat(FormatKey::Cash, Pfr, 3, 60), stat(FormatKey::Cash, Wtsd, 17, 40)];
    let weakest = legs.iter().min_by(|a, b| a.confidence.total_cmp(&b.confidence)).unwrap();
    let unit = if matches!(weakest.key, Vpip | Pfr) { SampleUnit::Hands } else { SampleUnit::Opportunities };
    assert_eq!(sample_of(&r), Some((weakest.opportunities, unit)));
    assert_eq!(r.confidence_pct, Some(weakest.confidence_pct), "sample and confidence come from the same leg");

    // Head-to-head: the spot against the hero.
    let reads = evaluate_rules(&h2h_input(h2h(H2hKey::ThreeBetVsHeroOpen, 4, 11, 0.08)));
    let r = read(&reads, "h2h.3bet_vs_hero.high").unwrap();
    assert_eq!(sample_of(r), Some((11, SampleUnit::Opportunities)));

    // Recent form: the baseline hands its confidence uses.
    let mut v = cash(&[]);
    v.recent_form = Some(form(12, 9, 49, Some(FormFlag::Tilt), 3.33));
    assert_eq!(sample_of(read(&evaluate_rules(&v), "rec.tilt").unwrap()), Some((49, SampleUnit::Hands)));

    // Showdown tells: the hands shown down in that size.
    let mut v = cash(&[]);
    v.sizing = vec![tell(SizeBucket::Overbet, 3, 0, 1)];
    assert_eq!(sample_of(read(&evaluate_rules(&v), "sd.tell.big_is_value").unwrap()), Some((4, SampleUnit::Hands)));

    // A fact of the latest hand has no sample.
    let mut short = busy_villain(FormatKey::Mtt);
    short.context = Some(context(FormatKey::Mtt, 12.4));
    let reads = evaluate_rules(&short);
    assert_eq!(read(&reads, "ctx.short_stack").unwrap().sample, None);
    // Every other read has one.
    for r in reads.iter().filter(|r| r.family != Family::Context) {
        assert!(r.sample.is_some_and(|s| s.count > 0), "{} has no sample", r.rule_id);
    }
}

#[test]
fn read_sample_serializes_as_count_and_unit() {
    let reads = evaluate_rules(&cash(&[(FoldTo3betIp, 24, 30)]));
    let json = serde_json::to_value(read(&reads, "pf.fold_to_3bet.high").unwrap()).unwrap();
    assert_eq!(json["sample"], serde_json::json!({ "count": 30, "unit": "opportunities" }));
    let mut short = busy_villain(FormatKey::Mtt);
    short.context = Some(context(FormatKey::Mtt, 12.4));
    let reads = evaluate_rules(&short);
    let json = serde_json::to_value(read(&reads, "ctx.short_stack").unwrap()).unwrap();
    assert!(json["sample"].is_null());
    let r: ReadSample = ReadSample { count: 60, unit: SampleUnit::Hands };
    assert_eq!(serde_json::to_value(r).unwrap(), serde_json::json!({ "count": 60, "unit": "hands" }));
}

// --------------------------------------------- contract and text guards (G03)

#[test]
fn reads_keep_rule_result_contract_with_shrunk_evidence() {
    let reads = evaluate_rules(&cash(&[(FoldTo3betIp, 24, 30), (Pfr, 10, 60), (Vpip, 13, 60)]));
    let r = read(&reads, "pf.fold_to_3bet.high").unwrap();
    assert_eq!(r.category, RuleCategory::Exploit);
    assert_eq!(r.scenario_id, "P06");
    assert_eq!(r.family, Family::Preflop);
    assert_eq!(r.tag.as_deref(), Some("F3B"));
    // Confidence from fold-to-3-bet's own 30 opportunities, k 15: 30/45.
    assert_eq!(r.confidence_pct, Some(67));
    assert_eq!(r.confidence_tier, ConfidenceTier::High);
    let shrunk = (24.0 + 15.0 * 0.55) / 45.0;
    assert!((r.score - (shrunk - 0.55) / 0.12 * (30.0 / 45.0)).abs() < 1e-9);

    let legacy = r.to_rule_result();
    assert_eq!(legacy.rule_id, r.rule_id);
    assert_eq!(legacy.observation, r.observation);
    assert_eq!(legacy.advice, r.advice);
    assert_eq!(legacy.confidence_pct, r.confidence_pct);
    assert_eq!(legacy.evidence[0].stat_name, "fold_to_3bet_ip");
    assert_eq!(legacy.evidence[0].value, Some(80.0));
    assert_eq!(legacy.evidence[0].opportunities, 30);

    let json = serde_json::to_value(r).unwrap();
    for key in ["ruleId", "category", "observation", "advice", "confidencePct", "confidenceTier", "evidence", "scenarioId", "family", "tag", "score"] {
        assert!(json.get(key).is_some(), "missing {key}");
    }
    assert!(json.get("deviation").is_none());
    let evidence = &json["evidence"][0];
    assert_eq!(evidence["statName"], "fold_to_3bet_ip");
    assert_eq!(evidence["hits"], 24);
    assert_eq!(evidence["opportunities"], 30);
    assert_eq!(evidence["value"], 80.0);
    assert_eq!(evidence["shrunk"], ((shrunk * 1000.0_f64).round() / 10.0));

    // Below every floor, nothing at all: never a fabricated number.
    assert!(evaluate_rules(&RuleInput::default()).is_empty());
}

const FORBIDDEN_PHRASES: [&str; 14] = [
    "this hand", "this pot", "this street", "right now", "currently", "at the moment",
    "in the pot now", "on this street", "your hand", "you hold", "your cards", "next card",
    "facing this", "this spot",
];

const IMPERATIVE_VERBS: [&str; 34] = [
    "fold", "call", "raise", "bet", "check", "shove", "jam", "reshove", "3-bet", "4-bet", "c-bet",
    "squeeze", "steal", "bluff", "value-bet", "float", "probe", "barrel", "isolate", "iso-raise",
    "attack", "avoid", "respect", "widen", "tighten", "defend", "exploit", "target", "pressure",
    "consider", "try", "use", "don't", "do",
];

/// Lower-case words; a hyphen and an apostrophe belong to the word.
fn words(text: &str) -> Vec<String> {
    text.split(|c: char| !(c.is_alphanumeric() || c == '-' || c == '\''))
        .filter(|w| !w.is_empty())
        .map(|w| w.trim_matches(|c| c == '-' || c == '\'').to_lowercase())
        .collect()
}

#[test]
fn no_template_contains_forbidden_in_hand_phrasing() {
    let all = templates();
    assert!(all.len() >= 2 * RULES.len());
    assert!(all.iter().any(|(_, _, t)| t == SEAT_SUFFIX));
    for (id, kind, text) in &all {
        let lower = text.to_lowercase();
        for phrase in FORBIDDEN_PHRASES {
            assert!(!lower.contains(phrase), "{id} {kind:?} contains \"{phrase}\": {text}");
        }
        assert!(!words(text).iter().any(|w| w == "now"), "{id} {kind:?} contains \"now\": {text}");
    }
}

#[test]
fn no_observation_contains_imperative_verbs() {
    let observations: Vec<_> =
        templates().into_iter().filter(|(_, kind, _)| *kind == TemplateKind::Observation).collect();
    assert_eq!(observations.len(), RULES.len());
    for (id, _, text) in observations {
        for word in words(&text) {
            assert!(!IMPERATIVE_VERBS.contains(&word.as_str()), "{id} observation uses \"{word}\": {text}");
        }
    }
    // The guard itself: a hyphenated plural is not the verb, the base form is.
    assert!(!words("3-bets your opens").iter().any(|w| IMPERATIVE_VERBS.contains(&w.as_str())));
    assert!(words("Fold more often").iter().any(|w| IMPERATIVE_VERBS.contains(&w.as_str())));
}

/// In-hand state a between-hands read may never point at (D107 review):
/// the street, bet or pot of a hand in progress. Substrings, lower case.
const IN_HAND_STATE: [&str; 18] = [
    "current", "this hand", "this street", "this bet", "this pot", "this round", "this flop",
    "this turn", "this river", "in progress", "live hand", "the hand you are in", "you are facing",
    "facing his bet", "to call now", "on the flop now", "still to act", "action is on",
];

fn assert_no_in_hand_state(id: &str, text: &str) {
    let lower = text.to_lowercase();
    for phrase in IN_HAND_STATE {
        assert!(!lower.contains(phrase), "{id} advice refers to in-hand state (\"{phrase}\"): {text}");
    }
    assert!(!words(text).iter().any(|w| w == "now"), "{id} advice contains \"now\": {text}");
}

#[test]
fn no_advice_references_in_hand_state() {
    // Every advice template and variant, as written.
    let advice: Vec<_> = templates().into_iter().filter(|(_, kind, _)| *kind == TemplateKind::Advice).collect();
    assert!(advice.len() > RULES.len());
    for (id, _, text) in &advice {
        assert_no_in_hand_state(id, text);
    }
    // The guard itself catches the phrasings it is about.
    for bad in ["Raise his current street bet.", "Fold to this hand's bet.", "Call now."] {
        let caught = std::panic::catch_unwind(|| assert_no_in_hand_state("probe", bad));
        assert!(caught.is_err(), "guard must reject: {bad}");
    }

    // And as rendered: a villain on whom most rules fire, in every context
    // that changes advice (late stage, knockout covered or not, seat left
    // and right, push/fold).
    let mut contexts = Vec::new();
    for (format, eff) in [(FormatKey::Cash, 100.0), (FormatKey::Cash, 200.0), (FormatKey::Mtt, 20.0), (FormatKey::Mtt, 12.0)] {
        for seat_at in [None, Some(seat(1, 6)), Some(seat(5, 6))] {
            let mut ctx = context(format, eff);
            ctx.seat = seat_at;
            contexts.push(Some(ctx.clone()));
            if format == FormatKey::Mtt {
                ctx.stage = Some(Stage::Late);
                for covers in [true, false] {
                    ctx.bounty = Some(BountyContext {
                        amount: 40.0,
                        currency: Some("USD".into()),
                        ratio: Some(3.0),
                        hero_covers: Some(covers),
                    });
                    contexts.push(Some(ctx.clone()));
                }
            }
        }
    }
    contexts.push(None);
    let mut seen = std::collections::BTreeSet::new();
    for ctx in contexts {
        let mut v = busy_villain(FormatKey::Mtt);
        for (key, hits, n) in [(RfiSb, 30, 40), (LimpCall, 15, 20), (FoldToCbetRiver, 25, 30), (FoldToStealBb, 28, 30), (IsoRaise, 18, 20)] {
            v.stats.insert(key, stat(FormatKey::Mtt, key, hits, n));
        }
        v.ko_call_vs_shove = Some(stat(FormatKey::Mtt, CallVsShove, 14, 20));
        v.late_fold_to_steal_bb = Some(stat(FormatKey::Mtt, FoldToStealBb, 16, 20));
        v.sizing = vec![tell(SizeBucket::Overbet, 4, 0, 0)];
        v.recent_form = Some(form(12, 9, 49, Some(FormFlag::Tilt), 3.3));
        // A tournament villain whose late-stage and bounty-split reads fire
        // (rows T02, K02); kept apart so the general reads they replace
        // render on the first villain.
        let mut t = input(FormatKey::Mtt, &[]);
        t.late_call_vs_shove = Some(stat(FormatKey::Mtt, CallVsShove, 12, 30));
        t.late_open_shove = Some(stat(FormatKey::Mtt, OpenShove, 12, 30));
        t.ko_call_covering = Some(stat(FormatKey::Mtt, CallVsShove, 14, 20));
        t.ko_call_covered = Some(stat(FormatKey::Mtt, CallVsShove, 2, 20));
        t.context = ctx.clone();
        let mut u = t.clone();
        u.ko_call_covering = None;
        v.context = ctx;
        for r in evaluate_rules(&v).into_iter().chain(evaluate_rules(&t)).chain(evaluate_rules(&u)) {
            assert_no_in_hand_state(&r.rule_id, &r.advice);
            assert_no_in_hand_state(&r.rule_id, &r.observation);
            seen.insert(r.rule_id);
        }
    }
    // The rendered pass reached advice variants and the new D107 reads.
    for id in ["ko.big_bounty", "ko.hunts_bounties", "stage.late.overfolds_blinds", "pf.rfi_sb.wide", "pf.limp_call.high", "post.fold_cbet_river.high", "rec.tilt", "ctx.short_stack", "ctx.deep_stack",
        "stage.late.calls_shoves", "stage.late.shoves_wide", "ko.calls_when_covering"] {
        assert!(seen.contains(id), "{id} was not rendered");
    }
}

#[test]
fn advice_never_restates_observation() {
    for def in RULES {
        let observation = words(&observation_template(def));
        let advice: Vec<Vec<String>> =
            std::iter::once(def.advice).chain(def.variants.iter().map(|(_, t)| *t)).map(words).collect();
        for text in advice {
            for window in observation.windows(4) {
                assert!(
                    !text.windows(4).any(|w| w == window),
                    "{} advice repeats \"{}\" from its observation",
                    def.id,
                    window.join(" ")
                );
            }
        }
    }
}

/// The seat suffix at its longest: "directly on" and "right".
fn longest_seat_suffix() -> String {
    SEAT_SUFFIX.replace("{where}", "directly on").replace("{side}", "right")
}

/// Every advice text a rule can show: the default and each variant, with
/// placeholders at their longest value and, for a rule with a seat side, the
/// longest seat suffix.
fn advice_as_shown(def: &velora_poker_lib::engine::RuleDef) -> Vec<String> {
    std::iter::once(def.advice)
        .chain(def.variants.iter().map(|(_, t)| *t))
        .map(|text| {
            let text = text.replace("{bucket}", "overbet-sized");
            match def.seat {
                Some(_) => format!("{}{}", text.trim_end_matches('.'), longest_seat_suffix()),
                None => text,
            }
        })
        .collect()
}

#[test]
fn every_advice_fits_the_hover_card() {
    // Section 17.1: at the medium table (800x570) `hoverCardMetrics` gives a
    // 248px card at 11px; its 7px padding and 1px border leave 232px, and
    // the advice block is clamped to two lines. 2 × 232 − 40px for a pushed
    // word = 424px at 4.9px per character (measured): 86 characters.
    assert_eq!(HOVER_CARD_ADVICE_BUDGET, 86);
    // The suffix measured here is the one the engine appends.
    let mut v = cash(&[(RfiSb, 30, 40)]);
    let mut ctx = context(FormatKey::Cash, 100.0);
    ctx.seat = Some(seat(5, 6));
    v.context = Some(ctx);
    let shown = read(&evaluate_rules(&v), "pf.rfi_sb.wide").unwrap().advice.clone();
    assert_eq!(advice_as_shown(rule_def("pf.rfi_sb.wide"))[0], shown);
    assert!(shown.ends_with(&longest_seat_suffix()), "{shown}");

    let mut checked = 0;
    for def in RULES.iter().filter(|r| r.id != "ctx.seat_relation") {
        for text in advice_as_shown(def) {
            assert!(!text.contains('{'), "{}: unrendered placeholder in {text}", def.id);
            let len = text.chars().count();
            assert!(len <= HOVER_CARD_ADVICE_BUDGET, "{} advice is {len} characters, over {HOVER_CARD_ADVICE_BUDGET}: {text}", def.id);
            checked += 1;
        }
    }
    assert!(checked > RULES.len(), "every rule and variant: {checked}");
}

/// Families whose advice must name a concrete lever (section 17.1). Each
/// audit task added the family it rewrote; since the head-to-head, showdown
/// and recency pass (17.7) the list holds every family.
const LEVER_FAMILIES: [Family; 9] = [
    Family::Preflop,
    Family::Stack,
    Family::Context,
    Family::Stage,
    Family::Bounty,
    Family::Postflop,
    Family::H2h,
    Family::Showdown,
    Family::Recency,
];

/// Position of a family in its enum. The match is exhaustive, so a new
/// `Family` variant does not compile until the lever guard is extended.
fn family_index(family: Family) -> usize {
    match family {
        Family::Preflop => 0,
        Family::Stack => 1,
        Family::Context => 2,
        Family::Stage => 3,
        Family::Bounty => 4,
        Family::Postflop => 5,
        Family::H2h => 6,
        Family::Showdown => 7,
        Family::Recency => 8,
    }
}

/// The concrete levers an advice names: hand classes (`A2s-A5s`, `QQ+`,
/// `Kx`, suited aces, broadways, top pair), sizing (`3x`, min-raise, jam,
/// small bets, pot fractions) or the hero's seat (from the button, your big
/// blind, in position). Empty when the advice names none ("3-bet more").
fn concrete_levers(text: &str) -> Vec<String> {
    let hands = regex::Regex::new(r"\b(?:[2-9TJQKA]{2}[so]?\+?(?:-[2-9TJQKA]{2}[so]?)?|[AKQJT]x[so]?)\b").unwrap();
    let words = regex::Regex::new(
        r"(?i)\b(?:suited|offsuit|broadways?|pairs?|overpairs?|overcards|gutshots|open-enders|aces|kings|connectors|any two|premiums|top pair|draws|\d+(?:\.\d+)?x|min-raise|jam|shove|reshove|all-in|squeeze-jam|small (?:bets?|sizes?|sizing|c-bets?)|overbets?|\d/\d pot|from the (?:button|cutoff|small blind|big blind|blinds)|from late seats|your (?:small|big) blind|in position|out of position)\b",
    )
    .unwrap();
    hands.find_iter(text).chain(words.find_iter(text)).map(|m| m.as_str().to_string()).collect()
}

#[test]
fn every_advice_names_a_concrete_lever() {
    // The guard itself: the generic texts the audit removed name no lever.
    for generic in [
        "3-bet more.",
        "Steal wider when he is in the blinds.",
        "Open bigger for value and squeeze more when he flats.",
        "4-bet bluff his 3-bets more.",
        "Give his raises credit and steal his blinds often.",
        // "small" counts only as a size, never inside "small-blind".
        "Attack his small-blind opens more.",
    ] {
        assert!(concrete_levers(generic).is_empty(), "{generic}: {:?}", concrete_levers(generic));
    }
    for concrete in ["3-bet him wider with suited wheel aces from the button.", "Jam 99+.", "Open 2.5x.", "Fold Kx."] {
        assert!(!concrete_levers(concrete).is_empty(), "{concrete}");
    }

    // The stack/context/stage/bounty audit's old texts name none either
    // ("Shove wider into him." and "Open freely into him; his reshove range
    // is narrow." name a shove; the range guard
    // `push_fold_advice_names_ranges_depth_and_cover` rejects them).
    for generic in [
        "Call his short-stack shoves wider.",
        "Isolate him wide while stacks are deep.",
        "Call his shoves by range; you win his bounty only when you cover him.",
    ] {
        assert!(concrete_levers(generic).is_empty(), "{generic}: {:?}", concrete_levers(generic));
    }
    // The postflop audit's old texts (section 17.6) name none either.
    for generic in [
        "Float and check-raise his flop c-bets more.",
        "Double-barrel him more.",
        "Fire more third barrels against him.",
        "Bluff-catch rivers wider against him.",
        "Apply postflop pressure; he gives up pots easily.",
        "Value bet thin and stop bluffing him.",
    ] {
        assert!(concrete_levers(generic).is_empty(), "{generic}: {:?}", concrete_levers(generic));
    }
    // The head-to-head, showdown and recency pass's old texts (section 17.7)
    // name none either; the showdown texts are checked as written, so the
    // `{bucket}` placeholder ("overbet-sized") never counts as a size.
    for generic in [
        "4-bet him lighter and open tighter in front of him.",
        "3-bet his opens wider.",
        "C-bet him at high frequency.",
        "3-bet his steals wider when you are in the blinds.",
        "Steal tighter into him and value bet more postflop.",
        "Fold more bluff-catchers to his {bucket} bets.",
        "Call his {bucket} bets down wider.",
        "Call his river bets wider.",
        "Value bet wider and give his aggression less credit while this lasts.",
        "Isolate and value bet him wider while this lasts.",
        "Give his recent entries more credit.",
    ] {
        assert!(concrete_levers(generic).is_empty(), "{generic}: {:?}", concrete_levers(generic));
    }

    let mut checked = 0;
    // The seat suffix is appended to other advice, never shown alone.
    for def in RULES.iter().filter(|r| LEVER_FAMILIES.contains(&r.family) && r.id != "ctx.seat_relation") {
        for text in std::iter::once(def.advice).chain(def.variants.iter().map(|(_, t)| *t)) {
            assert!(!concrete_levers(text).is_empty(), "{} advice names no hand class, size or seat: {text}", def.id);
            checked += 1;
        }
    }
    let covered = RULES.iter().filter(|r| LEVER_FAMILIES.contains(&r.family) && r.id != "ctx.seat_relation").count();
    assert!(checked >= covered);
    // The covered list is every family, each once, and every family has
    // rules, so the guard covers every rule but the seat suffix.
    let mut indices: Vec<usize> = LEVER_FAMILIES.iter().map(|f| family_index(*f)).collect();
    indices.sort_unstable();
    assert_eq!(indices, (0..9).collect::<Vec<_>>(), "LEVER_FAMILIES must list every Family exactly once");
    for family in LEVER_FAMILIES {
        assert!(RULES.iter().any(|r| r.family == family && r.id != "ctx.seat_relation"), "{family:?} has rules");
    }
    assert_eq!(covered, RULES.len() - 1, "every rule but the seat suffix is covered");
}

#[test]
fn push_fold_advice_names_ranges_depth_and_cover() {
    // Section 17.4, stack/context/stage/bounty rows: push/fold advice names
    // the shove or call range by hand class (or the bb depth it holds at),
    // and every knockout text states whose cover it depends on.
    let range = regex::Regex::new(
        // Case-sensitive, so "at" is not read as a hand.
        r"\b(?:[2-9TJQKA]{2}[so]?|any two|any (?:pair|ace)|pairs|aces|broadways|suited (?:king|aces|connectors)|\d+bb)\b",
    )
    .unwrap();
    // The guard rejects the texts the audit replaced; "at" is not a hand.
    for old in [
        "Shove wider into him.",
        "Call his short-stack shoves wider.",
        "Open freely into him; his reshove range is narrow.",
        "Shove wider into him at a loose table.",
    ] {
        assert!(!range.is_match(old), "the guard must reject: {old}");
    }
    for def in RULES.iter().filter(|r| matches!(r.family, Family::Stack | Family::Bounty) || r.id.starts_with("stage.late")) {
        for text in std::iter::once(def.advice).chain(def.variants.iter().map(|(_, t)| *t)) {
            assert!(range.is_match(text), "{} names no range or depth: {text}", def.id);
        }
    }
    for def in RULES.iter().filter(|r| r.family == Family::Bounty) {
        for text in std::iter::once(def.advice).chain(def.variants.iter().map(|(_, t)| *t)) {
            assert!(text.contains("cover"), "{} states no cover condition: {text}", def.id);
        }
    }
    for (_, text) in rule_def("stk.call_shove.wide").variants.iter().chain(rule_def("stk.call_shove.tight").variants) {
        assert!(text.contains("cover"), "bounty variant states no cover condition: {text}");
    }
}

#[test]
fn only_showdown_reads_speak_about_value_or_bluffs() {
    // player-descriptions.md hard rule: no range-composition claim unless a
    // showdown-backed read cites its counts.
    // Advice may name the hero's own bluffs ("before bluffing"); an
    // observation saying "bluffing" is about the villain.
    let composition_words = ["weak", "strong", "nuts", "air"];
    let composition_phrases = [
        "bluff-heavy", "value-heavy", "underbluff", "overbluff", "showed value", "has it",
        "value hands", "is bluffing", "usually bluffing",
    ];
    for (id, kind, text) in templates() {
        let lower = text.to_lowercase();
        let observed = kind == TemplateKind::Observation && words(&text).iter().any(|w| w == "bluffing");
        let claims = observed
            || words(&text).iter().any(|w| composition_words.contains(&w.as_str()))
            || composition_phrases.iter().any(|p| lower.contains(p));
        if id.starts_with("sd.") {
            if kind == TemplateKind::Observation {
                assert!(text.contains("{n}"), "{id} must cite its sample: {text}");
                assert!(text.contains("{valueCount}") || text.contains("{bluffCount}"), "{id} must cite its count: {text}");
            }
        } else {
            assert!(!claims, "{id} {kind:?} claims range composition: {text}");
        }
    }
}

#[test]
fn every_rule_has_a_unique_id_known_tag_and_catalogue_row() {
    let vocabulary = [
        "TILT", "NNbb", "NIT", "STN", "LAG", "LP", "ST+", "ST-", "FTS", "DEF", "OVD", "3B+", "3B-",
        "F3B", "C3B", "4B+", "F4B", "SQZ", "LMP", "ISO", "LRR", "LF", "CC+", "SHV", "NSV", "CLS",
        "TCS", "RSV", "RS-", "DEEP", "KO", "HNT", "CB+", "CB-", "FCB", "NFC", "BRL", "GIV", "TRB",
        "RV-", "DLY", "XR+", "XR-", "DNK", "FLT", "PRB", "RV+", "RR+", "WSD", "WWS", "H2H", "SDV",
        "SDB", "SVB", "LSE", "TGT", "SB+", "LPC", "RVF", "ICM-", "KO+", "OVB", "SRB", "FRR", "FOF",
    ];
    // docs/ is local-only (git-excluded), so the spec is read at runtime and the
    // spec-membership checks are skipped on a checkout without it;
    // scripts/check-engine-spec.mjs owns spec-to-code traceability.
    let spec_path = concat!(env!("CARGO_MANIFEST_DIR"), "/../docs/specs/opponent-engine.md");
    let spec = std::fs::read_to_string(spec_path).ok();
    if spec.is_none() {
        eprintln!("skipping spec-membership checks: {spec_path} is not present");
    }
    let mut seen = std::collections::HashSet::new();
    for def in RULES {
        assert!(seen.insert(def.id), "duplicate {}", def.id);
        assert_eq!(rule_def(def.id).id, def.id);
        if let Some(tag) = def.tag {
            assert!(vocabulary.contains(&tag), "{} tag {tag}", def.id);
        }
        if let Some(spec) = &spec {
            assert!(spec.contains(&format!("| {} |", def.scenario)), "{} row {}", def.id, def.scenario);
            assert!(spec.contains(&format!("`{}`", def.id)), "{} is not in the spec", def.id);
        }
    }
}

// ------------------------------------------------ end to end on imported hands

const SIX: [&str; 6] = [HERO, "Sam", "Bob", "Vil", "Hal", "Cole"];

#[test]
fn imported_hands_fire_fold_to_three_bet_with_seat_and_counts() {
    // Button = Hero (seat 1), Vil UTG (seat 4): three seats to the hero's
    // right of six. Vil opens and folds to the hero's 3-bet 30 times.
    let mut t = Table::cash("Rules Cash", 300_007_000, &SIX);
    let mut text = String::new();
    for i in 0..30 {
        for p in SIX {
            t.stacks.insert(p.to_string(), 100.0 * t.bb);
        }
        text += &t.play(
            i,
            &[&[
                ("Vil", Act::Raise(3.0)), ("Hal", Act::Fold), ("Cole", Act::Fold), (HERO, Act::Raise(9.0)),
                ("Sam", Act::Fold), ("Bob", Act::Fold), ("Vil", Act::Fold),
            ]],
            None,
        );
    }
    let conn = import(&text);
    let vil = player_id(&conn, "Vil");
    let hand = latest_table_hand(&conn, "Rules Cash").unwrap().unwrap();
    let ctx = villain_context(&hand, vil).unwrap();
    assert_eq!(ctx.seat.map(|s| s.side), Some(Side::Right));

    let input = player_rule_input(&conn, vil, Some(ctx), &|f, k| stat_spec(k).builtin_prior(f)).unwrap();
    let reads = evaluate_rules(&input);
    let r = read(&reads, "pf.fold_to_3bet.high").expect("folds every 3-bet");
    assert_eq!(r.observation, "Folds to 100% (30/30) of 3-bets.");
    assert_eq!(r.advice, "3-bet his opens light with A2s-A5s, K9s+ and 65s+ (on your right).");
    let e = &r.evidence[0];
    assert_eq!((e.stat_name.as_str(), e.hits, e.opportunities, e.value), ("fold_to_3bet_oop", 30, 30, Some(100.0)));
    // Recency-weighted shrink: under the plain (30 + 9.3)/45 = 87.3%, above the threshold.
    let shrunk = e.shrunk.unwrap();
    assert!(shrunk > 74.0 && shrunk <= 87.3, "shrunk {shrunk}");
    // His UTG opens: 30 of 30.
    assert!(read(&reads, "pf.rfi.loose_early").is_some());
    // Head-to-head: every 3-bet he faced was the hero's, but 30/30 shrunk
    // toward his own overall fold rate stays within 12 points of it.
    assert!(input.h2h.iter().any(|s| s.key == H2hKey::FoldToHeroThreeBet && s.opportunities == 30));
    // Nothing postflop was seen, so no postflop read.
    assert!(reads.iter().all(|r| r.family != Family::Postflop));
}

#[test]
fn imported_short_stack_tournament_hands_get_only_push_fold_reads() {
    // Vil opens UTG 30 times at 40bb, then sits on 12bb at the last hand.
    let mut t = Table::mtt("Rules Short", 300_007_100, &SIX, 40.0);
    let mut text = String::new();
    for i in 0..30 {
        for p in SIX {
            t.stacks.insert(p.to_string(), 40.0 * t.bb);
        }
        text += &t.play(
            i,
            &[&[
                ("Vil", Act::Raise(2.2)), ("Hal", Act::Fold), ("Cole", Act::Fold), (HERO, Act::Raise(6.0)),
                ("Sam", Act::Fold), ("Bob", Act::Fold), ("Vil", Act::Fold),
            ]],
            None,
        );
    }
    for p in SIX {
        t.stacks.insert(p.to_string(), 40.0 * t.bb);
    }
    t.stacks.insert("Vil".into(), 12.0 * t.bb);
    text += &t.walk(40);
    let conn = import(&text);
    let vil = player_id(&conn, "Vil");
    let hand = latest_table_hand(&conn, "Rules Short").unwrap().unwrap();
    let ctx = villain_context(&hand, vil).unwrap();
    assert_eq!(ctx.stack_bucket, Some(StackBucket::PushFold));

    let deep = player_rule_input(&conn, vil, None, &|f, k| stat_spec(k).builtin_prior(f)).unwrap();
    assert!(read(&evaluate_rules(&deep), "pf.fold_to_3bet.high").is_some());
    let short = player_rule_input(&conn, vil, Some(ctx), &|f, k| stat_spec(k).builtin_prior(f)).unwrap();
    let reads = evaluate_rules(&short);
    assert!(read(&reads, "pf.fold_to_3bet.high").is_none());
    assert_eq!(read(&reads, "ctx.short_stack").and_then(|r| r.tag.as_deref()), Some("12bb"));
}
