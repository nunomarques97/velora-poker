//! Small-sample honesty (`docs/specs/opponent-engine.md`, section 4): the
//! built-in per-format priors, shrinkage toward them (partial pooling), the
//! confidence of a read, and the local-database pool that refines the priors.
//!
//! Catalogue rows X01 (`pool.shrinkage`) and X02 (`pool.priors`).
//!
//! The pool is cached in memory and keyed by the database's import
//! generation (`engine_state`, bumped by triggers in `db::migrate_schema`).
//! Twelve overlays share one SQLite mutex, so an overlay refresh only reads
//! two counters; the pool is walked again only after an import committed,
//! and then only over the hands that import added.

use std::collections::{HashMap, HashSet};

use rusqlite::Connection;

use super::facts::{load_hands_after, HandFacts, StatKey};
use super::postflop::extract_postflop;
use super::preflop::extract_preflop;
use crate::description_rules::ConfidenceTier;

/// Spec rule id of the shrinkage and confidence (catalogue row X01).
pub const RULE_SHRINKAGE: &str = "pool.shrinkage";
/// Spec rule id of the pool refinement of the priors (catalogue row X02).
pub const RULE_POOL_PRIORS: &str = "pool.priors";

/// Weight of the built-in prior when the pool refines it (section 4.2).
pub const POOL_PRIOR_WEIGHT: f64 = 500.0;
/// Pool opportunities needed before a stat's prior is refined.
pub const POOL_MIN_OPPORTUNITIES: u64 = 2000;
/// Distinct non-hero players needed before a stat's prior is refined.
pub const POOL_MIN_PLAYERS: usize = 50;
/// Hands loaded per query while walking the pool.
const POOL_BATCH: i64 = 500;

/// The format key the priors are indexed by (spec section 3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum FormatKey {
    Mtt,
    Cash,
    Zoom,
    Spin,
}

impl FormatKey {
    pub const ALL: [FormatKey; 4] = [FormatKey::Mtt, FormatKey::Cash, FormatKey::Zoom, FormatKey::Spin];

    /// From `hands.variant`, falling back to `hands.format` for a row the
    /// reparse backfill has not reached.
    pub fn from_variant(variant: Option<&str>, format: &str) -> FormatKey {
        match variant {
            Some("zoom_cash") => FormatKey::Zoom,
            Some("tournament") | Some("zoom_tournament") => FormatKey::Mtt,
            Some("spin") => FormatKey::Spin,
            Some("cash") => FormatKey::Cash,
            _ if format == "tournament" => FormatKey::Mtt,
            _ => FormatKey::Cash,
        }
    }

    pub fn of_hand(hand: &HandFacts) -> FormatKey {
        FormatKey::from_variant(hand.variant.as_deref(), &hand.format)
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            FormatKey::Mtt => "mtt",
            FormatKey::Cash => "cash",
            FormatKey::Zoom => "zoom",
            FormatKey::Spin => "spin",
        }
    }

    fn column(&self) -> usize {
        match self {
            FormatKey::Mtt => 0,
            FormatKey::Cash => 1,
            FormatKey::Zoom => 2,
            FormatKey::Spin => 3,
        }
    }

    /// Recency half-life in villain hands (spec section 6).
    pub fn half_life(&self) -> f64 {
        match self {
            FormatKey::Mtt => 100.0,
            FormatKey::Cash => 150.0,
            FormatKey::Zoom => 200.0,
            FormatKey::Spin => 60.0,
        }
    }
}

/// One row of the spec's table 4.3.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StatSpec {
    pub k: f64,
    /// Minimum stat-specific opportunities before any read on the stat shows.
    pub n_min: u32,
    /// Typical between-player spread, for the ranking deviation.
    pub scale: f64,
    /// Built-in priors by format (MTT, 6-max cash, Zoom, Spin & Go); `None`
    /// where the spot does not exist in that format.
    pub priors: [Option<f64>; 4],
}

impl StatSpec {
    /// The built-in prior for a format. Where the spot does not exist in
    /// that format (an EP open in a 3-max Spin) but the player has it from
    /// another format, the cash prior stands in.
    pub fn builtin_prior(&self, format: FormatKey) -> f64 {
        self.priors[format.column()]
            .or(self.priors[FormatKey::Cash.column()])
            .unwrap_or(0.0)
    }
}

