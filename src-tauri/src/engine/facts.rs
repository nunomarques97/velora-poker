//! Hand facts for the opponent engine: one player's completed hands loaded
//! with everything an amount-aware replay needs, and the stat-event shape the
//! extractors emit (`docs/specs/opponent-engine.md`, sections 4 and 14).
//!
//! The loader runs a fixed number of queries per player (hands, seats,
//! actions), never one query per hand: twelve overlays share one SQLite
//! mutex, so a replay must not scale its round-trips with the sample size.

use std::collections::HashMap;

use rusqlite::{params, Connection};

use crate::parser::{ActionType, Street};

/// One player dealt into a stored hand.
#[derive(Debug, Clone, PartialEq)]
pub struct SeatFact {
    pub player_id: i64,
    pub seat: Option<i64>,
    /// Chips (or money, in cash games) in front of the player when the hand
    /// started, before antes and blinds.
    pub starting_stack: Option<f64>,
    pub position: Option<String>,
    pub is_hero: bool,
    pub hole_cards: Option<String>,
    pub went_to_showdown: bool,
    pub won_at_showdown: bool,
    pub net_result: Option<f64>,
    pub bounty: Option<f64>,
}

/// One logged action. `amount` keeps PokerStars' meaning: the increment for
/// posts, bets and calls, the street total ("raises X to Y" → Y) for raises.
#[derive(Debug, Clone, PartialEq)]
pub struct ActionFact {
    pub player_id: i64,
    pub street: Street,
    pub index: i64,
    pub kind: ActionType,
    pub amount: Option<f64>,
    pub is_all_in: bool,
}

/// A completed hand as stored, with its dealt-in seats (ascending seat
/// number) and its actions (in logged order).
#[derive(Debug, Clone, PartialEq)]
pub struct HandFacts {
    /// `hands.id`.
    pub id: i64,
    /// PokerStars' hand number (`hands.hand_id`).
    pub hand_ref: String,
    pub played_at: Option<String>,
    /// `cash` or `tournament`.
    pub format: String,
    /// `hands.variant` (`cash`, `zoom_cash`, `tournament`, `zoom_tournament`,
    /// `spin`); `None` only for a row the reparse backfill has not reached.
    pub variant: Option<String>,
    pub table_name: Option<String>,
    pub small_blind: Option<f64>,
    pub big_blind: Option<f64>,
    pub level: Option<String>,
    pub buy_in: Option<String>,
    pub max_seats: Option<i64>,
    pub button_seat: Option<i64>,
    pub seats: Vec<SeatFact>,
    pub actions: Vec<ActionFact>,
}

impl HandFacts {
    pub fn seat_of(&self, player_id: i64) -> Option<&SeatFact> {
        self.seats.iter().find(|s| s.player_id == player_id)
    }

    pub fn hero(&self) -> Option<&SeatFact> {
        self.seats.iter().find(|s| s.is_hero)
    }

    /// Effective stack in big blinds at hand start (spec section 14): the
    /// smaller of the player's starting stack and the largest other dealt-in
    /// starting stack. `None` when a stack or the big blind is unknown.
    pub fn effective_stack_bb(&self, player_id: i64) -> Option<f64> {
        let bb = self.big_blind.filter(|bb| *bb > 0.0)?;
        let own = self.seat_of(player_id)?.starting_stack?;
        let largest_other = self
            .seats
            .iter()
            .filter(|s| s.player_id != player_id)
            .filter_map(|s| s.starting_stack)
            .fold(None, |acc: Option<f64>, stack| Some(acc.map_or(stack, |a| a.max(stack))))?;
        Some(own.min(largest_other) / bb)
    }

    pub fn preflop_actions(&self) -> impl Iterator<Item = &ActionFact> {
        self.actions.iter().filter(|a| a.street == Street::Preflop)
    }
}

fn parse_street(value: &str) -> Option<Street> {
    match value {
        "preflop" => Some(Street::Preflop),
        "flop" => Some(Street::Flop),
        "turn" => Some(Street::Turn),
        "river" => Some(Street::River),
        _ => None,
    }
}

