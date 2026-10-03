//! Preflop and stack-depth stat extraction (catalogue rows P01–P12, S01–S03
//! of `docs/specs/opponent-engine.md`).
//!
//! One pass over a hand's preflop actions tracks the raises (with who made
//! them and whether they were all-in), the limpers and the callers of the
//! latest raise. At each of the scored player's decisions, the spots that
//! decision answers are emitted as [`StatEvent`]s.
//!
//! Two rules keep push/fold play out of the deep-stack frequencies:
//! - Every stat except `open_shove`, `call_vs_shove` and `reshove` counts only
//!   when the player's effective stack at hand start is above 15bb (section
//!   4.3). A folded-to spot at 15bb or less is an `open_shove` opportunity,
//!   never an RFI, steal or limp one.
//! - An all-in open is a shove, not an open: the players behind it face a
//!   `call_vs_shove` spot, not a 3-bet, squeeze, cold-call or steal-defence
//!   one. A player who already raised and is re-raised all-in still answers
//!   their own raise (`fold_to_3bet`, `fold_to_4bet`, limp follow-ups).

use std::collections::{HashMap, HashSet};

use super::facts::{
    Counterparty, HandEvents, HandFacts, Relation, StatEvent, StatKey,
};
use crate::parser::ActionType;

/// Effective stack (bb) at or below which a folded-to spot is push/fold.
pub const PUSH_FOLD_MAX_BB: f64 = 15.0;
/// Upper bound (inclusive) of the reshove band `(15, 25]`.
pub const RESHOVE_MAX_BB: f64 = 25.0;
/// A raise committing at least this share of the raiser's starting stack is
/// an open shove even when it does not say "and is all-in" (row S01).
pub const SHOVE_COMMIT_SHARE: f64 = 0.5;

/// Position group of an RFI seat (section 4.3); `None` for the big blind.
pub fn rfi_key(position: &str) -> Option<StatKey> {
    match position {
        "UTG" | "UTG+1" | "UTG+2" => Some(StatKey::RfiEp),
        "LJ" | "HJ" => Some(StatKey::RfiMp),
        "CO" => Some(StatKey::RfiCo),
        "BTN" => Some(StatKey::RfiBtn),
        "SB" => Some(StatKey::RfiSb),
        _ => None,
    }
}

/// Seats a raise first in is a steal from.
pub fn is_steal_position(position: &str) -> bool {
    matches!(position, "CO" | "BTN" | "SB")
}

/// Postflop acting order: lower acts first, the button acts last. Heads-up
/// the button (who posted the small blind) is last as well.
fn postflop_rank(position: &str) -> Option<u8> {
    Some(match position {
        "SB" => 0,
        "BB" => 1,
        "UTG" => 2,
        "UTG+1" => 3,
        "UTG+2" => 4,
        "LJ" => 5,
        "HJ" => 6,
        "CO" => 7,
        "BTN" => 8,
        _ => return None,
    })
}

/// The player's position relative to the spot's creator after the flop.
pub fn relation(player_position: Option<&str>, creator_position: Option<&str>) -> Option<Relation> {
    let player = postflop_rank(player_position?)?;
    let creator = postflop_rank(creator_position?)?;
    if player == creator {
        return None;
    }
    Some(if player > creator { Relation::InPosition } else { Relation::OutOfPosition })
}

#[derive(Debug, Clone, Copy)]
struct RaiseMade {
    player: i64,
    all_in: bool,
    /// The pot was folded to the raiser (no limper, no earlier raise).
    first_in: bool,
}