/// Table 4.3 of the spec.
pub fn stat_spec(key: StatKey) -> StatSpec {
    use StatKey::*;
    let (k, n_min, scale, priors) = match key {
        Vpip => (20.0, 15, 0.08, [0.24, 0.27, 0.23, 0.38]),
        Pfr => (20.0, 15, 0.06, [0.17, 0.20, 0.19, 0.25]),
        RfiEp => return spec_without_spin(15.0, 8, 0.05, [0.14, 0.16, 0.16]),
        RfiMp => return spec_without_spin(15.0, 8, 0.06, [0.19, 0.21, 0.21]),
        RfiCo => return spec_without_spin(15.0, 8, 0.08, [0.27, 0.28, 0.29]),
        RfiBtn => (15.0, 8, 0.10, [0.40, 0.42, 0.45, 0.50]),
        RfiSb => (15.0, 8, 0.10, [0.35, 0.36, 0.38, 0.45]),
        Steal => (15.0, 8, 0.09, [0.33, 0.35, 0.37, 0.48]),
        FoldToStealSb => (12.0, 8, 0.10, [0.75, 0.78, 0.80, 0.60]),
        FoldToStealBb => (12.0, 8, 0.12, [0.55, 0.60, 0.62, 0.45]),
        BbDefendVsSb => (12.0, 8, 0.12, [0.55, 0.52, 0.50, 0.60]),
        ThreeBetIp => (25.0, 12, 0.03, [0.07, 0.08, 0.09, 0.10]),
        ThreeBetOop => (25.0, 12, 0.03, [0.06, 0.07, 0.08, 0.09]),
        FoldTo3betIp => (15.0, 8, 0.12, [0.55, 0.55, 0.52, 0.45]),
        FoldTo3betOop => (15.0, 8, 0.12, [0.60, 0.62, 0.60, 0.50]),
        FourBet => (20.0, 8, 0.04, [0.07, 0.08, 0.09, 0.10]),
        FoldTo4bet => (10.0, 6, 0.15, [0.45, 0.50, 0.50, 0.40]),
        Squeeze => (15.0, 8, 0.03, [0.06, 0.07, 0.08, 0.09]),
        ColdCall => (20.0, 10, 0.05, [0.08, 0.09, 0.08, 0.10]),
        Limp => (20.0, 10, 0.06, [0.06, 0.05, 0.03, 0.12]),
        LimpFold => (8.0, 5, 0.15, [0.55, 0.55, 0.55, 0.55]),
        LimpCall => (8.0, 5, 0.15, [0.40, 0.40, 0.40, 0.40]),
        LimpReraise => (8.0, 5, 0.05, [0.05, 0.05, 0.05, 0.05]),
        IsoRaise => (12.0, 6, 0.15, [0.45, 0.55, 0.55, 0.40]),
        OpenShove => (10.0, 6, 0.15, [0.20, 0.10, 0.10, 0.25]),
        CallVsShove => (10.0, 6, 0.10, [0.25, 0.25, 0.25, 0.30]),
        Reshove => (10.0, 6, 0.06, [0.10, 0.08, 0.08, 0.14]),
        CbetFlop => (15.0, 8, 0.12, [0.55, 0.58, 0.60, 0.62]),
        CbetTurn => (12.0, 6, 0.12, [0.45, 0.48, 0.48, 0.45]),
        CbetRiver => (10.0, 6, 0.12, [0.45, 0.48, 0.48, 0.42]),
        FoldToCbetFlop => (15.0, 8, 0.12, [0.45, 0.45, 0.45, 0.40]),
        FoldToCbetTurn => (12.0, 6, 0.12, [0.42, 0.45, 0.45, 0.40]),
        FoldToCbetRiver => (10.0, 6, 0.12, [0.45, 0.48, 0.48, 0.45]),
        DelayedCbet => (10.0, 6, 0.12, [0.35, 0.38, 0.38, 0.35]),
        CheckRaiseFlop => (20.0, 10, 0.04, [0.08, 0.09, 0.10, 0.10]),
        DonkFlop => (20.0, 10, 0.04, [0.06, 0.05, 0.04, 0.08]),
        ProbeTurn => (12.0, 6, 0.12, [0.35, 0.35, 0.38, 0.38]),
        FloatFlop => (10.0, 6, 0.12, [0.40, 0.42, 0.45, 0.45]),
        RiverBet => (15.0, 8, 0.10, [0.30, 0.32, 0.32, 0.32]),
        RiverRaise => (15.0, 8, 0.04, [0.07, 0.07, 0.07, 0.08]),
        Wtsd => (20.0, 10, 0.06, [0.28, 0.27, 0.26, 0.30]),
        Wsd => (15.0, 8, 0.08, [0.50, 0.50, 0.50, 0.50]),
        Wwsf => (20.0, 10, 0.06, [0.45, 0.46, 0.45, 0.47]),
    };
    StatSpec { k, n_min, scale, priors: priors.map(Some) }
}

