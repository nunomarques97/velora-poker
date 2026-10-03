//! Scenario rules (`docs/specs/opponent-engine.md`, sections 1, 5, 7 and 8):
//! every read of the catalogue, evaluated on shrunk stats and adapted to the
//! between-hands context.
//!
//! [`RULES`] is the single source of every observation and advice template,
//! so the guard tests of section 1 scan exactly the text the engine can
//! show. Each read keeps the `RuleResult` contract of `description_rules`
//! (observation without imperatives, imperative advice that never restates
//! it, confidence from the basis stat's own opportunity count) and adds the
//! engine's evidence (`hits`, `shrunk`), family, tag and score.
//!
//! Reads are per opponent and valid between hands. Nothing here sees the
//! hand in progress: the context is the latest completed hand at the table.

use std::collections::BTreeMap;

use rusqlite::Connection;
use serde::Serialize;

use super::aggregate::{aggregate_player, H2hKey, H2hStat, PlayerAggregate, StatAgg, View};
use super::context::{average_stack_bb, EngineContext, Side, Stage, StackBucket};
use super::facts::{load_player_hands, HandFacts, StatKey};
use super::pooling::{confidence, stat_spec, FormatKey, ShrunkStat};
use super::pot::SizeBucket;
use super::preflop::extract_preflop;
use super::recency::{form_counts, FormCounts, FormFlag, RecentForm, TILT_MIN_BASELINE, TILT_MIN_WINDOW};
use super::showdown::{
    extract_showdowns, load_showdown_boards, sizing_tally, ShowdownRecord, SizingTell, ValueClass,
};
use crate::description_rules::{ConfidenceTier, Evidence, RuleCategory, RuleResult};
use crate::parser::Street;

/// Spec id of the between-hands text guard (catalogue row G03): no template
/// carries an in-hand phrasing, no observation an imperative verb.
pub const RULE_GUARD: &str = "guard.between_hands";

/// Spec id of the seat-relation suffix (catalogue row C01). It is appended
/// to preflop advice, never shown as a read of its own.
pub const RULE_SEAT_RELATION: &str = "ctx.seat_relation";

/// Ranking multiplier of the reshove family at 15–25bb (section 7).
pub const RESHOVE_PRIORITY: f64 = 1.5;
/// Ranking multiplier of head-to-head reads (section 7).
pub const H2H_PRIORITY: f64 = 1.25;
/// A sizing bucket needs this many classified showdowns before a tell rule
/// may fire (section 9; the payload shows buckets from 3).
pub const SD_TELL_RULE_MIN: u32 = 4;
/// River-aggression showdowns `sd.shows_bluffs` needs.
pub const SD_RIVER_BLUFF_MIN: u32 = 6;

/// Thresholds are compared with this slack so a value computed exactly at a
/// boundary fires on either side of floating-point rounding.
const EPS: f64 = 1e-9;

/// Profile thresholds (row P12) as deltas from the format's priors (D107):
/// nit VPIP ≤ prior − .12; loose VPIP ≥ prior + .13 with a VPIP–PFR gap at
/// least .08 wider than the priors' gap; station WTSD ≥ prior + .05; LAG
/// VPIP ≥ prior + .05 and PFR ≥ prior + .05. With the 6-max cash priors
/// these are D106's absolute .15, .40, .15, .32 and .32/.25.
pub const PROFILE_NIT: f64 = 0.12;
pub const PROFILE_LOOSE: f64 = 0.13;
pub const PROFILE_PASSIVE_GAP: f64 = 0.08;
pub const PROFILE_STATION_WTSD: f64 = 0.05;
pub const PROFILE_LAG_VPIP: f64 = 0.05;
pub const PROFILE_LAG_PFR: f64 = 0.05;

/// Payload family of a read (section 13).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Family {
    Preflop,
    Stack,
    Context,
    Stage,
    Bounty,
    Postflop,
    H2h,
    Showdown,
    Recency,
}

/// When an alternative advice text replaces the default (section 5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Variant {
    /// The table is a knockout game (the villain carries a bounty).
    Bounty,
    /// Tournament stage `late`: blind attacks become shoves.
    LateStage,
    /// Knockout game and the hero does not cover the villain.
    HeroCovered,
    /// Knockout game and the hero covers the villain.
    HeroCovers,
}

/// One catalogue rule: identity and every text it can show.
#[derive(Debug, Clone, Copy)]
pub struct RuleDef {
    pub id: &'static str,
    /// Catalogue row (section 14).
    pub scenario: &'static str,
    pub family: Family,
    pub category: RuleCategory,
    /// Chip tag; `NNbb` is filled with the effective stack.
    pub tag: Option<&'static str>,
    pub observation: &'static str,
    pub advice: &'static str,
    pub variants: &'static [(Variant, &'static str)],
    /// The side of the hero where the villain's seat matters for this read
    /// (section 5): `left` for reads about how he answers the hero's opens,
    /// `right` for reads about his own opens.
    pub seat: Option<Side>,
}

const fn rule(
    id: &'static str,
    scenario: &'static str,
    family: Family,
    category: RuleCategory,
    tag: Option<&'static str>,
    observation: &'static str,
    advice: &'static str,
) -> RuleDef {
    RuleDef { id, scenario, family, category, tag, observation, advice, variants: &[], seat: None }
}

const fn left(mut def: RuleDef) -> RuleDef {
    def.seat = Some(Side::Left);
    def
}

const fn right(mut def: RuleDef) -> RuleDef {
    def.seat = Some(Side::Right);
    def
}

const fn with(mut def: RuleDef, variants: &'static [(Variant, &'static str)]) -> RuleDef {
    def.variants = variants;
    def
}

use Family as F;
use RuleCategory::{Exploit as X, Tendency as T};

/// Seat suffix appended to preflop advice (row C01). `{where}` is `on` or
/// `directly on`.
pub const SEAT_SUFFIX: &str = ", and he sits {where} your {side}.";

