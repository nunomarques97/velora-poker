//! Read ranking and the chip tag (`docs/specs/opponent-engine.md`, section
//! 7): the reads worth the most money to the hero against this villain, in
//! the between-hands context, go first, and a 2–4 character tag names the
//! villain at a glance across a dozen tables.
//!
//! Catalogue rows G01 (`rank.score`), G02 (`rank.tag`) and X03
//! (`gate.min_hands_archetype`). Pure functions of the reads the rules
//! produced and the between-hands context; nothing here sees a hand in
//! progress.

use std::cmp::Ordering;

use serde::Serialize;

use super::context::{reseats_every_hand, EngineContext, Side, Stage, StackBucket};
use super::pooling::FormatKey;
use super::rules::{EngineRuleResult, Family, RuleDef, JAM_DEPTH_MAX_BB};

/// Spec id of the ranking (catalogue row G01).
pub const RULE_RANK_SCORE: &str = "rank.score";
/// Spec id of the chip tag (catalogue row G02).
pub const RULE_RANK_TAG: &str = "rank.tag";
/// Spec id of the archetype gate (catalogue row X03). The engine never
/// produces an archetype or a player-type colour: the chip colour stays
/// `classification::resolve_for_player`'s, which applies `min_hands`, and
/// the engine's tag is a separate segment next to it.
pub const RULE_MIN_HANDS_GATE: &str = "gate.min_hands_archetype";

/// Reads shown on the hover card and the side panel.
pub const TOP_READS: usize = 2;
/// In tournaments at or below this effective stack (bb) the chip names his
/// best all-in read, or his stack when he has none.
pub const STACK_TAG_MAX_BB: f64 = 25.0;
/// The rule id a stack tag reports when `ctx.short_stack` did not fire
/// (15–25bb): the effective-stack derivation itself (row S06).
const STACK_TAG_RULE: &str = "ctx.effective_stack";
const SHORT_STACK_RULE: &str = "ctx.short_stack";
const TILT_RULE: &str = "rec.tilt";

/// The money-relevance score (section 7): `deviation × confidence × weight
/// × context`, where `weight` is the rule's [`spot_weight`] and `context`
/// its [`context_multiplier`]. Context, recency and showdown reads carry a
/// fixed base instead of `deviation × confidence`; the rules apply that
/// themselves, so ranking only ever reads `score`.
pub fn score(deviation: f64, confidence: f64, weight: f64, context: f64) -> f64 {
    deviation * confidence * weight * context
}

// ------------------------------------------------------- spot weight (7.2)

/// Spot weight of head-to-head reads: they count the hero's own spots
/// against this villain, so every one of them is a spot the hero plays.
pub const H2H_WEIGHT: f64 = 1.25;
/// Spot weight of reads about rare lines (a squeeze needs an open and a
/// caller, a river raise a river bet first) and of outcome stats (won
/// pots), whose lever is diffuse: they rank below a frequency read of the
/// same size.
pub const MINOR_SPOT_WEIGHT: f64 = 0.8;

/// Reads whose spot is rare or whose stat is an outcome (section 7.2).
const MINOR_SPOTS: [&str; 11] = [
    "pf.squeeze.high",
    "pf.limp_reraise.seen",
    "post.delayed_cbet.high",
    "post.donk.high",
    "post.float.high",
    "post.probe.high",
    "post.river_raise.high",
    "post.fold_river_raise.high",
    "post.wwsf_low",
    "post.wwsf_high",
    "post.wsd_strong",
];

/// How often the read's spot comes up for the hero and how big its pot is,
/// relative to an ordinary preflop open or flop c-bet spot (1.0).
pub fn spot_weight(def: &RuleDef) -> f64 {
    if def.family == Family::H2h {
        H2H_WEIGHT
    } else if MINOR_SPOTS.contains(&def.id) {
        MINOR_SPOT_WEIGHT
    } else {
        1.0
    }
}

// --------------------------------------------------- context (section 7.2)

/// The kind of spot a read is about, which decides how the between-hands
/// context changes its money value (section 7.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lever {
    /// Blind battles: his steals and small-blind opens, his blind defence.
    Steal,
    /// His limps and the raises over them.
    Limp,
    /// His own 3-bets, 4-bets and squeezes.
    Reraise,
    /// His open shoves: the hero's calls.
    Shove,
    /// His reshoves over opens: the hero's opens and calls.
    Reshove,
    /// His calls of all-ins: the hero's shoves.
    CallShove,
    /// Postflop lines.
    Postflop,
    /// Everything else (opens, folds to 3-bets, profile, context, recency,
    /// showdown tells): no context factor beyond the seat.
    Other,
}

