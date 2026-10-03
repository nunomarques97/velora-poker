//! Showdown memory and sizing tells (catalogue rows M01, M03 and M04,
//! section 9 of `docs/specs/opponent-engine.md`).
//!
//! A showdown record is built only from what the table actually saw: a
//! villain who reached showdown and whose cards were shown (`shows`,
//! `showed` or `mucked [..]` in the summary). A villain who mucked unseen,
//! or who showed after winning uncontested, adds nothing.
//!
//! The board is not a stored column, so it comes from the hand history's
//! own `Board [..]` summary line; only showdown hands' text is loaded.

use std::collections::HashMap;

use rusqlite::{params, Connection};

use super::eval::{evaluate, hole_strength, parse_cards, Card, Category, HoleStrength};
use super::facts::HandFacts;
use super::pot::{replay_pot, SizeBucket};
use crate::parser::{ActionType, Street};

/// Spec rule id of the showdown record (catalogue row M01).
pub const RULE_SHOWDOWN_MEMORY: &str = "sd.memory";
/// Spec rule id of the value/bluff classification (catalogue row M03).
pub const RULE_CLASSIFY: &str = "sd.classify";
/// A sizing tell is emitted only from this many classified bets in its
/// bucket (section 9); the `sd.tell.*` rules need more.
pub const SIZING_TELL_MIN_SAMPLE: u32 = 3;

/// What the villain's last aggressive action was made with (section 9).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValueClass {
    Value,
    Bluff,
    Neither,
}

impl ValueClass {
    pub fn as_str(&self) -> &'static str {
        match self {
            ValueClass::Value => "value",
            ValueClass::Bluff => "bluff",
            ValueClass::Neither => "neither",
        }
    }

    /// The spec's mapping of hole-card strength to a class: two pair or
    /// better with a hole card, an overpair or top pair with a ten-plus
    /// kicker is value; no pair made with a hole card is a bluff; any other
    /// pair is neither.
    pub fn from_strength(strength: HoleStrength) -> ValueClass {
        match strength {
            HoleStrength::MadeHand | HoleStrength::Overpair | HoleStrength::TopPairGoodKicker => {
                ValueClass::Value
            }
            HoleStrength::NoPair => ValueClass::Bluff,
            HoleStrength::TopPairWeakKicker
            | HoleStrength::SecondPairOrLower
            | HoleStrength::Underpair => ValueClass::Neither,
        }
    }
}

/// How the villain's showdown ended for him.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShowdownResult {
    Won,
    Lost,
    /// He won and another shown hand of exactly equal value won too.
    Split,
}

impl ShowdownResult {
    pub fn as_str(&self) -> &'static str {
        match self {
            ShowdownResult::Won => "won",
            ShowdownResult::Lost => "lost",
            ShowdownResult::Split => "split",
        }
    }
}

/// One voluntary action of the villain's line (posts excluded).
#[derive(Debug, Clone, PartialEq)]
pub struct LineStep {
    pub street: Street,
    pub action: ActionType,
    pub is_all_in: bool,
    /// Bets and raises only.
    pub size_bucket: Option<SizeBucket>,
    pub pot_fraction: Option<f64>,
}

/// The villain's last postflop bet or raise, classified against the board
/// dealt by that street.
#[derive(Debug, Clone, PartialEq)]
pub struct LastAggression {
    pub street: Street,
    pub size_bucket: SizeBucket,
    pub pot_fraction: f64,
    pub class: ValueClass,
}

/// What one villain showed in one hand and how he played it.
#[derive(Debug, Clone, PartialEq)]
pub struct ShowdownRecord {
    /// `hands.id`.
    pub hand_id: i64,
    pub hand_ref: String,
    pub played_at: Option<String>,
    /// As stored: `"Jd 9d"`.
    pub cards: String,
    /// The final board: `"Ts 8h 2c 7d Ks"`.
    pub board: String,
    /// Best five of the hole cards and the final board.
    pub category: Category,
    pub line: Vec<LineStep>,
    pub result: ShowdownResult,
    /// `None` when he made no postflop bet or raise.
    pub last_aggression: Option<LastAggression>,
}

/// Per-bucket count of classified last aggressions (section 9).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SizingTell {
    pub bucket: SizeBucket,
    pub value: u32,
    pub bluff: u32,
    pub neither: u32,
    /// `value + bluff + neither`.
    pub n: u32,
}

/// The cards on a hand history's `Board [..]` summary line. `None` when the
/// hand has no such line (no flop, or a board run twice) or it is malformed.
pub fn parse_board(raw_text: &str) -> Option<Vec<Card>> {
    let line = raw_text.lines().find_map(|l| l.trim().strip_prefix("Board ["))?;
    let cards = parse_cards(line.split(']').next()?)?;
    (cards.len() <= 5).then_some(cards)
}

/// Board cards dealt by `street`: three on the flop, four on the turn, all
/// five on the river.
fn board_at(board: &[Card], street: Street) -> &[Card] {
    let dealt = match street {
        Street::Preflop => 0,
        Street::Flop => 3,
        Street::Turn => 4,
        Street::River => 5,
    };
    &board[..dealt.min(board.len())]
}

fn cards_text(cards: &[Card]) -> String {
    const RANKS: &[u8; 15] = b"??23456789TJQKA";
    const SUITS: &[u8; 4] = b"cdhs";
    cards
        .iter()
        .map(|c| format!("{}{}", RANKS[c.rank as usize] as char, SUITS[c.suit as usize] as char))
        .collect::<Vec<_>>()
        .join(" ")
}

