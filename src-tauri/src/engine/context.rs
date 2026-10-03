//! Between-hands context (`docs/specs/opponent-engine.md`, section 5): what
//! the latest **completed** hand at the scoped table says about a villain —
//! effective stack, tournament stage, bounty and where he sits relative to
//! the hero.
//!
//! Everything comes from a stored hand history. Nothing here reads the hand
//! in progress, the screen or the network: the context is as old as the last
//! hand PokerStars wrote, which is exactly the "between hands" boundary.

use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;

use super::facts::{load_hand, HandFacts, SeatFact};
use super::pooling::FormatKey;

/// Spec rule id of the effective-stack derivation (catalogue row S06).
pub const RULE_EFFECTIVE_STACK: &str = "ctx.effective_stack";

/// Stack buckets of section 5, in big blinds.
pub const PUSH_FOLD_MAX_BB: f64 = 15.0;
pub const RESHOVE_MAX_BB: f64 = 25.0;
pub const MID_MAX_BB: f64 = 60.0;
pub const STANDARD_MAX_BB: f64 = 150.0;
/// Average table stack (bb) at or above which a tournament is `early`.
pub const STAGE_EARLY_MIN_BB: f64 = 40.0;
/// Average table stack (bb) at or above which a tournament is `middle`.
pub const STAGE_MIDDLE_MIN_BB: f64 = 20.0;
/// Bounty ratio from which a villain is a `big_bounty`.
pub const BIG_BOUNTY_RATIO: f64 = 2.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StackBucket {
    PushFold,
    Reshove,
    Mid,
    Standard,
    Deep,
}

