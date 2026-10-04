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
use super::context::{
    average_stack_bb, reseats_every_hand, EngineContext, SeatRelation, Side, Stage, StackBucket,
    PUSH_FOLD_MAX_BB,
};
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

/// Spec id of the advice-quality guard (catalogue row G05, section 17.1):
/// every advice fits the hover card and names a concrete lever, and
/// opposite preflop reads never fire together.
pub const RULE_ADVICE_QUALITY: &str = "guard.advice_quality";

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

/// Preflop thresholds changed by the 2026-10 audit (section 17), as deltas
/// from the format's prior; arrays are `[MTT, 6-max cash, Zoom, Spin & Go]`.
/// `pf.fold_to_steal.high`, the small blind's leg (the big blind's stays .12).
pub const FOLD_TO_STEAL_SB_HIGH: f64 = 0.15;
/// `pf.fold_to_steal.low`: fold_to_steal_bb at most prior minus this.
pub const FOLD_TO_STEAL_LOW: [f64; 4] = [0.25, 0.20, 0.20, 0.20];
/// `pf.bb_vs_sb.overdefend`: bb_defend_vs_sb at least prior plus this.
pub const BB_VS_SB_OVERDEFEND: [f64; 4] = [0.25, 0.20, 0.20, 0.20];
/// `pf.limp.high`: limp at least prior plus this.
pub const LIMP_HIGH: [f64; 4] = [0.08, 0.08, 0.08, 0.12];
/// `stage.early.passive`: early-stage limp at least prior plus this. Spin
/// takes the + .12 of [`LIMP_HIGH`] (its small blind limps by design); cash
/// and Zoom have no stage.
pub const STAGE_EARLY_LIMP: [f64; 4] = [0.06, 0.06, 0.06, 0.12];
/// `stage.late.calls_shoves` (row T02): late-stage MTT `call_vs_shove` at
/// least prior plus this. Pay jumps make correct late calls tighter than
/// the all-stage prior, so the bar sits under the general + .12.
pub const LATE_CALLS_SHOVES: f64 = 0.10;
/// `stage.late.shoves_wide` (row T02): late-stage MTT `open_shove` at least
/// prior plus this, the general `stk.open_shove.wide` delta.
pub const LATE_SHOVES_WIDE: f64 = 0.15;
/// `ko.calls_when_covering` (row K02): knockout calls of shoves he covers
/// at least prior plus [`KO_COVERING_HIGH`], and at least [`KO_COVER_GAP`]
/// above his calls of shoves that cover him.
pub const KO_COVERING_HIGH: f64 = 0.15;
pub const KO_COVER_GAP: f64 = 0.15;

/// Upper bound of the 3-bet-jam depth (section 5): above push/fold and up
/// to this effective stack, preflop re-raises are all-ins, so the
/// [`Variant::JamDepth`] advice replaces 3-bet/4-bet sizing advice.
pub const JAM_DEPTH_MAX_BB: f64 = 40.0;

/// Character budget of one advice block on the hover card at the medium
/// table (section 17.1): `hoverCardMetrics(800, 570)` gives a 248px card,
/// whose 7px padding and 1px border leave 232px of text; the block is
/// clamped to two lines (`HoverReadCard.module.css`). Two lines are 464px;
/// 40px are kept for the word a line break pushes down; advice text in the
/// card's font (Segoe UI Variable, 11px, weight 500) measures 4.9px per
/// character in headless Edge, so `floor(424 / 4.9) = 86`. Measured over
/// the seat suffix too: every advice of up to 90 characters stayed on two
/// lines.
pub const HOVER_CARD_ADVICE_BUDGET: usize = 86;

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
    /// Effective stack above push/fold and at most [`JAM_DEPTH_MAX_BB`]:
    /// re-raises are all-ins, not sized 3-bets and 4-bets.
    JamDepth,
    /// MTT or Spin & Go: smaller opens (min-raises) and isolations than in
    /// cash, because of antes.
    Tournament,
    /// Effective stack at push/fold depth (15bb or less): steals are
    /// shoves (audit 2026-10; the late-stage blind read stays eligible there).
    PushFold,
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
/// `directly on`. Kept short (section 17.1): the advice and its suffix share
/// the hover card's two-line advice block.
pub const SEAT_SUFFIX: &str = " ({where} your {side}).";