/// Extracts every preflop and stack-depth event `player_id` produced in one
/// hand. A hand the player was not dealt into yields nothing.
pub fn extract_preflop(hand: &HandFacts, player_id: i64) -> Vec<StatEvent> {
    let Some(seat) = hand.seat_of(player_id) else {
        return Vec::new();
    };
    let position = seat.position.as_deref();
    let eff = hand.effective_stack_bb(player_id);
    let deep = eff.is_some_and(|e| e > PUSH_FOLD_MAX_BB);
    let hero = hand.hero();
    let positions: HashMap<i64, Option<&str>> = hand
        .seats
        .iter()
        .map(|s| (s.player_id, s.position.as_deref()))
        .collect();
    let position_of = |id: i64| positions.get(&id).copied().flatten();

    let event = |key: StatKey, success: bool, creator: Option<i64>| StatEvent {
        key,
        opportunity: true,
        success,
        player_id,
        position: seat.position.clone(),
        effective_stack_bb: eff,
        relation: creator.and_then(|c| relation(position, position_of(c))),
        counterparty: Counterparty {
            creator,
            hero_created: match (creator, hero) {
                (Some(c), Some(h)) => c == h.player_id,
                _ => false,
            },
            hero_in_hand: hero.is_some(),
            hero_position: hero.and_then(|h| h.position.clone()),
        },
    };

    let mut events = Vec::new();
    let mut raises: Vec<RaiseMade> = Vec::new();
    let mut limpers: Vec<i64> = Vec::new();
    let mut callers_since_raise: Vec<i64> = Vec::new();
    let mut voluntary: HashSet<i64> = HashSet::new();
    let mut open_limped = false;
    let mut limp_answered = false;
    let mut decided = false;
    let mut entered = false;
    let mut raised = false;

    for action in hand.preflop_actions() {
        let kind = action.kind;
        if matches!(
            kind,
            ActionType::PostSmallBlind | ActionType::PostBigBlind | ActionType::PostAnte
        ) {
            continue;
        }

        if action.player_id == player_id {
            let is_raise = kind == ActionType::Raise;
            let is_call = kind == ActionType::Call;
            let is_fold = kind == ActionType::Fold;
            let fresh = !voluntary.contains(&player_id);
            let folded_to = raises.is_empty() && limpers.is_empty();

            // Unopened pot: RFI, steal and limp above 15bb; open shove below.
            if folded_to && !decided {
                if let Some(pos) = position.filter(|p| *p != "BB") {
                    if deep {
                        if let Some(key) = rfi_key(pos) {
                            events.push(event(key, is_raise, None));
                        }
                        if is_steal_position(pos) {
                            events.push(event(StatKey::Steal, is_raise, None));
                        }
                        events.push(event(StatKey::Limp, is_call, None));
                        if is_call {
                            open_limped = true;
                        }
                    } else if eff.is_some() {
                        let shove = is_raise
                            && (action.is_all_in
                                || match (action.amount, seat.starting_stack) {
                                    (Some(to), Some(stack)) if stack > 0.0 => {
                                        to >= SHOVE_COMMIT_SHARE * stack
                                    }
                                    _ => false,
                                });
                        events.push(event(StatKey::OpenShove, shove, None));
                    }
                }
            }

            // Limpers ahead, no raise: isolation spot.
            if raises.is_empty() && !limpers.is_empty() && fresh && deep {
                events.push(event(StatKey::IsoRaise, is_raise, limpers.first().copied()));
            }

            // Facing exactly one raise, no money of our own in yet.
            if raises.len() == 1 && fresh && !raises[0].all_in {
                let open = raises[0];
                if deep {
                    match relation(position, position_of(open.player)) {
                        Some(Relation::InPosition) => {
                            events.push(event(StatKey::ThreeBetIp, is_raise, Some(open.player)))
                        }
                        Some(Relation::OutOfPosition) => {
                            events.push(event(StatKey::ThreeBetOop, is_raise, Some(open.player)))
                        }
                        None => {}
                    }
                    events.push(event(StatKey::ColdCall, is_call, Some(open.player)));
                    if !callers_since_raise.is_empty() {
                        events.push(event(StatKey::Squeeze, is_raise, Some(open.player)));
                    }
                    let opener_pos = position_of(open.player);
                    let steal = open.first_in
                        && callers_since_raise.is_empty()
                        && opener_pos.is_some_and(is_steal_position);
                    if steal {
                        match position {
                            Some("SB") => events.push(event(
                                StatKey::FoldToStealSb,
                                is_fold,
                                Some(open.player),
                            )),
                            Some("BB") => events.push(event(
                                StatKey::FoldToStealBb,
                                is_fold,
                                Some(open.player),
                            )),
                            _ => {}
                        }
                    }
                    if steal && position == Some("BB") && opener_pos == Some("SB") {
                        events.push(event(
                            StatKey::BbDefendVsSb,
                            is_call || is_raise,
                            Some(open.player),
                        ));
                    }
                }
                if eff.is_some_and(|e| e > PUSH_FOLD_MAX_BB && e <= RESHOVE_MAX_BB) {
                    events.push(event(
                        StatKey::Reshove,
                        is_raise && action.is_all_in,
                        Some(open.player),
                    ));
                }
            }

            // Answering a re-raise of our own open.
            if raises.len() == 2 && raises[0].player == player_id && deep {
                let three_bettor = raises[1].player;
                match relation(position, position_of(three_bettor)) {
                    Some(Relation::InPosition) => {
                        events.push(event(StatKey::FoldTo3betIp, is_fold, Some(three_bettor)))
                    }
                    Some(Relation::OutOfPosition) => {
                        events.push(event(StatKey::FoldTo3betOop, is_fold, Some(three_bettor)))
                    }
                    None => {}
                }
                if !raises[1].all_in {
                    events.push(event(StatKey::FourBet, is_raise, Some(three_bettor)));
                }
            }

            // Answering a re-raise of our own 3-bet.
            if raises.len() == 3 && raises[1].player == player_id && deep {
                events.push(event(StatKey::FoldTo4bet, is_fold, Some(raises[2].player)));
            }

            // First answer to a raise after open-limping.
            if open_limped && !limp_answered && !raises.is_empty() {
                limp_answered = true;
                let raiser = raises.last().map(|r| r.player);
                events.push(event(StatKey::LimpFold, is_fold, raiser));
                events.push(event(StatKey::LimpCall, is_call, raiser));
                events.push(event(StatKey::LimpReraise, is_raise, raiser));
            }

            // Facing an all-in raise nobody has called yet.
            if raises.last().is_some_and(|r| r.all_in && r.player != player_id)
                && callers_since_raise.is_empty()
            {
                events.push(event(
                    StatKey::CallVsShove,
                    is_call,
                    raises.last().map(|r| r.player),
                ));
            }

            decided = true;
            entered |= is_call || is_raise;
            raised |= is_raise;
        }

        match kind {
            ActionType::Raise | ActionType::Bet => {
                raises.push(RaiseMade {
                    player: action.player_id,
                    all_in: action.is_all_in,
                    first_in: raises.is_empty() && limpers.is_empty(),
                });
                callers_since_raise.clear();
                voluntary.insert(action.player_id);
            }
            ActionType::Call => {
                if raises.is_empty() {
                    limpers.push(action.player_id);
                } else {
                    callers_since_raise.push(action.player_id);
                }
                voluntary.insert(action.player_id);
            }
            _ => {}
        }
    }

    if decided && deep {
        events.insert(0, event(StatKey::Pfr, raised, None));
        events.insert(0, event(StatKey::Vpip, entered, None));
    }
    events
}

/// Runs [`extract_preflop`] over every hand, keeping the hand's identity next
/// to its events so aggregation can weight them by recency.
pub fn extract_player_preflop(hands: &[HandFacts], player_id: i64) -> Vec<HandEvents> {
    hands
        .iter()
        .map(|hand| HandEvents {
            hand_id: hand.id,
            hand_ref: hand.hand_ref.clone(),
            played_at: hand.played_at.clone(),
            variant: hand.variant.clone(),
            events: extract_preflop(hand, player_id),
        })
        .collect()
}
