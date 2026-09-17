//! Seat-line parsing and the dealt-in decision, against the user's REAL
//! hand-history text.
//!
//! Every fixture in this file is a verbatim hand from
//! `%LOCALAPPDATA%\PokerStars.PT\HandHistory`, with player names replaced and
//! nothing else changed. The substitution was proved reversible when the
//! fixtures were generated: mapping the names back reproduces the original file
//! byte for byte, so these are real formats, not invented ones.

use velora_poker_lib::parser::{parse_hand_block, split_hands};

const CASH_SITTING_OUT: &str = include_str!("fixtures/real_cash_sitting_out.txt");
const BOUNTY_TOURNAMENT: &str = include_str!("fixtures/real_bounty_tournament.txt");
const SEAT_MOVED_OUT_OF_HAND: &str = include_str!("fixtures/real_seat_moved_out_of_hand.txt");
const POSITIONS_BY_TABLE_SIZE: &str = include_str!("fixtures/real_positions_by_table_size.txt");

fn hands(text: &str) -> Vec<velora_poker_lib::parser::ParsedHand> {
    split_hands(text)
        .into_iter()
        .map(|block| parse_hand_block(&block).expect("fixture should parse"))
        .collect()
}

/// The defect this whole work unit starts from: a bounty tournament writes the
/// bounty inside the same parentheses as the stack, so requiring `in chips)`
/// matched nothing and the hand stored **no players at all** — 26 of 275 real
/// hands (9.5%).
#[test]
fn bounty_tournament_seat_lines_are_parsed() {
    let parsed = hands(BOUNTY_TOURNAMENT);
    assert_eq!(parsed.len(), 2, "fixture holds two real bounty hands");

    for hand in &parsed {
        assert_eq!(
            hand.seats.len(),
            3,
            "hand {} is a real 3-max bounty hand and must store all three players",
            hand.hand_id
        );
        // The bounty text must not leak into the player name or the stack.
        for seat in &hand.seats {
            assert!(
                !seat.player_name.contains("bounty") && !seat.player_name.contains('('),
                "player name {:?} absorbed the bounty text",
                seat.player_name
            );
            assert!(seat.starting_stack > 0.0, "stack must still parse");
        }
    }

    // Exact stacks from the real line `Seat 1: Player1 (11262 in chips, €13.50 bounty)`.
    let first = &parsed[0];
    let mut seats = first.seats.clone();
    seats.sort_by_key(|s| s.seat_number);
    assert_eq!(seats[0].starting_stack, 11262.0);
    assert_eq!(seats[1].starting_stack, 10507.0);
    assert_eq!(seats[2].starting_stack, 24875.0);
}

/// A seat can carry a bounty *and* a sitting-out marker at once. The second
/// fixture hand is that real combination.
#[test]
fn a_bounty_seat_that_also_says_sitting_out_still_parses() {
    let parsed = hands(BOUNTY_TOURNAMENT);
    let hand = &parsed[1];
    assert_eq!(hand.seats.len() + hand.skipped_seats.len(), 3);
    assert!(
        hand.seats.iter().all(|s| s.starting_stack > 0.0),
        "a combined bounty + sitting-out line must still yield a stack"
    );
}