/// Every rule of section 8, in catalogue order.
pub const RULES: &[RuleDef] = &[
    // ---- preflop profile (P12). Audit 2026-10 (section 17): every preflop
    // advice names its lever (hand classes, sizing, seat) and switches to
    // jam wording at 15–40bb and to min-raise sizing in tournaments.
    with(
        rule("pf.profile.nit", "P12", F::Preflop, T, Some("NIT"),
            "Plays few hands: VPIP {raw}% ({hits}/{n}).",
            "Steal his blinds with any two suited; fold AJo and KQo to his opens."),
        &[
            (Variant::LateStage, "Min-raise any two into his blinds; call his shoves with TT+ and AQ+ only."),
            (Variant::Tournament, "Min-raise his blinds with any two suited; fold AJo and KQo to his opens."),
        ],
    ),
    rule("pf.profile.station", "P12", F::Preflop, X, Some("STN"),
        "Enters {raw}% of pots and reaches showdown often.",
        "Value bet top pair on all three streets; cut your river bluffs."),
    rule("pf.profile.loose_passive", "P12", F::Preflop, X, Some("LP"),
        "Enters {raw}% of pots, mostly by calling.",
        "Isolate his limps with broadways and pairs; value bet top pair thin."),
    with(
        rule("pf.profile.lag", "P12", F::Preflop, T, Some("LAG"),
            "Plays {raw}% of hands and raises most of them.",
            "3-bet him for value with 99+, AJs+, KQs and AQo; call down with top pair."),
        &[(Variant::JamDepth, "Reshove 88+, AT+ and KQ over his opens; call down with top pair.")],
    ),
    // ---- opens and steals (P01, P02)
    right(rule("pf.rfi.tight_early", "P01", F::Preflop, T, Some("NIT"),
        "Opens {raw}% from early and middle seats.",
        "Fold AJo, KQo and ATs to his early opens.")),
    right(with(
        rule("pf.rfi.loose_early", "P01", F::Preflop, X, Some("LSE"),
            "Opens {raw}% from early and middle seats.",
            "3-bet his early opens with 99+, AJs+, KQs and AQo."),
        &[(Variant::JamDepth, "Reshove 99+ and AJ+ over his early opens.")],
    )),
    right(with(
        rule("pf.rfi.loose_late", "P02", F::Preflop, X, Some("ST+"),
            "Opens {raw}% from the cutoff and button.",
            "3-bet his late opens wider for value: 88+, ATs+, KJs+, AQo."),
        &[(Variant::JamDepth, "Reshove 66+, A9+ and KJ+ over his late opens.")],
    )),
    right(rule("pf.rfi.tight_late", "P02", F::Preflop, T, Some("ST-"),
        "Opens only {raw}% from the cutoff and button.",
        "Defend tighter: fold KTo, QJo and A9o to his late opens.")),
    right(with(
        rule("pf.steal.high", "P02", F::Preflop, X, Some("ST+"),
            "Steals {raw}% when folded to in late position.",
            "Defend your big blind with any suited king vs his steals."),
        &[(Variant::JamDepth, "Reshove 55+, A7+ and K9s+ over his steals from the blinds.")],
    )),
    right(rule("pf.steal.low", "P02", F::Preflop, T, Some("ST-"),
        "Steals only {raw}% when folded to in late position.",
        "From the blinds, fold offsuit Kx and Qx to his steals.")),
    // Pro review (D107, P13): the small blind's open against the big blind.
    right(with(
        rule("pf.rfi_sb.wide", "P13", F::Preflop, X, Some("SB+"),
            "Opens {raw}% of his small blinds when folded to.",
            "3-bet his small-blind opens with 77+, A8s+, KTs+ and ATo+."),
        &[(Variant::JamDepth, "Reshove 66+, A8+ and KJ+ over his small-blind opens.")],
    )),
    // ---- blind defence (P03, P04)
    left(with(
        rule("pf.fold_to_steal.high", "P03", F::Preflop, X, Some("FTS"),
            "Folds his blinds to {raw}% of steals.",
            "Steal any two suited from the button into his blinds."),
        &[
            (Variant::LateStage, "Min-raise any two from the button into his blinds."),
            (Variant::Tournament, "Min-raise any two suited from late seats into his blinds."),
        ],
    )),
    left(with(
        rule("pf.fold_to_steal.low", "P03", F::Preflop, T, Some("DEF"),
            "Defends his big blind against {raw}% of steals.",
            "Cut K8o and Q8o from your steals; open 3x for value."),
        &[(Variant::Tournament, "Cut K8o and Q8o from your steals; open 2.5x for value.")],
    )),
    left(with(
        rule("pf.bb_vs_sb.overfold", "P04", F::Preflop, X, Some("FTS"),
            "Defends his big blind against small-blind opens only {raw}%.",
            "Raise any two from the small blind into him at 2.5x."),
        &[
            (Variant::LateStage, "Min-raise or shove any two from the small blind into him."),
            (Variant::Tournament, "Min-raise any two from the small blind into him."),
        ],
    )),
    left(with(
        rule("pf.bb_vs_sb.overdefend", "P04", F::Preflop, T, Some("OVD"),
            "Defends his big blind against small-blind opens {raw}%.",
            "Cut K5o-type hands from your small-blind opens; raise 3.5x."),
        &[(Variant::Tournament, "Limp K5o-type hands from the small blind; raise 3x for value.")],
    )),
    // ---- 3-bets, 4-bets, squeezes (P05–P08)
    left(with(
        rule("pf.3bet.high_ip", "P05", F::Preflop, X, Some("3B+"),
            "3-bets in position {raw}% of the time.",
            "4-bet his 3-bets with QQ+, AK and A5s-A4s as bluffs."),
        &[(Variant::JamDepth, "Jam over his 3-bets with 99+, AQ+ and AJs.")],
    )),
    left(with(
        rule("pf.3bet.high_oop", "P05", F::Preflop, X, Some("3B+"),
            "3-bets from out of position {raw}% of the time.",
            "Call his 3-bets in position with 77-JJ, AQ, AJs and KQs."),
        &[(Variant::JamDepth, "Jam 88+, AJ+ and KQs over his blind 3-bets.")],
    )),
    left(rule("pf.3bet.low", "P05", F::Preflop, T, Some("3B-"),
        "Rarely 3-bets: {raw}%.",
        "Fold JJ and AQ to his 3-bets; open more suited hands.")),
    right(with(
        rule("pf.fold_to_3bet.high", "P06", F::Preflop, X, Some("F3B"),
            "Folds to {raw}% of 3-bets.",
            "3-bet his opens light with A2s-A5s, K9s+ and 65s+."),
        &[(Variant::JamDepth, "Reshove A2s+, K9s+ and 55+ over his opens.")],
    )),
    right(with(
        rule("pf.fold_to_3bet.low", "P06", F::Preflop, T, Some("C3B"),
            "Continues against {raw}% of 3-bets.",
            "3-bet his opens at 4x, for value only: no 65s-type bluffs."),
        &[(Variant::JamDepth, "Reshove his opens for value only; no A2s-A5s jams.")],
    )),
    right(with(
        rule("pf.4bet.high", "P07", F::Preflop, X, Some("4B+"),
            "4-bets {raw}% of the time against 3-bets.",
            "Jam TT+ and AQ+ over his 4-bets; fold your light 3-bets."),
        &[(Variant::JamDepth, "Jam, don't 3-bet small; call his 4-bet jam with 99+ and AQ+.")],
    )),
    left(with(
        rule("pf.fold_to_4bet.high", "P07", F::Preflop, X, Some("F4B"),
            "Folds to {raw}% of 4-bets after 3-betting.",
            "4-bet bluff his 3-bets with A2s-A5s, sized 2.2x."),
        &[(Variant::JamDepth, "4-bet jam over his 3-bets with A2s+ and K9s+.")],
    )),
    left(with(
        rule("pf.squeeze.high", "P08", F::Preflop, X, Some("SQZ"),
            "Squeezes {raw}% when there is an open and a caller.",
            "4-bet his squeezes with TT+ and AK; flat less with KJo."),
        &[(Variant::JamDepth, "Jam 99+ and AQ+ over his squeezes; flat less with KJo.")],
    )),
    // ---- limps and cold calls (P09–P11)
    right(rule("pf.limp.high", "P09", F::Preflop, X, Some("LMP"),
        "Open-limps {raw}% of hands.",
        "Isolate his limps with pairs, suited aces and broadways.")),
    left(rule("pf.iso.high", "P09", F::Preflop, T, Some("ISO"),
        "Raises over limpers {raw}% of the time.",
        "Raise or fold over limpers; limp-reraise QQ+ and AK.")),
    right(rule("pf.limp_fold.high", "P10", F::Preflop, X, Some("LF"),
        "Folds to {raw}% of raises after limping.",
        "Isolate his limps to 3x with any two suited cards.")),
    right(rule("pf.limp_reraise.seen", "P10", F::Preflop, T, Some("LRR"),
        "Has limp-reraised {hits} of {n} times after limping.",
        "Fold all but QQ+ and AK to his limp-reraise.")),
    // Pro review (D107, P14): the sticky limper.
    right(with(
        rule("pf.limp_call.high", "P14", F::Preflop, X, Some("LPC"),
            "Calls {raw}% of raises after limping.",
            "Isolate his limps to 5x with broadways and pairs; bluff less."),
        &[(Variant::Tournament, "Isolate his limps to 4x with broadways and pairs; bluff less.")],
    )),
    left(with(
        rule("pf.cold_call.high", "P11", F::Preflop, X, Some("CC+"),
            "Cold-calls {raw}% of opens.",
            "Open 3x for value; squeeze his flats with 99+, AQ+ and A5s."),
        &[
            (Variant::JamDepth, "Squeeze-jam 88+ and AJ+ when he flats an open."),
            (Variant::Tournament, "Open 2.5x for value; squeeze his flats with 99+, AQ+ and A5s."),
        ],
    )),
    // ---- stack depth (S01–S03). Audit 2026-10 (section 17): call and
    // shove ranges are named by hand class and depth, with the bounty and
    // cover condition where a knockout changes them.
    right(rule("stk.open_shove.wide", "S01", F::Stack, X, Some("SHV"),
        "Open-shoves {raw}% when folded to at 15bb or less.",
        "Call his shoves from the blinds with 22+, any ace and K9s+.")),
    right(rule("stk.open_shove.tight", "S01", F::Stack, T, Some("NSV"),
        "Open-shoves only {raw}% when folded to at 15bb or less.",
        "Call his rare shoves only with 88+, AJ+ and KQs.")),
    with(
        rule("stk.call_shove.wide", "S02", F::Stack, X, Some("CLS"),
            "Calls {raw}% of all-in shoves.",
            "Shove only pairs, aces and broadways into him; no K5o or 65s-type shoves."),
        &[(Variant::Bounty, "Shove only pairs, aces and broadways; tighter still when he covers you.")],
    ),
    with(
        rule("stk.call_shove.tight", "S02", F::Stack, X, Some("TCS"),
            "Calls only {raw}% of all-in shoves.",
            "Shove any two into him at 10bb or less; any pair, ace or suited king up to 20bb."),
        &[(Variant::Bounty, "Shove any two into him at 10bb or less, tighter when he covers your bounty.")],
    ),
    left(rule("stk.reshove.high", "S03", F::Stack, X, Some("RSV"),
        "Reshoves {raw}% over opens at 15–25bb.",
        "Call his reshoves with 66+, A9+ and KQ; cut K7o-type opens.")),
    left(rule("stk.reshove.low", "S03", F::Stack, T, Some("RS-"),
        "Reshoves only {raw}% over opens at 15–25bb.",
        "Min-raise wider into him; call his reshove with JJ+ and AK.")),
    // ---- context (S04, S05, C01)
    rule("ctx.short_stack", "S04", F::Context, T, Some("NNbb"),
        "Sits on {eff}bb effective after the last hand.",
        "Shove or fold against him; don't min-raise hands you'd fold to his jam."),
    rule("ctx.deep_stack", "S05", F::Context, T, Some("DEEP"),
        "Sits on {eff}bb effective after the last hand.",
        "Flat his opens in position with pairs and suited connectors; 4-bet bluff less."),
    rule(RULE_SEAT_RELATION, "C01", F::Context, T, None,
        "Sits {distance} seats to your {side}.",
        SEAT_SUFFIX),
    // ---- tournament stage (T01) and bounties (K01)
    with(
        rule("stage.late.overfolds_blinds", "T01", F::Stage, X, Some("FTS"),
            "Folds his big blind to {raw}% of steals late in the tournament.",
            "Min-raise any two from the button or small blind into his big blind."),
        &[(Variant::PushFold, "Shove any two from the button or small blind into his big blind at 10bb or less.")],
    ),
    rule("stage.early.passive", "T01", F::Stage, X, Some("LMP"),
        "Open-limps {raw}% in the early levels.",
        "Isolate his limps to 4x with pairs, suited aces and broadways."),
    // ---- late-stage push/fold (T02): ICM-insensitive calls and shoves,
    // MTT only (a Spin & Go pays the winner, so it has no ICM).
    rule("stage.late.calls_shoves", "T02", F::Stage, X, Some("ICM-"),
        "Calls {raw}% of all-in shoves late in the tournament.",
        "Late, shove into him for value only: 55+, A8+, KJ+; no K5o or 65s-type shoves."),
    rule("stage.late.shoves_wide", "T02", F::Stage, X, Some("SHV"),
        "Open-shoves {raw}% when folded to late in the tournament.",
        "Call his late shoves with 22+, A2+, K9s+ if you cover him; 66+, A9+, KQ if not."),
    with(
        rule("ko.big_bounty", "K01", F::Bounty, T, Some("KO"),
            "Carries a {ratio}× bounty ({amount}).",
            "When you cover him, call his shoves with any pair, any ace and K9s+."),
        &[(Variant::HeroCovered, "You don't cover him: ignore his bounty and call his shoves with 55+, A9+, KQ.")],
    ),
    with(
        rule("ko.hunts_bounties", "K01", F::Bounty, X, Some("HNT"),
            "Calls {raw}% of shoves in bounty games.",
            "When he covers you, shove into him only with 77+, AT+ and KQ."),
        &[(Variant::HeroCovers, "While you cover him, shove pairs, aces and broadways into him.")],
    ),
    // ---- bounty-driven calling (K02): his knockout calls split by cover.
    with(
        rule("ko.calls_when_covering", "K02", F::Bounty, X, Some("KO+"),
            "Calls {raw}% of shoves he covers in bounty games, {lowRaw}% ({lowHits}/{lowN}) when covered.",
            "When he covers you, shove only 77+, AT+ and KQ; when you cover him, any pair or ace."),
        &[
            (Variant::HeroCovers, "While you cover him, shove any pair, any ace and broadways into him."),
            (Variant::HeroCovered, "While he covers you, shove into him only with 77+, AT+ and KQ."),
        ],
    ),
    // ---- postflop (F01–F12, F14–F17). Audit 2026-10 (section 17.6): every
    // postflop advice names its street and a size or hand class, and reads
    // that can fire on the same villain add a lever instead of repeating one.
    rule("post.cbet_flop.high", "F01", F::Postflop, X, Some("CB+"),
        "C-bets {raw}% of flops as the preflop raiser.",
        "Raise his flop c-bets with draws and gutshots; call them with any pair."),
    rule("post.cbet_flop.low", "F01", F::Postflop, T, Some("CB-"),
        "C-bets only {raw}% of flops as the preflop raiser.",
        "Fold bottom pair to his flop c-bets; bet 1/2 pot when he checks the flop."),
    rule("post.fold_cbet_flop.high", "F02", F::Postflop, X, Some("FCB"),
        "Folds to {raw}% of flop c-bets.",
        "C-bet him at 1/3 pot with your whole range on dry flops."),
    rule("post.fold_cbet_flop.low", "F02", F::Postflop, X, Some("NFC"),
        "Folds to only {raw}% of flop c-bets.",
        "C-bet flops at 2/3 pot with top pair+ and draws; check back overcards."),
    rule("post.cbet_turn.high", "F03", F::Postflop, X, Some("BRL"),
        "Fires the turn {raw}% after c-betting the flop.",
        "Call his turn barrels with second pair+; float flops with pairs, not gutshots."),
    rule("post.cbet_turn.giveup", "F03", F::Postflop, X, Some("GIV"),
        "Fires the turn only {raw}% after c-betting the flop.",
        "Float his flop c-bets in position with overcards; bet 2/3 pot if he checks the turn."),
    rule("post.fold_cbet_turn.high", "F05", F::Postflop, X, Some("TRB"),
        "Folds to {raw}% of turn barrels.",
        "Barrel the turn at 2/3 pot with draws and overcards after he calls the flop."),
    rule("post.cbet_river.high", "F04", F::Postflop, X, Some("BRL"),
        "Fires the river {raw}% after barrelling the turn.",
        "Bluff-catch his river barrels with top pair and good second pairs."),
    rule("post.cbet_river.low", "F04", F::Postflop, T, Some("RV-"),
        "Fires the river only {raw}% after barrelling the turn.",
        "Fold one pair to his river barrels; call his turn barrels with draws."),
    rule("post.fold_cbet_river.low", "F05", F::Postflop, X, Some("STN"),
        "Folds to only {raw}% of river barrels.",
        "Size river value bets up to pot with two pair or better after two calls."),
    // Pro review (D107, F14): calls flop and turn, then gives up the river.
    rule("post.fold_cbet_river.high", "F14", F::Postflop, X, Some("RVF"),
        "Folds to {raw}% of river barrels after calling the turn.",
        "Fire third barrels on the river at 3/4 pot with missed draws and ace blockers."),
    rule("post.delayed_cbet.high", "F06", F::Postflop, T, Some("DLY"),
        "Bets {raw}% of turns after checking the flop as raiser.",
        "Call his delayed turn bets with any pair; raise them with draws."),
    // The stat counts every flop bet he faces after checking, not only
    // c-bets (section 9.2), and the observation says so.
    rule("post.check_raise.high", "F07", F::Postflop, X, Some("XR+"),
        "Check-raises {raw}% of flop bets he faces after checking.",
        "Call his flop check-raises with top pair and open-enders; never fold overpairs."),
    rule("post.check_raise.never", "F07", F::Postflop, T, Some("XR-"),
        "Has check-raised {hits} of {n} flop bets after checking.",
        "C-bet second pair on the flop for thin value; fold one pair to his rare check-raise."),
    rule("post.donk.high", "F08", F::Postflop, X, Some("DNK"),
        "Leads into the raiser on {raw}% of flops.",
        "Raise his flop leads to 3x with top pair, draws and overcards."),
    // The spot needs the raiser to check the turn to him (section 9.2).
    rule("post.float.high", "F09", F::Postflop, X, Some("FLT"),
        "Stabs {raw}% of turns checked to him after calling the flop in position.",
        "Check-raise his turn stabs with top pair+ and draws after he calls your c-bet."),
    rule("post.probe.high", "F10", F::Postflop, X, Some("PRB"),
        "Bets {raw}% of turns after the raiser checks the flop.",
        "Call his turn probes with second pair+; raise them with flush draws."),
    rule("post.river_bet.high", "F11", F::Postflop, X, Some("RV+"),
        "Bets {raw}% of rivers when checked to or first to act.",
        "Check top pair to him on the river; call his river bets with second pair+."),
    rule("post.river_bet.low", "F11", F::Postflop, T, Some("RV-"),
        "Bets only {raw}% of rivers when given the chance.",
        "Fold one pair to his river bets; call them with two pair or better."),
    rule("post.river_raise.high", "F11", F::Postflop, T, Some("RR+"),
        "Raises {raw}% of river bets he faces.",
        "Check second pair on the river; call his river raises with top pair or better."),
    rule("post.station", "F12", F::Postflop, X, Some("STN"),
        "Goes to showdown {raw}% and wins there only {wsd}%.",
        "Value bet middle pair on the river; bet pot with two pair or better."),
    rule("post.wsd_strong", "F12", F::Postflop, T, Some("WSD"),
        "Wins {raw}% of the showdowns he reaches.",
        "Value bet top pair on flop and turn only; bet rivers with two pair or better."),
    rule("post.wwsf_low", "F12", F::Postflop, X, Some("WWS"),
        "Wins only {raw}% of pots after seeing the flop.",
        "Stab 1/2 pot on flops and turns he checks; barrel draws twice."),
    rule("post.wwsf_high", "F12", F::Postflop, T, Some("WWS"),
        "Wins {raw}% of pots after seeing the flop.",
        "Bluff him less on turns and rivers; call his flop bets with any pair."),
    // River sizing (F15): shares of his river bets by size bucket. They
    // count sizes only, never what he holds (section 8.1); the advice is
    // the price the size gives the hero's bluff-catchers and raises.
    rule("post.river_sizing.polar", "F15", F::Postflop, X, Some("OVB"),
        "Sizes {raw}% of his river bets as overbets or all-ins.",
        "Call his river overbets with top pair+ and fold second pair; raise only with sets+."),
    rule("post.river_sizing.small", "F15", F::Postflop, X, Some("SRB"),
        "Sizes {raw}% of his river bets under 40% of the pot.",
        "Raise his small river bets to 3x with two pair+ and some missed draws; call any pair."),
    // Fold to a river raise (F16): his river bet raised, he acts again.
    rule("post.fold_river_raise.high", "F16", F::Postflop, X, Some("FRR"),
        "Folds {raw}% of his river bets when raised.",
        "Raise his river bets 2.5x with missed draws; call, never raise, with one pair."),
    // Fit-or-fold (F17): folds flops to c-bets and rarely reaches showdown.
    rule("post.fit_or_fold", "F17", F::Postflop, X, Some("FOF"),
        "Folds to {raw}% of flop c-bets and reaches showdown only {wtsd}%.",
        "C-bet any two at 1/3 pot on the flop; barrel the turn only with draws or top pair+."),
    // ---- head-to-head (H01–H03). Audit 2026-10 (section 17.7): each
    // note names the hand classes or size for the hero's own spots against
    // him, adds a lever to the general read on the same leak, and reshoves
    // at 15–40bb.
    with(
        rule("h2h.3bet_vs_hero.high", "H01", F::H2h, X, Some("H2H"),
            "3-bets your opens {raw}% ({hits}/{n}).",
            "Cut KTo-type opens in front of him; 4-bet his 3-bets with QQ+, AK and A5s-A4s."),
        &[(Variant::JamDepth, "Jam 99+, AJ+ and KQs over his 3-bets of your opens.")],
    ),
    with(
        rule("h2h.fold_to_hero_3bet.high", "H02", F::H2h, X, Some("F3B"),
            "Folds to your 3-bets {raw}% ({hits}/{n}).",
            "3-bet his opens light at 3x in position: A2s-A5s, K9s+ and 65s+."),
        &[(Variant::JamDepth, "Reshove A2s+, K9s+, 55+ and QJs over his opens.")],
    ),
    rule("h2h.fold_to_hero_cbet.high", "H02", F::H2h, X, Some("FCB"),
        "Folds to your flop c-bets {raw}% ({hits}/{n}).",
        "C-bet him with any two at 1/3 pot on the flop, wet boards included."),
    with(
        rule("h2h.steal_vs_hero.high", "H03", F::H2h, X, Some("TGT"),
            "Steals into your blinds {raw}% ({hits}/{n}).",
            "3-bet his steals from your blinds with 77+, A8s+, KTs+, ATo+ and A5s."),
        &[(Variant::JamDepth, "From your blinds, reshove 44+, A5+, K9s+ and QJs over his steals.")],
    ),
    rule("h2h.defend_vs_hero_steal.high", "H03", F::H2h, T, Some("DEF"),
        "Defends against your steals {raw}% ({hits}/{n}).",
        "Steal into him only with pairs, suited aces and broadways; value bet top pair."),
    // ---- showdown-backed (M04, M05): the only reads that may speak about
    // value and bluffs, always with their counts. Audit 2026-10: the advice
    // names the hand classes that fold, call or raise against the size (or
    // street) the counts are about.
    rule("sd.tell.big_is_value", "M04", F::Showdown, X, Some("SDV"),
        "Showed value in {valueCount} of {n} {bucket} bets.",
        "Fold weak top pair and worse to his {bucket} bets; call with two pair or better."),
    rule("sd.tell.big_is_bluff", "M04", F::Showdown, X, Some("SDB"),
        "Was bluffing in {bluffCount} of {n} shown {bucket} bets.",
        "Call his {bucket} bets down with second pair or better; never fold top pair."),
    rule("sd.tell.small_is_value", "M04", F::Showdown, X, Some("SDV"),
        "Showed value in {valueCount} of {n} {bucket} bets.",
        "Fold second pair or worse to his {bucket} bets; raise two pair or better."),
    rule("sd.shows_bluffs", "M05", F::Showdown, X, Some("SVB"),
        "Was bluffing in {bluffCount} of {n} river bets that reached showdown.",
        "Call his river bets with any pair; check top pair to him to let him bluff."),
    // ---- recent form (R02). Audit 2026-10: what to change while the
    // window lasts, by hand class; at push/fold the change is in shove and
    // call ranges, at 15–40bb a 3-bet is a reshove.
    with(
        rule("rec.tilt", "R02", F::Recency, X, Some("TILT"),
            "Played {x} of his last {n} hands after a big lost pot (usual {p0}%).",
            "While this lasts, value bet top pair three streets; call his bets with second pair."),
        &[(Variant::PushFold, "While this lasts, call his shoves with 22+, A7+ and KTs+; shove for value only.")],
    ),
    with(
        rule("rec.looser", "R02", F::Recency, T, Some("LSE"),
            "Played {x} of his last {n} hands (usual {p0}%).",
            "While this lasts, 3-bet his opens with 99+, AJ+ and KQ; value bet top pair thin."),
        &[
            (Variant::PushFold, "While this lasts, call his shoves with 33+, A8+ and KTs+."),
            (Variant::JamDepth, "While this lasts, reshove 88+, AT+ and KQ over his opens."),
        ],
    ),
    with(
        rule("rec.tighter", "R02", F::Recency, T, Some("NIT"),
            "Played only {x} of his last {n} hands (usual {p0}%).",
            "While this lasts, fold AJo and KQo to his opens; steal his blinds with any suited."),
        &[(Variant::PushFold, "While this lasts, call his shoves only with 88+ and AJ+; shove any ace into him.")],
    ),
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

/// What a read's sample counts (section 13).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SampleUnit {
    /// Hands: VPIP/PFR (one decision per hand dealt), showdowns, recent form.
    Hands,
    /// Times the spot came up: every other stat, head-to-head.
    Opportunities,
}

