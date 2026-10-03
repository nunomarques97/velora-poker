//! In-house Hold'em hand evaluator (catalogue row M02, section 9 of
//! `docs/specs/opponent-engine.md`): the best five of five to seven cards,
//! plus the pair classes the showdown value/bluff classification needs.
//!
//! Five to seven cards are few enough to try every five-card combination
//! (at most 21), which keeps the evaluator short and obviously correct; it
//! only runs on shown hands, never on a hot path.

use std::cmp::Ordering;

/// Spec rule id of the hand evaluator (catalogue row M02).
pub const RULE_EVAL: &str = "sd.eval";

/// Rank of a ten or better: the spec's "good kicker" threshold.
const TEN: u8 = 10;
const ACE: u8 = 14;

/// One card: rank 2..=14 (ace high) and suit 0..=3.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Card {
    pub rank: u8,
    pub suit: u8,
}

impl Card {
    /// Parses PokerStars' two-character card text (`Ah`, `Td`, `9c`).
    pub fn parse(text: &str) -> Option<Card> {
        let mut chars = text.chars();
        let rank = match chars.next()? {
            c @ '2'..='9' => c as u8 - b'0',
            'T' => 10,
            'J' => 11,
            'Q' => 12,
            'K' => 13,
            'A' => ACE,
            _ => return None,
        };
        let suit = match chars.next()? {
            'c' => 0,
            'd' => 1,
            'h' => 2,
            's' => 3,
            _ => return None,
        };
        chars.next().is_none().then_some(Card { rank, suit })
    }
}

/// Parses space-separated cards (`"Ah Kd"`). `None` when any card is
/// malformed or a card repeats.
pub fn parse_cards(text: &str) -> Option<Vec<Card>> {
    let cards: Vec<Card> = text.split_whitespace().map(Card::parse).collect::<Option<_>>()?;
    distinct(&cards).then_some(cards)
}

fn distinct(cards: &[Card]) -> bool {
    cards.iter().enumerate().all(|(i, c)| !cards[..i].contains(c))
}

/// Hand category, weakest first (the derived order is hand strength).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Category {
    HighCard,
    Pair,
    TwoPair,
    Trips,
    Straight,
    Flush,
    FullHouse,
    Quads,
    StraightFlush,
}

impl Category {
    pub fn as_str(&self) -> &'static str {
        match self {
            Category::HighCard => "high_card",
            Category::Pair => "pair",
            Category::TwoPair => "two_pair",
            Category::Trips => "trips",
            Category::Straight => "straight",
            Category::Flush => "flush",
            Category::FullHouse => "full_house",
            Category::Quads => "quads",
            Category::StraightFlush => "straight_flush",
        }
    }

    /// How many leading entries of `HandValue::ranks` define the made hand
    /// (the rest are kickers): the pair rank, both two-pair ranks, a
    /// straight's top card, all five flush cards.
    fn defining_len(&self) -> usize {
        match self {
            Category::HighCard => 0,
            Category::Pair | Category::Trips | Category::Quads => 1,
            Category::Straight | Category::StraightFlush => 1,
            Category::TwoPair | Category::FullHouse => 2,
            Category::Flush => 5,
        }
    }
}

/// A hand's strength: its category, then its ranks in comparison order
/// (made-hand ranks first, kickers after). Values compare as hands do;
/// equal values split the pot.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct HandValue {
    pub category: Category,
    pub ranks: Vec<u8>,
}

impl Ord for HandValue {
    fn cmp(&self, other: &Self) -> Ordering {
        self.category.cmp(&other.category).then_with(|| self.ranks.cmp(&other.ranks))
    }
}