/// Every rule of section 8, in catalogue order.
pub const RULES: &[RuleDef] = &[
    // ---- preflop profile (P12)
    with(
        rule("pf.profile.nit", "P12", F::Preflop, T, Some("NIT"),
            "Plays few hands: VPIP {raw}% ({hits}/{n}).",
            "Give his raises credit and steal his blinds often."),
        &[(Variant::LateStage, "Give his raises credit and attack his blinds with shoves and min-raises.")],
    ),
    rule("pf.profile.station", "P12", F::Preflop, X, Some("STN"),
        "Enters {raw}% of pots and reaches showdown often.",
        "Value bet thinner and cut your bluffs against him."),
    rule("pf.profile.loose_passive", "P12", F::Preflop, X, Some("LP"),
        "Enters {raw}% of pots, mostly by calling.",
        "Isolate his limps and value bet him relentlessly."),
    rule("pf.profile.lag", "P12", F::Preflop, T, Some("LAG"),
        "Plays {raw}% of hands and raises most of them.",
        "3-bet him for value wider and call down lighter."),
    // ---- opens and steals (P01, P02)
    right(rule("pf.rfi.tight_early", "P01", F::Preflop, T, Some("NIT"),
        "Opens {raw}% from early and middle seats.",
        "Fold marginal hands to his early opens.")),
    right(rule("pf.rfi.loose_early", "P01", F::Preflop, X, Some("LSE"),
        "Opens {raw}% from early and middle seats.",
        "3-bet his early opens wider.")),
    right(rule("pf.rfi.loose_late", "P02", F::Preflop, X, Some("ST+"),
        "Opens {raw}% from the cutoff and button.",
        "3-bet and defend wider against his late opens.")),
    right(rule("pf.rfi.tight_late", "P02", F::Preflop, T, Some("ST-"),
        "Opens only {raw}% from the cutoff and button.",
        "Respect his late opens; 3-bet them mostly for value.")),
    right(rule("pf.steal.high", "P02", F::Preflop, X, Some("ST+"),
        "Steals {raw}% when folded to in late position.",
        "3-bet his steals wider from the blinds.")),
    right(rule("pf.steal.low", "P02", F::Preflop, T, Some("ST-"),
        "Steals only {raw}% when folded to in late position.",
        "Fold more blind hands to his steals.")),
    // Pro review (D107, P13): the small blind's open against the big blind.
    right(rule("pf.rfi_sb.wide", "P13", F::Preflop, X, Some("SB+"),
        "Opens {raw}% of his small blinds when folded to.",
        "3-bet and defend your big blind wider against his small-blind opens.")),
    // ---- blind defence (P03, P04)
    left(with(
        rule("pf.fold_to_steal.high", "P03", F::Preflop, X, Some("FTS"),
            "Folds his blinds to {raw}% of steals.",
            "Steal wider when he is in the blinds."),
        &[(Variant::LateStage, "Shove or min-raise wider when he is in the blinds.")],
    )),
    left(rule("pf.fold_to_steal.low", "P03", F::Preflop, T, Some("DEF"),
        "Defends his big blind against {raw}% of steals.",
        "Steal tighter into him and value bet more postflop.")),
    left(with(
        rule("pf.bb_vs_sb.overfold", "P04", F::Preflop, X, Some("FTS"),
            "Defends his big blind against small-blind opens only {raw}%.",
            "Open your small blind wide against him."),
        &[(Variant::LateStage, "Shove or min-raise your small blind wide against him.")],
    )),
    left(rule("pf.bb_vs_sb.overdefend", "P04", F::Preflop, T, Some("OVD"),
        "Defends his big blind against small-blind opens {raw}%.",
        "Open tighter from the small blind and value bet postflop.")),
    // ---- 3-bets, 4-bets, squeezes (P05–P08)
    left(rule("pf.3bet.high_ip", "P05", F::Preflop, X, Some("3B+"),
        "3-bets in position {raw}% of the time.",
        "4-bet him lighter and flat fewer opens he can 3-bet.")),
    left(rule("pf.3bet.high_oop", "P05", F::Preflop, X, Some("3B+"),
        "3-bets from out of position {raw}% of the time.",
        "4-bet him lighter and call his 3-bets in position more.")),
    left(rule("pf.3bet.low", "P05", F::Preflop, T, Some("3B-"),
        "Rarely 3-bets: {raw}%.",
        "Give his 3-bets credit and open freely in front of him.")),
    right(rule("pf.fold_to_3bet.high", "P06", F::Preflop, X, Some("F3B"),
        "Folds to {raw}% of 3-bets.",
        "3-bet his opens wider as a bluff.")),
    right(rule("pf.fold_to_3bet.low", "P06", F::Preflop, T, Some("C3B"),
        "Continues against {raw}% of 3-bets.",
        "3-bet him for value only and size up.")),
    right(rule("pf.4bet.high", "P07", F::Preflop, X, Some("4B+"),
        "4-bets {raw}% of the time against 3-bets.",
        "3-bet him with hands that can call a 4-bet; 5-bet shove lighter.")),
    left(rule("pf.fold_to_4bet.high", "P07", F::Preflop, X, Some("F4B"),
        "Folds to {raw}% of 4-bets after 3-betting.",
        "4-bet bluff his 3-bets more.")),
    left(rule("pf.squeeze.high", "P08", F::Preflop, X, Some("SQZ"),
        "Squeezes {raw}% when there is an open and a caller.",
        "Flat less with hands that hate a squeeze; 4-bet his squeezes lighter.")),
    // ---- limps and cold calls (P09–P11)
    right(rule("pf.limp.high", "P09", F::Preflop, X, Some("LMP"),
        "Open-limps {raw}% of hands.",
        "Isolate his limps with a wide raising range.")),
    left(rule("pf.iso.high", "P09", F::Preflop, T, Some("ISO"),
        "Raises over limpers {raw}% of the time.",
        "Overlimp less in front of him; when you limp, plan to limp-reraise.")),
    right(rule("pf.limp_fold.high", "P10", F::Preflop, X, Some("LF"),
        "Folds to {raw}% of raises after limping.",
        "Raise his limps with any two playable cards.")),
    right(rule("pf.limp_reraise.seen", "P10", F::Preflop, T, Some("LRR"),
        "Has limp-reraised {hits} of {n} times after limping.",
        "Isolate his limps with hands that can call a reraise.")),
    // Pro review (D107, P14): the sticky limper.
    right(rule("pf.limp_call.high", "P14", F::Preflop, X, Some("LPC"),
        "Calls {raw}% of raises after limping.",
        "Isolate his limps bigger with hands that play well postflop, and bluff less once he calls.")),
    left(rule("pf.cold_call.high", "P11", F::Preflop, X, Some("CC+"),
        "Cold-calls {raw}% of opens.",
        "Open bigger for value and squeeze more when he flats.")),
    // ---- stack depth (S01–S03)
    right(rule("stk.open_shove.wide", "S01", F::Stack, X, Some("SHV"),
        "Open-shoves {raw}% when folded to at 15bb or less.",
        "Call his short-stack shoves wider.")),
    right(rule("stk.open_shove.tight", "S01", F::Stack, T, Some("NSV"),
        "Open-shoves only {raw}% when folded to at 15bb or less.",
        "Give his short-stack shoves credit; steal into him more.")),
    with(
        rule("stk.call_shove.wide", "S02", F::Stack, X, Some("CLS"),
            "Calls {raw}% of all-in shoves.",
            "Shove only for value against him; stop pure bluff shoves."),
        &[(Variant::Bounty, "Shove only for value against him; bounty equity widens his calls further.")],
    ),
    with(
        rule("stk.call_shove.tight", "S02", F::Stack, X, Some("TCS"),
            "Calls only {raw}% of all-in shoves.",
            "Shove wider into him."),
        &[(Variant::Bounty, "Shove wider into him, but less when he covers you and your bounty is at stake.")],
    ),
    left(rule("stk.reshove.high", "S03", F::Stack, X, Some("RSV"),
        "Reshoves {raw}% over opens at 15–25bb.",
        "Open tighter or smaller into him and call his reshoves wider.")),
    left(rule("stk.reshove.low", "S03", F::Stack, T, Some("RS-"),
        "Reshoves only {raw}% over opens at 15–25bb.",
        "Open freely into him; his reshove range is narrow.")),
    // ---- context (S04, S05, C01)
    rule("ctx.short_stack", "S04", F::Context, T, Some("NNbb"),
        "Sits on {eff}bb effective after the last hand.",
        "Treat his entries as push/fold; call his shoves by range, not by feel."),
    rule("ctx.deep_stack", "S05", F::Context, T, Some("DEEP"),
        "Sits on {eff}bb effective after the last hand.",
        "4-bet bluff less and value bet bigger against him."),
    rule(RULE_SEAT_RELATION, "C01", F::Context, T, None,
        "Sits {distance} seats to your {side}.",
        SEAT_SUFFIX),
    // ---- tournament stage (T01) and bounties (K01)
    rule("stage.late.overfolds_blinds", "T01", F::Stage, X, Some("FTS"),
        "Folds his big blind to {raw}% of steals late in the tournament.",
        "Shove or min-raise any two playable cards into his big blind."),
    rule("stage.early.passive", "T01", F::Stage, X, Some("LMP"),
        "Open-limps {raw}% in the early levels.",
        "Isolate him wide while stacks are deep."),
    with(
        rule("ko.big_bounty", "K01", F::Bounty, T, Some("KO"),
            "Carries a {ratio}× bounty ({amount}).",
            "Call his shoves wider when you cover him; the bounty adds equity."),
        &[(Variant::HeroCovered, "Call his shoves by range; you win his bounty only when you cover him.")],
    ),
    with(
        rule("ko.hunts_bounties", "K01", F::Bounty, X, Some("HNT"),
            "Calls {raw}% of shoves in bounty games.",
            "When he covers you, shove into him for value only."),
        &[(Variant::HeroCovers, "Shove wider into him while you cover him; he cannot win your bounty.")],
    ),
    // ---- postflop (F01–F12)
    rule("post.cbet_flop.high", "F01", F::Postflop, X, Some("CB+"),
        "C-bets {raw}% of flops as the preflop raiser.",
        "Float and check-raise his flop c-bets more."),
    rule("post.cbet_flop.low", "F01", F::Postflop, T, Some("CB-"),
        "C-bets only {raw}% of flops as the preflop raiser.",
        "Give his flop c-bets credit; stab when he checks."),
    rule("post.fold_cbet_flop.high", "F02", F::Postflop, X, Some("FCB"),
        "Folds to {raw}% of flop c-bets.",
        "C-bet him often with small sizes."),
    rule("post.fold_cbet_flop.low", "F02", F::Postflop, X, Some("NFC"),
        "Folds to only {raw}% of flop c-bets.",
        "C-bet him mostly for value; plan a turn barrel before bluffing."),
    rule("post.cbet_turn.high", "F03", F::Postflop, X, Some("BRL"),
        "Fires the turn {raw}% after c-betting the flop.",
        "Call flops wider with hands that can face a turn barrel."),
    rule("post.cbet_turn.giveup", "F03", F::Postflop, X, Some("GIV"),
        "Fires the turn only {raw}% after c-betting the flop.",
        "Float his flop c-bets and take the pot when he checks the turn."),
    rule("post.fold_cbet_turn.high", "F05", F::Postflop, X, Some("TRB"),
        "Folds to {raw}% of turn barrels.",
        "Double-barrel him more."),
    rule("post.cbet_river.high", "F04", F::Postflop, X, Some("BRL"),
        "Fires the river {raw}% after barrelling the turn.",
        "Bluff-catch his river barrels more often."),
    rule("post.cbet_river.low", "F04", F::Postflop, T, Some("RV-"),
        "Fires the river only {raw}% after barrelling the turn.",
        "Give his river barrels credit."),
    rule("post.fold_cbet_river.low", "F05", F::Postflop, X, Some("STN"),
        "Folds to only {raw}% of river barrels.",
        "Stop bluffing his rivers; value bet thinner."),
    // Pro review (D107, F14): calls flop and turn, then gives up the river.
    rule("post.fold_cbet_river.high", "F14", F::Postflop, X, Some("RVF"),
        "Folds to {raw}% of river barrels after calling the turn.",
        "Fire more third barrels against him."),
    rule("post.delayed_cbet.high", "F06", F::Postflop, T, Some("DLY"),
        "Bets {raw}% of turns after checking the flop as raiser.",
        "Check back fewer flops in position; bet the turn yourself when he checks twice."),
    rule("post.check_raise.high", "F07", F::Postflop, X, Some("XR+"),
        "Check-raises {raw}% of flop c-bets.",
        "C-bet tighter into him and call his check-raises wider."),
    rule("post.check_raise.never", "F07", F::Postflop, T, Some("XR-"),
        "Has check-raised {hits} of {n} flop c-bets.",
        "C-bet freely into him; his check-raise range is narrow."),
    rule("post.donk.high", "F08", F::Postflop, X, Some("DNK"),
        "Leads into the raiser on {raw}% of flops.",
        "Raise his donk bets more."),
    rule("post.float.high", "F09", F::Postflop, X, Some("FLT"),
        "Bets {raw}% of turns after calling flop c-bets in position.",
        "Check-raise more turns against him."),
    rule("post.probe.high", "F10", F::Postflop, X, Some("PRB"),
        "Bets {raw}% of turns after the raiser checks the flop.",
        "Check back weaker hands and call his probes wider."),
    rule("post.river_bet.high", "F11", F::Postflop, X, Some("RV+"),
        "Bets {raw}% of rivers when checked to or first to act.",
        "Bluff-catch rivers wider against him."),
    rule("post.river_bet.low", "F11", F::Postflop, T, Some("RV-"),
        "Bets only {raw}% of rivers when given the chance.",
        "Fold more to his river bets."),
    rule("post.river_raise.high", "F11", F::Postflop, T, Some("RR+"),
        "Raises {raw}% of river bets he faces.",
        "Value bet thin only with hands that can call his raise."),
    rule("post.station", "F12", F::Postflop, X, Some("STN"),
        "Goes to showdown {raw}% and wins there only {wsd}%.",
        "Value bet thin and stop bluffing him."),
    rule("post.wsd_strong", "F12", F::Postflop, T, Some("WSD"),
        "Wins {raw}% of the showdowns he reaches.",
        "Fold more bluff-catchers to his river bets."),
    rule("post.wwsf_low", "F12", F::Postflop, X, Some("WWS"),
        "Wins only {raw}% of pots after seeing the flop.",
        "Apply postflop pressure; he gives up pots easily."),
    rule("post.wwsf_high", "F12", F::Postflop, T, Some("WWS"),
        "Wins {raw}% of pots after seeing the flop.",
        "Bluff him less postflop."),
    // ---- head-to-head (H01–H03)
    rule("h2h.3bet_vs_hero.high", "H01", F::H2h, X, Some("H2H"),
        "3-bets your opens {raw}% ({hits}/{n}).",
        "4-bet him lighter and open tighter in front of him."),
    rule("h2h.fold_to_hero_3bet.high", "H02", F::H2h, X, Some("F3B"),
        "Folds to your 3-bets {raw}% ({hits}/{n}).",
        "3-bet his opens wider."),
    rule("h2h.fold_to_hero_cbet.high", "H02", F::H2h, X, Some("FCB"),
        "Folds to your flop c-bets {raw}% ({hits}/{n}).",
        "C-bet him at high frequency."),
    rule("h2h.steal_vs_hero.high", "H03", F::H2h, X, Some("TGT"),
        "Steals into your blinds {raw}% ({hits}/{n}).",
        "3-bet his steals wider when you are in the blinds."),
    rule("h2h.defend_vs_hero_steal.high", "H03", F::H2h, T, Some("DEF"),
        "Defends against your steals {raw}% ({hits}/{n}).",
        "Steal tighter into him and value bet more postflop."),
    // ---- showdown-backed (M04, M05): the only reads that may speak about
    // value and bluffs, always with their counts.
    rule("sd.tell.big_is_value", "M04", F::Showdown, X, Some("SDV"),
        "Showed value in {valueCount} of {n} {bucket} bets.",
        "Fold more bluff-catchers to his {bucket} bets."),
    rule("sd.tell.big_is_bluff", "M04", F::Showdown, X, Some("SDB"),
        "Was bluffing in {bluffCount} of {n} shown {bucket} bets.",
        "Call his {bucket} bets down wider."),
    rule("sd.tell.small_is_value", "M04", F::Showdown, X, Some("SDV"),
        "Showed value in {valueCount} of {n} {bucket} bets.",
        "Raise or fold against his small bets; stop paying them off light."),
    rule("sd.shows_bluffs", "M05", F::Showdown, X, Some("SVB"),
        "Was bluffing in {bluffCount} of {n} river bets that reached showdown.",
        "Call his river bets wider."),
    // ---- recent form (R02)
    rule("rec.tilt", "R02", F::Recency, X, Some("TILT"),
        "Played {x} of his last {n} hands after a big lost pot (usual {p0}%).",
        "Value bet wider and give his aggression less credit while this lasts."),
    rule("rec.looser", "R02", F::Recency, T, Some("LSE"),
        "Played {x} of his last {n} hands (usual {p0}%).",
        "Isolate and value bet him wider while this lasts."),
    rule("rec.tighter", "R02", F::Recency, T, Some("NIT"),
        "Played only {x} of his last {n} hands (usual {p0}%).",
        "Give his recent entries more credit."),
];

