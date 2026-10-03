//! Auto-notes (catalogue row N01, section 11 of
//! `docs/specs/opponent-engine.md`): notable hands a villain *showed*,
//! written to his notes without typing.
//!
//! Every detector is a pure function of one completed, stored hand and its
//! final board; nothing reads a hand in progress. A note needs the
//! villain's cards on the table (`shows`, `showed` or `mucked [..]`), so the
//! hero's own `Dealt to` cards never yield one. Texts are factual: they say
//! what happened and what was shown, never what to do.
//!
//! Generation runs only in the `strategic-analysis` build (gating matrix,
//! section 2): after an import commits, over the hands that import added,
//! and once over every stored hand behind a settings flag
//! (`db::AUTO_NOTES_BACKFILL_FLAG`). Storage is `INSERT OR IGNORE` on
//! `UNIQUE(player_id, hand_id, kind)`, so a re-import or a second pass never
//! duplicates a note.

use std::collections::HashMap;

use rusqlite::{params, Connection};

use super::eval::{evaluate, hole_strength, parse_cards, Card, Category, HoleStrength};
use super::facts::{load_shown_hands_after, HandFacts};
use super::pot::{replay_pot, SizeBucket};
use super::preflop::{RESHOVE_MAX_BB, SHOVE_COMMIT_SHARE};
use super::showdown::{parse_board, ValueClass};
use crate::db;
use crate::parser::{ActionType, Street};

/// Spec rule ids of the detectors (catalogue row N01).
pub const RULE_LIMP_RERAISE_PREMIUM: &str = "an.limp_reraise_premium";
pub const RULE_RIVER_OVERBET_BLUFF: &str = "an.river_overbet_bluff";
pub const RULE_LIGHT_4BET_SHOWN: &str = "an.light_4bet_shown";
pub const RULE_SLOWPLAY_MONSTER: &str = "an.slowplay_monster";
pub const RULE_WEAK_SHOVE_SHOWN: &str = "an.weak_shove_shown";

/// Hands loaded per batch when generating notes, so a backfill never holds
/// every shown hand in memory at once.
const BATCH: i64 = 500;

/// The kinds of section 11, in table order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum AutoNoteKind {
    LimpReraisePremium,
    RiverOverbetBluff,
    Light4betShown,
    SlowplayMonster,
    WeakShoveShown,
}

impl AutoNoteKind {
    pub const ALL: [AutoNoteKind; 5] = [
        AutoNoteKind::LimpReraisePremium,
        AutoNoteKind::RiverOverbetBluff,
        AutoNoteKind::Light4betShown,
        AutoNoteKind::SlowplayMonster,
        AutoNoteKind::WeakShoveShown,
    ];

    /// The stored `kind` column.
    pub fn as_str(&self) -> &'static str {
        match self {
            AutoNoteKind::LimpReraisePremium => "limp_reraise_premium",
            AutoNoteKind::RiverOverbetBluff => "river_overbet_bluff",
            AutoNoteKind::Light4betShown => "light_4bet_shown",
            AutoNoteKind::SlowplayMonster => "slowplay_monster",
            AutoNoteKind::WeakShoveShown => "weak_shove_shown",
        }
    }

    pub fn rule_id(&self) -> &'static str {
        match self {
            AutoNoteKind::LimpReraisePremium => RULE_LIMP_RERAISE_PREMIUM,
            AutoNoteKind::RiverOverbetBluff => RULE_RIVER_OVERBET_BLUFF,
            AutoNoteKind::Light4betShown => RULE_LIGHT_4BET_SHOWN,
            AutoNoteKind::SlowplayMonster => RULE_SLOWPLAY_MONSTER,
            AutoNoteKind::WeakShoveShown => RULE_WEAK_SHOVE_SHOWN,
        }
    }
}

/// One detected note, before it is stored.
#[derive(Debug, Clone, PartialEq)]
pub struct DetectedNote {
    pub player_id: i64,
    /// `hands.id`.
    pub hand_id: i64,
    /// PokerStars' hand number.
    pub hand_ref: String,
    pub kind: AutoNoteKind,
    pub text: String,
}

/// A number without trailing zeros, at most one decimal: `12`, `12.5`.
fn one_decimal(value: f64) -> String {
    let rounded = (value * 10.0).round() / 10.0;
    if rounded.fract() == 0.0 {
        format!("{rounded:.0}")
    } else {
        format!("{rounded:.1}")
    }
}

fn category_text(category: Category) -> &'static str {
    match category {
        Category::HighCard => "high card",
        Category::Pair => "a pair",
        Category::TwoPair => "two pair",
        Category::Trips => "trips",
        Category::Straight => "a straight",
        Category::Flush => "a flush",
        Category::FullHouse => "a full house",
        Category::Quads => "quads",
        Category::StraightFlush => "a straight flush",
    }
}

