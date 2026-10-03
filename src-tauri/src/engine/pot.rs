//! Street-by-street pot reconstruction and bet sizing (catalogue row F13 and
//! section 9 of `docs/specs/opponent-engine.md`).
//!
//! The replay walks a stored hand's actions in order, tracking every
//! player's commitment on the current street. Antes go straight into the
//! pot; blinds, bets and calls add their logged increment; a raise adds the
//! difference between its raise-to amount and what the raiser had already
//! put in on that street.
//!
//! Uncalled bets are not stored as actions, so they are derived the way
//! PokerStars computes them: when a street closes, the part of the largest
//! commitment that nobody matched (largest minus second largest, folded
//! players' money included) goes back to its owner. The final pot then
//! equals the hand history's `Total pot` line, rake included.
//!
//! `posts small & big blinds` (a dead blind in cash games) is not a stored
//! action, so a hand with one reconstructs short by that amount.

use std::collections::HashMap;

use super::facts::HandFacts;
use crate::parser::{ActionType, Street};

/// Spec rule id of the pot reconstruction (catalogue row F13).
pub const RULE_POT_RECONSTRUCTION: &str = "post.pot_reconstruction";

/// Amounts closer than this are equal (cents and chips are far coarser).
const EPS: f64 = 1e-6;

/// Sizing bucket of a bet or raise (section 9).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum SizeBucket {
    /// Under 40% of the pot.
    Small,
    /// 40% up to 75%.
    Medium,
    /// 75% up to 110%.
    Large,
    /// 110% of the pot or more.
    Overbet,
    /// Any all-in, whatever its fraction.
    AllIn,
}

impl SizeBucket {
    /// The bucket of a pot fraction; an all-in overrides the fraction.
    pub fn classify(fraction: f64, all_in: bool) -> SizeBucket {
        if all_in {
            SizeBucket::AllIn
        } else if fraction < 0.40 {
            SizeBucket::Small
        } else if fraction < 0.75 {
            SizeBucket::Medium
        } else if fraction < 1.10 {
            SizeBucket::Large
        } else {
            SizeBucket::Overbet
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            SizeBucket::Small => "small",
            SizeBucket::Medium => "medium",
            SizeBucket::Large => "large",
            SizeBucket::Overbet => "overbet",
            SizeBucket::AllIn => "allin",
        }
    }
}

/// One logged action with the pot around it.
#[derive(Debug, Clone, PartialEq)]
pub struct SizedAction {
    /// `ActionFact::index` of the action.
    pub index: i64,
    pub player_id: i64,
    pub street: Street,
    pub kind: ActionType,
    pub is_all_in: bool,
    /// Chips this action added to the pot.
    pub put_in: f64,
    /// The pot before the action: every ante, blind and bet so far,
    /// including the bets already made on this street.
    pub pot_before: f64,
    /// What the player had to add to call before acting.
    pub to_call: f64,
    /// Bets: `amount / pot_before`. Raises: `(raise_to − facing_bet) /
    /// (pot_before + to_call)`. `None` for every other action.
    pub fraction: Option<f64>,
    /// Bucket of `fraction` (all-in overrides it); `None` with `fraction`.
    pub bucket: Option<SizeBucket>,
}

/// An uncalled bet handed back when its street closed.
#[derive(Debug, Clone, PartialEq)]
pub struct UncalledReturn {
    pub street: Street,
    pub player_id: i64,
    pub amount: f64,
}

/// A hand's pot, street by street.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PotReplay {
    /// Every action in logged order.
    pub actions: Vec<SizedAction>,
    /// The pot when each street that had any action began, after the
    /// previous street's uncalled bet went back.
    pub street_start: Vec<(Street, f64)>,
    pub uncalled: Vec<UncalledReturn>,
    /// What each player put in, net of uncalled returns.
    pub contributed: HashMap<i64, f64>,
    /// The final pot (rake included), as PokerStars' `Total pot` line.
    pub total: f64,
}

impl PotReplay {
    /// The sized action with this `ActionFact::index`.
    pub fn action(&self, index: i64) -> Option<&SizedAction> {
        self.actions.iter().find(|a| a.index == index)
    }