/// The definition of a rule id.
pub fn rule_def(id: &str) -> &'static RuleDef {
    RULES.iter().find(|r| r.id == id).unwrap_or_else(|| panic!("unknown rule id {id}"))
}

/// The observation template as rendered: a `{raw}%` without its own count
/// gets `({hits}/{n})` after it (section 8: observations name the count).
pub fn observation_template(def: &RuleDef) -> String {
    if def.observation.contains("{hits}") || !def.observation.contains("{raw}%") {
        def.observation.to_string()
    } else {
        def.observation.replacen("{raw}%", "{raw}% ({hits}/{n})", 1)
    }
}

/// Whether a template is an observation or advice (for the guard tests).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TemplateKind {
    Observation,
    Advice,
}

/// Every text template the engine can show: observations as rendered, the
/// default advice, every advice variant and the seat suffix.
pub fn templates() -> Vec<(&'static str, TemplateKind, String)> {
    let mut out = Vec::new();
    for def in RULES {
        out.push((def.id, TemplateKind::Observation, observation_template(def)));
        out.push((def.id, TemplateKind::Advice, def.advice.to_string()));
        for (_, text) in def.variants {
            out.push((def.id, TemplateKind::Advice, text.to_string()));
        }
    }
    out
}

// ------------------------------------------------------------------ results