/// The sample a read's confidence rests on: the count of its weakest basis
/// stat (the one its confidence was taken from). `None` for context facts
/// of the latest hand (stack depth, bounty), which have no sample.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadSample {
    pub count: u32,
    pub unit: SampleUnit,
}

impl ReadSample {
    fn of(stat: &ShrunkStat) -> ReadSample {
        let unit = match stat.key {
            StatKey::Vpip | StatKey::Pfr => SampleUnit::Hands,
            _ => SampleUnit::Opportunities,
        };
        ReadSample { count: stat.opportunities, unit }
    }
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
    /// What the confidence rests on; the chip shows it next to the tag.
    pub sample: Option<ReadSample>,
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
    /// The format the stats were shrunk for (the replay's); `None` falls
    /// back to the context's format, then cash. Per-format thresholds and
    /// the tournament advice variant read it.
    pub format: Option<FormatKey>,
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
    /// `call_vs_shove` and `open_shove` over late-stage MTT hands (row T02).
    pub late_call_vs_shove: Option<ShrunkStat>,
    pub late_open_shove: Option<ShrunkStat>,
    /// `call_vs_shove` over knockout hands, split by whether he covered the
    /// shover (his starting stack at least the shover's) or not (row K02).
    pub ko_call_covering: Option<ShrunkStat>,
    pub ko_call_covered: Option<ShrunkStat>,
}