    /// The pot when `street` began, if it had any action.
    pub fn pot_at_start(&self, street: Street) -> Option<f64> {
        self.street_start.iter().find(|(s, _)| *s == street).map(|(_, pot)| *pot)
    }
}

/// Hands the unmatched part of the street's largest commitment back to its
/// owner (largest minus second largest; folded players' money counts).
fn return_uncalled(
    street: Street,
    commits: &HashMap<i64, f64>,
    replay: &mut PotReplay,
) {
    let mut top: Option<(i64, f64)> = None;
    let mut second = 0.0_f64;
    // Sorted by player for a deterministic owner when two commitments tie
    // (a tie returns nothing anyway).
    let mut entries: Vec<(i64, f64)> = commits.iter().map(|(p, c)| (*p, *c)).collect();
    entries.sort_by_key(|(p, _)| *p);
    for (player, commit) in entries {
        match top {
            Some((_, best)) if commit <= best => second = second.max(commit),
            Some((_, best)) => {
                second = second.max(best);
                top = Some((player, commit));
            }
            None => top = Some((player, commit)),
        }
    }
    let Some((player, best)) = top else { return };
    let amount = best - second;
    if amount > EPS {
        replay.total -= amount;
        *replay.contributed.entry(player).or_insert(0.0) -= amount;
        replay.uncalled.push(UncalledReturn { street, player_id: player, amount });
    }
}

/// Replays `hand`'s actions into a street-by-street pot with a size for
/// every bet and raise.
pub fn replay_pot(hand: &HandFacts) -> PotReplay {
    let mut replay = PotReplay::default();
    let mut commits: HashMap<i64, f64> = HashMap::new();
    let mut street: Option<Street> = None;

    for action in &hand.actions {
        if street != Some(action.street) {
            if let Some(previous) = street {
                return_uncalled(previous, &commits, &mut replay);
            }
            commits.clear();
            street = Some(action.street);
            if action.street != Street::Preflop {
                replay.street_start.push((action.street, replay.total));
            }
        }

        let own = commits.get(&action.player_id).copied().unwrap_or(0.0);
        let facing = commits
            .iter()
            .filter(|(p, _)| **p != action.player_id)
            .map(|(_, c)| *c)
            .fold(0.0_f64, f64::max);
        let to_call = (facing - own).max(0.0);
        let pot_before = replay.total;
        let amount = action.amount.unwrap_or(0.0);

        let (put_in, fraction) = match action.kind {
            ActionType::PostAnte => (amount, None),
            ActionType::PostSmallBlind | ActionType::PostBigBlind | ActionType::Call => {
                (amount, None)
            }
            ActionType::Bet => {
                let fraction = (pot_before > EPS).then(|| amount / pot_before);
                (amount, fraction)
            }
            ActionType::Raise => {
                let put_in = (amount - own).max(0.0);
                let denominator = pot_before + to_call;
                let fraction =
                    (denominator > EPS).then(|| (amount - facing).max(0.0) / denominator);
                (put_in, fraction)
            }
            ActionType::Fold | ActionType::Check => (0.0, None),
        };

        if action.kind != ActionType::PostAnte && put_in > 0.0 {
            *commits.entry(action.player_id).or_insert(0.0) += put_in;
        }
        replay.total += put_in;
        *replay.contributed.entry(action.player_id).or_insert(0.0) += put_in;
        replay.actions.push(SizedAction {
            index: action.index,
            player_id: action.player_id,
            street: action.street,
            kind: action.kind,
            is_all_in: action.is_all_in,
            put_in,
            pot_before,
            to_call,
            fraction,
            bucket: fraction.map(|f| SizeBucket::classify(f, action.is_all_in)),
        });
    }
    if let Some(last) = street {
        return_uncalled(last, &commits, &mut replay);
    }
    replay
}

/// The amount on a hand history's `Total pot` line (`Total pot $3.75 | Rake
/// $0`, `Total pot 5400 Main pot 3600. Side pot 1800. | Rake 0`).
pub fn parse_total_pot(raw_text: &str) -> Option<f64> {
    let line = raw_text.lines().find_map(|l| l.trim().strip_prefix("Total pot "))?;
    let token = line.split(|c: char| c.is_whitespace() || c == '|').next()?;
    let digits: String = token.chars().filter(|c| c.is_ascii_digit() || *c == '.').collect();
    digits.parse().ok()
}