/// Every trailing-content variant found across the user's 22 files parses,
/// including the plain forms that already worked — so a fix for the bounty case
/// cannot regress the common case.
#[test]
fn every_real_trailing_content_variant_parses() {
    let mut seen_plain = 0;
    let mut seen_currency = 0;
    let mut seen_bounty = 0;
    let mut seen_sitting_out = 0;
    let mut seen_out_of_hand = 0;

    for text in [
        CASH_SITTING_OUT,
        BOUNTY_TOURNAMENT,
        SEAT_MOVED_OUT_OF_HAND,
        POSITIONS_BY_TABLE_SIZE,
    ] {
        for hand in hands(text) {
            // Every seat line in the raw text must have produced a seat, whether
            // it was dealt in or deliberately skipped.
            let seat_lines = hand
                .raw_text
                .split("*** HOLE CARDS ***")
                .next()
                .unwrap_or("")
                .lines()
                .filter(|l| l.starts_with("Seat ") && l.contains(" in chips"))
                .count();
            assert_eq!(
                seat_lines,
                hand.seats.len() + hand.skipped_seats.len(),
                "hand {} dropped a seat line",
                hand.hand_id
            );

            for line in hand.raw_text.lines() {
                if !line.starts_with("Seat ") || !line.contains(" in chips") {
                    continue;
                }
                if line.contains("bounty") {
                    seen_bounty += 1;
                }
                if line.contains("out of hand") {
                    seen_out_of_hand += 1;
                } else if line.contains("is sitting out") {
                    seen_sitting_out += 1;
                }
                if line.contains('€') {
                    seen_currency += 1;
                } else if line.trim_end().ends_with("in chips)") {
                    seen_plain += 1;
                }
            }
        }
    }

    assert!(seen_plain > 0, "fixtures must cover a plain chip-count seat line");
    assert!(seen_currency > 0, "fixtures must cover a currency-prefixed seat line");
    assert!(seen_bounty > 0, "fixtures must cover a bounty seat line");
    assert!(seen_sitting_out > 0, "fixtures must cover a sitting-out seat line");
    assert!(seen_out_of_hand > 0, "fixtures must cover an out-of-hand seat line");
}

/// Regression for the second write-time defect: a seat that was never dealt into
/// the hand must not become a player row.
///
/// This real cash hand seats five players; the sixth was sitting out, took no
/// action, and appears in the summary as a bare `Seat 6: <name>` with no
/// description.
#[test]
fn a_seat_that_was_never_dealt_in_does_not_become_a_player() {
    let parsed = hands(CASH_SITTING_OUT);
    let hand = &parsed[0];

    assert_eq!(hand.seats.len(), 4, "four players were actually dealt in");
    assert_eq!(hand.skipped_seats.len(), 1);

    let skipped = &hand.skipped_seats[0];
    assert_eq!(skipped.seat_number, 6);
    assert_eq!(skipped.seat_line_marker, ") is sitting out");
    assert!(
        !hand.seats.iter().any(|s| s.player_name == skipped.player_name),
        "the skipped player must not also appear as dealt in"
    );
}

/// A player moved in from another table is shown at the table but is explicitly
/// not in the hand. PokerStars omits them from the summary entirely.
#[test]
fn a_seat_moved_in_from_another_table_is_not_dealt_in() {
    let parsed = hands(SEAT_MOVED_OUT_OF_HAND);
    let hand = &parsed[0];

    assert_eq!(hand.seats.len(), 5);
    assert_eq!(hand.skipped_seats.len(), 1);
    let skipped = &hand.skipped_seats[0];
    assert_eq!(skipped.seat_number, 6);
    assert_eq!(
        skipped.seat_line_marker,
        ") out of hand (moved from another table into small blind)"
    );
}

/// The single most important guard in this file.
///
/// `is sitting out` is **not** a dealt-in test. Across the user's real files,
/// 174 of the 197 seats carrying that marker were dealt in and played the hand
/// normally — PokerStars writes the marker from the player's sit-out flag when
/// the hand is written, not from whether they were dealt cards. Excluding on the
/// marker would have deleted real players.
///
/// In this real hand the hero's own seat says `is sitting out`, and the hero
/// posts the ante, is dealt cards, and folds.
#[test]
fn a_sitting_out_marker_does_not_by_itself_mean_not_dealt_in() {
    let parsed = hands(SEAT_MOVED_OUT_OF_HAND);
    let hand = &parsed[0];

    let hero_seat_line = hand
        .raw_text
        .lines()
        .find(|l| l.starts_with("Seat 5: "))
        .expect("seat 5 line");
    assert!(
        hero_seat_line.contains("is sitting out"),
        "this fixture is only meaningful while seat 5 carries the marker: {hero_seat_line}"
    );

    let hero = hand
        .seats
        .iter()
        .find(|s| s.seat_number == 5)
        .expect("seat 5 must still be dealt in despite the sitting-out marker");
    assert_eq!(hero.position.as_deref(), Some("BTN"));
    assert!(
        hand.actions.iter().any(|a| a.player_name == hero.player_name),
        "the player marked sitting out really did act in this hand"
    );
}
