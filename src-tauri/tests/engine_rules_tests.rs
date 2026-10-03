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
use velora_poker_lib::engine::rules::{observation_template, SEAT_SUFFIX};
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
    let mut input = RuleInput::default();
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
    assert_eq!(r.advice, "3-bet and defend your big blind wider against his small-blind opens.");
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
        "3-bet and defend your big blind wider against his small-blind opens, and he sits directly on your right."
    );
}

// ------------------------------------------------ blind defence (P03, P04)

#[test]
fn fold_to_steal_rules_fire_at_threshold_and_not_below() {
    let c = FormatKey::Cash;
    assert_above(c, "pf.fold_to_steal.high", FoldToStealSb, 30, 0.12, &[]);
    assert_above(c, "pf.fold_to_steal.high", FoldToStealBb, 30, 0.12, &[]);
    let h = assert_below(c, "pf.fold_to_steal.low", FoldToStealBb, 30, 0.15, &[]);
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
    let c = FormatKey::Cash;
    assert_below(c, "pf.bb_vs_sb.overfold", BbDefendVsSb, 25, 0.15, &[]);
    assert_above(c, "pf.bb_vs_sb.overdefend", BbDefendVsSb, 25, 0.15, &[]);
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
    let c = FormatKey::Cash;
    assert_above(c, "pf.limp.high", Limp, 50, 0.08, &[]);
    assert_above(c, "pf.iso.high", IsoRaise, 20, 0.15, &[]);
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
                || r.rule_id == "ctx.short_stack",
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

    // 15bb is still push/fold; 15.1bb is not.
    short.context = Some(context(FormatKey::Mtt, 15.0));
    assert!(read(&evaluate_rules(&short), "pf.3bet.high_ip").is_none());
    short.context = Some(context(FormatKey::Mtt, 15.1));
    let reads = evaluate_rules(&short);
    assert!(read(&reads, "pf.3bet.high_ip").is_some());
    assert!(read(&reads, "ctx.short_stack").is_none());
}