/// One engine evidence item: the `description_rules` evidence plus the
/// successes and the shrunk value (section 13). `value` and `shrunk` are
/// percentages; `shrunk` is `None` for counts that are not shrunk
/// (showdown tallies, context facts).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineEvidence {
    pub stat_name: String,
    pub value: Option<f64>,
    pub opportunities: i64,
    pub hits: u32,
    pub shrunk: Option<f64>,
}

/// One read: the `RuleResult` contract plus the engine's fields (section
/// 13). Unranked here; ordering and the chip tag are the ranking's job.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineRuleResult {
    pub rule_id: String,
    pub category: RuleCategory,
    pub observation: String,
    pub advice: String,
    pub confidence_pct: Option<u8>,
    pub confidence_tier: ConfidenceTier,
    pub evidence: Vec<EngineEvidence>,
    pub scenario_id: String,
    pub family: Family,
    pub tag: Option<String>,
    /// `deviation × confidence × multiplier` (section 7).
    pub score: f64,
    /// `|shrunk − prior| / scale` of the basis stat; for context and
    /// recency reads the spec's fixed base score.
    #[serde(skip)]
    pub deviation: f64,
    /// Confidence of the read (0–1).
    #[serde(skip)]
    pub confidence: f64,
    /// Context multiplier applied to the score.
    #[serde(skip)]
    pub multiplier: f64,
    /// Context rules that changed this read (`ctx.seat_relation`, advice
    /// variants).
    #[serde(skip)]
    pub adapted_by: Vec<&'static str>,
}

impl EngineRuleResult {
    /// The read in the existing `description_rules` contract.
    pub fn to_rule_result(&self) -> RuleResult {
        RuleResult {
            rule_id: self.rule_id.clone(),
            category: self.category,
            observation: self.observation.clone(),
            advice: self.advice.clone(),
            confidence_pct: self.confidence_pct,
            confidence_tier: self.confidence_tier,
            evidence: self
                .evidence
                .iter()
                .map(|e| Evidence {
                    stat_name: e.stat_name.clone(),
                    value: e.value,
                    opportunities: e.opportunities,
                })
                .collect(),
        }
    }
}

// -------------------------------------------------------------------- input

/// Head-to-head stat as rules see it.
pub type H2hInput = H2hStat;

/// Classified showdown counts of one group of bets.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ShowdownTally {
    pub value: u32,
    pub bluff: u32,
    pub neither: u32,
    pub n: u32,
}

/// Everything the rules read about one villain. Built by [`rule_input`];
/// tests may build it directly.
#[derive(Debug, Clone, Default)]
pub struct RuleInput {
    /// Recency-view stats (rules evaluate `shrunk`, confidence uses the raw
    /// opportunity count).
    pub stats: BTreeMap<StatKey, ShrunkStat>,
    /// Between-hands context from the latest completed hand at the table;
    /// `None` when there is no table scope.
    pub context: Option<EngineContext>,
    pub h2h: Vec<H2hInput>,
    /// Sizing tally per bucket (every bucket, counts kept).
    pub sizing: Vec<SizingTell>,
    /// Showdowns whose last aggression was on the river.
    pub river_showdowns: ShowdownTally,
    pub recent_form: Option<RecentForm>,
    /// `call_vs_shove` over knockout hands only (row K01).
    pub ko_call_vs_shove: Option<ShrunkStat>,
    /// `fold_to_steal_bb` over late-stage tournament hands (row T01).
    pub late_fold_to_steal_bb: Option<ShrunkStat>,
    /// `limp` over early-stage tournament hands (row T01).
    pub early_limp: Option<ShrunkStat>,
}

impl RuleInput {
    pub fn stat(&self, key: StatKey) -> Option<&ShrunkStat> {
        self.stats.get(&key)
    }
}

fn shrunk_of(counts: StatAgg, key: StatKey, prior: f64) -> ShrunkStat {
    ShrunkStat::new(key, counts.hits, counts.opportunities, prior)
}

fn tournament(hand: &HandFacts) -> bool {
    matches!(FormatKey::of_hand(hand), FormatKey::Mtt | FormatKey::Spin)
}

/// The prior-free part of one villain's rule input: everything a replay of
/// his hands yields. Building it is the expensive step (every hand is
/// replayed); [`PlayerReplay::rule_input`] then only shrinks counts toward
/// the current priors, so a cached replay is reused until he plays a new
/// hand.
#[derive(Debug, Clone)]
pub struct PlayerReplay {
    pub player_id: i64,
    /// Format the recency half-life and the priors were chosen for.
    pub format: FormatKey,
    pub agg: PlayerAggregate,
    /// `call_vs_shove` over knockout hands, and how many knockout hands he
    /// played (row K01).
    pub ko: StatAgg,
    pub ko_hands: u32,
    /// `fold_to_steal_bb` over late-stage and `limp` over early-stage
    /// tournament hands (row T01).
    pub late: StatAgg,
    pub early: StatAgg,
    /// Every showdown record, oldest first.
    pub showdowns: Vec<ShowdownRecord>,
    pub form: Option<FormCounts>,
}

impl PlayerReplay {
    /// Replays the villain's hands (oldest first, as `load_player_hands`
    /// returns them) with his showdown records.
    pub fn build(
        hands: &[HandFacts],
        player_id: i64,
        format: FormatKey,
        showdowns: Vec<ShowdownRecord>,
    ) -> PlayerReplay {
        let mut ko = StatAgg::default();
        let mut late = StatAgg::default();
        let mut early = StatAgg::default();
        let mut ko_hands = 0;
        for hand in hands.iter().filter(|h| h.seat_of(player_id).is_some()) {
            let knockout = hand.seats.iter().any(|s| s.bounty.is_some());
            let stage = tournament(hand).then(|| average_stack_bb(hand).map(Stage::of)).flatten();
            if !knockout && stage.is_none() {
                continue;
            }
            ko_hands += u32::from(knockout);
            for event in extract_preflop(hand, player_id).iter().filter(|e| e.opportunity) {
                let target = match (event.key, stage) {
                    (StatKey::CallVsShove, _) if knockout => &mut ko,
                    (StatKey::FoldToStealBb, Some(Stage::Late)) => &mut late,
                    (StatKey::Limp, Some(Stage::Early)) => &mut early,
                    _ => continue,
                };
                target.opportunities += 1;
                target.hits += u32::from(event.success);
            }
        }
        PlayerReplay {
            player_id,
            format,
            agg: aggregate_player(hands, player_id, format),
            ko,
            ko_hands,
            late,
            early,
            showdowns,
            form: form_counts(hands, player_id),
        }
    }