fn parse_action_type(value: &str) -> Option<ActionType> {
    match value {
        "post_small_blind" => Some(ActionType::PostSmallBlind),
        "post_big_blind" => Some(ActionType::PostBigBlind),
        "post_ante" => Some(ActionType::PostAnte),
        "fold" => Some(ActionType::Fold),
        "check" => Some(ActionType::Check),
        "call" => Some(ActionType::Call),
        "bet" => Some(ActionType::Bet),
        "raise" => Some(ActionType::Raise),
        _ => None,
    }
}

/// Loads every stored hand the player was dealt into, oldest first (by
/// `played_at`, then row id), with all of each hand's seats and actions.
///
/// Exactly three queries, whatever the number of hands.
pub fn load_player_hands(conn: &Connection, player_id: i64) -> rusqlite::Result<Vec<HandFacts>> {
    let mut hands: Vec<HandFacts> = conn
        .prepare(
            "SELECT h.id, h.hand_id, h.played_at, h.format, h.variant, h.table_name,
                    h.small_blind, h.big_blind, h.level, h.buy_in, h.max_seats, h.button_seat
             FROM hands h
             WHERE h.id IN (SELECT hand_id FROM player_hands WHERE player_id = ?1)
             ORDER BY h.played_at, h.id",
        )?
        .query_map(params![player_id], |row| {
            Ok(HandFacts {
                id: row.get(0)?,
                hand_ref: row.get(1)?,
                played_at: row.get(2)?,
                format: row.get(3)?,
                variant: row.get(4)?,
                table_name: row.get(5)?,
                small_blind: row.get(6)?,
                big_blind: row.get(7)?,
                level: row.get(8)?,
                buy_in: row.get(9)?,
                max_seats: row.get(10)?,
                button_seat: row.get(11)?,
                seats: Vec::new(),
                actions: Vec::new(),
            })
        })?
        .collect::<rusqlite::Result<_>>()?;

    let index: HashMap<i64, usize> = hands.iter().enumerate().map(|(i, h)| (h.id, i)).collect();

    let mut seats = conn.prepare(
        "SELECT ph.hand_id, ph.player_id, ph.seat, ph.starting_stack, ph.position, ph.is_hero,
                ph.hole_cards, ph.went_to_showdown, ph.won_at_showdown, ph.net_result, ph.bounty
         FROM player_hands ph
         WHERE ph.hand_id IN (SELECT hand_id FROM player_hands WHERE player_id = ?1)
         ORDER BY ph.hand_id, ph.seat",
    )?;
    let mut rows = seats.query(params![player_id])?;
    while let Some(row) = rows.next()? {
        let hand_id: i64 = row.get(0)?;
        let Some(&i) = index.get(&hand_id) else { continue };
        hands[i].seats.push(SeatFact {
            player_id: row.get(1)?,
            seat: row.get(2)?,
            starting_stack: row.get(3)?,
            position: row.get(4)?,
            is_hero: row.get::<_, i64>(5)? != 0,
            hole_cards: row.get(6)?,
            went_to_showdown: row.get::<_, i64>(7)? != 0,
            won_at_showdown: row.get::<_, i64>(8)? != 0,
            net_result: row.get(9)?,
            bounty: row.get(10)?,
        });
    }

    let mut actions = conn.prepare(
        "SELECT a.hand_id, a.player_id, a.street, a.action_index, a.action_type, a.amount, a.is_all_in
         FROM actions a
         WHERE a.hand_id IN (SELECT hand_id FROM player_hands WHERE player_id = ?1)
         ORDER BY a.hand_id, a.action_index, a.id",
    )?;
    let mut rows = actions.query(params![player_id])?;
    while let Some(row) = rows.next()? {
        let hand_id: i64 = row.get(0)?;
        let Some(&i) = index.get(&hand_id) else { continue };
        let street: String = row.get(2)?;
        let kind: String = row.get(4)?;
        // A row the current parser could never have written is skipped, not
        // guessed at; the replay then sees that hand as if the action were
        // absent, which only ever removes opportunities.
        let (Some(street), Some(kind)) = (parse_street(&street), parse_action_type(&kind)) else {
            continue;
        };
        hands[i].actions.push(ActionFact {
            player_id: row.get(1)?,
            street,
            index: row.get(3)?,
            kind,
            amount: row.get(5)?,
            is_all_in: row.get::<_, i64>(6)? != 0,
        });
    }

    Ok(hands)
}

