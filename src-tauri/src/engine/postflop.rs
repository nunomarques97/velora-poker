//! Postflop stat extraction (catalogue rows F01–F12 of
//! `docs/specs/opponent-engine.md`; definitions in section 9.2).
//!
//! The replay reads each postflop street's actions in order and emits, at
//! the scored player's decisions, the spots those decisions answer. Every
//! event carries heads-up vs multiway (players in the hand when the street
//! began, all-ins included), IP/OOP (against the spot's creator, or against
//! the whole field when the spot has none) and the hero counterparty.
//!
//! A player who folded preflop, or was all-in preflop, made no postflop
//! decision and gets no postflop event at all — WTSD, W$SD and WWSF
//! included. Likewise, a hand where everyone else was all-in preflop leaves
//! the player with no flop action and so no opportunity.

use std::collections::{HashMap, HashSet};

use super::facts::{
    ActionFact, Counterparty, HandEvents, HandFacts, Relation, StatEvent, StatKey,
};
use super::preflop::{postflop_rank, relation};
use crate::parser::{ActionType, Street};

fn is_aggression(action: &ActionFact) -> bool {
    matches!(action.kind, ActionType::Bet | ActionType::Raise)
}

/// One street's actions in logged order.
struct StreetActions<'a>(Vec<&'a ActionFact>);

impl<'a> StreetActions<'a> {
    fn of(hand: &'a HandFacts, street: Street) -> Self {
        StreetActions(hand.actions.iter().filter(|a| a.street == street).collect())
    }

    fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    fn get(&self, pos: usize) -> &'a ActionFact {
        self.0[pos]
    }

    /// Action index of the street's first action.
    fn start(&self) -> Option<i64> {
        self.0.first().map(|a| a.index)
    }

    fn first_of(&self, player: i64) -> Option<usize> {
        self.0.iter().position(|a| a.player_id == player)
    }

    fn next_of(&self, player: i64, after: usize) -> Option<usize> {
        (after + 1..self.0.len()).find(|&i| self.0[i].player_id == player)
    }

    fn last_of(&self, player: i64) -> Option<&'a ActionFact> {
        self.0.iter().rev().find(|a| a.player_id == player).copied()
    }

    /// Any bet or raise strictly between two positions.
    fn aggression_between(&self, from: usize, to: usize) -> bool {
        self.0[from + 1..to].iter().any(|a| is_aggression(a))
    }

    fn aggression_before(&self, pos: usize) -> bool {
        self.0[..pos].iter().any(|a| is_aggression(a))
    }

    fn first_aggression(&self) -> Option<usize> {
        self.0.iter().position(|a| is_aggression(a))
    }

    fn aggression_count(&self) -> usize {
        self.0.iter().filter(|a| is_aggression(a)).count()
    }

    fn last_aggressor(&self) -> Option<i64> {
        self.0.iter().rev().find(|a| is_aggression(a)).map(|a| a.player_id)
    }

    /// The player's first action on the street when nobody had bet before
    /// it: a spot where betting first was possible.
    fn first_unopened(&self, player: i64) -> Option<usize> {
        self.first_of(player).filter(|&pos| !self.aggression_before(pos))
    }

    /// The player's first answer to the aggression at `at`, if nobody raised
    /// in between.
    fn answer_to(&self, player: i64, at: usize) -> Option<usize> {
        self.next_of(player, at).filter(|&pos| !self.aggression_between(at, pos))
    }

    /// The street's first bet when `player` made it (a c-bet or a barrel).
    fn led_by(&self, player: i64) -> Option<usize> {
        self.first_aggression().filter(|&pos| self.0[pos].player_id == player)
    }
}