#[test]
fn reshove_family_takes_priority_at_20bb() {
    // Reshove 5/12 in an MTT: shrunk (5 + 1)/22 = .273, d = 2.88,
    // confidence 12/22 → score 1.57. 3-bet IP 8/40: (8 + 1.75)/65 = .15,
    // d = 2.67, confidence 40/65 → score 1.64.
    let mut v = input(FormatKey::Mtt, &[(Reshove, 5, 12), (ThreeBetIp, 8, 40), (CallVsShove, 15, 30)]);
    v.context = Some(context(FormatKey::Mtt, 40.0));
    let deep = evaluate_rules(&v);
    v.context = Some(context(FormatKey::Mtt, 20.0));
    let mid = evaluate_rules(&v);

    let score = |reads: &[EngineRuleResult], id: &str| read(reads, id).unwrap().score;
    assert!(score(&deep, "pf.3bet.high_ip") > score(&deep, "stk.reshove.high"));
    assert!(score(&mid, "stk.reshove.high") > score(&mid, "pf.3bet.high_ip"));
    for id in ["stk.reshove.high", "stk.call_shove.wide"] {
        let ratio = score(&mid, id) / score(&deep, id);
        assert!((ratio - 1.5).abs() < 1e-9, "{id}: ×{ratio}");
    }
    assert_eq!(score(&mid, "pf.3bet.high_ip"), score(&deep, "pf.3bet.high_ip"));
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

    // Middle stage: plain steal advice, no late-stage read.
    let reads = evaluate_rules(&v);
    assert_eq!(read(&reads, "pf.fold_to_steal.high").unwrap().advice, "Steal wider when he is in the blinds.");
    assert!(read(&reads, "stage.late.overfolds_blinds").is_none());

    v.context.as_mut().unwrap().stage = Some(Stage::Late);
    let reads = evaluate_rules(&v);
    let steal = read(&reads, "pf.fold_to_steal.high").unwrap();
    assert_eq!(steal.advice, "Shove or min-raise wider when he is in the blinds.");
    assert!(steal.adapted_by.contains(&"stage.late"));
    let late = read(&reads, "stage.late.overfolds_blinds").expect("late-stage read");
    // Over late-stage hands only: 16/20, prior .55, k 12: (16 + 6.6)/32 = .706 ≥ .65.
    assert_eq!(late.observation, "Folds his big blind to 80% (16/20) of steals late in the tournament.");
    assert!(late.advice.starts_with("Shove"));
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
    assert_eq!(ko.advice, "Call his shoves wider when you cover him; the bounty adds equity.");
    assert_eq!(ko.tag.as_deref(), Some("KO"));
    // KO: call-vs-shove advice switches to the bounty variant.
    assert_eq!(
        read(&reads, "stk.call_shove.wide").unwrap().advice,
        "Shove only for value against him; bounty equity widens his calls further."
    );
    // Over KO hands: (14 + 2.5)/30 = .55 ≥ .40. The hero covers him, so he
    // cannot win the hero's bounty.
    let hunter = read(&reads, "ko.hunts_bounties").expect("bounty hunter read");
    assert_eq!(hunter.advice, "Shove wider into him while you cover him; he cannot win your bounty.");

    // The hero does not cover him: no bounty to win, call by range.
    ctx.bounty.as_mut().unwrap().hero_covers = Some(false);
    v.context = Some(ctx.clone());
    let reads = evaluate_rules(&v);
    assert_eq!(
        read(&reads, "ko.big_bounty").unwrap().advice,
        "Call his shoves by range; you win his bounty only when you cover him."
    );
    assert_eq!(read(&reads, "ko.hunts_bounties").unwrap().advice, "When he covers you, shove into him for value only.");

    // Under 2× the initial bounty: no big-bounty read.
    ctx.bounty.as_mut().unwrap().ratio = Some(1.99);
    v.context = Some(ctx.clone());
    assert!(!fires(&v, "ko.big_bounty"));
    // No bounty at all: plain call-vs-shove advice.
    ctx.bounty = None;
    v.context = Some(ctx);
    assert_eq!(
        read(&evaluate_rules(&v), "stk.call_shove.wide").unwrap().advice,
        "Shove only for value against him; stop pure bluff shoves."
    );
    // ko.hunts_bounties needs its KO-hand sample: 5 opportunities is short.
    v.ko_call_vs_shove = Some(stat(m, CallVsShove, 5, 5));
    assert!(!fires(&v, "ko.hunts_bounties"));
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
    assert_eq!(three.advice, "4-bet him lighter and flat fewer opens he can 3-bet, and he sits directly on your left.");
    assert!(three.adapted_by.contains(&"ctx.seat_relation"));
    // His fold-to-3-bet matters when he opens in front of the hero.
    assert_eq!(read(&reads, "pf.fold_to_3bet.high").unwrap().advice, "3-bet his opens wider as a bluff.");
    // Postflop reads never carry the seat.
    assert_eq!(read(&reads, "post.cbet_flop.high").unwrap().advice, "Float and check-raise his flop c-bets more.");

    ctx.seat = Some(seat(4, 6));
    v.context = Some(ctx.clone());
    let reads = evaluate_rules(&v);
    assert_eq!(read(&reads, "pf.3bet.high_ip").unwrap().advice, "4-bet him lighter and flat fewer opens he can 3-bet.");
    assert_eq!(
        read(&reads, "pf.fold_to_3bet.high").unwrap().advice,
        "3-bet his opens wider as a bluff, and he sits on your right."
    );
    ctx.seat = Some(seat(5, 6));
    v.context = Some(ctx);
    assert_eq!(
        read(&evaluate_rules(&v), "pf.fold_to_3bet.high").unwrap().advice,
        "3-bet his opens wider as a bluff, and he sits directly on your right."
    );
    // The seat relation is a suffix, never a read of its own.
    assert!(!fires(&v, "ctx.seat_relation"));
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
    assert_eq!(r.advice, "Fire more third barrels against him.");
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
    assert_eq!(read(&reads, "post.check_raise.never").unwrap().observation, "Has check-raised 0 of 80 flop c-bets.");
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
    assert_below(c, "post.wwsf_low", Wwsf, 60, 0.08, &[]);
    assert_above(c, "post.wwsf_high", Wwsf, 60, 0.08, &[]);
}

#[test]
fn preflop_all_in_creates_no_postflop_rule_reads() {
    // Rules see only what extraction counted: no postflop opportunity means
    // no postflop read, whatever the preflop numbers.
    let reads = evaluate_rules(&cash(&[(Vpip, 50, 60), (Pfr, 40, 60)]));
    assert!(reads.iter().all(|r| r.family != Family::Postflop));
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
    assert_eq!(r.advice, "Fold more bluff-catchers to his overbet-sized bets.");
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
        v.context = ctx;
        for r in evaluate_rules(&v) {
            assert_no_in_hand_state(&r.rule_id, &r.advice);
            assert_no_in_hand_state(&r.rule_id, &r.observation);
            seen.insert(r.rule_id);
        }
    }
    // The rendered pass reached advice variants and the new D107 reads.
    for id in ["ko.big_bounty", "ko.hunts_bounties", "stage.late.overfolds_blinds", "pf.rfi_sb.wide", "pf.limp_call.high", "post.fold_cbet_river.high", "rec.tilt", "ctx.short_stack", "ctx.deep_stack"] {
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
        "SDB", "SVB", "LSE", "TGT", "SB+", "LPC", "RVF",
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
    assert_eq!(r.advice, "3-bet his opens wider as a bluff, and he sits on your right.");
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
