//! Recency and recent form (`docs/specs/opponent-engine.md`, section 6):
//! recent hands weigh more, and a villain whose last hands break sharply
//! from his own baseline is flagged — `tilt` when the break follows a big
//! loss.
//!
//! Catalogue rows R01 (`rec.weighting`) and R02 (`rec.tilt`, `rec.looser`,
//! `rec.tighter`). Only completed hands are read; the "last hands" are the
//! last ones PokerStars wrote, never the hand in progress.

use chrono::NaiveDateTime;
use serde::Serialize;

use super::facts::{HandFacts, StatKey};
use super::pooling::{shrink, stat_spec, FormatKey};
use super::preflop::extract_preflop;

/// Spec rule id of the recency weighting and views (catalogue row R01).
pub const RULE_WEIGHTING: &str = "rec.weighting";

/// Size of the last-N view and of the tilt window, in villain hands.
pub const LAST_N: usize = 12;
/// The tilt window keeps only hands this close to the villain's latest.
pub const TILT_WINDOW_MINUTES: i64 = 60;
/// VPIP opportunities the tilt window needs.
pub const TILT_MIN_WINDOW: u32 = 10;
/// VPIP opportunities the baseline needs.
pub const TILT_MIN_BASELINE: u32 = 40;
/// Villain hands, ending at the latest, searched for a big loss.
pub const BIG_LOSS_LOOKBACK: usize = 15;
/// Big loss in a cash game (cash, Zoom), in big blinds.
pub const BIG_LOSS_CASH_BB: f64 = 40.0;
/// Big loss in a tournament, in big blinds, together with losing at least
/// half the starting stack.
pub const BIG_LOSS_TOURNAMENT_BB: f64 = 15.0;
/// `looser` needs the window this far above the baseline (and z ≥ 2.5).
pub const LOOSER_MIN_GAP: f64 = 0.20;
/// `tighter` needs the window this far below the baseline (and z ≤ −2.5).
pub const TIGHTER_MIN_GAP: f64 = 0.15;
pub const FORM_MIN_Z: f64 = 2.5;

/// `0.5 ^ (age / H)`: `age` = how many of the villain's hands are newer.
pub fn recency_weight(age: usize, half_life: f64) -> f64 {
    0.5_f64.powf(age as f64 / half_life)
}

/// The weight of each of `count` hands ordered oldest first.
pub fn hand_weights(count: usize, half_life: f64) -> Vec<f64> {
    (0..count).map(|i| recency_weight(count - 1 - i, half_life)).collect()
}

/// Index of the first hand of the last-N view over `count` hands ordered
/// oldest first.
pub fn last_n_start(count: usize, n: usize) -> usize {
    count.saturating_sub(n)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FormFlag {
    Looser,
    Tighter,
    Tilt,
}

impl FormFlag {
    /// The spec rule id this flag fires (catalogue row R02).
    pub fn rule_id(&self) -> &'static str {
        match self {
            FormFlag::Looser => "rec.looser",
            FormFlag::Tighter => "rec.tighter",
            FormFlag::Tilt => "rec.tilt",
        }
    }
}

/// The `recentForm` object of `PlayerPayload.engine` (section 13). Present
/// only when window and baseline are large enough; `flag` is `None` for
/// ordinary variance.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecentForm {
    /// VPIP opportunities in the window.
    pub window: u32,
    pub window_hits: u32,
    pub baseline_opportunities: u32,
    pub window_vpip_pct: f64,
    /// The baseline VPIP, shrunk (k = 20) toward the format prior.
    pub baseline_vpip_pct: f64,
    pub z: f64,
    pub flag: Option<FormFlag>,
    pub after_big_loss: bool,
}

fn played_at(hand: &HandFacts) -> Option<NaiveDateTime> {
    NaiveDateTime::parse_from_str(hand.played_at.as_deref()?, "%Y-%m-%dT%H:%M:%S").ok()
}

fn round(value: f64, places: i32) -> f64 {
    let factor = 10_f64.powi(places);
    (value * factor).round() / factor
}