fn is_voluntary(kind: ActionType) -> bool {
    matches!(kind, ActionType::Call | ActionType::Raise | ActionType::Bet)
}

fn is_post(kind: ActionType) -> bool {
    matches!(
        kind,
        ActionType::PostSmallBlind | ActionType::PostBigBlind | ActionType::PostAnte
    )
}

/// AA, KK, QQ or AK.
fn is_premium(hole: &[Card]) -> bool {
    let (hi, lo) = ranks(hole);
    (hi == lo && hi >= 12) || (hi == 14 && lo == 13)
}

/// TT+ or AQ+: the range a 4-bet is expected to hold.
fn is_strong_4bet_hand(hole: &[Card]) -> bool {
    let (hi, lo) = ranks(hole);
    (hi == lo && hi >= 10) || (hi == 14 && lo >= 12)
}

/// None of: a pair, an ace, two cards T or higher, a suited king.
fn is_weak_shove_hand(hole: &[Card]) -> bool {
    let (hi, lo) = ranks(hole);
    let suited = hole[0].suit == hole[1].suit;
    !(hi == lo || hi == 14 || lo >= 10 || (hi == 13 && suited))
}

fn ranks(hole: &[Card]) -> (u8, u8) {
    let (a, b) = (hole[0].rank, hole[1].rank);
    (a.max(b), a.min(b))
}

/// The villain open-limped (his first voluntary action was a call into a
/// pot nobody had entered) and later re-raised someone else's raise.
fn limp_reraised(hand: &HandFacts, player_id: i64) -> bool {
    let mut entered = false;
    let mut open_limped = false;
    let mut raised_by_other = false;
    for action in hand.preflop_actions().filter(|a| !is_post(a.kind)) {
        if action.player_id == player_id {
            if raised_by_other {
                // His first answer to the raise behind his limp.
                return action.kind == ActionType::Raise;
            }
            if open_limped || entered || action.kind != ActionType::Call {
                return false;
            }
            open_limped = true;
        } else if action.kind == ActionType::Raise && open_limped {
            raised_by_other = true;
        }
        entered |= is_voluntary(action.kind);
    }
    false
}

/// The villain made the third raise preflop (a 4-bet), or a later raise
/// all-in (a 5-bet shove).
fn four_bet(hand: &HandFacts, player_id: i64) -> bool {
    let mut raises = 0;
    for action in hand.preflop_actions() {
        if action.kind != ActionType::Raise {
            continue;
        }
        if action.player_id == player_id && (raises == 2 || (raises >= 3 && action.is_all_in)) {
            return true;
        }
        raises += 1;
    }
    false
}

/// The villain's effective stack when he open-shoved or reshoved preflop
/// at 25bb or less: his first voluntary action was an all-in raise (or a
/// raise committing at least half his stack) into at most one raise.
fn shove_depth(hand: &HandFacts, player_id: i64) -> Option<f64> {
    let eff = hand.effective_stack_bb(player_id).filter(|e| *e <= RESHOVE_MAX_BB)?;
    let stack = hand.seat_of(player_id)?.starting_stack;
    let mut raises = 0;
    for action in hand.preflop_actions().filter(|a| !is_post(a.kind)) {
        if action.player_id == player_id {
            if action.kind != ActionType::Raise || raises > 1 {
                return None;
            }
            let commits = match (action.amount, stack) {
                (Some(to), Some(stack)) if stack > 0.0 => to >= SHOVE_COMMIT_SHARE * stack,
                _ => false,
            };
            return (action.is_all_in || commits).then_some(eff);
        }
        if action.kind == ActionType::Raise {
            raises += 1;
        }
    }
    None
}

/// The villain's last river bet or raise, when it is an overbet or an
/// all-in: its pot fraction.
fn river_overbet(hand: &HandFacts, player_id: i64) -> Option<f64> {
    let replay = replay_pot(hand);
    let last = replay
        .actions
        .iter()
        .filter(|a| a.player_id == player_id && a.street == Street::River)
        .filter(|a| matches!(a.kind, ActionType::Bet | ActionType::Raise))
        .next_back()?;
    matches!(last.bucket?, SizeBucket::Overbet | SizeBucket::AllIn).then_some(last.fraction?)
}

/// The villain acted on the flop and the turn, only checking or calling,
/// with at least one call.
fn passive_flop_and_turn(hand: &HandFacts, player_id: i64) -> bool {
    let mut acted = (false, false);
    let mut called = false;
    for action in hand.actions.iter().filter(|a| a.player_id == player_id) {
        let street = match action.street {
            Street::Flop => &mut acted.0,
            Street::Turn => &mut acted.1,
            _ => continue,
        };
        match action.kind {
            ActionType::Check => {}
            ActionType::Call => called = true,
            _ => return false,
        }
        *street = true;
    }
    acted.0 && acted.1 && called
}

