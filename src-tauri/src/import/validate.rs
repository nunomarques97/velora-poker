//! Import-time integrity gate.
//!
//! Every check here corresponds to a defect that was found by querying the
//! stored database *after the fact*. The point of this
//! module is that the same questions are now asked **before** a hand is written,
//! so a structurally broken hand is rejected and counted instead of landing in
//! the database and being discovered months later.
//!
//! # Reject vs. record
//!
//! A [`Severity::Reject`] problem means the hand cannot be stored without
//! corrupting downstream aggregates — the row would be worse than the row's
//! absence. A [`Severity::Warn`] problem is recorded and surfaced but the hand
//! is still imported, because the data is usable and dropping it would lose more
//! than it protects.
//!
//! Every rejecting check was run against a full corpus of real hand histories
//! (275 stored hands, 22 files) and rejects **none** of them, so the gate cannot
//! silently eat a legitimate hand during a live session. That measurement is
//! locked in by `tests/import_validation_tests.rs`.

use crate::parser::ParsedHand;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    /// The hand is not written to the database.
    Reject,
    /// The hand is written, but the problem is counted and surfaced.
    Warn,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Problem {
    pub severity: Severity,
    /// Stable machine-readable code, also what the diagnostics report groups by.
    pub code: &'static str,
    pub detail: String,
}

impl Problem {
    fn reject(code: &'static str, detail: String) -> Self {
        Problem { severity: Severity::Reject, code, detail }
    }
    fn warn(code: &'static str, detail: String) -> Self {
        Problem { severity: Severity::Warn, code, detail }
    }
}

/// Checks one parsed hand. An empty result means the hand is clean.
pub fn check(hand: &ParsedHand) -> Vec<Problem> {
    let mut problems = Vec::new();

    if hand.hand_id.trim().is_empty() {
        problems.push(Problem::reject("missing_hand_id", "hand id is empty".to_string()));
    }
    if hand.played_at.trim().is_empty() {
        problems.push(Problem::reject(
            "missing_played_at",
            format!("hand {} has no timestamp", hand.hand_id),
        ));
    }

    // No dealt-in players at all. This is exactly the bounty-seat-line failure:
    // the hand imported fine, kept its actions, and stored nobody.
    if hand.seats.is_empty() {
        problems.push(Problem::reject(
            "no_dealt_in_players",
            format!(
                "hand {} parsed {} seat line(s) but none were dealt in",
                hand.hand_id,
                hand.seats.len() + hand.skipped_seats.len()
            ),
        ));
    }

    if hand.max_seats <= 0 {
        problems.push(Problem::reject(
            "missing_max_seats",
            format!("hand {} has max_seats = {}", hand.hand_id, hand.max_seats),
        ));
    } else {
        if hand.button_seat < 1 || hand.button_seat > hand.max_seats {
            problems.push(Problem::reject(
                "button_seat_out_of_range",
                format!(
                    "hand {} button seat {} outside 1..={}",
                    hand.hand_id, hand.button_seat, hand.max_seats
                ),
            ));
        }
        for seat in &hand.seats {
            if seat.seat_number < 1 || seat.seat_number > hand.max_seats {
                problems.push(Problem::reject(
                    "seat_out_of_range",
                    format!(
                        "hand {} seat {} outside 1..={}",
                        hand.hand_id, seat.seat_number, hand.max_seats
                    ),
                ));
            }
        }
        if hand.seats.len() as i64 > hand.max_seats {
            problems.push(Problem::reject(
                "more_players_than_seats",
                format!(
                    "hand {} dealt in {} players at a {}-max table",
                    hand.hand_id,
                    hand.seats.len(),
                    hand.max_seats
                ),
            ));
        }
    }

    // A duplicate seat number or player name makes the positional ring
    // ambiguous and breaks `UNIQUE(hand_id, player_id)` on insert anyway.
    let mut seat_numbers: Vec<i64> = hand.seats.iter().map(|s| s.seat_number).collect();
    seat_numbers.sort_unstable();
    let unique_seats = {
        let mut s = seat_numbers.clone();
        s.dedup();
        s.len()
    };
    if unique_seats != seat_numbers.len() {
        problems.push(Problem::reject(
            "duplicate_seat_number",
            format!("hand {} has two players in the same seat", hand.hand_id),
        ));
    }
    let mut names: Vec<&str> = hand.seats.iter().map(|s| s.player_name.as_str()).collect();
    names.sort_unstable();
    let unique_names = {
        let mut n = names.clone();
        n.dedup();
        n.len()
    };
    if unique_names != names.len() {
        problems.push(Problem::reject(
            "duplicate_player_in_hand",
            format!("hand {} lists the same player twice", hand.hand_id),
        ));
    }

    // An action by somebody who is not a dealt-in seat is the second half of the
    // bounty failure: it is precisely what produced 226 orphan action rows.
    let seated: std::collections::HashSet<&str> =
        hand.seats.iter().map(|s| s.player_name.as_str()).collect();
    let mut orphan_actors: Vec<&str> = hand
        .actions
        .iter()
        .map(|a| a.player_name.as_str())
        .filter(|n| !seated.contains(n))
        .collect();
    orphan_actors.sort_unstable();
    orphan_actors.dedup();
    if !orphan_actors.is_empty() {
        problems.push(Problem::reject(
            "action_by_unseated_player",
            format!(
                "hand {} has {} action(s) by {} player(s) with no dealt-in seat",
                hand.hand_id,
                hand.actions.len(),
                orphan_actors.len()
            ),
        ));
    }

    // Action ordering must never contradict street ordering.
    let mut last_rank = 0u8;
    let mut last_order = i64::MIN;
    for action in &hand.actions {
        let rank = street_rank(action.street);
        if rank < last_rank {
            problems.push(Problem::reject(
                "action_order_inconsistent",
                format!(
                    "hand {} action {} moves backwards from {:?} to {:?}",
                    hand.hand_id, action.order, last_rank, rank
                ),
            ));
            break;
        }
        if action.order <= last_order {
            problems.push(Problem::reject(
                "action_index_not_increasing",
                format!("hand {} action index {} does not increase", hand.hand_id, action.order),
            ));
            break;
        }
        last_rank = rank;
        last_order = action.order;
    }

    // Warnings — stored, but worth the user knowing about.
    if hand.seats.len() >= 2 && hand.seats.iter().any(|s| s.position.is_none()) {
        problems.push(Problem::warn(
            "position_not_derived",
            format!(
                "hand {} has {} dealt-in player(s) but the positional ring could not be oriented",
                hand.hand_id,
                hand.seats.len()
            ),
        ));
    }
    if hand.actions.is_empty() {
        problems.push(Problem::warn(
            "no_actions",
            format!("hand {} has no parsed actions", hand.hand_id),
        ));
    }

    problems
}

fn street_rank(street: crate::parser::Street) -> u8 {
    use crate::parser::Street::*;
    match street {
        Preflop => 0,
        Flop => 1,
        Turn => 2,
        River => 3,
    }
}

/// True when nothing in `problems` blocks the hand from being stored.
pub fn is_storable(problems: &[Problem]) -> bool {
    !problems.iter().any(|p| p.severity == Severity::Reject)
}