    /// The rule input against the current priors and the between-hands
    /// context. Cheap: no hand is replayed.
    pub fn rule_input(&self, prior: &dyn Fn(StatKey) -> f64, context: Option<EngineContext>) -> RuleInput {
        let agg = &self.agg;
        let stats = StatKey::PREFLOP
            .iter()
            .chain(StatKey::POSTFLOP.iter())
            .map(|key| (*key, agg.stat(View::Recency, *key, prior(*key))))
            .collect();
        let h2h = H2hKey::ALL.iter().map(|key| agg.h2h_stat(*key, prior)).collect();

        let mut river_showdowns = ShowdownTally::default();
        for aggression in self.showdowns.iter().filter_map(|r| r.last_aggression.as_ref()) {
            if aggression.street != Street::River {
                continue;
            }
            match aggression.class {
                ValueClass::Value => river_showdowns.value += 1,
                ValueClass::Bluff => river_showdowns.bluff += 1,
                ValueClass::Neither => river_showdowns.neither += 1,
            }
            river_showdowns.n += 1;
        }

        RuleInput {
            stats,
            context,
            h2h,
            sizing: sizing_tally(&self.showdowns),
            river_showdowns,
            recent_form: self.form.map(|f| f.evaluate(prior(StatKey::Vpip))),
            ko_call_vs_shove: (self.ko_hands > 0)
                .then(|| shrunk_of(self.ko, StatKey::CallVsShove, prior(StatKey::CallVsShove))),
            late_fold_to_steal_bb: (self.late.opportunities > 0)
                .then(|| shrunk_of(self.late, StatKey::FoldToStealBb, prior(StatKey::FoldToStealBb))),
            early_limp: (self.early.opportunities > 0)
                .then(|| shrunk_of(self.early, StatKey::Limp, prior(StatKey::Limp))),
        }
    }
}

/// Builds the rule input from the villain's hands (oldest first, as
/// `load_player_hands` returns them) and his showdown records.
pub fn rule_input(
    hands: &[HandFacts],
    player_id: i64,
    format: FormatKey,
    prior: &dyn Fn(StatKey) -> f64,
    context: Option<EngineContext>,
    showdowns: &[ShowdownRecord],
) -> RuleInput {
    PlayerReplay::build(hands, player_id, format, showdowns.to_vec()).rule_input(prior, context)
}

/// Loads one villain's hands and showdowns and replays them. The format is
/// `format` when given (the table context's), else his latest hand's.
pub fn player_replay(
    conn: &Connection,
    player_id: i64,
    format: Option<FormatKey>,
) -> rusqlite::Result<PlayerReplay> {
    let hands = load_player_hands(conn, player_id)?;
    let boards = load_showdown_boards(conn, player_id)?;
    let showdowns = extract_showdowns(&hands, &boards, player_id);
    let format = format
        .or_else(|| hands.last().map(FormatKey::of_hand))
        .unwrap_or(FormatKey::Cash);
    Ok(PlayerReplay::build(&hands, player_id, format, showdowns))
}

/// Loads one villain's hands and showdowns and builds his rule input.
/// `prior(format, key)` is the (pool-refined) prior; the villain's format is
/// the context's, else his latest hand's.
pub fn player_rule_input(
    conn: &Connection,
    player_id: i64,
    context: Option<EngineContext>,
    prior: &dyn Fn(FormatKey, StatKey) -> f64,
) -> rusqlite::Result<RuleInput> {
    let replay = player_replay(conn, player_id, context.as_ref().map(|c| c.format))?;
    let format = replay.format;
    Ok(replay.rule_input(&|key| prior(format, key), context))
}

// --------------------------------------------------------------- evaluation

fn pct(value: f64) -> f64 {
    (value * 1000.0).round() / 10.0
}

fn tier_of(c: f64) -> ConfidenceTier {
    if c >= 0.60 {
        ConfidenceTier::High
    } else if c >= 0.35 {
        ConfidenceTier::Medium
    } else {
        ConfidenceTier::Low
    }
}

fn render(template: &str, values: &[(&str, String)]) -> String {
    let mut out = template.to_string();
    for (name, value) in values {
        out = out.replace(&format!("{{{name}}}"), value);
    }
    out
}

fn whole_pct(rate: f64) -> String {
    format!("{:.0}", rate * 100.0)
}

/// `12`, `12.4`: one decimal, none when whole.
fn one_decimal(value: f64) -> String {
    let rounded = (value * 10.0).round() / 10.0;
    if rounded.fract() == 0.0 {
        format!("{rounded:.0}")
    } else {
        format!("{rounded:.1}")
    }
}

fn stat_evidence(stat: &ShrunkStat) -> EngineEvidence {
    EngineEvidence {
        stat_name: stat.key.as_str().to_string(),
        value: stat.raw.map(pct),
        opportunities: i64::from(stat.opportunities),
        hits: stat.hits,
        shrunk: Some(pct(stat.shrunk)),
    }
}

fn deviation(stat: &ShrunkStat) -> f64 {
    (stat.shrunk - stat.prior).abs() / stat_spec(stat.key).scale
}

/// Which side of the prior a threshold sits on.
#[derive(Debug, Clone, Copy)]
enum Dir {
    /// `shrunk ≥ prior + delta`.
    Above(f64),
    /// `shrunk ≤ prior − delta`.
    Below(f64),
}

fn passes(stat: &ShrunkStat, dir: Dir) -> bool {
    match dir {
        Dir::Above(delta) => stat.shrunk >= stat.prior + delta - EPS,
        Dir::Below(delta) => stat.shrunk <= stat.prior - delta + EPS,
    }
}

/// What a fired stat rule shows: the displayed rate and counts (possibly
/// the complement, "defends" for a fold stat), the basis stats, deviation
/// and confidence.
struct StatHit {
    raw: f64,
    hits: u32,
    n: u32,
    basis: Vec<ShrunkStat>,
    deviation: f64,
    confidence: f64,
    tier: ConfidenceTier,
}

impl StatHit {
    /// One stat; `complement` shows `1 − raw` and `n − hits`.
    fn single(stat: &ShrunkStat, complement: bool) -> Option<StatHit> {
        let raw = stat.raw?;
        Some(StatHit {
            raw: if complement { 1.0 - raw } else { raw },
            hits: if complement { stat.opportunities - stat.hits } else { stat.hits },
            n: stat.opportunities,
            basis: vec![*stat],
            deviation: deviation(stat),
            confidence: stat.confidence,
            tier: stat.tier,
        })
    }

    /// Adds a secondary basis stat: the read is only as sure as its weaker
    /// leg.
    fn and(mut self, other: &ShrunkStat) -> StatHit {
        if other.confidence < self.confidence {
            self.confidence = other.confidence;
            self.tier = other.tier;
        }
        self.basis.push(*other);
        self
    }
}

/// The displayable stat of `key` that passes `dir`, if any.
fn one(input: &RuleInput, key: StatKey, dir: Dir) -> Option<&ShrunkStat> {
    input.stat(key).filter(|s| s.displayable() && passes(s, dir))
}

/// "a or b": the displayable passing stat with the largest deviation.
fn any_of<'a>(input: &'a RuleInput, keys: &[StatKey], dir: Dir) -> Option<&'a ShrunkStat> {
    keys.iter()
        .filter_map(|k| one(input, *k, dir))
        .fold(None, |best: Option<&ShrunkStat>, s| match best {
            Some(b) if deviation(b) >= deviation(s) => Some(b),
            _ => Some(s),
        })
}