/// `(opportunities, hits)` of VPIP over hands.
fn vpip(hands: &[HandFacts], villain_id: i64) -> (u32, u32) {
    hands.iter().fold((0, 0), |(opps, hits), hand| {
        match extract_preflop(hand, villain_id).iter().find(|e| e.key == StatKey::Vpip) {
            Some(event) => (opps + 1, hits + u32::from(event.success)),
            None => (opps, hits),
        }
    })
}

/// What the villain lost in `hands[i]`, in big blinds, and the stack he
/// ended it with. From `net_result`, or — when it is unknown, as in every
/// tournament hand — from his starting stack in his next hand at the same
/// table.
fn loss_in(hands: &[HandFacts], i: usize, villain_id: i64) -> Option<(f64, f64, f64)> {
    let hand = &hands[i];
    let bb = hand.big_blind.filter(|bb| *bb > 0.0)?;
    let seat = hand.seat_of(villain_id)?;
    let start = seat.starting_stack?;
    let end = match seat.net_result {
        Some(net) => start + net,
        None => hands[i + 1..]
            .iter()
            .find(|next| next.table_name.is_some() && next.table_name == hand.table_name)?
            .seat_of(villain_id)?
            .starting_stack?,
    };
    Some(((start - end) / bb, start, end))
}

/// A big loss among the villain's last [`BIG_LOSS_LOOKBACK`] hands.
pub fn big_loss_recently(hands: &[HandFacts], villain_id: i64) -> bool {
    let from = hands.len().saturating_sub(BIG_LOSS_LOOKBACK);
    (from..hands.len()).any(|i| {
        let Some((loss_bb, start, end)) = loss_in(hands, i, villain_id) else { return false };
        match FormatKey::of_hand(&hands[i]) {
            FormatKey::Cash | FormatKey::Zoom => loss_bb >= BIG_LOSS_CASH_BB,
            FormatKey::Mtt | FormatKey::Spin => {
                loss_bb >= BIG_LOSS_TOURNAMENT_BB && end <= 0.5 * start
            }
        }
    })
}

/// The recent-form test of section 6 over the villain's hands (all tables,
/// oldest first). `vpip_prior` is the villain's format VPIP prior. `None`
/// when the window has fewer than 10 VPIP opportunities within 60 minutes of
/// the latest hand, or the baseline before it fewer than 40.
pub fn recent_form(hands: &[HandFacts], villain_id: i64, vpip_prior: f64) -> Option<RecentForm> {
    let latest = played_at(hands.last()?)?;
    let start = last_n_start(hands.len(), LAST_N);
    let window_start = (start..hands.len())
        .find(|&i| {
            played_at(&hands[i]).is_some_and(|t| (latest - t).num_minutes() <= TILT_WINDOW_MINUTES)
        })
        .unwrap_or(hands.len());
    let (n, x) = vpip(&hands[window_start..], villain_id);
    let (base_n, base_hits) = vpip(&hands[..window_start], villain_id);
    if n < TILT_MIN_WINDOW || base_n < TILT_MIN_BASELINE {
        return None;
    }

    let k = stat_spec(StatKey::Vpip).k;
    let p0 = shrink(f64::from(base_hits), f64::from(base_n), k, vpip_prior).clamp(0.01, 0.99);
    let nf = f64::from(n);
    let rate = f64::from(x) / nf;
    let z = (f64::from(x) - nf * p0) / (nf * p0 * (1.0 - p0)).sqrt();
    let after_big_loss = big_loss_recently(hands, villain_id);
    let looser = rate - p0 >= LOOSER_MIN_GAP && z >= FORM_MIN_Z;
    let tighter = p0 - rate >= TIGHTER_MIN_GAP && z <= -FORM_MIN_Z;
    let flag = match (looser, tighter) {
        (true, _) if after_big_loss => Some(FormFlag::Tilt),
        (true, _) => Some(FormFlag::Looser),
        (_, true) => Some(FormFlag::Tighter),
        _ => None,
    };
    Some(RecentForm {
        window: n,
        window_hits: x,
        baseline_opportunities: base_n,
        window_vpip_pct: round(100.0 * rate, 1),
        baseline_vpip_pct: round(100.0 * p0, 1),
        z: round(z, 2),
        flag,
        after_big_loss,
    })
}
