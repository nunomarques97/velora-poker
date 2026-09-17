//! Derives a table position label for every player dealt into a hand.
//!
//! # Why this lives here and not in a query
//!
//! the notes §E established two things empirically
//! against the user's real hand histories, using PokerStars' own
//! `*** SUMMARY ***` blind tags as ground truth:
//!
//! * The ring **must** be built from the players actually dealt into the hand.
//!   Building it from every parsed `Seat` line is wrong on 12 of 249 hands
//!   (4.8%), because a seat that was sitting out or was moved in from another
//!   table occupies a place in the ring that the real table never had.
//! * With the dealt-in ring, the derivation is correct on **249 of 249** hands
//!   at every table size from heads-up to 9-handed.
//!
//! [`derive`] therefore takes only dealt-in seats; `parse_hand_block` filters
//! them before calling it (see `pokerstars.rs`).
//!
//! # Label convention
//!
//! Blinds and button are fixed points. The seats between the big blind and the
//! button ("middle seats", `m = n - 3`) are named from the button backwards —
//! `CO`, then `HJ`, then `LJ` — and whatever is left is numbered from the front
//! as `UTG`, `UTG+1`, … That yields the conventional tables:
//!
//! | Dealt in | Labels, clockwise from the button |
//! |---|---|
//! | 2 | `BTN`, `BB` — heads-up: the button posts the small blind |
//! | 3 | `BTN`, `SB`, `BB` |
//! | 4 | `BTN`, `SB`, `BB`, `CO` |
//! | 5 | `BTN`, `SB`, `BB`, `UTG`, `CO` |
//! | 6 | `BTN`, `SB`, `BB`, `UTG`, `HJ`, `CO` |
//! | 7 | `BTN`, `SB`, `BB`, `UTG`, `LJ`, `HJ`, `CO` |
//! | 8 | `BTN`, `SB`, `BB`, `UTG`, `UTG+1`, `LJ`, `HJ`, `CO` |
//! | 9 | `BTN`, `SB`, `BB`, `UTG`, `UTG+1`, `UTG+2`, `LJ`, `HJ`, `CO` |
//!
//! Heads-up deliberately has no `SB` label: the button *is* the small blind, and
//! emitting both would make "is this player on the button" and "is this player
//! in the small blind" disagree about the same seat.

pub const BTN: &str = "BTN";
pub const SB: &str = "SB";
pub const BB: &str = "BB";

/// Position labels for `seat_numbers`, in the same order as the input.
///
/// `seat_numbers` must contain the absolute PokerStars seat number of every
/// player **dealt into the hand**, and `button_seat` the hand's button seat.
/// Returns `None` for every seat when the ring cannot be oriented — fewer than
/// two players dealt in, or a button that is not one of them — rather than
/// guessing an orientation.
pub fn derive(seat_numbers: &[i64], button_seat: i64) -> Vec<Option<String>> {
    let none = || vec![None; seat_numbers.len()];

    let mut ring: Vec<i64> = seat_numbers.to_vec();
    ring.sort_unstable();
    ring.dedup();
    if ring.len() < 2 || ring.len() != seat_numbers.len() {
        return none();
    }
    let Some(button_index) = ring.iter().position(|s| *s == button_seat) else {
        return none();
    };

    let n = ring.len();
    // Walk clockwise from the button so `ordered[0]` is always the button.
    let ordered: Vec<i64> = (0..n).map(|i| ring[(button_index + i) % n]).collect();
    let labels = labels_for(n);

    seat_numbers
        .iter()
        .map(|seat| {
            ordered
                .iter()
                .position(|s| s == seat)
                .map(|i| labels[i].clone())
        })
        .collect()
}

