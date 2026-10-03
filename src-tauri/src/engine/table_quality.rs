//! Table quality and multi-table villains (catalogue rows Q01 and Q02,
//! section 12 of `docs/specs/opponent-engine.md`).
//!
//! Both are pure. The quality score reads each non-hero villain's shrunk
//! VPIP, PFR and WTSD against his format's priors and weighs him by how
//! many hands back them, so a table of strangers scores neutral rather
//! than soft or tough. Multi-table detection only compares the rosters of
//! the tracked tables; it never looks at a hand in progress.

use std::collections::BTreeMap;

use serde::Serialize;

/// Spec rule id of the quality score (row Q01).
pub const RULE_TABLE_QUALITY: &str = "table.quality";
/// Spec rule id of multi-table detection (row Q02).
pub const RULE_MULTI_TABLE: &str = "table.multi_table";

/// `L` and `P` divide by this spread (the VPIP scale of section 4.3).
const LOOSE_SCALE: f64 = 0.08;
/// `S` divides by this spread (the WTSD scale).
const SHOWDOWN_SCALE: f64 = 0.06;
/// Weights of `L` (loose), `P` (passive) and `S` (sticky) in `soft_i`
/// (D107): calling preflop is the surest marker of a losing player, so `P`
/// outweighs `L`, and a loose but aggressive regular is not counted soft.
const LOOSE_WEIGHT: f64 = 0.35;
const PASSIVE_WEIGHT: f64 = 0.45;
const STICKY_WEIGHT: f64 = 0.20;
/// The weighted sum is divided by this before clamping. D106 divided by 2,
/// which left a table of solid regulars at 42 ("average"); 1.25 puts it in
/// "tough" while a pool-average player still scores exactly 0.
const SOFTNESS_DIVISOR: f64 = 1.25;
/// `c_i = hands / (hands + HANDS_K)`.
const HANDS_K: f64 = 20.0;
/// No score below this many villain hands in total...
pub const QUALITY_MIN_TOTAL_HANDS: u32 = 30;
/// ...or when no villain has this many.
pub const QUALITY_MIN_VILLAIN_HANDS: u32 = 10;
/// Label thresholds on the 0–100 score.
const SOFT_AT: u8 = 60;
const TOUGH_AT: u8 = 40;

/// One non-hero villain as the quality score sees him. Rates are 0–1.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VillainQuality {
    /// Hands he was dealt into (all-time).
    pub hands: u32,
    pub vpip: f64,
    pub pfr: f64,
    pub wtsd: f64,
    pub prior_vpip: f64,
    pub prior_pfr: f64,
    pub prior_wtsd: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum QualityLabel {
    Soft,
    Average,
    Tough,
}

/// A table's score: 0 (tough) to 100 (soft), 50 neutral.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TableQuality {
    pub score: u8,
    pub label: QualityLabel,
    /// The villains' hands in total: the sample behind the score.
    pub basis_hands: u32,
}

/// `soft_i` of section 12, in −1..=1: looser than the prior, more passive
/// (a wider VPIP–PFR gap) and more showdown-bound is softer.
pub fn villain_softness(v: &VillainQuality) -> f64 {
    let loose = (v.vpip - v.prior_vpip) / LOOSE_SCALE;
    let passive = ((v.vpip - v.pfr) - (v.prior_vpip - v.prior_pfr)) / LOOSE_SCALE;
    let sticky = (v.wtsd - v.prior_wtsd) / SHOWDOWN_SCALE;
    ((LOOSE_WEIGHT * loose + PASSIVE_WEIGHT * passive + STICKY_WEIGHT * sticky) / SOFTNESS_DIVISOR)
        .clamp(-1.0, 1.0)
}

/// The table's quality from its non-hero villains, or `None` below the
/// sample: fewer than 30 villain hands in total, or no villain with 10+.
/// Each villain counts with weight `hands / (hands + 20)` but the sum is
/// divided by the number of villains, so unknown players pull the score
/// toward neutral (50).
pub fn table_quality(villains: &[VillainQuality]) -> Option<TableQuality> {
    let basis_hands: u32 = villains.iter().map(|v| v.hands).sum();
    if basis_hands < QUALITY_MIN_TOTAL_HANDS
        || !villains.iter().any(|v| v.hands >= QUALITY_MIN_VILLAIN_HANDS)
    {
        return None;
    }
    let weighted: f64 = villains
        .iter()
        .map(|v| {
            let hands = f64::from(v.hands);
            hands / (hands + HANDS_K) * villain_softness(v)
        })
        .sum();
    let table_soft = weighted / villains.len() as f64;
    let score = (50.0 + 50.0 * table_soft).round().clamp(0.0, 100.0) as u8;
    let label = if score >= SOFT_AT {
        QualityLabel::Soft
    } else if score <= TOUGH_AT {
        QualityLabel::Tough
    } else {
        QualityLabel::Average
    };
    Some(TableQuality { score, label, basis_hands })
}

/// Which tracked tables each player is seated at.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MultiTableIndex {
    tables_of: BTreeMap<i64, Vec<u32>>,
}

impl MultiTableIndex {
    /// The other tracked tables where `player_id` is seated, ascending;
    /// empty when he sits only at `table_id` (or is the hero).
    pub fn other_tables(&self, table_id: u32, player_id: i64) -> Vec<u32> {
        self.tables_of
            .get(&player_id)
            .map(|tables| tables.iter().copied().filter(|t| *t != table_id).collect())
            .unwrap_or_default()
    }

    /// Every villain seated at two or more tables, with all of his tables.
    pub fn multi_tabling(&self) -> impl Iterator<Item = (i64, &[u32])> {
        self.tables_of
            .iter()
            .filter(|(_, tables)| tables.len() > 1)
            .map(|(player, tables)| (*player, tables.as_slice()))
    }
}

/// Builds the index from each tracked table's roster: `(table id,
/// [(player id, is hero)])`. The hero is left out, so he is never flagged
/// for sitting at all of his own tables.
pub fn multi_table_index(rosters: &[(u32, Vec<(i64, bool)>)]) -> MultiTableIndex {
    let mut tables_of: BTreeMap<i64, Vec<u32>> = BTreeMap::new();
    for (table_id, roster) in rosters {
        for (player_id, is_hero) in roster {
            if *is_hero {
                continue;
            }
            let tables = tables_of.entry(*player_id).or_default();
            if !tables.contains(table_id) {
                tables.push(*table_id);
            }
        }
    }
    for tables in tables_of.values_mut() {
        tables.sort_unstable();
    }
    MultiTableIndex { tables_of }
}
