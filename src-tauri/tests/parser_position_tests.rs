//! Position derivation against the user's REAL hands, at every table size.
//!
//! The expected values are not invented: PokerStars tags the blinds itself in
//! the `*** SUMMARY ***` section, and `assert_blinds_match_pokerstars` checks
//! the derivation against those tags rather than against a hand-written
//! constant. the notes §E ran the same comparison
//! across all 249 usable stored hands and found 249 correct.

use std::collections::HashMap;
use velora_poker_lib::parser::{parse_hand_block, split_hands, ParsedHand};

const POSITIONS_BY_TABLE_SIZE: &str = include_str!("fixtures/real_positions_by_table_size.txt");
const SEAT_MOVED_OUT_OF_HAND: &str = include_str!("fixtures/real_seat_moved_out_of_hand.txt");
const BOUNTY_TOURNAMENT: &str = include_str!("fixtures/real_bounty_tournament.txt");

fn hands(text: &str) -> Vec<ParsedHand> {
    split_hands(text)
        .into_iter()
        .map(|block| parse_hand_block(&block).expect("fixture should parse"))
        .collect()
}

fn positions_by_seat(hand: &ParsedHand) -> HashMap<i64, String> {
    hand.seats
        .iter()
        .filter_map(|s| s.position.clone().map(|p| (s.seat_number, p)))
        .collect()
}

/// Seat numbers PokerStars itself tagged in the summary, as `(tag, seat)`.
fn pokerstars_blind_tags(hand: &ParsedHand) -> HashMap<&'static str, i64> {
    let summary = hand
        .raw_text
        .split("*** SUMMARY ***")
        .nth(1)
        .unwrap_or_default();
    let mut out = HashMap::new();
    for line in summary.lines() {
        let Some(rest) = line.strip_prefix("Seat ") else { continue };
        let Some((seat, rest)) = rest.split_once(american_colon()) else { continue };
        let Ok(seat) = seat.trim().parse::<i64>() else { continue };
        for (tag, label) in [
            ("(small blind)", "SB"),
            ("(big blind)", "BB"),
            ("(button)", "BTN"),
        ] {
            if rest.contains(tag) {
                out.insert(label, seat);
            }
        }
    }
    out
}

fn american_colon() -> &'static str {
    ": "
}

/// Ground-truth check: whatever we derived for the blinds must equal what
/// PokerStars wrote in its own summary.
fn assert_blinds_match_pokerstars(hand: &ParsedHand) {
    let derived = positions_by_seat(hand);
    let truth = pokerstars_blind_tags(hand);

    if let Some(bb_seat) = truth.get("BB") {
        assert_eq!(
            derived.get(bb_seat).map(String::as_str),
            Some("BB"),
            "hand {}: PokerStars says seat {} is the big blind, we derived {:?}",
            hand.hand_id,
            bb_seat,
            derived.get(bb_seat)
        );
    }
    if let Some(sb_seat) = truth.get("SB") {
        let derived_label = derived.get(sb_seat).map(String::as_str);
        // Heads-up: the button posts the small blind and keeps the `BTN` label.
        let acceptable = if hand.seats.len() == 2 { "BTN" } else { "SB" };
        assert_eq!(
            derived_label,
            Some(acceptable),
            "hand {}: PokerStars says seat {} is the small blind, we derived {:?}",
            hand.hand_id,
            sb_seat,
            derived_label
        );
    }
    if let Some(btn_seat) = truth.get("BTN") {
        assert_eq!(
            derived.get(btn_seat).map(String::as_str),
            Some("BTN"),
            "hand {}: PokerStars says seat {} is the button, we derived {:?}",
            hand.hand_id,
            btn_seat,
            derived.get(btn_seat)
        );
    }
}

#[test]
fn every_fixture_hand_agrees_with_pokerstars_own_blind_tags() {
    let mut checked = 0;
    for text in [POSITIONS_BY_TABLE_SIZE, SEAT_MOVED_OUT_OF_HAND, BOUNTY_TOURNAMENT] {
        for hand in hands(text) {
            assert_blinds_match_pokerstars(&hand);
            checked += 1;
        }
    }
    assert_eq!(checked, 11, "all fixture hands must be covered");
}