/// Every note `hand` yields, given its final board (`None` when the hand
/// history has no board line). Pure; in kind order per player, players in
/// seat order.
pub fn detect_hand(hand: &HandFacts, board: Option<&[Card]>) -> Vec<DetectedNote> {
    let mut notes = Vec::new();
    for seat in hand.seats.iter().filter(|s| !s.is_hero) {
        let Some(cards_text) = seat.hole_cards.as_deref() else { continue };
        let Some(hole) = parse_cards(cards_text).filter(|c| c.len() == 2) else { continue };
        let id = seat.player_id;
        let mut note = |kind: AutoNoteKind, text: String| {
            notes.push(DetectedNote {
                player_id: id,
                hand_id: hand.id,
                hand_ref: hand.hand_ref.clone(),
                kind,
                text,
            })
        };
        let hand_no = &hand.hand_ref;

        if limp_reraised(hand, id) && is_premium(&hole) {
            note(
                AutoNoteKind::LimpReraisePremium,
                format!("Hand #{hand_no}: limp-reraised preflop and showed {cards_text}."),
            );
        }

        if let Some(board) = board.filter(|b| b.len() == 5 && !hole.iter().any(|c| b.contains(c))) {
            let bluff = hole_strength(&hole, board)
                .is_some_and(|s| ValueClass::from_strength(s) == ValueClass::Bluff);
            if bluff {
                if let Some(fraction) = river_overbet(hand, id) {
                    note(
                        AutoNoteKind::RiverOverbetBluff,
                        format!(
                            "Hand #{hand_no}: overbet the river ({}× pot) and showed {cards_text}.",
                            one_decimal(fraction)
                        ),
                    );
                }
            }
        }

        if four_bet(hand, id) && !is_strong_4bet_hand(&hole) {
            note(
                AutoNoteKind::Light4betShown,
                format!("Hand #{hand_no}: re-raised a re-raise preflop and showed {cards_text}."),
            );
        }

        if let Some(turn) = board.filter(|b| b.len() >= 4).map(|b| &b[..4]) {
            if !hole.iter().any(|c| turn.contains(c)) && passive_flop_and_turn(hand, id) {
                let all: Vec<Card> = hole.iter().chain(turn).copied().collect();
                let made = hole_strength(&hole, turn) == Some(HoleStrength::MadeHand);
                if let Some(best) = evaluate(&all).filter(|v| made && v.category >= Category::Trips) {
                    note(
                        AutoNoteKind::SlowplayMonster,
                        format!(
                            "Hand #{hand_no}: only called flop and turn with {}.",
                            category_text(best.category)
                        ),
                    );
                }
            }
        }

        if let Some(eff) = shove_depth(hand, id).filter(|_| is_weak_shove_hand(&hole)) {
            note(
                AutoNoteKind::WeakShoveShown,
                format!("Hand #{hand_no}: shoved {}bb and showed {cards_text}.", one_decimal(eff)),
            );
        }
    }
    notes
}

/// The final board of each listed hand, from its `Board [..]` summary line.
fn load_boards(conn: &Connection, hand_ids: &[i64]) -> rusqlite::Result<HashMap<i64, Vec<Card>>> {
    let mut boards = HashMap::new();
    let mut stmt = conn.prepare("SELECT COALESCE(raw_text, '') FROM hands WHERE id = ?1")?;
    for id in hand_ids {
        let raw_text: String = stmt.query_row(params![id], |row| row.get(0))?;
        if let Some(board) = parse_board(&raw_text) {
            boards.insert(*id, board);
        }
    }
    Ok(boards)
}

/// Runs the detectors over every stored hand with `hands.id > after_id`
/// that has a shown villain hand, and stores what they find. Returns how
/// many notes were new. Callers gate it on the `strategic-analysis` build.
pub fn generate_auto_notes_after(conn: &Connection, after_id: i64) -> rusqlite::Result<usize> {
    let mut inserted = 0;
    let mut cursor = after_id;
    loop {
        let hands = load_shown_hands_after(conn, cursor, BATCH)?;
        let Some(max_id) = hands.iter().map(|h| h.id).max() else { break };
        let ids: Vec<i64> = hands.iter().map(|h| h.id).collect();
        let boards = load_boards(conn, &ids)?;
        for hand in &hands {
            for note in detect_hand(hand, boards.get(&hand.id).map(Vec::as_slice)) {
                if db::insert_auto_note(conn, note.player_id, note.hand_id, note.kind.as_str(), &note.text)? {
                    inserted += 1;
                }
            }
        }
        cursor = max_id;
    }
    Ok(inserted)
}

/// The largest `hands.id` stored (0 on an empty database): the cursor an
/// import takes before writing, so only the hands it adds are scanned.
pub fn max_hand_row_id(conn: &Connection) -> rusqlite::Result<i64> {
    conn.query_row("SELECT COALESCE(MAX(id), 0) FROM hands", [], |row| row.get(0))
}