/// "max(a, b) ≤ …" / "min(a, b) ≥ …": every displayable stat passes (at
/// least one). Shown as the combined rate; deviation is the smallest.
fn all_of(input: &RuleInput, keys: &[StatKey], dir: Dir, complement: bool) -> Option<StatHit> {
    let shown: Vec<&ShrunkStat> =
        keys.iter().filter_map(|k| input.stat(*k)).filter(|s| s.displayable()).collect();
    if shown.is_empty() || !shown.iter().all(|s| passes(s, dir)) {
        return None;
    }
    let hits: u32 = shown.iter().map(|s| s.hits).sum();
    let n: u32 = shown.iter().map(|s| s.opportunities).sum();
    let weakest = shown.iter().min_by(|a, b| a.confidence.total_cmp(&b.confidence))?;
    let raw = f64::from(hits) / f64::from(n);
    Some(StatHit {
        raw: if complement { 1.0 - raw } else { raw },
        hits: if complement { n - hits } else { hits },
        n,
        basis: shown.iter().map(|s| **s).collect(),
        deviation: shown.iter().map(|s| deviation(s)).fold(f64::INFINITY, f64::min),
        confidence: weakest.confidence,
        tier: weakest.tier,
    })
}

/// The context facts rules adapt to.
struct Adapt<'a> {
    context: Option<&'a EngineContext>,
}

impl Adapt<'_> {
    fn bucket(&self) -> Option<StackBucket> {
        self.context.and_then(|c| c.stack_bucket)
    }

    fn knockout(&self) -> bool {
        self.context.is_some_and(|c| c.bounty.is_some())
    }

    fn late(&self) -> bool {
        self.context.is_some_and(|c| c.stage == Some(Stage::Late))
    }

    fn hero_covers(&self) -> Option<bool> {
        self.context.and_then(|c| c.bounty.as_ref()).and_then(|b| b.hero_covers)
    }

    /// Section 5: at `push_fold` only stack, short-stack, bounty, recency
    /// and showdown reads apply.
    fn eligible(&self, def: &RuleDef) -> bool {
        if self.bucket() != Some(StackBucket::PushFold) {
            return true;
        }
        match def.family {
            Family::Stack | Family::Bounty | Family::Recency | Family::Showdown => true,
            Family::Context => def.id == "ctx.short_stack",
            Family::Preflop | Family::Postflop | Family::Stage | Family::H2h => false,
        }
    }

    fn multiplier(&self, def: &RuleDef) -> f64 {
        let reshove_family =
            def.id.starts_with("stk.reshove.") || def.id.starts_with("stk.call_shove.");
        if reshove_family && self.bucket() == Some(StackBucket::Reshove) {
            RESHOVE_PRIORITY
        } else if def.family == Family::H2h {
            H2H_PRIORITY
        } else {
            1.0
        }
    }

    /// The advice text and the context rules that chose it.
    fn advice(&self, def: &RuleDef) -> (String, Vec<&'static str>) {
        let mut adapted = Vec::new();
        let wanted = |variant: Variant| match variant {
            Variant::Bounty => self.knockout(),
            Variant::LateStage => self.late(),
            Variant::HeroCovered => self.knockout() && self.hero_covers() == Some(false),
            Variant::HeroCovers => self.knockout() && self.hero_covers() == Some(true),
        };
        let mut advice = def.advice.to_string();
        if let Some((variant, text)) = def.variants.iter().find(|(v, _)| wanted(*v)) {
            advice = text.to_string();
            adapted.push(match variant {
                Variant::LateStage => "stage.late",
                _ => "ko.variant",
            });
        }
        if let (Some(side), Some(seat)) = (def.seat, self.context.and_then(|c| c.seat)) {
            if seat.side == side {
                let place = if seat.direct_left || seat.direct_right { "directly on" } else { "on" };
                let side = match seat.side {
                    Side::Left => "left",
                    Side::Right => "right",
                };
                let suffix = render(SEAT_SUFFIX, &[("where", place.into()), ("side", side.into())]);
                advice = format!("{}{suffix}", advice.trim_end_matches('.'));
                adapted.push(RULE_SEAT_RELATION);
            }
        }
        (advice, adapted)
    }
}

/// How a read's score is formed (section 7).
#[derive(Debug, Clone, Copy)]
enum Score {
    /// Stat reads: `deviation × confidence × multiplier`.
    Deviation(f64),
    /// Context and recency reads: the spec's fixed value × multiplier.
    Fixed(f64),
}

struct Out<'a> {
    adapt: Adapt<'a>,
    reads: Vec<EngineRuleResult>,
}

impl Out<'_> {
    #[allow(clippy::too_many_arguments)]
    fn push(
        &mut self,
        id: &str,
        values: Vec<(&str, String)>,
        evidence: Vec<EngineEvidence>,
        confidence: f64,
        tier: ConfidenceTier,
        score: Score,
        tag: Option<String>,
    ) {
        let def = rule_def(id);
        if !self.adapt.eligible(def) || tier == ConfidenceTier::InsufficientData {
            return;
        }
        let multiplier = self.adapt.multiplier(def);
        let (advice, adapted_by) = self.adapt.advice(def);
        let (deviation, score) = match score {
            Score::Deviation(d) => (d, super::rank::score(d, confidence, multiplier)),
            Score::Fixed(s) => (s, s * multiplier),
        };
        self.reads.push(EngineRuleResult {
            rule_id: def.id.to_string(),
            category: def.category,
            observation: render(&observation_template(def), &values),
            advice: render(&advice, &values),
            confidence_pct: Some((100.0 * confidence).round() as u8),
            confidence_tier: tier,
            evidence,
            scenario_id: def.scenario.to_string(),
            family: def.family,
            tag: tag.or_else(|| def.tag.map(str::to_string)),
            score,
            deviation,
            confidence,
            multiplier,
            adapted_by,
        });
    }

    fn stat(&mut self, id: &str, hit: Option<StatHit>) {
        self.stat_with(id, hit, Vec::new());
    }

    fn stat_with(&mut self, id: &str, hit: Option<StatHit>, mut extra: Vec<(&str, String)>) {
        let Some(hit) = hit else { return };
        extra.extend([
            ("raw", whole_pct(hit.raw)),
            ("hits", hit.hits.to_string()),
            ("n", hit.n.to_string()),
        ]);
        let evidence = hit.basis.iter().map(stat_evidence).collect();
        self.push(id, extra, evidence, hit.confidence, hit.tier, Score::Deviation(hit.deviation), None);
    }
}

fn single(stat: Option<&ShrunkStat>) -> Option<StatHit> {
    stat.and_then(|s| StatHit::single(s, false))
}

fn defended(stat: Option<&ShrunkStat>) -> Option<StatHit> {
    stat.and_then(|s| StatHit::single(s, true))
}