impl PartialOrd for HandValue {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl HandValue {
    /// The made-hand part of `ranks`, without kickers.
    fn defining(&self) -> &[u8] {
        &self.ranks[..self.category.defining_len().min(self.ranks.len())]
    }
}

/// Pairs, trips and quads from rank counts: works for any number of cards,
/// so it also rates a three- or four-card board.
fn group_value(cards: &[Card]) -> HandValue {
    let mut counts = [0u8; 15];
    for card in cards {
        counts[card.rank as usize] += 1;
    }
    // (count, rank), most numerous first, then highest.
    let mut groups: Vec<(u8, u8)> =
        (2..=ACE).filter(|r| counts[*r as usize] > 0).map(|r| (counts[r as usize], r)).collect();
    groups.sort_by(|a, b| b.cmp(a));
    let top = groups.first().map_or(0, |g| g.0);
    let second = groups.get(1).map_or(0, |g| g.0);
    let category = match (top, second) {
        (4, _) => Category::Quads,
        (3, 2) => Category::FullHouse,
        (3, _) => Category::Trips,
        (2, 2) => Category::TwoPair,
        (2, _) => Category::Pair,
        _ => Category::HighCard,
    };
    let ranks = groups.iter().map(|g| g.1).collect();
    HandValue { category, ranks }
}

/// The top card of a straight in five distinct ranks; the wheel
/// (A-2-3-4-5) tops at five.
fn straight_top(cards: &[Card]) -> Option<u8> {
    let mut ranks: Vec<u8> = cards.iter().map(|c| c.rank).collect();
    ranks.sort_unstable_by(|a, b| b.cmp(a));
    ranks.dedup();
    if ranks.len() != 5 {
        return None;
    }
    if ranks[0] - ranks[4] == 4 {
        Some(ranks[0])
    } else if ranks == [ACE, 5, 4, 3, 2] {
        Some(5)
    } else {
        None
    }
}

fn evaluate_five(cards: &[Card]) -> HandValue {
    let flush = cards.iter().all(|c| c.suit == cards[0].suit);
    match (straight_top(cards), flush) {
        (Some(top), true) => HandValue { category: Category::StraightFlush, ranks: vec![top] },
        (Some(top), false) => HandValue { category: Category::Straight, ranks: vec![top] },
        (None, true) => {
            let mut ranks: Vec<u8> = cards.iter().map(|c| c.rank).collect();
            ranks.sort_unstable_by(|a, b| b.cmp(a));
            HandValue { category: Category::Flush, ranks }
        }
        (None, false) => group_value(cards),
    }
}

/// The best five-card hand among five to seven distinct cards. `None` for
/// any other count or a repeated card.
pub fn evaluate(cards: &[Card]) -> Option<HandValue> {
    if !(5..=7).contains(&cards.len()) || !distinct(cards) {
        return None;
    }
    let n = cards.len();
    // Every five-card subset, as a bit mask over the cards.
    (0u32..1 << n)
        .filter(|mask| mask.count_ones() == 5)
        .map(|mask| {
            let five: Vec<Card> = (0..n).filter(|i| mask & (1 << i) != 0).map(|i| cards[i]).collect();
            evaluate_five(&five)
        })
        .max()
}

/// The strength of the board's own cards: the best five on a full board,
/// the pairs and trips of a three- or four-card board.
fn board_value(board: &[Card]) -> HandValue {
    evaluate(board).unwrap_or_else(|| group_value(board))
}

/// "Board plays": the best hand equals the board's own best five (a full
/// five-card board only).
pub fn board_plays(hole: &[Card], board: &[Card]) -> bool {
    if board.len() != 5 {
        return false;
    }
    let all: Vec<Card> = hole.iter().chain(board).copied().collect();
    match (evaluate(&all), evaluate(board)) {
        (Some(best), Some(own)) => best == own,
        _ => false,
    }
}

/// What two hole cards make with a board, in the classes the spec's
/// value/bluff rule reads (section 9).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HoleStrength {
    /// Two pair or better made with a hole card: a set, trips, a straight,
    /// a flush, two pair with both hole cards, and so on. Two pair that is a
    /// board pair plus one hole-card pair is rated as that single pair.
    MadeHand,
    /// A pocket pair above every board card.
    Overpair,
    /// A hole card pairs the highest board card and the other hole card is
    /// a ten or better.
    TopPairGoodKicker,
    /// A hole card pairs the highest board card with a kicker below ten.
    TopPairWeakKicker,
    /// A hole card pairs a lower board card, or a pocket pair sits between
    /// the board cards.
    SecondPairOrLower,
    /// A pocket pair below every board card.
    Underpair,
    /// No pair made with a hole card: high card, a board-only pair or the
    /// board playing, including missed draws.
    NoPair,
}

/// Classifies two hole cards against a board of three to five cards.
/// `None` for any other card count or a repeated card.
pub fn hole_strength(hole: &[Card], board: &[Card]) -> Option<HoleStrength> {
    if hole.len() != 2 || !(3..=5).contains(&board.len()) {
        return None;
    }
    let all: Vec<Card> = hole.iter().chain(board).copied().collect();
    let best = evaluate(&all)?;
    let own = board_value(board);

    // A hole card took part only when it beat what the board makes alone:
    // a better category, or the same category with better made-hand ranks
    // (a higher flush card, a higher top pair). A better kicker is not it.
    let uses_hole_card = best.category > own.category
        || (best.category == own.category && best.defining() > own.defining());
    if !uses_hole_card {
        return Some(HoleStrength::NoPair);
    }

    let board_paired = own.category >= Category::Pair;
    let pairs_board = |card: &Card| board.iter().any(|b| b.rank == card.rank);
    let (a, b) = (hole[0], hole[1]);
    let pocket_pair = a.rank == b.rank;
    let both_pair_board = !pocket_pair && pairs_board(&a) && pairs_board(&b);

    let single_pair_hand = match best.category {
        Category::Pair => true,
        // A board pair plus one pair from the hole is one pair in practice.
        Category::TwoPair => board_paired && !both_pair_board,
        _ => false,
    };
    if !single_pair_hand {
        return Some(HoleStrength::MadeHand);
    }

    let top_board = board.iter().map(|c| c.rank).max()?;
    let low_board = board.iter().map(|c| c.rank).min()?;
    if pocket_pair {
        return Some(if a.rank > top_board {
            HoleStrength::Overpair
        } else if a.rank < low_board {
            HoleStrength::Underpair
        } else {
            HoleStrength::SecondPairOrLower
        });
    }
    // The hole card that pairs the board (the higher one if both do), and
    // the other one as the kicker.
    let (paired, kicker) = match (pairs_board(&a), pairs_board(&b)) {
        (true, true) if b.rank > a.rank => (b, a),
        (true, _) => (a, b),
        (false, true) => (b, a),
        (false, false) => return Some(HoleStrength::NoPair),
    };
    Some(if paired.rank == top_board {
        if kicker.rank >= TEN {
            HoleStrength::TopPairGoodKicker
        } else {
            HoleStrength::TopPairWeakKicker
        }
    } else {
        HoleStrength::SecondPairOrLower
    })
}