/// The lever of a rule id.
pub fn lever(rule_id: &str) -> Lever {
    match rule_id {
        "pf.steal.high" | "pf.steal.low" | "pf.rfi_sb.wide" | "pf.fold_to_steal.high" | "pf.fold_to_steal.low"
        | "pf.bb_vs_sb.overfold" | "pf.bb_vs_sb.overdefend" | "stage.late.overfolds_blinds"
        | "h2h.steal_vs_hero.high" | "h2h.defend_vs_hero_steal.high" => Lever::Steal,
        "pf.limp.high" | "pf.iso.high" | "pf.limp_fold.high" | "pf.limp_reraise.seen" | "pf.limp_call.high"
        | "stage.early.passive" => Lever::Limp,
        "pf.3bet.high_ip" | "pf.3bet.high_oop" | "pf.3bet.low" | "pf.4bet.high" | "pf.squeeze.high"
        | "h2h.3bet_vs_hero.high" => Lever::Reraise,
        "stk.open_shove.wide" | "stk.open_shove.tight" | "stage.late.shoves_wide" => Lever::Shove,
        "stk.reshove.high" | "stk.reshove.low" => Lever::Reshove,
        "stk.call_shove.wide" | "stk.call_shove.tight" | "stage.late.calls_shoves" | "ko.hunts_bounties"
        | "ko.calls_when_covering" => Lever::CallShove,
        "h2h.fold_to_hero_cbet.high" => Lever::Postflop,
        id if id.starts_with("post.") => Lever::Postflop,
        _ => Lever::Other,
    }
}

/// Seat factor of a read about the side of the hero the villain sits on
/// (his answers to the hero's opens on the left, his own opens on the
/// right), directly next to the hero.
pub const SEAT_DIRECT: f64 = 1.25;
/// Seat factor on the read's side, further away.
pub const SEAT_SAME_SIDE: f64 = 1.1;
/// Seat factor on the other side, further away: the spot still comes up,
/// less often.
pub const SEAT_OTHER_SIDE: f64 = 0.9;
/// Seat factor directly next to the hero on the other side.
pub const SEAT_OTHER_SIDE_DIRECT: f64 = 0.8;
/// All-in reads at 15–25bb (`reshove`): his reshoves and his calls of
/// shoves decide most pots there.
pub const RESHOVE_PRIORITY: f64 = 1.5;
/// His reshoves at 25–40bb (jam depth above the reshove bucket).
pub const JAM_DEPTH_RESHOVE: f64 = 1.25;
/// All-in reads at 60–150bb (`standard`) and above 150bb (`deep`): their
/// samples come from 25bb or less and their spots rarely arise this deep.
pub const STANDARD_ALL_IN: f64 = 0.6;
pub const DEEP_ALL_IN: f64 = 0.5;
/// His sized 3-bets, 4-bets and squeezes at 15–25bb: there his re-raise is
/// a jam, which the depth-specific reshove read counts.
pub const RESHOVE_RERAISE: f64 = 0.8;
/// Postflop reads at 15–25bb (few streets left to play) and above 150bb
/// (most of the money is won postflop).
pub const RESHOVE_POSTFLOP: f64 = 0.75;
pub const DEEP_POSTFLOP: f64 = 1.2;
/// Late tournament stage: blind battles and all-in spots decide the stage,
/// postflop play shrinks.
pub const LATE_STAGE_PRIORITY: f64 = 1.25;
pub const LATE_STAGE_POSTFLOP: f64 = 0.8;
/// Early tournament stage: limped pots are the cheapest money.
pub const EARLY_STAGE_LIMP: f64 = 1.25;
/// Knockout, effective stack 25bb or less, the hero covers a villain
/// carrying a big bounty: calling his shoves and reshoves also wins it.
pub const BOUNTY_CALL_PRIORITY: f64 = 1.5;
/// Knockout, effective stack 25bb or less, he covers the hero: the hero's
/// bounty pays his calls.
pub const BOUNTY_COVERED_PRIORITY: f64 = 1.25;
/// Effective stack up to which the bounty factors apply (`reshove`'s top).
const BOUNTY_MAX_BB: f64 = 25.0;

fn all_in(lever: Lever) -> bool {
    matches!(lever, Lever::Shove | Lever::Reshove | Lever::CallShove)
}