/// One real hand per table size, 9-handed down to heads-up.
///
/// Each expectation below was read off the hand's own summary tags, then the
/// remaining labels follow the documented ring order.
#[test]
fn derives_positions_at_every_real_table_size() {
    let parsed = hands(POSITIONS_BY_TABLE_SIZE);
    let by_size: HashMap<usize, &ParsedHand> =
        parsed.iter().map(|h| (h.seats.len(), h)).collect();

    // Heads-up: the button posts the small blind, so there is no separate SB.
    let hu = by_size[&2];
    assert_eq!(hu.button_seat, 1);
    assert_eq!(positions_by_seat(hu)[&1], "BTN");
    assert_eq!(positions_by_seat(hu)[&4], "BB");
    assert!(
        !positions_by_seat(hu).values().any(|p| p == "SB"),
        "heads-up must not emit a separate small blind label"
    );

    let three = by_size[&3];
    assert_eq!(three.button_seat, 4);
    let p = positions_by_seat(three);
    assert_eq!((p[&4].as_str(), p[&1].as_str(), p[&2].as_str()), ("BTN", "SB", "BB"));

    let four = by_size[&4];
    assert_eq!(four.button_seat, 2);
    let p = positions_by_seat(four);
    assert_eq!(
        (p[&2].as_str(), p[&3].as_str(), p[&4].as_str(), p[&1].as_str()),
        ("BTN", "SB", "BB", "CO")
    );

    let five = by_size[&5];
    assert_eq!(five.button_seat, 1);
    let p = positions_by_seat(five);
    assert_eq!(
        (p[&1].as_str(), p[&2].as_str(), p[&4].as_str(), p[&5].as_str(), p[&6].as_str()),
        ("BTN", "SB", "BB", "UTG", "CO")
    );

    let six = by_size[&6];
    assert_eq!(six.button_seat, 4);
    let p = positions_by_seat(six);
    assert_eq!(
        (
            p[&4].as_str(), p[&5].as_str(), p[&6].as_str(),
            p[&1].as_str(), p[&2].as_str(), p[&3].as_str()
        ),
        ("BTN", "SB", "BB", "UTG", "HJ", "CO")
    );

    let seven = by_size[&7];
    assert_eq!(seven.button_seat, 1);
    let p = positions_by_seat(seven);
    assert_eq!(
        (
            p[&1].as_str(), p[&2].as_str(), p[&3].as_str(), p[&4].as_str(),
            p[&5].as_str(), p[&6].as_str(), p[&7].as_str()
        ),
        ("BTN", "SB", "BB", "UTG", "LJ", "HJ", "CO")
    );

    let eight = by_size[&8];
    assert_eq!(eight.button_seat, 2);
    let p = positions_by_seat(eight);
    assert_eq!(
        (
            p[&2].as_str(), p[&3].as_str(), p[&4].as_str(), p[&5].as_str(),
            p[&6].as_str(), p[&7].as_str(), p[&8].as_str(), p[&1].as_str()
        ),
        ("BTN", "SB", "BB", "UTG", "UTG+1", "LJ", "HJ", "CO")
    );

    let nine = by_size[&9];
    assert_eq!(nine.button_seat, 4);
    let p = positions_by_seat(nine);
    assert_eq!(
        (
            p[&4].as_str(), p[&5].as_str(), p[&6].as_str(), p[&7].as_str(), p[&8].as_str(),
            p[&9].as_str(), p[&1].as_str(), p[&2].as_str(), p[&3].as_str()
        ),
        ("BTN", "SB", "BB", "UTG", "UTG+1", "UTG+2", "LJ", "HJ", "CO")
    );
}

/// Locks in the rule that matters, by construction.
///
/// In this real hand seat 6 was moved in from another table and is not in the
/// hand. Building the ring from every seat line — the naive rule — puts seat 6
/// next after the button and therefore calls it the small blind. PokerStars'
/// own summary says the small blind is seat 1. Across the stored database the
/// naive rule is wrong on 12 of 249 hands; this is one of them.
#[test]
fn the_naive_seat_ring_would_get_this_real_hand_wrong() {
    let hand = &hands(SEAT_MOVED_OUT_OF_HAND)[0];
    assert_eq!(hand.button_seat, 5);

    // What the naive rule would have produced.
    let mut all_seat_numbers: Vec<i64> = hand
        .seats
        .iter()
        .map(|s| s.seat_number)
        .chain(hand.skipped_seats.iter().map(|s| s.seat_number))
        .collect();
    all_seat_numbers.sort_unstable();
    let button_index = all_seat_numbers
        .iter()
        .position(|s| *s == hand.button_seat)
        .expect("button is seated");
    let naive_small_blind = all_seat_numbers[(button_index + 1) % all_seat_numbers.len()];
    assert_eq!(
        naive_small_blind, 6,
        "the naive ring really does pick the seat that was not in the hand"
    );

    // What PokerStars says, and what we derive.
    let truth = pokerstars_blind_tags(hand);
    assert_eq!(truth.get("SB"), Some(&1));
    let derived = positions_by_seat(hand);
    assert_eq!(derived[&1], "SB");
    assert_eq!(derived[&2], "BB");
    assert!(
        !derived.contains_key(&6),
        "seat 6 was not dealt in and must have no position at all"
    );
}

/// Bounty hands were previously invisible, so they also never had a position.
#[test]
fn bounty_hands_now_get_positions_too() {
    for hand in hands(BOUNTY_TOURNAMENT) {
        assert_eq!(hand.seats.len(), 3);
        assert!(
            hand.seats.iter().all(|s| s.position.is_some()),
            "hand {} left a player without a position",
            hand.hand_id
        );
        assert_blinds_match_pokerstars(&hand);
    }
}