fn spec_without_spin(k: f64, n_min: u32, scale: f64, priors: [f64; 3]) -> StatSpec {
    StatSpec { k, n_min, scale, priors: [Some(priors[0]), Some(priors[1]), Some(priors[2]), None] }
}

/// `(hits + k·prior) / (opps + k)` (section 4.1). Works on weighted counts
/// too, for the recency view.
pub fn shrink(hits: f64, opps: f64, k: f64, prior: f64) -> f64 {
    (hits + k * prior) / (opps + k)
}

/// `n / (n + k)` on the stat-specific opportunity count (section 4.4).
pub fn confidence(opportunities: u32, k: f64) -> f64 {
    let n = f64::from(opportunities);
    n / (n + k)
}

/// The tier of a confidence: below `n_min` the read is not shown at all.
pub fn confidence_tier(opportunities: u32, spec: &StatSpec) -> ConfidenceTier {
    if opportunities < spec.n_min {
        return ConfidenceTier::InsufficientData;
    }
    let c = confidence(opportunities, spec.k);
    if c >= 0.60 {
        ConfidenceTier::High
    } else if c >= 0.35 {
        ConfidenceTier::Medium
    } else {
        ConfidenceTier::Low
    }
}

/// One player stat after partial pooling. `raw` is the player's own number
/// and is `None` with no opportunity: the prior is never shown as the
/// player's value. Rules evaluate `shrunk`; the UI shows `raw` with
/// `hits/opportunities`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShrunkStat {
    pub key: StatKey,
    pub hits: u32,
    pub opportunities: u32,
    pub raw: Option<f64>,
    pub prior: f64,
    pub shrunk: f64,
    pub confidence: f64,
    pub confidence_pct: u8,
    pub tier: ConfidenceTier,
}

impl ShrunkStat {
    pub fn new(key: StatKey, hits: u32, opportunities: u32, prior: f64) -> ShrunkStat {
        let spec = stat_spec(key);
        let c = confidence(opportunities, spec.k);
        ShrunkStat {
            key,
            hits,
            opportunities,
            raw: (opportunities > 0).then(|| f64::from(hits) / f64::from(opportunities)),
            prior,
            shrunk: shrink(f64::from(hits), f64::from(opportunities), spec.k, prior),
            confidence: c,
            confidence_pct: (100.0 * c).round() as u8,
            tier: confidence_tier(opportunities, &spec),
        }
    }

    /// Whether any read on this stat may be shown (`n >= n_min`).
    pub fn displayable(&self) -> bool {
        self.tier != ConfidenceTier::InsufficientData
    }
}

/// Pool counts of one stat in one format.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PoolEntry {
    pub hits: u64,
    pub opportunities: u64,
    pub players: HashSet<i64>,
}

/// The local-database pool: every non-hero player's events, summed per
/// format and stat. Pure; [`PoolCache`] feeds it from the database.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PoolTally {
    entries: HashMap<(FormatKey, StatKey), PoolEntry>,
    hands: u64,
}

impl PoolTally {
    /// Adds one completed hand: the events of every dealt-in player except
    /// the hero.
    pub fn add_hand(&mut self, hand: &HandFacts) {
        let format = FormatKey::of_hand(hand);
        self.hands += 1;
        for seat in hand.seats.iter().filter(|s| !s.is_hero) {
            let events = extract_preflop(hand, seat.player_id)
                .into_iter()
                .chain(extract_postflop(hand, seat.player_id));
            for event in events.filter(|e| e.opportunity) {
                self.add(format, event.key, seat.player_id, event.success);
            }
        }
    }