impl RuleInput {
    pub fn stat(&self, key: StatKey) -> Option<&ShrunkStat> {
        self.stats.get(&key)
    }

    /// The format thresholds and advice variants are chosen for.
    pub fn format_key(&self) -> FormatKey {
        self.format.or_else(|| self.context.as_ref().map(|c| c.format)).unwrap_or(FormatKey::Cash)
    }
}

fn shrunk_of(counts: StatAgg, key: StatKey, prior: f64) -> ShrunkStat {
    ShrunkStat::new(key, counts.hits, counts.opportunities, prior)
}

fn tournament(hand: &HandFacts) -> bool {
    matches!(FormatKey::of_hand(hand), FormatKey::Mtt | FormatKey::Spin)
}

/// Whether `player_id` covered the shover he faced (row K02): his starting
/// stack is at least the shover's, so a call could win the shover's bounty.
/// `None` when the shover or a stack is unknown.
fn covers_shover(hand: &HandFacts, player_id: i64, shover: Option<i64>) -> Option<bool> {
    let own = hand.seat_of(player_id)?.starting_stack?;
    let theirs = hand.seat_of(shover?)?.starting_stack?;
    Some(own >= theirs)
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
    /// `call_vs_shove` and `open_shove` over late-stage MTT hands (row T02).
    /// Spin & Go hands are left out: a Spin pays the winner, so its late
    /// stage carries no ICM pressure to be insensitive to.
    pub late_call_vs_shove: StatAgg,
    pub late_open_shove: StatAgg,
    /// Knockout `call_vs_shove` split by cover (row K02): `ko_covering` when
    /// his starting stack was at least the shover's (a call could win the
    /// shover's bounty), `ko_covered` otherwise. A shove whose stacks are
    /// unknown counts in neither.
    pub ko_covering: StatAgg,
    pub ko_covered: StatAgg,
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
        let mut late_call_vs_shove = StatAgg::default();
        let mut late_open_shove = StatAgg::default();
        let mut ko_covering = StatAgg::default();
        let mut ko_covered = StatAgg::default();
        let mut ko_hands = 0;
        for hand in hands.iter().filter(|h| h.seat_of(player_id).is_some()) {
            let knockout = hand.seats.iter().any(|s| s.bounty.is_some());
            let stage = tournament(hand)
                .then(|| average_stack_bb(hand).map(|avg| Stage::in_format(avg, FormatKey::of_hand(hand))))
                .flatten();
            if !knockout && stage.is_none() {
                continue;
            }
            ko_hands += u32::from(knockout);
            let late_mtt = stage == Some(Stage::Late) && FormatKey::of_hand(hand) == FormatKey::Mtt;
            for event in extract_preflop(hand, player_id).iter().filter(|e| e.opportunity) {
                let mut targets: Vec<&mut StatAgg> = Vec::new();
                match (event.key, stage) {
                    (StatKey::CallVsShove, _) => {
                        if knockout {
                            targets.push(&mut ko);
                            match covers_shover(hand, player_id, event.counterparty.creator) {
                                Some(true) => targets.push(&mut ko_covering),
                                Some(false) => targets.push(&mut ko_covered),
                                None => {}
                            }
                        }
                        if late_mtt {
                            targets.push(&mut late_call_vs_shove);
                        }
                    }
                    (StatKey::OpenShove, _) if late_mtt => targets.push(&mut late_open_shove),
                    (StatKey::FoldToStealBb, Some(Stage::Late)) => targets.push(&mut late),
                    (StatKey::Limp, Some(Stage::Early)) => targets.push(&mut early),
                    _ => {}
                }
                for target in targets {
                    target.opportunities += 1;
                    target.hits += u32::from(event.success);
                }
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
            late_call_vs_shove,
            late_open_shove,
            ko_covering,
            ko_covered,
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
            format: Some(self.format),
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
            late_call_vs_shove: (self.late_call_vs_shove.opportunities > 0)
                .then(|| shrunk_of(self.late_call_vs_shove, StatKey::CallVsShove, prior(StatKey::CallVsShove))),
            late_open_shove: (self.late_open_shove.opportunities > 0)
                .then(|| shrunk_of(self.late_open_shove, StatKey::OpenShove, prior(StatKey::OpenShove))),
            ko_call_covering: (self.ko_covering.opportunities > 0)
                .then(|| shrunk_of(self.ko_covering, StatKey::CallVsShove, prior(StatKey::CallVsShove))),
            ko_call_covered: (self.ko_covered.opportunities > 0)
                .then(|| shrunk_of(self.ko_covered, StatKey::CallVsShove, prior(StatKey::CallVsShove))),
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
    /// The weakest leg's sample: the one the confidence came from.
    sample: ReadSample,
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
            sample: ReadSample::of(stat),
        })
    }

    /// Adds a secondary basis stat: the read is only as sure as its weaker
    /// leg.
    fn and(mut self, other: &ShrunkStat) -> StatHit {
        if other.confidence < self.confidence {
            self.confidence = other.confidence;
            self.tier = other.tier;
            self.sample = ReadSample::of(other);
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

/// The passing stat of two with the larger deviation ("a or b" with a
/// threshold per leg).
fn larger<'a>(a: Option<&'a ShrunkStat>, b: Option<&'a ShrunkStat>) -> Option<&'a ShrunkStat> {
    match (a, b) {
        (Some(x), Some(y)) => Some(if deviation(x) >= deviation(y) { x } else { y }),
        (x, y) => x.or(y),
    }
}

/// A per-format threshold delta, `[MTT, 6-max cash, Zoom, Spin & Go]`.
fn by_format(format: FormatKey, deltas: [f64; 4]) -> f64 {
    match format {
        FormatKey::Mtt => deltas[0],
        FormatKey::Cash => deltas[1],
        FormatKey::Zoom => deltas[2],
        FormatKey::Spin => deltas[3],
    }
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
        sample: ReadSample::of(weakest),
    })
}