/// Every stat the engine extracts. Keys match the spec's table 4.3; each
/// extraction family adds its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum StatKey {
    Vpip,
    Pfr,
    RfiEp,
    RfiMp,
    RfiCo,
    RfiBtn,
    RfiSb,
    Steal,
    FoldToStealSb,
    FoldToStealBb,
    BbDefendVsSb,
    ThreeBetIp,
    ThreeBetOop,
    FoldTo3betIp,
    FoldTo3betOop,
    FourBet,
    FoldTo4bet,
    Squeeze,
    ColdCall,
    Limp,
    LimpFold,
    LimpCall,
    LimpReraise,
    IsoRaise,
    OpenShove,
    CallVsShove,
    Reshove,
    CbetFlop,
    CbetTurn,
    CbetRiver,
    FoldToCbetFlop,
    FoldToCbetTurn,
    FoldToCbetRiver,
    DelayedCbet,
    CheckRaiseFlop,
    DonkFlop,
    ProbeTurn,
    FloatFlop,
    RiverBet,
    RiverRaise,
    Wtsd,
    Wsd,
    Wwsf,
}

impl StatKey {
    /// Every preflop and stack-depth key (catalogue rows P01–P12, S01–S03).
    pub const PREFLOP: [StatKey; 27] = [
        StatKey::Vpip,
        StatKey::Pfr,
        StatKey::RfiEp,
        StatKey::RfiMp,
        StatKey::RfiCo,
        StatKey::RfiBtn,
        StatKey::RfiSb,
        StatKey::Steal,
        StatKey::FoldToStealSb,
        StatKey::FoldToStealBb,
        StatKey::BbDefendVsSb,
        StatKey::ThreeBetIp,
        StatKey::ThreeBetOop,
        StatKey::FoldTo3betIp,
        StatKey::FoldTo3betOop,
        StatKey::FourBet,
        StatKey::FoldTo4bet,
        StatKey::Squeeze,
        StatKey::ColdCall,
        StatKey::Limp,
        StatKey::LimpFold,
        StatKey::LimpCall,
        StatKey::LimpReraise,
        StatKey::IsoRaise,
        StatKey::OpenShove,
        StatKey::CallVsShove,
        StatKey::Reshove,
    ];

    /// Every postflop key (catalogue rows F01–F12).
    pub const POSTFLOP: [StatKey; 16] = [
        StatKey::CbetFlop,
        StatKey::CbetTurn,
        StatKey::CbetRiver,
        StatKey::FoldToCbetFlop,
        StatKey::FoldToCbetTurn,
        StatKey::FoldToCbetRiver,
        StatKey::DelayedCbet,
        StatKey::CheckRaiseFlop,
        StatKey::DonkFlop,
        StatKey::ProbeTurn,
        StatKey::FloatFlop,
        StatKey::RiverBet,
        StatKey::RiverRaise,
        StatKey::Wtsd,
        StatKey::Wsd,
        StatKey::Wwsf,
    ];

    /// The spec's stat key (table 4.3).
    pub fn as_str(&self) -> &'static str {
        match self {
            StatKey::Vpip => "vpip",
            StatKey::Pfr => "pfr",
            StatKey::RfiEp => "rfi_ep",
            StatKey::RfiMp => "rfi_mp",
            StatKey::RfiCo => "rfi_co",
            StatKey::RfiBtn => "rfi_btn",
            StatKey::RfiSb => "rfi_sb",
            StatKey::Steal => "steal",
            StatKey::FoldToStealSb => "fold_to_steal_sb",
            StatKey::FoldToStealBb => "fold_to_steal_bb",
            StatKey::BbDefendVsSb => "bb_defend_vs_sb",
            StatKey::ThreeBetIp => "three_bet_ip",
            StatKey::ThreeBetOop => "three_bet_oop",
            StatKey::FoldTo3betIp => "fold_to_3bet_ip",
            StatKey::FoldTo3betOop => "fold_to_3bet_oop",
            StatKey::FourBet => "four_bet",
            StatKey::FoldTo4bet => "fold_to_4bet",
            StatKey::Squeeze => "squeeze",
            StatKey::ColdCall => "cold_call",
            StatKey::Limp => "limp",
            StatKey::LimpFold => "limp_fold",
            StatKey::LimpCall => "limp_call",
            StatKey::LimpReraise => "limp_reraise",
            StatKey::IsoRaise => "iso_raise",
            StatKey::OpenShove => "open_shove",
            StatKey::CallVsShove => "call_vs_shove",
            StatKey::Reshove => "reshove",
            StatKey::CbetFlop => "cbet_flop",
            StatKey::CbetTurn => "cbet_turn",
            StatKey::CbetRiver => "cbet_river",
            StatKey::FoldToCbetFlop => "fold_to_cbet_flop",
            StatKey::FoldToCbetTurn => "fold_to_cbet_turn",
            StatKey::FoldToCbetRiver => "fold_to_cbet_river",
            StatKey::DelayedCbet => "delayed_cbet",
            StatKey::CheckRaiseFlop => "check_raise_flop",
            StatKey::DonkFlop => "donk_flop",
            StatKey::ProbeTurn => "probe_turn",
            StatKey::FloatFlop => "float_flop",
            StatKey::RiverBet => "river_bet",
            StatKey::RiverRaise => "river_raise",
            StatKey::Wtsd => "wtsd",
            StatKey::Wsd => "wsd",
            StatKey::Wwsf => "wwsf",
        }
    }
}

