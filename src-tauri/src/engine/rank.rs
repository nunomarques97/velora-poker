//! Read ranking and the chip tag (`docs/specs/opponent-engine.md`, section
//! 7): the biggest, surest deviations go first, and a 2–4 character tag
//! names the villain at a glance across a dozen tables.
//!
//! Catalogue rows G01 (`rank.score`), G02 (`rank.tag`) and X03
//! (`gate.min_hands_archetype`). Pure functions of the reads the rules
//! produced and the between-hands context; nothing here sees a hand in
//! progress.

use std::cmp::Ordering;

use serde::Serialize;

use super::context::EngineContext;
use super::pooling::FormatKey;
use super::rules::EngineRuleResult;

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
/// The stack tag replaces the read tag in tournaments at or below this
/// effective stack (bb).
pub const STACK_TAG_MAX_BB: f64 = 25.0;
/// The rule id a stack tag reports when `ctx.short_stack` did not fire
/// (15–25bb): the effective-stack derivation itself (row S06).
const STACK_TAG_RULE: &str = "ctx.effective_stack";
const SHORT_STACK_RULE: &str = "ctx.short_stack";
const TILT_RULE: &str = "rec.tilt";

/// `deviation × confidence × multiplier` (section 7). Context and recency
/// reads carry a fixed base instead of `deviation × confidence`; the rules
/// apply that themselves, so ranking only ever reads `score`.
pub fn score(deviation: f64, confidence: f64, multiplier: f64) -> f64 {
    deviation * confidence * multiplier
}

/// Section 7's order: score descending, then confidence descending, then
/// rule id ascending. Total and deterministic, whatever order the reads
/// came in.
pub fn compare(a: &EngineRuleResult, b: &EngineRuleResult) -> Ordering {
    b.score
        .total_cmp(&a.score)
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
    /// A tournament villain on 25bb or less.
    Stack,
    /// The top-ranked read's own tag.
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
/// `rec.tilt` fired; `NNbb` (effective stack rounded down) for a tournament
/// villain on 25bb or less; the top-ranked read's tag; otherwise none.
/// `ranked` must be in [`rank`] order.
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
            let short = ranked.iter().any(|r| r.rule_id == SHORT_STACK_RULE);
            return Some(ChipTag {
                text: format!("{}bb", eff.max(0.0).floor() as u32),
                rule_id: if short { SHORT_STACK_RULE } else { STACK_TAG_RULE }.into(),
                source: TagSource::Stack,
            });
        }
    }
    let top = ranked.first()?;
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