    /// Adds one opportunity of one player.
    pub fn add(&mut self, format: FormatKey, key: StatKey, player_id: i64, success: bool) {
        let entry = self.entries.entry((format, key)).or_default();
        entry.opportunities += 1;
        entry.hits += u64::from(success);
        entry.players.insert(player_id);
    }

    pub fn entry(&self, format: FormatKey, key: StatKey) -> Option<&PoolEntry> {
        self.entries.get(&(format, key))
    }

    /// Hands walked into the pool so far.
    pub fn hands(&self) -> u64 {
        self.hands
    }

    /// The prior a player in `format` is shrunk toward on `key`: the
    /// built-in value, refined by the pool once it has at least 2,000
    /// opportunities from at least 50 distinct players (section 4.2).
    pub fn prior(&self, format: FormatKey, key: StatKey) -> f64 {
        let builtin = stat_spec(key).builtin_prior(format);
        match self.entry(format, key) {
            Some(pool) if self.is_refined(format, key) => {
                (pool.hits as f64 + POOL_PRIOR_WEIGHT * builtin)
                    / (pool.opportunities as f64 + POOL_PRIOR_WEIGHT)
            }
            _ => builtin,
        }
    }

    /// Whether the pool is large enough to refine the prior of `key` in
    /// `format`.
    pub fn is_refined(&self, format: FormatKey, key: StatKey) -> bool {
        self.entry(format, key).is_some_and(|p| {
            p.opportunities >= POOL_MIN_OPPORTUNITIES && p.players.len() >= POOL_MIN_PLAYERS
        })
    }
}

/// The database's import generation (`engine_state`): `imports` moves on
/// every new hand, `rebuilds` when stored hands were changed or removed
/// (ingestion repair, reparse backfill), which an incremental walk cannot
/// account for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Generation {
    pub imports: i64,
    pub rebuilds: i64,
}

impl Generation {
    pub fn read(conn: &Connection) -> rusqlite::Result<Generation> {
        let get = |key: &str| -> rusqlite::Result<i64> {
            conn.query_row("SELECT value FROM engine_state WHERE key = ?1", [key], |row| row.get(0))
        };
        Ok(Generation { imports: get("import_generation")?, rebuilds: get("rebuild_generation")? })
    }
}

/// How often the cache walked the database: a test hook proving that a
/// refresh without a new import recomputes nothing.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PoolPasses {
    pub full: u32,
    pub incremental: u32,
}

/// The in-memory pool, keyed by the import generation. One instance lives
/// next to the database connection and is shared by every overlay.
#[derive(Debug, Default)]
pub struct PoolCache {
    generation: Option<Generation>,
    /// Highest `hands.id` walked into the tally.
    watermark: i64,
    tally: PoolTally,
    passes: PoolPasses,
}

impl PoolCache {
    pub fn new() -> PoolCache {
        PoolCache::default()
    }

    /// The pool, current with the database. Reads the generation (one
    /// query); walks hands only when an import committed since the last
    /// call: the new hands after an import, every hand after a rebuild.
    pub fn tally(&mut self, conn: &Connection) -> rusqlite::Result<&PoolTally> {
        let current = Generation::read(conn)?;
        match self.generation {
            Some(seen) if seen == current => {}
            Some(seen) if seen.rebuilds == current.rebuilds => {
                self.walk(conn)?;
                self.passes.incremental += 1;
            }
            _ => {
                self.tally = PoolTally::default();
                self.watermark = 0;
                self.walk(conn)?;
                self.passes.full += 1;
            }
        }
        self.generation = Some(current);
        Ok(&self.tally)
    }

    /// Shorthand for `tally(conn)?.prior(format, key)`.
    pub fn prior(&mut self, conn: &Connection, format: FormatKey, key: StatKey) -> rusqlite::Result<f64> {
        Ok(self.tally(conn)?.prior(format, key))
    }

    pub fn passes(&self) -> PoolPasses {
        self.passes
    }

    /// Drops the cached pool; the next call walks the database again.
    pub fn invalidate(&mut self) {
        self.generation = None;
    }

    fn walk(&mut self, conn: &Connection) -> rusqlite::Result<()> {
        loop {
            let batch = load_hands_after(conn, self.watermark, POOL_BATCH)?;
            let Some(last) = batch.iter().map(|h| h.id).max() else { break };
            for hand in &batch {
                self.tally.add_hand(hand);
            }
            self.watermark = last;
        }
        Ok(())
    }
}
