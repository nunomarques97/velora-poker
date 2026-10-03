//! `PlayerPayload.engine` (`docs/specs/opponent-engine.md`, section 13): the
//! JSON the overlay chip, hover card and drawer read. Pure: built from a
//! [`PlayerReplay`], the current priors and the between-hands context.
//!
//! The payload is only ever built in the `strategic-analysis` build; the
//! default build sends `engine: null` (gating matrix, section 2).

use serde::Serialize;

use super::context::EngineContext;
use super::aggregate::HeadToHead;
use super::facts::StatKey;
use super::rank::{rank_reads, ChipTag};
use super::recency::RecentForm;
use super::rules::{evaluate, EngineRuleResult, PlayerReplay};
use super::showdown::{sizing_tells, LastAggression, LineStep, ShowdownRecord, SizingTell};

/// Version of the `engine` JSON contract.
pub const ENGINE_PAYLOAD_VERSION: u32 = 1;
/// Showdowns the payload carries, newest first.
pub const MAX_SHOWDOWNS: usize = 10;

fn round2(value: f64) -> f64 {
    (value * 100.0).round() / 100.0
}

/// One step of a shown line.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LineStepPayload {
    pub street: &'static str,
    pub action: &'static str,
    pub is_all_in: bool,
    pub size_bucket: Option<&'static str>,
    pub pot_fraction: Option<f64>,
}

impl From<&LineStep> for LineStepPayload {
    fn from(step: &LineStep) -> Self {
        LineStepPayload {
            street: step.street.as_str(),
            action: step.action.as_str(),
            is_all_in: step.is_all_in,
            size_bucket: step.size_bucket.map(|b| b.as_str()),
            pot_fraction: step.pot_fraction.map(round2),
        }
    }
}

/// The classified last aggression of a shown line.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LastAggressionPayload {
    pub street: &'static str,
    pub size_bucket: &'static str,
    pub pot_fraction: f64,
    pub class: &'static str,
}

impl From<&LastAggression> for LastAggressionPayload {
    fn from(a: &LastAggression) -> Self {
        LastAggressionPayload {
            street: a.street.as_str(),
            size_bucket: a.size_bucket.as_str(),
            pot_fraction: round2(a.pot_fraction),
            class: a.class.as_str(),
        }
    }
}

/// One `showdowns` item.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShowdownPayload {
    /// PokerStars' hand number.
    pub hand_id: String,
    pub played_at: Option<String>,
    pub cards: String,
    pub board: String,
    pub category: &'static str,
    pub line: Vec<LineStepPayload>,
    pub result: &'static str,
    pub last_aggression: Option<LastAggressionPayload>,
}

impl From<&ShowdownRecord> for ShowdownPayload {
    fn from(r: &ShowdownRecord) -> Self {
        ShowdownPayload {
            hand_id: r.hand_ref.clone(),
            played_at: r.played_at.clone(),
            cards: r.cards.clone(),
            board: r.board.clone(),
            category: r.category.as_str(),
            line: r.line.iter().map(LineStepPayload::from).collect(),
            result: r.result.as_str(),
            last_aggression: r.last_aggression.as_ref().map(LastAggressionPayload::from),
        }
    }
}

/// One `sizingTells` item.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SizingTellPayload {
    pub bucket: &'static str,
    pub value: u32,
    pub bluff: u32,
    pub neither: u32,
    pub n: u32,
}

impl From<&SizingTell> for SizingTellPayload {
    fn from(t: &SizingTell) -> Self {
        SizingTellPayload {
            bucket: t.bucket.as_str(),
            value: t.value,
            bluff: t.bluff,
            neither: t.neither,
            n: t.n,
        }
    }
}

/// Where a note came from; auto-notes always say `auto`, so the drawer
/// keeps them apart from the editable manual note.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NoteSource {
    Auto,
}

/// One `autoNotes` item (section 11).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AutoNotePayload {
    pub id: i64,
    /// PokerStars' hand number.
    pub hand_id: String,
    pub kind: String,
    pub text: String,
    pub created_at: String,
    pub source: NoteSource,
}

/// `PlayerPayload.engine` in the `strategic-analysis` build (section 13).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EnginePayload {
    pub version: u32,
    pub tag: Option<ChipTag>,
    /// The first two of `reads`.
    pub top_reads: Vec<EngineRuleResult>,
    /// Every eligible read, ranked (section 7).
    pub reads: Vec<EngineRuleResult>,
    /// `None` without a table scope (Players list, drawer outside a table).
    pub context: Option<EngineContext>,
    pub head_to_head: Option<HeadToHead>,
    pub showdowns: Vec<ShowdownPayload>,
    pub sizing_tells: Vec<SizingTellPayload>,
    pub recent_form: Option<RecentForm>,
    pub auto_notes: Vec<AutoNotePayload>,
}

/// Builds the payload from a replay, the priors of its format and the
/// between-hands context. No hand is replayed here.
pub fn engine_payload(
    replay: &PlayerReplay,
    prior: &dyn Fn(StatKey) -> f64,
    context: Option<EngineContext>,
) -> EnginePayload {
    let input = replay.rule_input(prior, context);
    let ranked = rank_reads(evaluate(&input), input.context.as_ref());
    EnginePayload {
        version: ENGINE_PAYLOAD_VERSION,
        tag: ranked.tag,
        top_reads: ranked.top_reads,
        reads: ranked.reads,
        head_to_head: replay.agg.head_to_head(prior),
        showdowns: replay.showdowns.iter().rev().take(MAX_SHOWDOWNS).map(ShowdownPayload::from).collect(),
        sizing_tells: sizing_tells(&replay.showdowns).iter().map(SizingTellPayload::from).collect(),
        recent_form: input.recent_form,
        context: input.context,
        auto_notes: Vec::new(),
    }
}