/// Where the player sits relative to the spot's creator once the hand goes
/// postflop (the button acts last; heads-up, the button is in position). A
/// postflop spot with no creator (c-bet, river bet, WTSD) is placed against
/// the whole field: in position only when the player acts last.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Relation {
    InPosition,
    OutOfPosition,
}

/// Who created the spot a stat event records, and how the hero relates to it.
/// Head-to-head views (spec section 10) are built from this.
#[derive(Debug, Clone, PartialEq)]
pub struct Counterparty {
    /// The player whose action created the spot: the raiser being faced (the
    /// opener, 3-bettor, 4-bettor, stealer or shover), the 3-bettor or
    /// 4-bettor answering this player's own raise, the first limper for an
    /// isolation spot. `None` for an unopened pot (RFI, steal, limp, open
    /// shove) and for the dealt-in stats (VPIP, PFR).
    ///
    /// Postflop: the c-bettor or barreller faced (fold to c-bet, float), the
    /// bettor faced (check-raise, river raise), the preflop raiser bet into
    /// (donk) or who checked back (probe). `None` for the player's own
    /// c-bets, delayed c-bets and river bets, and for WTSD, W$SD and WWSF.
    pub creator: Option<i64>,
    /// The hero opened, raised or otherwise created the spot (`creator` is
    /// the hero).
    pub hero_created: bool,
    /// The hero was dealt into the hand.
    pub hero_in_hand: bool,
    /// The hero's position in the hand, when the hero was dealt in. With an
    /// unopened spot this tells whether the hero sat in the blinds the player
    /// was attacking (spec section 10, `steal_vs_hero`).
    pub hero_position: Option<String>,
    /// The hero is someone else and had not folded when the spot came up:
    /// the spot was played against the hero among others.
    pub hero_in_pot: bool,
}

/// One opportunity for one player on one stat: `(key, opportunity, success)`
/// plus the context the spot was played in. Extractors emit an event only
/// when the opportunity existed, so `opportunity` is always `true`; it is kept
/// so aggregation code sums the same shape for every family.
#[derive(Debug, Clone, PartialEq)]
pub struct StatEvent {
    pub key: StatKey,
    pub opportunity: bool,
    pub success: bool,
    pub player_id: i64,
    /// The player's position in the hand.
    pub position: Option<String>,
    /// The player's effective stack in big blinds at hand start.
    pub effective_stack_bb: Option<f64>,
    /// In or out of position relative to the spot's creator; `None` when the
    /// spot has no creator or a position is unknown.
    pub relation: Option<Relation>,
    /// Postflop: more than two players were in the hand (all-in players
    /// included) when the spot's street began. `None` for preflop spots.
    pub multiway: Option<bool>,
    pub counterparty: Counterparty,
}

/// The events one hand produced for one player.
#[derive(Debug, Clone, PartialEq)]
pub struct HandEvents {
    /// `hands.id`.
    pub hand_id: i64,
    pub hand_ref: String,
    pub played_at: Option<String>,
    pub variant: Option<String>,
    pub events: Vec<StatEvent>,
}