/// Extracts every postflop event `player_id` produced in one hand.
pub fn extract_postflop(hand: &HandFacts, player_id: i64) -> Vec<StatEvent> {
    let Some(seat) = hand.seat_of(player_id) else {
        return Vec::new();
    };

    let mut fold_at: HashMap<i64, i64> = HashMap::new();
    let mut all_in_preflop: HashSet<i64> = HashSet::new();
    let mut preflop_raiser: Option<i64> = None;
    for action in &hand.actions {
        if action.kind == ActionType::Fold {
            fold_at.entry(action.player_id).or_insert(action.index);
        }
        if action.street == Street::Preflop {
            if action.is_all_in {
                all_in_preflop.insert(action.player_id);
            }
            if action.kind == ActionType::Raise {
                preflop_raiser = Some(action.player_id);
            }
        }
    }

    let flop = StreetActions::of(hand, Street::Flop);
    let turn = StreetActions::of(hand, Street::Turn);
    let river = StreetActions::of(hand, Street::River);

    // No postflop decision: folded or all-in preflop, or nobody left to play.
    let Some(first_flop) = flop.first_of(player_id) else {
        return Vec::new();
    };
    if all_in_preflop.contains(&player_id) {
        return Vec::new();
    }

    let positions: HashMap<i64, Option<&str>> = hand
        .seats
        .iter()
        .map(|s| (s.player_id, s.position.as_deref()))
        .collect();
    let position = seat.position.as_deref();
    let in_hand_at = |index: i64| -> Vec<i64> {
        hand.seats
            .iter()
            .map(|s| s.player_id)
            .filter(|p| fold_at.get(p).map_or(true, |f| *f >= index))
            .collect()
    };
    let eff = hand.effective_stack_bb(player_id);
    let hero = hand.hero();

    // `street` is the spot's street, `at` the action index of the player's
    // decision.
    let event = |key: StatKey, success: bool, creator: Option<i64>, street: &StreetActions, at: i64| {
        let field = street.start().map(&in_hand_at).unwrap_or_default();
        let relation = match creator {
            Some(c) => relation(position, positions.get(&c).copied().flatten()),
            None => against_field(player_id, position, &field, &positions),
        };
        StatEvent {
            key,
            opportunity: true,
            success,
            player_id,
            position: seat.position.clone(),
            effective_stack_bb: eff,
            relation,
            multiway: Some(field.len() > 2),
            counterparty: Counterparty {
                creator,
                hero_created: match (creator, hero) {
                    (Some(c), Some(h)) => c == h.player_id,
                    _ => false,
                },
                hero_in_hand: hero.is_some(),
                hero_position: hero.and_then(|h| h.position.clone()),
                hero_in_pot: hero.is_some_and(|h| {
                    h.player_id != player_id && fold_at.get(&h.player_id).map_or(true, |f| *f > at)
                }),
            },
        }
    };
    let is = |street: &StreetActions, pos: usize, kind: ActionType| street.get(pos).kind == kind;
    let mut events = Vec::new();

    // C-bet chain of the preflop raiser: flop c-bet, then turn and river
    // barrels while the raiser stays the street's last aggressor.
    let raiser = preflop_raiser.filter(|r| !all_in_preflop.contains(r));
    let flop_cbet = raiser.and_then(|r| flop.led_by(r));
    let turn_barrel = raiser
        .filter(|r| flop_cbet.is_some() && flop.last_aggressor() == Some(*r))
        .and_then(|r| turn.led_by(r));
    let river_barrel = raiser
        .filter(|r| turn_barrel.is_some() && turn.last_aggressor() == Some(*r))
        .and_then(|r| river.led_by(r));
    let flop_checked_through = !flop.is_empty() && flop.aggression_count() == 0;

    // ---- Flop
    if raiser == Some(player_id) {
        if let Some(pos) = flop.first_unopened(player_id) {
            let at = flop.get(pos).index;
            events.push(event(StatKey::CbetFlop, is(&flop, pos, ActionType::Bet), None, &flop, at));
        }
    }
    if let Some(r) = raiser.filter(|r| *r != player_id) {
        let r_in = fold_at.get(&r).map_or(true, |f| Some(*f) >= flop.start());
        let oop = relation(position, positions.get(&r).copied().flatten())
            == Some(Relation::OutOfPosition);
        if r_in && oop {
            if let Some(pos) = flop.first_unopened(player_id) {
                let at = flop.get(pos).index;
                events.push(event(StatKey::DonkFlop, is(&flop, pos, ActionType::Bet), Some(r), &flop, at));
            }
        }
        if let Some(cbet) = flop_cbet {
            if let Some(pos) = flop.answer_to(player_id, cbet) {
                let at = flop.get(pos).index;
                events.push(event(
                    StatKey::FoldToCbetFlop,
                    is(&flop, pos, ActionType::Fold),
                    Some(r),
                    &flop,
                    at,
                ));
            }
        }
    }
    if is(&flop, first_flop, ActionType::Check) {
        let bet = (first_flop + 1..flop.0.len()).find(|&i| is_aggression(flop.get(i)));
        if let Some(bet) = bet {
            if let Some(pos) = flop.answer_to(player_id, bet) {
                let at = flop.get(pos).index;
                let bettor = flop.get(bet).player_id;
                events.push(event(
                    StatKey::CheckRaiseFlop,
                    is(&flop, pos, ActionType::Raise),
                    Some(bettor),
                    &flop,
                    at,
                ));
            }
        }
    }

    // ---- Turn
    let called_flop = flop.last_of(player_id).is_some_and(|a| a.kind == ActionType::Call);
    if raiser == Some(player_id) {
        if let Some(pos) = turn.first_unopened(player_id) {
            let at = turn.get(pos).index;
            let bet = is(&turn, pos, ActionType::Bet);
            if flop_cbet.is_some() && flop.last_aggressor() == Some(player_id) {
                events.push(event(StatKey::CbetTurn, bet, None, &turn, at));
            }
            if flop_checked_through {
                events.push(event(StatKey::DelayedCbet, bet, None, &turn, at));
            }
        }
    }
    if let Some(r) = raiser.filter(|r| *r != player_id) {
        let r_rel = relation(position, positions.get(&r).copied().flatten());
        // Float: called the flop c-bet in position (the c-bet was the flop's
        // only aggression) and the raiser checks the turn.
        let raiser_checks_turn = turn
            .first_of(r)
            .is_some_and(|pos| is(&turn, pos, ActionType::Check));
        if flop_cbet.is_some()
            && flop.aggression_count() == 1
            && called_flop
            && r_rel == Some(Relation::InPosition)
            && raiser_checks_turn
        {
            if let Some(pos) = turn.first_unopened(player_id) {
                let at = turn.get(pos).index;
                events.push(event(StatKey::FloatFlop, is(&turn, pos, ActionType::Bet), Some(r), &turn, at));
            }
        }
        // Probe: the raiser checked back the flop from position.
        let r_on_turn = turn.first_of(r).is_some();
        if flop_checked_through && r_rel == Some(Relation::OutOfPosition) && r_on_turn {
            if let Some(pos) = turn.first_unopened(player_id) {
                let at = turn.get(pos).index;
                events.push(event(StatKey::ProbeTurn, is(&turn, pos, ActionType::Bet), Some(r), &turn, at));
            }
        }
        if let (Some(barrel), true) = (turn_barrel, called_flop) {
            if let Some(pos) = turn.answer_to(player_id, barrel) {
                let at = turn.get(pos).index;
                events.push(event(
                    StatKey::FoldToCbetTurn,
                    is(&turn, pos, ActionType::Fold),
                    Some(r),
                    &turn,
                    at,
                ));
            }
        }
    }

    // ---- River
    let called_turn = turn.last_of(player_id).is_some_and(|a| a.kind == ActionType::Call);
    if raiser == Some(player_id) && turn_barrel.is_some() && turn.last_aggressor() == Some(player_id) {
        if let Some(pos) = river.first_unopened(player_id) {
            let at = river.get(pos).index;
            events.push(event(StatKey::CbetRiver, is(&river, pos, ActionType::Bet), None, &river, at));
        }
    }
    if let Some(r) = raiser.filter(|r| *r != player_id) {
        if let (Some(barrel), true) = (river_barrel, called_turn) {
            if let Some(pos) = river.answer_to(player_id, barrel) {
                let at = river.get(pos).index;
                events.push(event(
                    StatKey::FoldToCbetRiver,
                    is(&river, pos, ActionType::Fold),
                    Some(r),
                    &river,
                    at,
                ));
            }
        }
    }
    if let Some(pos) = river.first_unopened(player_id) {
        let at = river.get(pos).index;
        events.push(event(StatKey::RiverBet, is(&river, pos, ActionType::Bet), None, &river, at));
    }
    // River raise: the player's first river action taken while facing a
    // bet, before they bet or raised on the river themselves.
    for pos in (0..river.0.len()).filter(|&i| river.get(i).player_id == player_id) {
        if river.aggression_before(pos) {
            let at = river.get(pos).index;
            let bettor = river.0[..pos].iter().rev().find(|a| is_aggression(a)).map(|a| a.player_id);
            events.push(event(StatKey::RiverRaise, is(&river, pos, ActionType::Raise), bettor, &river, at));
            break;
        }
        if is_aggression(river.get(pos)) {
            break;
        }
    }

    // ---- Showdown tendencies (the player saw the flop with a decision).
    let at = flop.get(first_flop).index;
    events.push(event(StatKey::Wtsd, seat.went_to_showdown, None, &flop, at));
    if seat.went_to_showdown {
        events.push(event(StatKey::Wsd, seat.won_at_showdown, None, &flop, at));
    }
    let last_standing = !fold_at.contains_key(&player_id)
        && hand
            .seats
            .iter()
            .all(|s| s.player_id == player_id || fold_at.contains_key(&s.player_id));
    events.push(event(
        StatKey::Wwsf,
        seat.won_at_showdown || last_standing,
        None,
        &flop,
        at,
    ));

    events
}

/// In position when the player acts after everyone else in `field`; `None`
/// when any position is unknown or the player is alone.
fn against_field(
    player: i64,
    position: Option<&str>,
    field: &[i64],
    positions: &HashMap<i64, Option<&str>>,
) -> Option<Relation> {
    let own = postflop_rank(position?)?;
    let mut others = field.iter().filter(|p| **p != player).peekable();
    others.peek()?;
    let mut last = true;
    for other in others {
        let rank = postflop_rank(positions.get(other).copied().flatten()?)?;
        if rank > own {
            last = false;
        }
    }
    Some(if last { Relation::InPosition } else { Relation::OutOfPosition })
}

/// Runs [`extract_postflop`] over every hand, keeping each hand's identity.
pub fn extract_player_postflop(hands: &[HandFacts], player_id: i64) -> Vec<HandEvents> {
    hands
        .iter()
        .map(|hand| HandEvents {
            hand_id: hand.id,
            hand_ref: hand.hand_ref.clone(),
            played_at: hand.played_at.clone(),
            variant: hand.variant.clone(),
            events: extract_postflop(hand, player_id),
        })
        .collect()
}
