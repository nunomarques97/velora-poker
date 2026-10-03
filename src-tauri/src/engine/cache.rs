//! The engine's in-memory cache: the pool priors and one replay per player.
//!
//! Twelve overlays share one SQLite mutex, and every hand that completes at
//! any table refreshes them all. Replaying a regular's whole history on each
//! of those refreshes would hold the mutex for nothing, so a player's
//! [`PlayerReplay`] is kept until he has a new hand: the key is his hand
//! count and his latest `hands.id`, plus the database's rebuild generation
//! (a repair or reparse backfill changes stored hands in place) and the
//! format the replay was made for. A refresh with no new hand for him costs
//! one count query and the cheap shrink-and-evaluate step.

use std::collections::HashMap;

use rusqlite::{params, Connection};

use super::context::EngineContext;
use super::payload::{engine_payload, EnginePayload};
use super::pooling::{FormatKey, Generation, PoolCache};
use super::rules::{player_replay, PlayerReplay};

/// Players kept in memory; the least recently used one is dropped beyond.
pub const PLAYER_CACHE_CAP: usize = 512;

/// What a cached replay is valid for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ReplayKey {
    hands: i64,
    latest_hand_id: i64,
    rebuilds: i64,
}

#[derive(Debug)]
struct Entry {
    key: ReplayKey,
    replay: PlayerReplay,
    last_used: u64,
}

/// How often players were replayed or served from the cache: a test hook
/// proving a refresh without a new hand replays nothing.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ReplayCounts {
    pub replays: u32,
    pub hits: u32,
}

/// Pool priors plus per-player replays. One instance lives next to the
/// database connection (`AppState`); lock it after the connection.
#[derive(Debug, Default)]
pub struct EngineCache {
    pool: PoolCache,
    players: HashMap<i64, Entry>,
    tick: u64,
    counts: ReplayCounts,
}

fn replay_key(conn: &Connection, player_id: i64, rebuilds: i64) -> rusqlite::Result<ReplayKey> {
    conn.query_row(
        "SELECT COUNT(*), COALESCE(MAX(hand_id), 0) FROM player_hands WHERE player_id = ?1",
        params![player_id],
        |row| Ok(ReplayKey { hands: row.get(0)?, latest_hand_id: row.get(1)?, rebuilds }),
    )
}

impl EngineCache {
    pub fn new() -> EngineCache {
        EngineCache::default()
    }

    pub fn counts(&self) -> ReplayCounts {
        self.counts
    }

    /// The pool cache (shared priors).
    pub fn pool(&mut self) -> &mut PoolCache {
        &mut self.pool
    }

    /// The player's replay, current with the database. Replays his hands
    /// only when he has a new one (or stored hands were rebuilt, or the
    /// table context asks for another format).
    pub fn replay(
        &mut self,
        conn: &Connection,
        player_id: i64,
        format: Option<FormatKey>,
    ) -> rusqlite::Result<&PlayerReplay> {
        let generation = Generation::read(conn)?;
        let key = replay_key(conn, player_id, generation.rebuilds)?;
        self.tick += 1;
        let fresh = self.players.get(&player_id).is_some_and(|e| {
            e.key == key && format.map_or(true, |f| f == e.replay.format)
        });
        if fresh {
            self.counts.hits += 1;
        } else {
            let replay = player_replay(conn, player_id, format)?;
            self.counts.replays += 1;
            self.players.insert(player_id, Entry { key, replay, last_used: self.tick });
            self.evict();
        }
        let entry = self.players.get_mut(&player_id).expect("entry just ensured");
        entry.last_used = self.tick;
        Ok(&entry.replay)
    }

    /// The `engine` payload of one player, adapted to `context` (the latest
    /// completed hand at his table, when there is a table scope).
    pub fn payload(
        &mut self,
        conn: &Connection,
        player_id: i64,
        context: Option<EngineContext>,
    ) -> rusqlite::Result<EnginePayload> {
        let format = context.as_ref().map(|c| c.format);
        self.replay(conn, player_id, format)?;
        // The pool walks only the hands imported since its last call.
        let tally = self.pool.tally(conn)?;
        let replay = &self.players[&player_id].replay;
        let prior = |key| tally.prior(replay.format, key);
        Ok(engine_payload(replay, &prior, context))
    }

    fn evict(&mut self) {
        while self.players.len() > PLAYER_CACHE_CAP {
            let Some(oldest) = self.players.iter().min_by_key(|(_, e)| e.last_used).map(|(id, _)| *id)
            else {
                break;
            };
            self.players.remove(&oldest);
        }
    }
}