/// Evaluates every catalogue rule for one villain, adapted to his context.
/// Reads come in catalogue order, unranked; reads below their sample never
/// appear.
pub fn evaluate(input: &RuleInput) -> Vec<EngineRuleResult> {
    use StatKey::*;
    let mut out = Out { adapt: Adapt { context: input.context.as_ref() }, reads: Vec::new() };

    // ---- profile (P12): shrunk VPIP / PFR / WTSD against the format's
    // priors (D107). The deltas are the D106 6-max cash anchors (.15, .40,
    // a .15 gap, .32, .32/.25) expressed relative to the cash priors, so a
    // Spin (VPIP prior .38) is no longer read on cash numbers.
    let shown = |key| input.stat(key).filter(|s| s.displayable());
    if let (Some(vpip), Some(pfr)) = (shown(Vpip), shown(Pfr)) {
        let gap = vpip.shrunk - pfr.shrunk;
        let prior_gap = vpip.prior - pfr.prior;
        let passive = vpip.shrunk >= vpip.prior + PROFILE_LOOSE - EPS && gap >= prior_gap + PROFILE_PASSIVE_GAP - EPS;
        let wtsd = shown(Wtsd).filter(|w| w.shrunk >= w.prior + PROFILE_STATION_WTSD - EPS);
        if vpip.shrunk <= vpip.prior - PROFILE_NIT + EPS {
            out.stat("pf.profile.nit", single(Some(vpip)));
        }
        match (passive, wtsd) {
            (true, Some(wtsd)) => out.stat("pf.profile.station", single(Some(vpip)).map(|h| h.and(pfr).and(wtsd))),
            (true, None) => out.stat("pf.profile.loose_passive", single(Some(vpip)).map(|h| h.and(pfr))),
            _ => {}
        }
        // "raises most of them": PFR at least half of VPIP.
        if vpip.shrunk >= vpip.prior + PROFILE_LAG_VPIP - EPS
            && pfr.shrunk >= pfr.prior + PROFILE_LAG_PFR - EPS
            && pfr.shrunk >= 0.5 * vpip.shrunk - EPS
        {
            out.stat("pf.profile.lag", single(Some(vpip)).map(|h| h.and(pfr)));
        }
    }

    // ---- opens and steals (P01, P02)
    out.stat("pf.rfi.tight_early", single(any_of(input, &[RfiEp, RfiMp], Dir::Below(0.06))));
    out.stat("pf.rfi.loose_early", single(any_of(input, &[RfiEp, RfiMp], Dir::Above(0.08))));
    out.stat("pf.rfi.loose_late", single(any_of(input, &[RfiCo, RfiBtn], Dir::Above(0.12))));
    out.stat("pf.rfi.tight_late", single(any_of(input, &[RfiCo, RfiBtn], Dir::Below(0.12))));
    out.stat("pf.steal.high", single(one(input, Steal, Dir::Above(0.12))));
    out.stat("pf.steal.low", single(one(input, Steal, Dir::Below(0.12))));
    out.stat("pf.rfi_sb.wide", single(one(input, RfiSb, Dir::Above(0.15))));

    // ---- blind defence (P03, P04)
    out.stat(
        "pf.fold_to_steal.high",
        single(any_of(input, &[FoldToStealSb, FoldToStealBb], Dir::Above(0.12))),
    );
    out.stat("pf.fold_to_steal.low", defended(one(input, FoldToStealBb, Dir::Below(0.15))));
    out.stat("pf.bb_vs_sb.overfold", single(one(input, BbDefendVsSb, Dir::Below(0.15))));
    out.stat("pf.bb_vs_sb.overdefend", single(one(input, BbDefendVsSb, Dir::Above(0.15))));

    // ---- 3-bets, 4-bets, squeezes (P05–P08)
    out.stat("pf.3bet.high_ip", single(one(input, ThreeBetIp, Dir::Above(0.05))));
    out.stat("pf.3bet.high_oop", single(one(input, ThreeBetOop, Dir::Above(0.05))));
    out.stat("pf.3bet.low", all_of(input, &[ThreeBetIp, ThreeBetOop], Dir::Below(0.03), false));
    out.stat(
        "pf.fold_to_3bet.high",
        single(any_of(input, &[FoldTo3betIp, FoldTo3betOop], Dir::Above(0.12))),
    );
    out.stat("pf.fold_to_3bet.low", all_of(input, &[FoldTo3betIp, FoldTo3betOop], Dir::Below(0.15), true));
    out.stat("pf.4bet.high", single(one(input, FourBet, Dir::Above(0.06))));
    out.stat("pf.fold_to_4bet.high", single(one(input, FoldTo4bet, Dir::Above(0.20))));
    out.stat("pf.squeeze.high", single(one(input, Squeeze, Dir::Above(0.05))));

    // ---- limps and cold calls (P09–P11)
    out.stat("pf.limp.high", single(one(input, Limp, Dir::Above(0.08))));
    out.stat("pf.iso.high", single(one(input, IsoRaise, Dir::Above(0.15))));
    out.stat("pf.limp_fold.high", single(one(input, LimpFold, Dir::Above(0.20))));
    out.stat("pf.limp_call.high", single(one(input, LimpCall, Dir::Above(0.20))));
    out.stat(
        "pf.limp_reraise.seen",
        single(shown(LimpReraise).filter(|s| s.shrunk >= 0.15 - EPS && s.hits >= 2)),
    );
    out.stat("pf.cold_call.high", single(one(input, ColdCall, Dir::Above(0.07))));

    // ---- stack depth (S01–S03)
    out.stat("stk.open_shove.wide", single(one(input, OpenShove, Dir::Above(0.15))));
    out.stat("stk.open_shove.tight", single(one(input, OpenShove, Dir::Below(0.10))));
    out.stat("stk.call_shove.wide", single(one(input, CallVsShove, Dir::Above(0.12))));
    out.stat("stk.call_shove.tight", single(one(input, CallVsShove, Dir::Below(0.10))));
    out.stat("stk.reshove.high", single(one(input, Reshove, Dir::Above(0.08))));
    out.stat("stk.reshove.low", single(one(input, Reshove, Dir::Below(0.06))));

    // ---- context (S04, S05): facts of the latest completed hand.
    if let Some(ctx) = input.context.as_ref() {
        if let Some(eff) = ctx.effective_stack_bb {
            let evidence = vec![EngineEvidence {
                stat_name: "effective_stack_bb".into(),
                value: Some(eff),
                opportunities: 1,
                hits: 0,
                shrunk: None,
            }];
            let tournament = matches!(ctx.format, FormatKey::Mtt | FormatKey::Spin);
            if tournament && ctx.stack_bucket == Some(StackBucket::PushFold) {
                let tag = format!("{}bb", eff.floor() as u32);
                out.push("ctx.short_stack", vec![("eff", one_decimal(eff))], evidence.clone(), 1.0,
                    ConfidenceTier::High, Score::Fixed(2.5), Some(tag));
            }
            if ctx.stack_bucket == Some(StackBucket::Deep) {
                out.push("ctx.deep_stack", vec![("eff", one_decimal(eff))], evidence, 1.0,
                    ConfidenceTier::High, Score::Fixed(0.5), None);
            }
        }

        // ---- stage (T01): the villain's own stage-specific frequencies.
        if ctx.stage == Some(Stage::Late) {
            out.stat(
                "stage.late.overfolds_blinds",
                single(input.late_fold_to_steal_bb.as_ref().filter(|s| s.displayable() && passes(s, Dir::Above(0.10)))),
            );
        }
        if ctx.stage == Some(Stage::Early) {
            out.stat(
                "stage.early.passive",
                single(input.early_limp.as_ref().filter(|s| s.displayable() && passes(s, Dir::Above(0.06)))),
            );
        }

        // ---- bounty (K01)
        if let Some(bounty) = ctx.bounty.as_ref().filter(|b| b.is_big()) {
            let ratio = bounty.ratio.unwrap_or_default();
            let symbol = match bounty.currency.as_deref() {
                Some("EUR") => "€",
                Some("USD") => "$",
                Some("GBP") => "£",
                _ => "",
            };
            let evidence = vec![EngineEvidence {
                stat_name: "bounty_ratio".into(),
                value: Some(ratio),
                opportunities: 1,
                hits: 0,
                shrunk: None,
            }];
            out.push(
                "ko.big_bounty",
                vec![("ratio", one_decimal(ratio)), ("amount", format!("{symbol}{:.2}", bounty.amount))],
                evidence,
                1.0,
                ConfidenceTier::High,
                Score::Fixed((1.0 + ratio / 2.0).min(3.0)),
                None,
            );
        }
    }
    out.stat(
        "ko.hunts_bounties",
        single(input.ko_call_vs_shove.as_ref().filter(|s| s.displayable() && passes(s, Dir::Above(0.15)))),
    );

    // ---- postflop (F01–F12)
    out.stat("post.cbet_flop.high", single(one(input, CbetFlop, Dir::Above(0.15))));
    out.stat("post.cbet_flop.low", single(one(input, CbetFlop, Dir::Below(0.15))));
    out.stat("post.fold_cbet_flop.high", single(one(input, FoldToCbetFlop, Dir::Above(0.12))));
    out.stat("post.fold_cbet_flop.low", single(one(input, FoldToCbetFlop, Dir::Below(0.15))));
    out.stat("post.cbet_turn.high", single(one(input, CbetTurn, Dir::Above(0.15))));
    out.stat("post.cbet_turn.giveup", single(one(input, CbetTurn, Dir::Below(0.15))));
    out.stat("post.fold_cbet_turn.high", single(one(input, FoldToCbetTurn, Dir::Above(0.15))));
    out.stat("post.cbet_river.high", single(one(input, CbetRiver, Dir::Above(0.15))));
    out.stat("post.cbet_river.low", single(one(input, CbetRiver, Dir::Below(0.15))));
    out.stat("post.fold_cbet_river.low", single(one(input, FoldToCbetRiver, Dir::Below(0.15))));
    out.stat("post.fold_cbet_river.high", single(one(input, FoldToCbetRiver, Dir::Above(0.15))));
    out.stat("post.delayed_cbet.high", single(one(input, DelayedCbet, Dir::Above(0.15))));
    out.stat("post.check_raise.high", single(one(input, CheckRaiseFlop, Dir::Above(0.06))));
    out.stat(
        "post.check_raise.never",
        single(shown(CheckRaiseFlop).filter(|s| s.opportunities >= 20 && s.shrunk <= 0.02 + EPS)),
    );
    out.stat("post.donk.high", single(one(input, DonkFlop, Dir::Above(0.06))));
    out.stat("post.float.high", single(one(input, FloatFlop, Dir::Above(0.15))));
    out.stat("post.probe.high", single(one(input, ProbeTurn, Dir::Above(0.15))));
    out.stat("post.river_bet.high", single(one(input, RiverBet, Dir::Above(0.12))));
    out.stat("post.river_bet.low", single(one(input, RiverBet, Dir::Below(0.12))));
    out.stat("post.river_raise.high", single(one(input, RiverRaise, Dir::Above(0.06))));
    if let (Some(wtsd), Some(wsd)) = (one(input, Wtsd, Dir::Above(0.10)), one(input, Wsd, Dir::Below(0.06))) {
        let wsd_raw = whole_pct(wsd.raw.unwrap_or_default());
        out.stat_with("post.station", single(Some(wtsd)).map(|h| h.and(wsd)), vec![("wsd", wsd_raw)]);
    }
    if let (Some(wsd), Some(wtsd)) = (one(input, Wsd, Dir::Above(0.08)), one(input, Wtsd, Dir::Below(0.0))) {
        out.stat("post.wsd_strong", single(Some(wsd)).map(|h| h.and(wtsd)));
    }
    out.stat("post.wwsf_low", single(one(input, Wwsf, Dir::Below(0.08))));
    out.stat("post.wwsf_high", single(one(input, Wwsf, Dir::Above(0.08))));

    // ---- head-to-head (H01–H03): shrunk toward the villain's own overall
    // value, n ≥ 10.
    for (key, id, delta) in [
        (H2hKey::ThreeBetVsHeroOpen, "h2h.3bet_vs_hero.high", 0.05),
        (H2hKey::FoldToHeroThreeBet, "h2h.fold_to_hero_3bet.high", 0.12),
        (H2hKey::FoldToHeroCbet, "h2h.fold_to_hero_cbet.high", 0.12),
        (H2hKey::StealVsHero, "h2h.steal_vs_hero.high", 0.12),
        (H2hKey::DefendVsHeroSteal, "h2h.defend_vs_hero_steal.high", 0.15),
    ] {
        let Some(stat) = input.h2h.iter().find(|s| s.key == key) else { continue };
        let Some(raw) = stat.raw else { continue };
        if !stat.rule_eligible() || stat.shrunk < stat.villain_shrunk + delta - EPS {
            continue;
        }
        let spec = stat_spec(key.basis().0[0]);
        let c = confidence(stat.opportunities, spec.k);
        let evidence = vec![EngineEvidence {
            stat_name: key.as_str().to_string(),
            value: Some(pct(raw)),
            opportunities: i64::from(stat.opportunities),
            hits: stat.hits,
            shrunk: Some(pct(stat.shrunk)),
        }];
        let values = vec![
            ("raw", whole_pct(raw)),
            ("hits", stat.hits.to_string()),
            ("n", stat.opportunities.to_string()),
        ];
        let d = (stat.shrunk - stat.villain_shrunk).abs() / spec.scale;
        out.push(id, values, evidence, c, tier_of(c), Score::Deviation(d), None);
    }

    // ---- showdown-backed tells (M04, M05)
    let big = [SizeBucket::Large, SizeBucket::Overbet, SizeBucket::AllIn];
    let small = [SizeBucket::Small, SizeBucket::Medium];
    for (id, buckets, want_value, min_share) in [
        ("sd.tell.big_is_value", &big[..], true, 0.75),
        ("sd.tell.big_is_bluff", &big[..], false, 0.50),
        ("sd.tell.small_is_value", &small[..], true, 0.75),
    ] {
        let count = |t: &SizingTell| if want_value { t.value } else { t.bluff };
        let share = |t: &SizingTell| f64::from(count(t)) / f64::from(t.n);
        let best = input
            .sizing
            .iter()
            .filter(|t| buckets.contains(&t.bucket) && t.n >= SD_TELL_RULE_MIN)
            .filter(|t| share(t) >= min_share - EPS)
            .max_by(|a, b| share(a).total_cmp(&share(b)).then(a.n.cmp(&b.n)));
        if let Some(t) = best {
            let hits = count(t);
            push_showdown(&mut out, id, bucket_text(t.bucket), t.value, t.bluff, t.n, hits);
        }
    }
    let river = input.river_showdowns;
    if river.n >= SD_RIVER_BLUFF_MIN && f64::from(river.bluff) / f64::from(river.n) >= 0.40 - EPS {
        push_showdown(&mut out, "sd.shows_bluffs", "river", river.value, river.bluff, river.n, river.bluff);
    }

    // ---- recent form (R02): only when window and baseline are large enough.
    if let Some(form) = input
        .recent_form
        .as_ref()
        .filter(|f| f.window >= TILT_MIN_WINDOW && f.baseline_opportunities >= TILT_MIN_BASELINE)
    {
        if let Some(flag) = form.flag {
            let base = form.z.abs().min(5.0);
            let d = if flag == FormFlag::Tilt { base } else { base * 0.8 };
            let k = stat_spec(Vpip).k;
            let c = confidence(form.baseline_opportunities, k);
            let values = vec![
                ("x", form.window_hits.to_string()),
                ("n", form.window.to_string()),
                ("p0", format!("{:.0}", form.baseline_vpip_pct)),
            ];
            let evidence = vec![EngineEvidence {
                stat_name: "vpip_last_window".into(),
                value: Some(form.window_vpip_pct),
                opportunities: i64::from(form.window),
                hits: form.window_hits,
                shrunk: Some(form.baseline_vpip_pct),
            }];
            out.push(flag.rule_id(), values, evidence, c, tier_of(c), Score::Fixed(d), None);
        }
    }

    out.reads
}

fn bucket_text(bucket: SizeBucket) -> &'static str {
    match bucket {
        SizeBucket::Small => "small",
        SizeBucket::Medium => "medium-sized",
        SizeBucket::Large => "large",
        SizeBucket::Overbet => "overbet-sized",
        SizeBucket::AllIn => "all-in",
    }
}

fn push_showdown(out: &mut Out, id: &str, bucket: &str, value: u32, bluff: u32, n: u32, hits: u32) {
    let share = f64::from(hits) / f64::from(n);
    let c = f64::from(n) / f64::from(n + 4);
    let values = vec![
        ("valueCount", value.to_string()),
        ("bluffCount", bluff.to_string()),
        ("n", n.to_string()),
        ("bucket", bucket.to_string()),
    ];
    let evidence = vec![EngineEvidence {
        stat_name: format!("showdown_{}", bucket.replace([' ', '-'], "_")),
        value: Some(pct(share)),
        opportunities: i64::from(n),
        hits,
        shrunk: None,
    }];
    // Section 7: 4 × |share − 0.5| × n/(n + 4); the confidence is n/(n + 4).
    out.push(id, values, evidence, c, tier_of(c), Score::Deviation(4.0 * (share - 0.5).abs()), None);
}
