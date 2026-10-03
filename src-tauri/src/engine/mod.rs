//! Opponent engine (`docs/specs/opponent-engine.md`): an amount-aware replay
//! of completed hands that turns each opponent's history into scenario stats.
//!
//! The engine sits next to `stats` and `description_rules` rather than
//! replacing them: their tests are the current HUD contract, and the engine
//! only feeds the `strategic-analysis` payload. Everything here is a pure
//! function of stored, completed hands — no in-hand state, no network.

pub mod facts;
pub mod preflop;

use rusqlite::Connection;

pub use facts::{
    load_player_hands, ActionFact, Counterparty, HandEvents, HandFacts, Relation, SeatFact,
    StatEvent, StatKey,
};
pub use preflop::{extract_player_preflop, extract_preflop};

/// Loads one player's hands (a bounded number of queries) and extracts their
/// preflop and stack-depth events, oldest hand first.
pub fn player_preflop_events(conn: &Connection, player_id: i64) -> rusqlite::Result<Vec<HandEvents>> {
    let hands = load_player_hands(conn, player_id)?;
    Ok(extract_player_preflop(&hands, player_id))
}