/// The context facts rules adapt to.
struct Adapt<'a> {
    context: Option<&'a EngineContext>,
    format: FormatKey,
}

impl Adapt<'_> {
    fn bucket(&self) -> Option<StackBucket> {
        self.context.and_then(|c| c.stack_bucket)
    }

    /// Section 5: above push/fold and at most [`JAM_DEPTH_MAX_BB`].
    fn jam_depth(&self) -> bool {
        self.context
            .and_then(|c| c.effective_stack_bb)
            .is_some_and(|eff| eff > PUSH_FOLD_MAX_BB && eff <= JAM_DEPTH_MAX_BB)
    }

    fn tournament(&self) -> bool {
        matches!(self.format, FormatKey::Mtt | FormatKey::Spin)
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
    /// and showdown reads apply, plus the late-stage reads: late-stage blind
    /// battles (audit 2026-10) and the late shoves and calls of row T02 are
    /// played at that depth.
    fn eligible(&self, def: &RuleDef) -> bool {
        if self.bucket() != Some(StackBucket::PushFold) {
            return true;
        }
        match def.family {
            Family::Stack | Family::Bounty | Family::Recency | Family::Showdown => true,
            Family::Context => def.id == "ctx.short_stack",
            Family::Stage => def.id.starts_with("stage.late."),
            Family::Preflop | Family::Postflop | Family::H2h => false,
        }
    }

    /// The seat behind the seat suffix. Zoom reseats every player every
    /// hand, so a Zoom context never gives one, even when a caller supplies
    /// it (section 5).
    fn seat(&self) -> Option<SeatRelation> {
        self.context.filter(|c| !reseats_every_hand(c.variant.as_deref())).and_then(|c| c.seat)
    }

    fn multiplier(&self, def: &RuleDef) -> f64 {
        // The late-stage call read is `stk.call_shove.*` over late hands and
        // replaces it at the late stage (row T02), so it ranks like it.
        let reshove_family = def.id.starts_with("stk.reshove.")
            || def.id.starts_with("stk.call_shove.")
            || def.id == "stage.late.calls_shoves";
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
            Variant::JamDepth => self.jam_depth(),
            Variant::Tournament => self.tournament(),
            Variant::PushFold => self.bucket() == Some(StackBucket::PushFold),
        };
        // The first matching variant in the rule's list wins, so each list
        // is written in precedence order (late stage, jam depth, format).
        let mut advice = def.advice.to_string();
        if let Some((variant, text)) = def.variants.iter().find(|(v, _)| wanted(*v)) {
            advice = text.to_string();
            adapted.push(match variant {
                Variant::LateStage => "stage.late",
                Variant::JamDepth => "stack.jam_depth",
                Variant::Tournament => "format.tournament",
                Variant::PushFold => "stack.push_fold",
                Variant::Bounty | Variant::HeroCovered | Variant::HeroCovers => "ko.variant",
            });
        }
        if let (Some(side), Some(seat)) = (def.seat, self.seat()) {
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
        sample: Option<ReadSample>,
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
            sample,
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
        self.push(id, extra, evidence, Some(hit.sample), hit.confidence, hit.tier, Score::Deviation(hit.deviation), None);
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
    let mut out = Out { adapt: Adapt { context: input.context.as_ref(), format: input.format_key() }, reads: Vec::new() };

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
    // Audit 2026-10 (section 17): the small blind's leg needs .15, since a
    // small blind folding ~85% to a button open is close to correct; the
    // defence reads use per-format deltas, since antes make wide big-blind
    // defence correct in tournaments.
    let format = input.format_key();
    let sb_folds = one(input, FoldToStealSb, Dir::Above(FOLD_TO_STEAL_SB_HIGH));
    let bb_folds = one(input, FoldToStealBb, Dir::Above(0.12));
    out.stat("pf.fold_to_steal.high", single(larger(sb_folds, bb_folds)));
    out.stat(
        "pf.fold_to_steal.low",
        defended(one(input, FoldToStealBb, Dir::Below(by_format(format, FOLD_TO_STEAL_LOW)))),
    );
    out.stat("pf.bb_vs_sb.overfold", single(one(input, BbDefendVsSb, Dir::Below(0.15))));
    out.stat(
        "pf.bb_vs_sb.overdefend",
        single(one(input, BbDefendVsSb, Dir::Above(by_format(format, BB_VS_SB_OVERDEFEND)))),
    );

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
    out.stat("pf.limp.high", single(one(input, Limp, Dir::Above(by_format(format, LIMP_HIGH)))));
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
                out.push("ctx.short_stack", vec![("eff", one_decimal(eff))], evidence.clone(), None, 1.0,
                    ConfidenceTier::High, Score::Fixed(2.5), Some(tag));
            }
            if ctx.stack_bucket == Some(StackBucket::Deep) {
                out.push("ctx.deep_stack", vec![("eff", one_decimal(eff))], evidence, None, 1.0,
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
                single(input.early_limp.as_ref().filter(|s| {
                    s.displayable() && passes(s, Dir::Above(by_format(input.format_key(), STAGE_EARLY_LIMP)))
                })),
            );
        }
        // ---- late-stage push/fold (T02): MTT only; a Spin & Go pays the
        // winner, so wide late calls and shoves there ignore no ICM.
        if ctx.stage == Some(Stage::Late) && input.format_key() == FormatKey::Mtt {
            let late = |stat: &Option<ShrunkStat>, delta| stat.filter(|s| s.displayable() && passes(s, Dir::Above(delta)));
            out.stat("stage.late.calls_shoves", single(late(&input.late_call_vs_shove, LATE_CALLS_SHOVES).as_ref()));
            out.stat("stage.late.shoves_wide", single(late(&input.late_open_shove, LATE_SHOVES_WIDE).as_ref()));
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
                None,
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
    // ---- bounty-driven calling (K02): both cover legs shown, the covering
    // one high and well above the covered one.
    let shown_leg = |stat: &Option<ShrunkStat>| stat.filter(|s| s.displayable());
    if let (Some(covering), Some(covered)) = (shown_leg(&input.ko_call_covering), shown_leg(&input.ko_call_covered)) {
        if passes(&covering, Dir::Above(KO_COVERING_HIGH)) && covering.shrunk - covered.shrunk >= KO_COVER_GAP - EPS {
            let low = vec![
                ("lowRaw", whole_pct(covered.raw.unwrap_or_default())),
                ("lowHits", covered.hits.to_string()),
                ("lowN", covered.opportunities.to_string()),
            ];
            out.stat_with("ko.calls_when_covering", single(Some(&covering)).map(|h| h.and(&covered)), low);
        }
    }

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
    // Audit 2026-10 (section 17.6): a low WWSF is a "gives up" read only
    // when he reaches showdown no more often than the prior; a station's low
    // WWSF comes from calling down and losing, where pressure loses money.
    if let Some(wtsd) = one(input, Wtsd, Dir::Below(0.0)) {
        out.stat("post.wwsf_low", single(one(input, Wwsf, Dir::Below(0.08))).map(|h| h.and(wtsd)));
    }
    out.stat("post.wwsf_high", single(one(input, Wwsf, Dir::Above(0.08))));
    // ---- river sizing and raised river bets (F15, F16)
    out.stat("post.river_sizing.polar", single(one(input, RiverBetPolar, Dir::Above(0.12))));
    out.stat("post.river_sizing.small", single(one(input, RiverBetSmall, Dir::Above(0.15))));
    out.stat("post.fold_river_raise.high", single(one(input, FoldToRiverRaise, Dir::Above(0.15))));
    // ---- fit-or-fold (F17): a flop overfold confirmed by a low showdown
    // frequency; as sure as the weaker leg.
    if let (Some(fcb), Some(wtsd)) = (one(input, FoldToCbetFlop, Dir::Above(0.10)), one(input, Wtsd, Dir::Below(0.05))) {
        let wtsd_raw = whole_pct(wtsd.raw.unwrap_or_default());
        out.stat_with("post.fit_or_fold", single(Some(fcb)).map(|h| h.and(wtsd)), vec![("wtsd", wtsd_raw)]);
    }

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
        let sample = ReadSample { count: stat.opportunities, unit: SampleUnit::Opportunities };
        out.push(id, values, evidence, Some(sample), c, tier_of(c), Score::Deviation(d), None);
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
            let sample = ReadSample { count: form.baseline_opportunities, unit: SampleUnit::Hands };
            out.push(flag.rule_id(), values, evidence, Some(sample), c, tier_of(c), Score::Fixed(d), None);
        }
    }

    drop_opposites(&mut out.reads);
    out.reads
}