impl StackBucket {
    pub fn of(effective_bb: f64) -> StackBucket {
        if effective_bb <= PUSH_FOLD_MAX_BB {
            StackBucket::PushFold
        } else if effective_bb <= RESHOVE_MAX_BB {
            StackBucket::Reshove
        } else if effective_bb <= MID_MAX_BB {
            StackBucket::Mid
        } else if effective_bb <= STANDARD_MAX_BB {
            StackBucket::Standard
        } else {
            StackBucket::Deep
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    Early,
    Middle,
    Late,
}

impl Stage {
    pub fn of(avg_stack_bb: f64) -> Stage {
        if avg_stack_bb >= STAGE_EARLY_MIN_BB {
            Stage::Early
        } else if avg_stack_bb >= STAGE_MIDDLE_MIN_BB {
            Stage::Middle
        } else {
            Stage::Late
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BountyContext {
    /// The villain's current bounty, from his seat line.
    pub amount: f64,
    /// ISO code from the buy-in's currency symbol, when recognised.
    pub currency: Option<String>,
    /// `amount / initial bounty`; `None` when the buy-in has no bounty
    /// component.
    pub ratio: Option<f64>,
    /// The hero's stack after the hand covers the villain's; `None` when the
    /// hero was not seated.
    pub hero_covers: Option<bool>,
}

impl BountyContext {
    pub fn is_big(&self) -> bool {
        self.ratio.is_some_and(|r| r >= BIG_BOUNTY_RATIO)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Side {
    Left,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SeatRelation {
    /// Occupied seats counted clockwise from the hero to the villain (1 =
    /// directly on the hero's left).
    pub distance: u32,
    pub side: Side,
    pub acts_after_hero: bool,
    pub direct_left: bool,
    pub direct_right: bool,
}

/// The `context` object of `PlayerPayload.engine` (section 13).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineContext {
    /// PokerStars' number of the hand the context was read from.
    pub source_hand_id: String,
    pub variant: Option<String>,
    pub format: FormatKey,
    pub effective_stack_bb: Option<f64>,
    pub stack_bucket: Option<StackBucket>,
    /// Tournaments only.
    pub stage: Option<Stage>,
    /// The blind level's number (`Level VI` → 6), reported alongside the
    /// stage, never used for it.
    pub level: Option<u32>,
    pub avg_stack_bb: Option<f64>,
    pub bounty: Option<BountyContext>,
    /// `None` when the hero was not seated.
    pub seat: Option<SeatRelation>,
}

/// The player's stack once the hand was over: starting stack plus net
/// result, or the starting stack alone where the net is unknown (tournament
/// hands store none).
pub fn stack_after(seat: &SeatFact) -> Option<f64> {
    let start = seat.starting_stack?;
    Some(start + seat.net_result.unwrap_or(0.0))
}

fn round1(value: f64) -> f64 {
    (value * 10.0).round() / 10.0
}

/// Effective stack in big blinds after the hand (section 5, row S06):
/// against the hero when the hero was seated, otherwise against the
/// biggest other stack. One decimal.
pub fn effective_stack_after_bb(hand: &HandFacts, villain_id: i64) -> Option<f64> {
    let bb = hand.big_blind.filter(|bb| *bb > 0.0)?;
    let villain = stack_after(hand.seat_of(villain_id)?)?;
    let opponent = match hand.hero().filter(|h| h.player_id != villain_id) {
        Some(hero) => stack_after(hero)?,
        None => hand
            .seats
            .iter()
            .filter(|s| s.player_id != villain_id)
            .filter_map(stack_after)
            .reduce(f64::max)?,
    };
    Some(round1(villain.min(opponent) / bb))
}

/// The Roman level number of a tournament header (`VI` → 6).
pub fn parse_level(level: &str) -> Option<u32> {
    let digit = |c: char| match c {
        'I' => Some(1),
        'V' => Some(5),
        'X' => Some(10),
        'L' => Some(50),
        'C' => Some(100),
        _ => None,
    };
    let level = level.trim();
    if let Ok(number) = level.parse::<u32>() {
        return Some(number).filter(|n| *n > 0);
    }
    let values: Vec<u32> = level.chars().map(digit).collect::<Option<_>>()?;
    let mut total = 0;
    for (i, value) in values.iter().enumerate() {
        match values.get(i + 1) {
            Some(next) if next > value => total -= *value as i64,
            _ => total += *value as i64,
        }
    }
    u32::try_from(total).ok().filter(|n| *n > 0)
}

fn is_tournament(hand: &HandFacts) -> bool {
    FormatKey::of_hand(hand) == FormatKey::Mtt || FormatKey::of_hand(hand) == FormatKey::Spin
}

/// Mean starting stack in big blinds over every dealt-in seat.
pub fn average_stack_bb(hand: &HandFacts) -> Option<f64> {
    let bb = hand.big_blind.filter(|bb| *bb > 0.0)?;
    let stacks: Vec<f64> = hand.seats.iter().filter_map(|s| s.starting_stack).collect();
    if stacks.is_empty() {
        return None;
    }
    Some(round1(stacks.iter().sum::<f64>() / stacks.len() as f64 / bb))
}

fn money(component: &str) -> Option<f64> {
    let digits: String =
        component.chars().filter(|c| c.is_ascii_digit() || *c == '.').collect();
    digits.parse().ok()
}

/// The initial bounty: the second component of a three-component buy-in
/// (`€13.50+€13.50+€3.00` → 13.50). `None` without a bounty component.
pub fn initial_bounty(buy_in: &str) -> Option<f64> {
    let parts: Vec<&str> = buy_in.split('+').collect();
    if parts.len() != 3 {
        return None;
    }
    money(parts[1]).filter(|b| *b > 0.0)
}

/// ISO currency of a buy-in, from its symbol.
pub fn buy_in_currency(buy_in: &str) -> Option<String> {
    let code = match buy_in.trim().chars().next()? {
        '€' => "EUR",
        '$' => "USD",
        '£' => "GBP",
        _ => return None,
    };
    Some(code.to_string())
}

/// The villain's bounty context, when his seat line carried a bounty.
pub fn bounty_context(hand: &HandFacts, villain_id: i64) -> Option<BountyContext> {
    let seat = hand.seat_of(villain_id)?;
    let amount = seat.bounty?;
    let buy_in = hand.buy_in.as_deref().unwrap_or("");
    let hero_covers = hand
        .hero()
        .filter(|h| h.player_id != villain_id)
        .and_then(|hero| Some(stack_after(hero)? >= stack_after(seat)?));
    Some(BountyContext {
        amount,
        currency: buy_in_currency(buy_in),
        ratio: initial_bounty(buy_in).map(|initial| (amount / initial * 100.0).round() / 100.0),
        hero_covers,
    })
}

/// Where the villain sits relative to the hero over the seats occupied in
/// the hand (section 5). `None` when the hero was not seated, or the villain
/// is the hero.
pub fn seat_relation(hand: &HandFacts, villain_id: i64) -> Option<SeatRelation> {
    let hero = hand.hero()?;
    if hero.player_id == villain_id {
        return None;
    }
    let mut seats: Vec<(i64, i64)> =
        hand.seats.iter().filter_map(|s| Some((s.seat?, s.player_id))).collect();
    seats.sort_unstable();
    let n = seats.len();
    let hero_at = seats.iter().position(|(_, p)| *p == hero.player_id)?;
    let villain_at = seats.iter().position(|(_, p)| *p == villain_id)?;
    let distance = ((villain_at + n - hero_at) % n) as u32;
    let side = if f64::from(distance) <= (n as f64 - 1.0) / 2.0 { Side::Left } else { Side::Right };
    Some(SeatRelation {
        distance,
        side,
        acts_after_hero: side == Side::Left,
        direct_left: distance == 1,
        direct_right: distance as usize == n - 1,
    })
}

/// The whole context of one villain from one completed hand. `None` when
/// the villain was not dealt into it.
pub fn villain_context(hand: &HandFacts, villain_id: i64) -> Option<EngineContext> {
    hand.seat_of(villain_id)?;
    let tournament = is_tournament(hand);
    let avg_stack_bb = average_stack_bb(hand);
    let effective_stack_bb = effective_stack_after_bb(hand, villain_id);
    Some(EngineContext {
        source_hand_id: hand.hand_ref.clone(),
        variant: hand.variant.clone(),
        format: FormatKey::of_hand(hand),
        effective_stack_bb,
        stack_bucket: effective_stack_bb.map(StackBucket::of),
        stage: if tournament { avg_stack_bb.map(Stage::of) } else { None },
        level: if tournament { hand.level.as_deref().and_then(parse_level) } else { None },
        avg_stack_bb,
        bounty: bounty_context(hand, villain_id),
        seat: seat_relation(hand, villain_id),
    })
}

/// `hands.id` of the latest completed hand stored for a table, by play
/// order (the same order the active-table roster uses).
pub fn latest_table_hand_id(conn: &Connection, table_name: &str) -> rusqlite::Result<Option<i64>> {
    conn.query_row(
        "SELECT id FROM hands WHERE table_name = ?1 ORDER BY played_at DESC, id DESC LIMIT 1",
        params![table_name],
        |row| row.get(0),
    )
    .optional()
}

/// Loads the latest completed hand at a table (three queries).
pub fn latest_table_hand(conn: &Connection, table_name: &str) -> rusqlite::Result<Option<HandFacts>> {
    match latest_table_hand_id(conn, table_name)? {
        Some(id) => load_hand(conn, id),
        None => Ok(None),
    }
}

/// Loads the hand the active-table roster is read from: the latest
/// completed hand at `table_name` (any table when `None`) played no earlier
/// than `since`, by play order — the same query as
/// `db::list_active_table_players_with_seats`, so the context and the
/// roster always come from one hand.
pub fn latest_hand_in_scope(
    conn: &Connection,
    table_name: Option<&str>,
    since: Option<&str>,
) -> rusqlite::Result<Option<HandFacts>> {
    let id: Option<i64> = conn
        .query_row(
            "SELECT id FROM hands
             WHERE (?1 IS NULL OR table_name = ?1)
               AND (?2 IS NULL OR played_at >= ?2)
             ORDER BY played_at DESC, id DESC LIMIT 1",
            params![table_name, since],
            |row| row.get(0),
        )
        .optional()?;
    match id {
        Some(id) => load_hand(conn, id),
        None => Ok(None),
    }
}
