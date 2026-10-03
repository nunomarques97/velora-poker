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
use super::aggregate::View;
use super::facts::StatKey;
use super::payload::{engine_payload, AutoNotePayload, EnginePayload, NoteSource};
use super::pooling::{FormatKey, Generation, PoolCache};
use super::rules::{player_replay, PlayerReplay};
use super::table_quality::VillainQuality;
use crate::db;

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
    /// His auto-notes, newest first. Notes are only written for hands as
    /// they are imported (or by the one-time backfill, before any replay),
    /// so they change exactly when the key does.
    notes: Vec<AutoNotePayload>,
    last_used: u64,
}

fn load_notes(conn: &Connection, player_id: i64) -> rusqlite::Result<Vec<AutoNotePayload>> {
    Ok(db::list_auto_notes(conn, player_id)?
        .into_iter()
        .map(|row| AutoNotePayload {
            id: row.id,
            hand_id: row.hand_ref,
            kind: row.kind,
            text: row.text,
            created_at: row.created_at,
            source: NoteSource::Auto,
        })
        .collect())
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
            let notes = load_notes(conn, player_id)?;
            self.counts.replays += 1;
            self.players.insert(player_id, Entry { key, replay, notes, last_used: self.tick });
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
        let entry = &self.players[&player_id];
        let replay = &entry.replay;
        let prior = |key| tally.prior(replay.format, key);
        let mut payload = engine_payload(replay, &prior, context);
        payload.auto_notes = entry.notes.clone();
        Ok(payload)
    }

    /// What the table quality score (section 12) reads of one villain: his
    /// recency-view shrunk VPIP, PFR and WTSD against the priors of
    /// `format` (the table's), and his hand count. Reuses his cached replay.
    pub fn villain_quality(
        &mut self,
        conn: &Connection,
        player_id: i64,
        format: Option<FormatKey>,
    ) -> rusqlite::Result<VillainQuality> {
        self.replay(conn, player_id, format)?;
        let tally = self.pool.tally(conn)?;
        let replay = &self.players[&player_id].replay;
        let stat = |key: StatKey| {
            let prior = tally.prior(replay.format, key);
            (replay.agg.stat(View::Recency, key, prior).shrunk, prior)
        };
        let (vpip, prior_vpip) = stat(StatKey::Vpip);
        let (pfr, prior_pfr) = stat(StatKey::Pfr);
        let (wtsd, prior_wtsd) = stat(StatKey::Wtsd);
        Ok(VillainQuality {
            hands: replay.agg.hands,
            vpip,
            pfr,
            wtsd,
            prior_vpip,
            prior_pfr,
            prior_wtsd,
        })
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