/// Preflop reads whose advice points opposite ways (audit 2026-10, section
/// 17). "a or b" reads fire on either leg, so a villain who is tight under
/// the gun and loose in the hijack could get both "fold to his early opens"
/// and "3-bet his early opens"; only the read with the higher score stays.
pub const PREFLOP_OPPOSITES: [(&str, &str); 5] = [
    ("pf.rfi.tight_early", "pf.rfi.loose_early"),
    ("pf.rfi.tight_late", "pf.rfi.loose_late"),
    ("pf.rfi.tight_late", "pf.steal.high"),
    ("pf.rfi.loose_late", "pf.steal.low"),
    ("pf.fold_to_steal.high", "pf.fold_to_steal.low"),
];

/// Postflop reads whose advice points opposite ways (audit 2026-10, section
/// 17.6). They rest on different stats that one villain can show together,
/// e.g. "Bluff him less on turns and rivers" (high WWSF) next to "Barrel the
/// turn" (turn overfold); only the read with the higher score stays.
pub const POSTFLOP_OPPOSITES: [(&str, &str); 6] = [
    ("post.wwsf_high", "post.fold_cbet_turn.high"),
    ("post.wwsf_high", "post.fold_cbet_river.high"),
    ("post.river_bet.low", "post.cbet_river.high"),
    ("post.river_bet.high", "post.cbet_river.low"),
    ("post.station", "post.river_raise.high"),
    ("post.fit_or_fold", "post.fold_cbet_turn.high"),
];

