//! Aggregation of one villain's stat events (`docs/specs/opponent-engine.md`,
//! sections 4, 6 and 10): the all-time, recency-weighted and last-N views,
//! and the head-to-head view of the spots he played against the hero.
//!
//! One pass over the villain's completed hands builds every view, so a
//! refresh replays each hand once.

use std::collections::BTreeMap;

use serde::Serialize;

use super::facts::{HandFacts, StatEvent, StatKey};
use super::pooling::{shrink, stat_spec, FormatKey, ShrunkStat};
use super::postflop::extract_postflop;
use super::preflop::extract_preflop;
use super::recency::{hand_weights, last_n_start, LAST_N};
use crate::parser::ActionType;

/// Head-to-head stats need this many opportunities to be displayed.
pub const H2H_DISPLAY_MIN: u32 = 8;
/// Head-to-head rules need this many opportunities to fire.
pub const H2H_RULE_MIN: u32 = 10;

/// Counts of one stat: raw, and recency-weighted.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct StatAgg {
    pub hits: u32,
    pub opportunities: u32,
    pub hits_w: f64,
    pub opportunities_w: f64,
}

impl StatAgg {
    fn add(&mut self, success: bool, weight: f64) {
        self.opportunities += 1;
        self.hits += u32::from(success);
        self.opportunities_w += weight;
        self.hits_w += if success { weight } else { 0.0 };
    }

    fn merge(&mut self, other: &StatAgg) {
        self.hits += other.hits;
        self.opportunities += other.opportunities;
        self.hits_w += other.hits_w;
        self.opportunities_w += other.opportunities_w;
    }
}

/// The three views of section 6.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    AllTime,
    /// Weighted by `0.5 ^ (age / H)`; rules evaluate this view.
    Recency,
    /// The villain's last [`LAST_N`] hands.
    LastN,
}

/// The head-to-head stats of section 10.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum H2hKey {
    ThreeBetVsHeroOpen,
    /// `fold_to_hero_3bet`, as the spec names it.
    #[serde(rename = "fold_to_hero_3bet")]
    FoldToHeroThreeBet,
    FoldToHeroCbet,
    StealVsHero,
    DefendVsHeroSteal,
}

impl H2hKey {
    pub const ALL: [H2hKey; 5] = [
        H2hKey::ThreeBetVsHeroOpen,
        H2hKey::FoldToHeroThreeBet,
        H2hKey::FoldToHeroCbet,
        H2hKey::StealVsHero,
        H2hKey::DefendVsHeroSteal,
    ];

    pub fn as_str(&self) -> &'static str {
        match self {
            H2hKey::ThreeBetVsHeroOpen => "three_bet_vs_hero_open",
            H2hKey::FoldToHeroThreeBet => "fold_to_hero_3bet",
            H2hKey::FoldToHeroCbet => "fold_to_hero_cbet",
            H2hKey::StealVsHero => "steal_vs_hero",
            H2hKey::DefendVsHeroSteal => "defend_vs_hero_steal",
        }
    }

    /// The villain's overall stats the head-to-head value shrinks toward,
    /// and whether the h2h success is their complement (defending a steal
    /// is not folding to it).
    pub fn basis(&self) -> (&'static [StatKey], bool) {
        match self {
            H2hKey::ThreeBetVsHeroOpen => (&[StatKey::ThreeBetIp, StatKey::ThreeBetOop], false),
            H2hKey::FoldToHeroThreeBet => (&[StatKey::FoldTo3betIp, StatKey::FoldTo3betOop], false),
            H2hKey::FoldToHeroCbet => (&[StatKey::FoldToCbetFlop], false),
            H2hKey::StealVsHero => (&[StatKey::Steal], false),
            H2hKey::DefendVsHeroSteal => (&[StatKey::FoldToStealSb, StatKey::FoldToStealBb], true),
        }
    }
}

/// One head-to-head stat, shrunk toward the villain's own overall value.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct H2hStat {
    pub key: H2hKey,
    pub hits: u32,
    pub opportunities: u32,
    pub raw: Option<f64>,
    /// The villain's overall shrunk value on the basis stat(s).
    pub villain_shrunk: f64,
    pub shrunk: f64,
}