/// The seat factor (section 7.2): only for reads with a seat side, never
/// at Zoom, where the seats change every hand.
fn seat_factor(def: &RuleDef, ctx: &EngineContext) -> f64 {
    if reseats_every_hand(ctx.variant.as_deref()) {
        return 1.0;
    }
    let (Some(side), Some(seat)) = (def.seat, ctx.seat) else { return 1.0 };
    let direct = match seat.side {
        Side::Left => seat.direct_left,
        Side::Right => seat.direct_right,
    };
    match (seat.side == side, direct) {
        (true, true) => SEAT_DIRECT,
        (true, false) => SEAT_SAME_SIDE,
        (false, false) => SEAT_OTHER_SIDE,
        (false, true) => SEAT_OTHER_SIDE_DIRECT,
    }
}

fn stack_factor(lever: Lever, ctx: &EngineContext) -> f64 {
    let Some(bucket) = ctx.stack_bucket else { return 1.0 };
    let jam_depth = ctx.effective_stack_bb.is_some_and(|eff| eff <= JAM_DEPTH_MAX_BB + 1e-9);
    match (bucket, lever) {
        (StackBucket::Reshove, Lever::Reshove | Lever::CallShove) => RESHOVE_PRIORITY,
        (StackBucket::Reshove, Lever::Reraise) => RESHOVE_RERAISE,
        (StackBucket::Reshove, Lever::Postflop) => RESHOVE_POSTFLOP,
        (StackBucket::Mid, Lever::Reshove) if jam_depth => JAM_DEPTH_RESHOVE,
        (StackBucket::Standard, l) if all_in(l) => STANDARD_ALL_IN,
        (StackBucket::Deep, l) if all_in(l) => DEEP_ALL_IN,
        (StackBucket::Deep, Lever::Postflop) => DEEP_POSTFLOP,
        _ => 1.0,
    }
}

fn stage_factor(lever: Lever, ctx: &EngineContext) -> f64 {
    match (ctx.stage, lever) {
        (Some(Stage::Late), Lever::Steal) => LATE_STAGE_PRIORITY,
        (Some(Stage::Late), l) if all_in(l) => LATE_STAGE_PRIORITY,
        (Some(Stage::Late), Lever::Postflop) => LATE_STAGE_POSTFLOP,
        (Some(Stage::Early), Lever::Limp) => EARLY_STAGE_LIMP,
        _ => 1.0,
    }
}

fn bounty_factor(lever: Lever, ctx: &EngineContext) -> f64 {
    let Some(bounty) = &ctx.bounty else { return 1.0 };
    if !ctx.effective_stack_bb.is_some_and(|eff| eff <= BOUNTY_MAX_BB + 1e-9) {
        return 1.0;
    }
    match (bounty.hero_covers, lever) {
        (Some(true), Lever::Shove | Lever::Reshove) if bounty.is_big() => BOUNTY_CALL_PRIORITY,
        (Some(false), Lever::CallShove) => BOUNTY_COVERED_PRIORITY,
        _ => 1.0,
    }
}

/// The context multiplier of a read (section 7.2): seat × stack depth ×
/// tournament stage × bounty, each 1.0 where it does not apply. `None`
/// (no table scope) is 1.0.
pub fn context_multiplier(def: &RuleDef, context: Option<&EngineContext>) -> f64 {
    let Some(ctx) = context else { return 1.0 };
    let lever = lever(def.id);
    seat_factor(def, ctx) * stack_factor(lever, ctx) * stage_factor(lever, ctx) * bounty_factor(lever, ctx)
}

/// Facts of the latest hand rather than tendencies of the player: his stack
/// depth (`ctx.short_stack`, `ctx.deep_stack`) and his bounty
/// (`ko.big_bounty`). They carry a fixed score at 100% confidence, so they
/// would outrank most real reads; section 7 ranks them after every
/// tendency (mock MTT session report, item 5).
pub fn is_fact(rule_id: &str) -> bool {
    rule_id.starts_with("ctx.") || rule_id == "ko.big_bounty"
}

/// Section 7's order: tendencies before facts ([`is_fact`]), then score
/// descending, then confidence descending, then rule id ascending. Total
/// and deterministic, whatever order the reads came in.
pub fn compare(a: &EngineRuleResult, b: &EngineRuleResult) -> Ordering {
    is_fact(&a.rule_id)
        .cmp(&is_fact(&b.rule_id))
        .then_with(|| b.score.total_cmp(&a.score))
        .then_with(|| b.confidence_pct.cmp(&a.confidence_pct))
        .then_with(|| a.rule_id.cmp(&b.rule_id))
}

/// Every read in ranking order.
pub fn rank(mut reads: Vec<EngineRuleResult>) -> Vec<EngineRuleResult> {
    reads.sort_by(compare);
    reads
}