/// A postflop read and the more general read it refines (rows F15, F17):
/// the size of his river bets decides what calls them, so "call his river
/// bets with second pair+" yields to "fold second pair to his overbets",
/// and the fit-or-fold read is the flop overfold confirmed at showdown.
/// The first read of each pair always stays.
pub const REFINED_OPPOSITES: [(&str, &str); 5] = [
    ("post.river_sizing.polar", "post.river_bet.high"),
    ("post.river_sizing.polar", "post.cbet_river.high"),
    ("post.river_sizing.small", "post.river_bet.low"),
    ("post.river_sizing.small", "post.cbet_river.low"),
    ("post.fit_or_fold", "post.fold_cbet_flop.high"),
];

/// A head-to-head read and a general read on another stat whose advice
/// points the opposite way (audit 2026-10, section 17.7), e.g. "4-bet his
/// 3-bets with QQ+" against "Fold JJ and AQ to his 3-bets" on a villain who
/// 3-bets the hero far more than anyone else. The head-to-head stat is
/// already shrunk toward his overall value (section 4.1), so when it still
/// fires it is the evidence about the hero's own spots: the first read of
/// each pair always stays.
pub const H2H_OPPOSITES: [(&str, &str); 5] = [
    ("h2h.3bet_vs_hero.high", "pf.3bet.low"),
    ("h2h.fold_to_hero_3bet.high", "pf.fold_to_3bet.low"),
    ("h2h.fold_to_hero_cbet.high", "post.fold_cbet_flop.low"),
    ("h2h.steal_vs_hero.high", "pf.steal.low"),
    ("h2h.defend_vs_hero_steal.high", "pf.fold_to_steal.high"),
];