/// The showdown record of `player_id` in `hand`, given the hand's final
/// board. `None` unless he went to showdown with shown Hold'em cards on a
/// complete board; the hero's own seat never yields one (the hero is not a
/// villain, and his `Dealt to` cards are not a shown hand).
pub fn showdown_record(hand: &HandFacts, board: &[Card], player_id: i64) -> Option<ShowdownRecord> {
    let seat = hand.seat_of(player_id)?;
    if seat.is_hero || !seat.went_to_showdown || board.len() != 5 {
        return None;
    }
    let hole = parse_cards(seat.hole_cards.as_deref()?)?;
    if hole.len() != 2 || hole.iter().any(|c| board.contains(c)) {
        return None;
    }
    let all: Vec<Card> = hole.iter().chain(board).copied().collect();
    let best = evaluate(&all)?;

    let result = if !seat.won_at_showdown {
        ShowdownResult::Lost
    } else {
        let tied = hand.seats.iter().any(|other| {
            other.player_id != player_id
                && other.won_at_showdown
                && other
                    .hole_cards
                    .as_deref()
                    .and_then(parse_cards)
                    .filter(|cards| cards.len() == 2)
                    .and_then(|cards| {
                        evaluate(&cards.iter().chain(board).copied().collect::<Vec<_>>())
                    })
                    .is_some_and(|value| value == best)
        });
        if tied {
            ShowdownResult::Split
        } else {
            ShowdownResult::Won
        }
    };

    let replay = replay_pot(hand);
    let line: Vec<LineStep> = replay
        .actions
        .iter()
        .filter(|a| a.player_id == player_id)
        .filter(|a| {
            !matches!(
                a.kind,
                ActionType::PostAnte | ActionType::PostSmallBlind | ActionType::PostBigBlind
            )
        })
        .map(|a| LineStep {
            street: a.street,
            action: a.kind,
            is_all_in: a.is_all_in,
            size_bucket: a.bucket,
            pot_fraction: a.fraction,
        })
        .collect();

    // A preflop raise has no board to be value or a bluff against, so only
    // postflop aggression is classified.
    let last_aggression = replay
        .actions
        .iter()
        .filter(|a| a.player_id == player_id && a.street != Street::Preflop)
        .filter(|a| matches!(a.kind, ActionType::Bet | ActionType::Raise))
        .next_back()
        .and_then(|a| {
            let strength = hole_strength(&hole, board_at(board, a.street))?;
            Some(LastAggression {
                street: a.street,
                size_bucket: a.bucket?,
                pot_fraction: a.fraction?,
                class: ValueClass::from_strength(strength),
            })
        });

    Some(ShowdownRecord {
        hand_id: hand.id,
        hand_ref: hand.hand_ref.clone(),
        played_at: hand.played_at.clone(),
        cards: cards_text(&hole),
        board: cards_text(board),
        category: best.category,
        line,
        result,
        last_aggression,
    })
}

/// Every showdown record of `player_id`, in `hands` order (oldest first
/// from the loader). `boards` maps `hands.id` to the final board.
pub fn extract_showdowns(
    hands: &[HandFacts],
    boards: &HashMap<i64, Vec<Card>>,
    player_id: i64,
) -> Vec<ShowdownRecord> {
    hands
        .iter()
        .filter_map(|hand| showdown_record(hand, boards.get(&hand.id)?, player_id))
        .collect()
}

/// Aggregates classified last aggressions per bucket, every bucket kept,
/// in bucket order (small to all-in).
pub fn sizing_tally(records: &[ShowdownRecord]) -> Vec<SizingTell> {
    let mut tally: Vec<SizingTell> = Vec::new();
    for aggression in records.iter().filter_map(|r| r.last_aggression.as_ref()) {
        let index = match tally.iter().position(|t| t.bucket == aggression.size_bucket) {
            Some(index) => index,
            None => {
                tally.push(SizingTell {
                    bucket: aggression.size_bucket,
                    value: 0,
                    bluff: 0,
                    neither: 0,
                    n: 0,
                });
                tally.len() - 1
            }
        };
        let tell = &mut tally[index];
        match aggression.class {
            ValueClass::Value => tell.value += 1,
            ValueClass::Bluff => tell.bluff += 1,
            ValueClass::Neither => tell.neither += 1,
        }
        tell.n += 1;
    }
    tally.sort_by_key(|t| t.bucket);
    tally
}

/// The sizing tells to show: buckets with at least
/// `SIZING_TELL_MIN_SAMPLE` classified bets, always with their counts.
pub fn sizing_tells(records: &[ShowdownRecord]) -> Vec<SizingTell> {
    sizing_tally(records).into_iter().filter(|t| t.n >= SIZING_TELL_MIN_SAMPLE).collect()
}

/// The final board of every hand where `player_id` went to showdown with
/// shown cards, keyed by `hands.id`. One query; hands without a parsable
/// single board are left out.
pub fn load_showdown_boards(
    conn: &Connection,
    player_id: i64,
) -> rusqlite::Result<HashMap<i64, Vec<Card>>> {
    let mut statement = conn.prepare(
        "SELECT h.id, COALESCE(h.raw_text, '')
         FROM hands h
         JOIN player_hands ph ON ph.hand_id = h.id
         WHERE ph.player_id = ?1 AND ph.went_to_showdown = 1 AND ph.hole_cards IS NOT NULL",
    )?;
    let mut rows = statement.query(params![player_id])?;
    let mut boards = HashMap::new();
    while let Some(row) = rows.next()? {
        let raw_text: String = row.get(1)?;
        if let Some(board) = parse_board(&raw_text) {
            boards.insert(row.get(0)?, board);
        }
    }
    Ok(boards)
}