impl H2hStat {
    /// Whether a rule may fire on it (`n >= 10`).
    pub fn rule_eligible(&self) -> bool {
        self.opportunities >= H2H_RULE_MIN
    }
}

/// The `headToHead` stat item of section 13.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct H2hStatPayload {
    pub key: H2hKey,
    pub hits: u32,
    pub opportunities: u32,
    pub raw_pct: Option<f64>,
    pub shrunk_pct: f64,
}

/// The `headToHead` object of section 13.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HeadToHead {
    /// Hands the villain and the hero were both dealt into.
    pub hands: u32,
    pub stats: Vec<H2hStatPayload>,
}

/// Every view of one villain's stats.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PlayerAggregate {
    pub player_id: i64,
    /// Hands the villain was dealt into.
    pub hands: u32,
    pub all_time: BTreeMap<StatKey, StatAgg>,
    pub last_n: BTreeMap<StatKey, StatAgg>,
    pub h2h: BTreeMap<H2hKey, StatAgg>,
    /// Hands the villain and the hero were both dealt into.
    pub h2h_hands: u32,
}

fn pct(value: f64) -> f64 {
    (value * 1000.0).round() / 10.0
}

impl PlayerAggregate {
    pub fn counts(&self, view: View, key: StatKey) -> StatAgg {
        let source = match view {
            View::LastN => &self.last_n,
            View::AllTime | View::Recency => &self.all_time,
        };
        source.get(&key).copied().unwrap_or_default()
    }

    /// One stat, shrunk toward `prior`, in a view. Confidence always uses
    /// the stat-specific raw opportunity count. In the recency view `shrunk`
    /// is computed on the weighted counts while `raw` stays the player's
    /// plain rate, the number the UI shows next to its count.
    pub fn stat(&self, view: View, key: StatKey, prior: f64) -> ShrunkStat {
        let counts = self.counts(view, key);
        let mut stat = ShrunkStat::new(key, counts.hits, counts.opportunities, prior);
        if view == View::Recency {
            stat.shrunk = shrink(counts.hits_w, counts.opportunities_w, stat_spec(key).k, prior);
        }
        stat
    }

    /// The villain's overall (all-time) shrunk value on an h2h stat's basis:
    /// the basis stats' counts summed, shrunk with the first basis stat's k
    /// toward the mean of their priors.
    pub fn villain_overall(&self, key: H2hKey, prior: &dyn Fn(StatKey) -> f64) -> f64 {
        let (basis, complement) = key.basis();
        let mut counts = StatAgg::default();
        for stat in basis {
            counts.merge(&self.counts(View::AllTime, *stat));
        }
        let mean_prior = basis.iter().map(|s| prior(*s)).sum::<f64>() / basis.len() as f64;
        let k = stat_spec(basis[0]).k;
        let value = shrink(f64::from(counts.hits), f64::from(counts.opportunities), k, mean_prior);
        if complement {
            1.0 - value
        } else {
            value
        }
    }

    /// One head-to-head stat: `(h2h_hits + k·villain_shrunk) / (h2h_opps + k)`.
    pub fn h2h_stat(&self, key: H2hKey, prior: &dyn Fn(StatKey) -> f64) -> H2hStat {
        let counts = self.h2h.get(&key).copied().unwrap_or_default();
        let villain_shrunk = self.villain_overall(key, prior);
        let k = stat_spec(key.basis().0[0]).k;
        H2hStat {
            key,
            hits: counts.hits,
            opportunities: counts.opportunities,
            raw: (counts.opportunities > 0)
                .then(|| f64::from(counts.hits) / f64::from(counts.opportunities)),
            villain_shrunk,
            shrunk: shrink(f64::from(counts.hits), f64::from(counts.opportunities), k, villain_shrunk),
        }
    }