/// Shown river bluffs against a river read inferred from frequency (audit
/// 2026-10, section 17.7): a low betting frequency only proves a narrow
/// range (section 8.1), while the showdown read counts what he bet. The
/// same holds for the sizing shares of row F15 against the showdown tells
/// of the same sizes. The first read of each pair always stays.
pub const SHOWDOWN_OPPOSITES: [(&str, &str); 6] = [
    ("sd.shows_bluffs", "post.river_bet.low"),
    ("sd.shows_bluffs", "post.cbet_river.low"),
    ("sd.shows_bluffs", "post.river_sizing.polar"),
    ("sd.tell.big_is_value", "post.river_sizing.polar"),
    ("sd.tell.big_is_bluff", "post.river_sizing.polar"),
    ("sd.tell.small_is_value", "post.river_sizing.small"),
];

/// Tournament reads and the more general read they refine (row T02, K02).
/// The late-stage reads count the general stat over his late MTT hands
/// only, the late stage the context is in, and their advice adds the cover
/// condition pay jumps impose. The cover split explains his knockout calls
/// (the general bounty read) and his late calls, whose value-only advice
/// overstates his calls when he is covered. The first read of each pair
/// always stays.
pub const TOURNAMENT_OPPOSITES: [(&str, &str); 6] = [
    ("stage.late.calls_shoves", "stk.call_shove.wide"),
    ("stage.late.calls_shoves", "stk.call_shove.tight"),
    ("stage.late.shoves_wide", "stk.open_shove.wide"),
    ("stage.late.shoves_wide", "stk.open_shove.tight"),
    ("ko.calls_when_covering", "ko.hunts_bounties"),
    ("ko.calls_when_covering", "stage.late.calls_shoves"),
];

/// Removes the weaker read of every [`PREFLOP_OPPOSITES`] and
/// [`POSTFLOP_OPPOSITES`] pair that fired together: the lower score goes,
/// then the lower deviation; a tie keeps the pair's first read. Of every
/// [`H2H_OPPOSITES`], [`SHOWDOWN_OPPOSITES`], [`TOURNAMENT_OPPOSITES`] and
/// [`REFINED_OPPOSITES`] pair, the general read goes, whatever the scores.
fn drop_opposites(reads: &mut Vec<EngineRuleResult>) {
    let at = |reads: &[EngineRuleResult], id: &str| reads.iter().position(|r| r.rule_id == id);
    for (a, b) in PREFLOP_OPPOSITES.into_iter().chain(POSTFLOP_OPPOSITES) {
        let (Some(i), Some(j)) = (at(reads, a), at(reads, b)) else { continue };
        let (x, y) = (&reads[i], &reads[j]);
        let order = x.score.total_cmp(&y.score).then(x.deviation.total_cmp(&y.deviation));
        reads.remove(if order.is_lt() { i } else { j });
    }
    let specific_first =
        H2H_OPPOSITES.into_iter().chain(SHOWDOWN_OPPOSITES).chain(TOURNAMENT_OPPOSITES).chain(REFINED_OPPOSITES);
    for (specific, general) in specific_first {
        if let (Some(_), Some(j)) = (at(reads, specific), at(reads, general)) {
            reads.remove(j);
        }
    }
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
    let sample = ReadSample { count: n, unit: SampleUnit::Hands };
    out.push(id, values, evidence, Some(sample), c, tier_of(c), Score::Deviation(4.0 * (share - 0.5).abs()), None);
}