/// Why the chip shows its tag.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TagSource {
    /// `rec.tilt` fired.
    Tilt,
    /// A tournament villain on 25bb or less without an all-in read.
    Stack,
    /// A read's own tag: the top-ranked read, or at 25bb or less in a
    /// tournament his best-ranked all-in read.
    Read,
}

/// The `tag` object of `PlayerPayload.engine` (section 13).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChipTag {
    pub text: String,
    pub rule_id: String,
    pub source: TagSource,
}

/// Whether `text` is a valid chip tag: two to four of `A-Z`, `0-9`, `+`
/// and `-` (`F3B`, `ST-`, `TILT`), or a stack of one or two digits followed
/// by `bb` (`9bb`, `25bb`).
pub fn is_valid_tag(text: &str) -> bool {
    let code = (2..=4).contains(&text.len())
        && text.bytes().all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'+' || b == b'-');
    let stack = text
        .strip_suffix("bb")
        .is_some_and(|n| (1..=2).contains(&n.len()) && n.bytes().all(|b| b.is_ascii_digit()));
    code || stack
}

fn tournament(context: &EngineContext) -> bool {
    matches!(context.format, FormatKey::Mtt | FormatKey::Spin)
}

/// The chip tag by section 7's precedence, first match wins: `TILT` when
/// `rec.tilt` fired; for a tournament villain on 25bb or less, the tag of
/// his best-ranked all-in read (his shoves, reshoves and calls of shoves:
/// `TCS`, `SHV`, `RSV`, `ICM-`, `KO+`…), else `NNbb` when his own stack
/// after the last hand is 25bb or less too (his stack rounded down, never
/// the hero's); the top-ranked read's tag (`ctx.short_stack` excluded);
/// otherwise none. `ranked` must be in [`rank`] order.
pub fn chip_tag(ranked: &[EngineRuleResult], context: Option<&EngineContext>) -> Option<ChipTag> {
    if let Some(tilt) = ranked.iter().find(|r| r.rule_id == TILT_RULE) {
        return Some(ChipTag {
            text: tilt.tag.clone().unwrap_or_else(|| "TILT".into()),
            rule_id: tilt.rule_id.clone(),
            source: TagSource::Tilt,
        });
    }
    if let Some(ctx) = context.filter(|c| tournament(c)) {
        if let Some(eff) = ctx.effective_stack_bb.filter(|e| *e <= STACK_TAG_MAX_BB + 1e-9) {
            // The stack is already on the PokerStars table; his shove/call
            // tendency is what wins chips at this depth (mock MTT session
            // report, item 1).
            let all_in_read = ranked
                .iter()
                .filter(|r| all_in(lever(&r.rule_id)))
                .find_map(|r| r.tag.as_ref().filter(|t| is_valid_tag(t)).map(|t| (t, r)));
            if let Some((text, read)) = all_in_read {
                return Some(ChipTag { text: text.clone(), rule_id: read.rule_id.clone(), source: TagSource::Read });
            }
            // His own stack, not the effective one: when the hero is the
            // short one, the effective stack is the hero's, and every chip
            // at the table showed the same number (report item 4). A deep
            // villain gets no stack tag at all.
            let own = ctx.stack_bb.unwrap_or(eff);
            if own <= STACK_TAG_MAX_BB + 1e-9 {
                let short = ranked.iter().any(|r| r.rule_id == SHORT_STACK_RULE);
                return Some(ChipTag {
                    text: format!("{}bb", own.max(0.0).floor() as u32),
                    rule_id: if short { SHORT_STACK_RULE } else { STACK_TAG_RULE }.into(),
                    source: TagSource::Stack,
                });
            }
        }
    }
    // `ctx.short_stack`'s own tag is the effective stack, so it never names
    // the chip here: that is the case above, or the hero's stack.
    let top = ranked.iter().find(|r| r.rule_id != SHORT_STACK_RULE)?;
    let text = top.tag.clone().filter(|t| is_valid_tag(t))?;
    Some(ChipTag { text, rule_id: top.rule_id.clone(), source: TagSource::Read })
}

/// One villain's ranked reads: all of them, the top two and the chip tag.
#[derive(Debug, Clone, PartialEq)]
pub struct RankedReads {
    pub reads: Vec<EngineRuleResult>,
    pub top_reads: Vec<EngineRuleResult>,
    pub tag: Option<ChipTag>,
}

/// Ranks the rules' reads and picks the top reads and the chip tag.
pub fn rank_reads(reads: Vec<EngineRuleResult>, context: Option<&EngineContext>) -> RankedReads {
    let reads = rank(reads);
    let tag = chip_tag(&reads, context);
    let top_reads = reads.iter().take(TOP_READS).cloned().collect();
    RankedReads { reads, top_reads, tag }
}
