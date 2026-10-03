//! Opponent engine (`docs/specs/opponent-engine.md`): an amount-aware replay
//! of completed hands that turns each opponent's history into scenario stats.
//!
//! The engine sits next to `stats` and `description_rules` rather than
//! replacing them: their tests are the current HUD contract, and the engine
//! only feeds the `strategic-analysis` payload. Everything here is a pure
//! function of stored, completed hands — no in-hand state, no network.

pub mod aggregate;
pub mod context;
pub mod eval;
pub mod facts;
pub mod postflop;
pub mod pooling;
pub mod pot;
pub mod preflop;
pub mod recency;
pub mod rules;
pub mod showdown;

use rusqlite::Connection;

pub use aggregate::{
    aggregate_player, h2h_events, H2hKey, H2hStat, HeadToHead, PlayerAggregate, StatAgg, View,
    H2H_DISPLAY_MIN, H2H_RULE_MIN,
};
pub use context::{
    latest_table_hand, seat_relation, villain_context, BountyContext, EngineContext, SeatRelation,
    Side, StackBucket, Stage,
};
pub use eval::{
    board_plays, evaluate, hole_strength, parse_cards, Card, Category, HandValue, HoleStrength,
};
pub use facts::{
    load_hand, load_hands_after, load_player_hands, ActionFact, Counterparty, HandEvents, HandFacts, Relation, SeatFact,
    StatEvent, StatKey,
};
pub use postflop::{extract_player_postflop, extract_postflop};
pub use pooling::{
    confidence, confidence_tier, shrink, stat_spec, FormatKey, PoolCache, PoolTally, ShrunkStat,
    StatSpec,
};
pub use pot::{parse_total_pot, replay_pot, PotReplay, SizeBucket, SizedAction, UncalledReturn};
pub use preflop::{extract_player_preflop, extract_preflop};
pub use recency::{recent_form, recency_weight, FormFlag, RecentForm};
pub use rules::{
    evaluate as evaluate_rules, player_rule_input, rule_def, rule_input, templates, EngineEvidence, EngineRuleResult,
    Family, RuleDef, RuleInput, ShowdownTally, TemplateKind, RULES,
};
pub use showdown::{
    extract_showdowns, load_showdown_boards, parse_board, showdown_record, sizing_tally,
    sizing_tells, LastAggression, LineStep, ShowdownRecord, ShowdownResult, SizingTell, ValueClass,
    SIZING_TELL_MIN_SAMPLE,
};

/// Loads one player's hands (a bounded number of queries) and extracts their
/// preflop and stack-depth events, oldest hand first.
pub fn player_preflop_events(conn: &Connection, player_id: i64) -> rusqlite::Result<Vec<HandEvents>> {
    let hands = load_player_hands(conn, player_id)?;
    Ok(extract_player_preflop(&hands, player_id))
}

/// Loads one player's hands and extracts their postflop events, oldest hand
/// first.
pub fn player_postflop_events(conn: &Connection, player_id: i64) -> rusqlite::Result<Vec<HandEvents>> {
    let hands = load_player_hands(conn, player_id)?;
    Ok(extract_player_postflop(&hands, player_id))
}

/// Loads one player's hands and the boards of his shown showdowns, and
/// builds his showdown records, oldest hand first.
pub fn player_showdowns(conn: &Connection, player_id: i64) -> rusqlite::Result<Vec<ShowdownRecord>> {
    let hands = load_player_hands(conn, player_id)?;
    let boards = load_showdown_boards(conn, player_id)?;
    Ok(extract_showdowns(&hands, &boards, player_id))
}