    /// The displayable head-to-head view: stats with at least
    /// [`H2H_DISPLAY_MIN`] opportunities; `None` when none qualifies.
    pub fn head_to_head(&self, prior: &dyn Fn(StatKey) -> f64) -> Option<HeadToHead> {
        let stats: Vec<H2hStatPayload> = H2hKey::ALL
            .iter()
            .map(|key| self.h2h_stat(*key, prior))
            .filter(|s| s.opportunities >= H2H_DISPLAY_MIN)
            .map(|s| H2hStatPayload {
                key: s.key,
                hits: s.hits,
                opportunities: s.opportunities,
                raw_pct: s.raw.map(pct),
                shrunk_pct: pct(s.shrunk),
            })
            .collect();
        (!stats.is_empty()).then_some(HeadToHead { hands: self.h2h_hands, stats })
    }
}

/// The hero's first preflop raise was the first voluntary action of the
/// hand (no limper or raiser before it).
fn hero_opened_first_in(hand: &HandFacts, hero_id: i64) -> bool {
    hand.preflop_actions()
        .find(|a| matches!(a.kind, ActionType::Call | ActionType::Raise | ActionType::Bet))
        .is_some_and(|a| a.player_id == hero_id && a.kind != ActionType::Call)
}

/// The head-to-head opportunities one hand's events give against the hero
/// (section 10). Only spots the hero created or faced count.
pub fn h2h_events(hand: &HandFacts, events: &[StatEvent], villain_id: i64) -> Vec<(H2hKey, bool)> {
    let Some(hero) = hand.hero().filter(|h| h.player_id != villain_id) else {
        return Vec::new();
    };
    let squeezed = events.iter().any(|e| e.key == StatKey::Squeeze);
    let mut out = Vec::new();
    for event in events.iter().filter(|e| e.opportunity) {
        let by_hero = event.counterparty.hero_created;
        match event.key {
            StatKey::ThreeBetIp | StatKey::ThreeBetOop
                if by_hero && !squeezed && hero_opened_first_in(hand, hero.player_id) =>
            {
                out.push((H2hKey::ThreeBetVsHeroOpen, event.success))
            }
            StatKey::FoldTo3betIp | StatKey::FoldTo3betOop if by_hero => {
                out.push((H2hKey::FoldToHeroThreeBet, event.success))
            }
            StatKey::FoldToCbetFlop if by_hero => out.push((H2hKey::FoldToHeroCbet, event.success)),
            StatKey::Steal
                if matches!(hero.position.as_deref(), Some("SB") | Some("BB")) =>
            {
                out.push((H2hKey::StealVsHero, event.success))
            }
            StatKey::FoldToStealSb | StatKey::FoldToStealBb if by_hero => {
                out.push((H2hKey::DefendVsHeroSteal, !event.success))
            }
            _ => {}
        }
    }
    out
}

/// Every event of one hand for one player: preflop, then postflop.
pub fn hand_events(hand: &HandFacts, player_id: i64) -> Vec<StatEvent> {
    let mut events = extract_preflop(hand, player_id);
    events.extend(extract_postflop(hand, player_id));
    events
}

/// Builds every view from the villain's hands, oldest first (as
/// `load_player_hands` returns them). Hands he was not dealt into are
/// skipped. `format` sets the recency half-life.
pub fn aggregate_player(hands: &[HandFacts], player_id: i64, format: FormatKey) -> PlayerAggregate {
    let own: Vec<&HandFacts> = hands.iter().filter(|h| h.seat_of(player_id).is_some()).collect();
    let weights = hand_weights(own.len(), format.half_life());
    let recent_from = last_n_start(own.len(), LAST_N);
    let mut agg = PlayerAggregate { player_id, hands: own.len() as u32, ..Default::default() };
    for (i, hand) in own.iter().enumerate() {
        let events = hand_events(hand, player_id);
        for event in events.iter().filter(|e| e.opportunity) {
            agg.all_time.entry(event.key).or_default().add(event.success, weights[i]);
            if i >= recent_from {
                agg.last_n.entry(event.key).or_default().add(event.success, 1.0);
            }
        }
        if hand.hero().is_some_and(|h| h.player_id != player_id) {
            agg.h2h_hands += 1;
        }
        for (key, success) in h2h_events(hand, &events, player_id) {
            agg.h2h.entry(key).or_default().add(success, weights[i]);
        }
    }
    agg
}