/// Labels for an `n`-handed ring, index 0 being the button.
fn labels_for(n: usize) -> Vec<String> {
    if n == 2 {
        // Heads-up: the button posts the small blind, so it keeps `BTN` alone.
        return vec![BTN.to_string(), BB.to_string()];
    }

    let mut labels = vec![BTN.to_string(), SB.to_string(), BB.to_string()];

    // Middle seats sit between the big blind and the button.
    let middle = n - 3;
    let mut middle_labels = vec![String::new(); middle];

    // Name the late seats from the button backwards.
    let late = ["CO", "HJ", "LJ"];
    let late_count = match middle {
        0 => 0,
        1 | 2 => 1, // only the cutoff
        3 => 2,     // cutoff + hijack
        _ => 3,     // cutoff + hijack + lojack
    };
    for (offset, name) in late.iter().take(late_count).enumerate() {
        middle_labels[middle - 1 - offset] = (*name).to_string();
    }

    // Everything still unnamed is early position, numbered from the front.
    let mut early = 0;
    for slot in middle_labels.iter_mut() {
        if slot.is_empty() {
            *slot = if early == 0 {
                "UTG".to_string()
            } else {
                format!("UTG+{early}")
            };
            early += 1;
        }
    }

    labels.extend(middle_labels);
    labels
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labels(n: usize) -> Vec<String> {
        labels_for(n)
    }

    #[test]
    fn label_tables_match_the_documented_convention() {
        assert_eq!(labels(2), ["BTN", "BB"]);
        assert_eq!(labels(3), ["BTN", "SB", "BB"]);
        assert_eq!(labels(4), ["BTN", "SB", "BB", "CO"]);
        assert_eq!(labels(5), ["BTN", "SB", "BB", "UTG", "CO"]);
        assert_eq!(labels(6), ["BTN", "SB", "BB", "UTG", "HJ", "CO"]);
        assert_eq!(labels(7), ["BTN", "SB", "BB", "UTG", "LJ", "HJ", "CO"]);
        assert_eq!(labels(8), ["BTN", "SB", "BB", "UTG", "UTG+1", "LJ", "HJ", "CO"]);
        assert_eq!(
            labels(9),
            ["BTN", "SB", "BB", "UTG", "UTG+1", "UTG+2", "LJ", "HJ", "CO"]
        );
    }

    #[test]
    fn every_ring_has_exactly_one_button_and_one_big_blind() {
        for n in 2..=9 {
            let l = labels(n);
            assert_eq!(l.len(), n);
            assert_eq!(l.iter().filter(|x| *x == BTN).count(), 1, "n={n}");
            assert_eq!(l.iter().filter(|x| *x == BB).count(), 1, "n={n}");
            // Heads-up has no separate small blind — the button posts it.
            assert_eq!(
                l.iter().filter(|x| *x == SB).count(),
                if n == 2 { 0 } else { 1 },
                "n={n}"
            );
        }
    }

    #[test]
    fn seats_need_not_be_contiguous_or_start_at_one() {
        // Real tables leave gaps when players leave; the ring is the occupied
        // seats in seat order, not 1..=max_seats.
        let got = derive(&[2, 5, 9], 5);
        assert_eq!(
            got,
            vec![
                Some(BB.to_string()),  // seat 2 is two clockwise from the button
                Some(BTN.to_string()), // seat 5 is the button
                Some(SB.to_string()),  // seat 9 is next after the button
            ]
        );
    }

    #[test]
    fn input_order_is_preserved_regardless_of_seat_order() {
        let ascending = derive(&[1, 3, 6], 1);
        let shuffled = derive(&[6, 1, 3], 1);
        assert_eq!(ascending, vec![Some("BTN".into()), Some("SB".into()), Some("BB".into())]);
        assert_eq!(shuffled, vec![Some("BB".into()), Some("BTN".into()), Some("SB".into())]);
    }

    #[test]
    fn refuses_to_guess_when_the_ring_cannot_be_oriented() {
        // Button is not among the dealt-in seats.
        assert_eq!(derive(&[1, 2, 3], 6), vec![None, None, None]);
        // Fewer than two players.
        assert_eq!(derive(&[4], 4), vec![None]);
        assert_eq!(derive(&[], 1), Vec::<Option<String>>::new());
        // Duplicate seat numbers are a corrupt ring, not a recoverable one.
        assert_eq!(derive(&[2, 2, 5], 5), vec![None, None, None]);
    }
}
