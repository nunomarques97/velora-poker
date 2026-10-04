//! Hand-written PokerStars edge-case corpus (`tests/fixtures/audit/`).
//!
//! Every fixture hand is pinned fact by fact: dealt-in and skipped seats,
//! positions, the full action list, showdown flags, net results (cash),
//! bounties, currency and variant, and the `ImportSummary` of importing it.
//! The amounts were worked out by hand from each hand's lines; every cash
//! hand's net results also sum to minus its rake.
//!
//! Formats the parser does not support (a non-English client, another room,
//! currencies other than USD/EUR/GBP/play money, an unreadable `posts` line)
//! are pinned as counted and never stored.

use rusqlite::{params, Connection};
use velora_poker_lib::db;
use velora_poker_lib::import::{import_text, ImportSummary};
use velora_poker_lib::parser::{
    parse_hand_block, split_hands, ActionType, GameVariant, HandFormat, ParsedHand, Street,
};

const CASH_6MAX_CHAT: &str = include_str!("fixtures/audit/cash_6max_usd_chat.txt");
const CASH_9MAX_PLAYERS: &str = include_str!("fixtures/audit/cash_9max_eur_players.txt");
const CASH_POTS: &str = include_str!("fixtures/audit/cash_pots_usd.txt");
const MTT_BOUNTY: &str = include_str!("fixtures/audit/mtt_bounty_antes.txt");
const SPIN: &str = include_str!("fixtures/audit/spin_and_go.txt");
const ZOOM_GBP: &str = include_str!("fixtures/audit/zoom_cash_gbp.txt");
const PLAY_MONEY: &str = include_str!("fixtures/audit/cash_play_money.txt");
const NAMES: &str = include_str!("fixtures/audit/names_special.txt");
const SEAT_SHAPED_NAME: &str = include_str!("fixtures/audit/names_seat_shaped.txt");
const NON_ENGLISH: &str = include_str!("fixtures/audit/unsupported_non_english.txt");
const OTHER_CURRENCIES: &str = include_str!("fixtures/audit/unsupported_currency.txt");
const STRADDLE: &str = include_str!("fixtures/audit/unsupported_straddle.txt");
const OTHER_ROOM: &str = include_str!("fixtures/audit/other_room.txt");

/// Every supported fixture with its hand count.
const SUPPORTED: [(&str, usize); 9] = [
    (CASH_6MAX_CHAT, 2),
    (CASH_9MAX_PLAYERS, 2),
    (CASH_POTS, 2),
    (MTT_BOUNTY, 1),
    (SPIN, 1),
    (ZOOM_GBP, 1),
    (PLAY_MONEY, 1),
    (NAMES, 3),
    (SEAT_SHAPED_NAME, 1),
];

use ActionType::{Bet, Call, Check, Fold, PostAnte, PostBigBlind, PostSmallBlind, Raise};
use Street::{Flop, Preflop, River, Turn};

/// `(street, player, type, amount, all-in)`.
type Act<'a> = (Street, &'a str, ActionType, Option<f64>, bool);

fn a(street: Street, player: &str, kind: ActionType, amount: f64) -> Act<'_> {
    (street, player, kind, Some(amount), false)
}

fn all_in(street: Street, player: &str, kind: ActionType, amount: f64) -> Act<'_> {
    (street, player, kind, Some(amount), true)
}

fn bare(street: Street, player: &str, kind: ActionType) -> Act<'_> {
    (street, player, kind, None, false)
}

fn parse(text: &str) -> Vec<ParsedHand> {
    split_hands(text)
        .iter()
        .map(|block| parse_hand_block(block).expect("fixture hand parses"))
        .collect()
}

fn hand<'a>(hands: &'a [ParsedHand], id: &str) -> &'a ParsedHand {
    hands
        .iter()
        .find(|h| h.hand_id == id)
        .unwrap_or_else(|| panic!("hand {id} not parsed"))
}

fn memory_db() -> Connection {
    db::open(std::path::Path::new(":memory:")).expect("open db")
}

/// `(imported, duplicate, failed, rejected, warnings, seats not dealt in)`.
fn counts(s: &ImportSummary) -> (i64, i64, i64, i64, i64, i64) {
    (
        s.hands_imported,
        s.hands_skipped_duplicate,
        s.hands_failed,
        s.hands_rejected_invalid,
        s.hands_with_warnings,
        s.seats_not_dealt_in,
    )
}

fn assert_actions(hand: &ParsedHand, expected: &[Act]) {
    let got: Vec<Act> = hand
        .actions
        .iter()
        .map(|a| (a.street, a.player_name.as_str(), a.action_type, a.amount, a.is_all_in))
        .collect();
    assert_eq!(got, expected, "actions of hand {}", hand.hand_id);
    for pair in hand.actions.windows(2) {
        assert!(pair[0].order < pair[1].order, "action order of hand {}", hand.hand_id);
    }
}

/// `(seat, player, position)` of every dealt-in seat, in seat-line order.
fn assert_seats(hand: &ParsedHand, expected: &[(i64, &str, &str)]) {
    let got: Vec<(i64, &str, Option<&str>)> = hand
        .seats
        .iter()
        .map(|s| (s.seat_number, s.player_name.as_str(), s.position.as_deref()))
        .collect();
    let expected: Vec<(i64, &str, Option<&str>)> =
        expected.iter().map(|(n, p, pos)| (*n, *p, Some(*pos))).collect();
    assert_eq!(got, expected, "dealt-in seats of hand {}", hand.hand_id);
}

fn assert_skipped(hand: &ParsedHand, expected: &[(i64, &str)]) {
    let got: Vec<(i64, &str)> = hand
        .skipped_seats
        .iter()
        .map(|s| (s.seat_number, s.player_name.as_str()))
        .collect();
    assert_eq!(got, expected, "skipped seats of hand {}", hand.hand_id);
}

/// `(player, went_to_showdown, won_at_showdown, net_result)` for every
/// dealt-in seat, which must be exactly the listed players.
fn assert_results(hand: &ParsedHand, expected: &[(&str, bool, bool, Option<f64>)]) {
    let mut seated: Vec<&str> = hand.seats.iter().map(|s| s.player_name.as_str()).collect();
    let mut listed: Vec<&str> = expected.iter().map(|e| e.0).collect();
    seated.sort_unstable();
    listed.sort_unstable();
    assert_eq!(seated, listed, "results must cover every dealt-in seat of hand {}", hand.hand_id);

    for (player, went, won, net) in expected {
        let result = hand
            .results
            .get(*player)
            .unwrap_or_else(|| panic!("no result for {player} in hand {}", hand.hand_id));
        assert_eq!(
            (result.went_to_showdown, result.won_at_showdown),
            (*went, *won),
            "showdown flags of {player} in hand {}",
            hand.hand_id
        );
        match (result.net_result, net) {
            (Some(got), Some(want)) => assert!(
                (got - want).abs() < 1e-9,
                "net result of {player} in hand {}: {got} != {want}",
                hand.hand_id
            ),
            (got, want) => assert_eq!(
                got, *want,
                "net result of {player} in hand {}",
                hand.hand_id
            ),
        }
    }
}

fn hole_cards<'a>(hand: &'a ParsedHand, player: &str) -> Option<&'a str> {
    hand.seats
        .iter()
        .find(|s| s.player_name == player)
        .unwrap_or_else(|| panic!("{player} not seated in hand {}", hand.hand_id))
        .hole_cards
        .as_deref()
}

/// The stored actions of one hand, in order, as `(street, player, type,
/// amount, all-in)`.
fn stored_actions(conn: &Connection, hand_id: &str) -> Vec<(String, String, String, Option<f64>, bool)> {
    let mut stmt = conn
        .prepare(
            "SELECT a.street, p.name, a.action_type, a.amount, a.is_all_in
               FROM actions a
               JOIN hands h ON h.id = a.hand_id
               JOIN players p ON p.id = a.player_id
              WHERE h.hand_id = ?1
              ORDER BY a.action_index",
        )
        .expect("prepare");
    stmt.query_map(params![hand_id], |r| {
        Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get::<_, i64>(4)? != 0))
    })
    .expect("query")
    .map(|r| r.expect("row"))
    .collect()
}

/// The stored player rows of one hand, by seat, as `(seat, player, position,
/// went_to_showdown, won_at_showdown, net_result, bounty)`.
#[allow(clippy::type_complexity)]
fn stored_players(
    conn: &Connection,
    hand_id: &str,
) -> Vec<(i64, String, Option<String>, bool, bool, Option<f64>, Option<f64>)> {
    let mut stmt = conn
        .prepare(
            "SELECT ph.seat, p.name, ph.position, ph.went_to_showdown, ph.won_at_showdown,
                    ph.net_result, ph.bounty
               FROM player_hands ph
               JOIN hands h ON h.id = ph.hand_id
               JOIN players p ON p.id = ph.player_id
              WHERE h.hand_id = ?1
              ORDER BY ph.seat",
        )
        .expect("prepare");
    stmt.query_map(params![hand_id], |r| {
        Ok((
            r.get(0)?,
            r.get(1)?,
            r.get(2)?,
            r.get::<_, i64>(3)? != 0,
            r.get::<_, i64>(4)? != 0,
            r.get(5)?,
            r.get(6)?,
        ))
    })
    .expect("query")
    .map(|r| r.expect("row"))
    .collect()
}

fn stored_hand_ids(conn: &Connection) -> Vec<String> {
    let mut stmt = conn.prepare("SELECT hand_id FROM hands ORDER BY hand_id").expect("prepare");
    stmt.query_map([], |r| r.get(0))
        .expect("query")
        .map(|r| r.expect("row"))
        .collect()
}

fn problem_codes(conn: &Connection, hand_id: &str) -> Vec<String> {
    let mut stmt = conn
        .prepare("SELECT code FROM import_problems WHERE hand_id = ?1 ORDER BY code")
        .expect("prepare");
    stmt.query_map(params![hand_id], |r| r.get(0))
        .expect("query")
        .map(|r| r.expect("row"))
        .collect()
}

// ---------------------------------------------------------------- cash formats

#[test]
fn cash_six_max_usd_hands_parse_exactly() {
    let hands = parse(CASH_6MAX_CHAT);
    assert_eq!(hands.len(), 2);

    let h = hand(&hands, "270000000001");
    assert_eq!((h.format, h.variant), (HandFormat::Cash, GameVariant::Cash));
    assert_eq!(h.currency, "USD");
    assert_eq!((h.small_blind, h.big_blind), (0.25, 0.5));
    assert_eq!((h.table_name.as_str(), h.max_seats, h.button_seat), ("Audit Alpha", 6, 1));
    assert_eq!(h.hero_name.as_deref(), Some("Hero"));
    assert!(h.complete);
    assert_seats(
        h,
        &[
            (1, "Alice", "BTN"),
            (2, "Bob", "SB"),
            (3, "Carol", "BB"),
            (4, "Dave", "UTG"),
            (5, "Erin", "HJ"),
            (6, "Hero", "CO"),
        ],
    );
    assert_skipped(h, &[]);
    assert_actions(
        h,
        &[
            a(Preflop, "Bob", PostSmallBlind, 0.25),
            a(Preflop, "Carol", PostBigBlind, 0.5),
            bare(Preflop, "Dave", Fold),
            bare(Preflop, "Erin", Fold),
            a(Preflop, "Hero", Raise, 1.5),
            bare(Preflop, "Alice", Fold),
            bare(Preflop, "Bob", Fold),
            a(Preflop, "Carol", Raise, 5.0),
            a(Preflop, "Hero", Call, 3.5),
            a(Flop, "Carol", Bet, 4.0),
            a(Flop, "Hero", Raise, 12.0),
            bare(Flop, "Carol", Fold),
        ],
    );
    // Hero: collected 17.75 + uncalled 8 - (5 preflop + 12 flop).
    assert_results(
        h,
        &[
            ("Alice", false, false, Some(0.0)),
            ("Bob", false, false, Some(-0.25)),
            ("Carol", false, false, Some(-9.0)),
            ("Dave", false, false, Some(0.0)),
            ("Erin", false, false, Some(0.0)),
            ("Hero", false, false, Some(8.75)),
        ],
    );

    let h = hand(&hands, "270000000002");
    assert_eq!(h.button_seat, 2);
    assert_seats(
        h,
        &[
            (1, "Alice", "CO"),
            (2, "Bob", "BTN"),
            (3, "Carol", "SB"),
            (4, "Dave", "BB"),
            (5, "Erin", "UTG"),
            (6, "Hero", "HJ"),
        ],
    );
    assert_actions(
        h,
        &[
            a(Preflop, "Carol", PostSmallBlind, 0.25),
            a(Preflop, "Dave", PostBigBlind, 0.5),
            a(Preflop, "Erin", Call, 0.5),
            a(Preflop, "Hero", Call, 0.5),
            bare(Preflop, "Alice", Fold),
            a(Preflop, "Bob", Call, 0.5),
            a(Preflop, "Carol", Call, 0.25),
            bare(Preflop, "Dave", Check),
            bare(Flop, "Carol", Check),
            bare(Flop, "Dave", Check),
            bare(Flop, "Erin", Check),
            bare(Flop, "Hero", Check),
            bare(Flop, "Bob", Check),
            bare(Turn, "Carol", Check),
            a(Turn, "Dave", Bet, 1.5),
            a(Turn, "Erin", Call, 1.5),
            a(Turn, "Hero", Call, 1.5),
            bare(Turn, "Bob", Fold),
            bare(Turn, "Carol", Fold),
            bare(River, "Dave", Check),
            bare(River, "Erin", Check),
            bare(River, "Hero", Check),
        ],
    );
    // Shows, a `mucks hand` at showdown, and a winner who shows.
    assert_results(
        h,
        &[
            ("Alice", false, false, Some(0.0)),
            ("Bob", false, false, Some(-0.5)),
            ("Carol", false, false, Some(-0.5)),
            ("Dave", true, false, Some(-2.0)),
            ("Erin", true, false, Some(-2.0)),
            ("Hero", true, true, Some(4.75)),
        ],
    );
    assert_eq!(hole_cards(h, "Dave"), Some("Jd Tc"));
    assert_eq!(hole_cards(h, "Erin"), Some("Ac 5c"));
    assert_eq!(hole_cards(h, "Hero"), Some("6s 6d"));
    assert_eq!(hole_cards(h, "Bob"), None);
}

/// Regression: a chat line whose text quotes an action (`Erin said, "Bob:
/// calls $5"`) was parsed as an action by a player named `Erin said, "Bob`,
/// so the integrity gate rejected the whole hand as `action_by_unseated_player`.
#[test]
fn chat_lines_that_quote_actions_add_no_action_and_keep_the_hand() {
    let hands = parse(CASH_6MAX_CHAT);
    for h in &hands {
        assert!(
            h.actions.iter().all(|a| !a.player_name.contains("said,")),
            "hand {} took a chat line for an action",
            h.hand_id
        );
    }
    assert_eq!(hand(&hands, "270000000001").actions.len(), 12);

    let mut conn = memory_db();
    let summary = import_text(&mut conn, CASH_6MAX_CHAT).expect("import");
    assert_eq!(counts(&summary), (2, 0, 0, 0, 0, 0), "{summary:?}");
    assert_eq!(stored_actions(&conn, "270000000001").len(), 12);
    let chat_players: i64 = conn
        .query_row("SELECT COUNT(*) FROM players WHERE name LIKE '%said,%'", [], |r| r.get(0))
        .expect("count");
    assert_eq!(chat_players, 0);
}

#[test]
fn nine_max_eur_hands_with_sit_outs_dead_blind_joins_and_disconnects() {
    let hands = parse(CASH_9MAX_PLAYERS);
    assert_eq!(hands.len(), 2);

    let h = hand(&hands, "270000000101");
    assert_eq!((h.variant, h.currency.as_str()), (GameVariant::Cash, "EUR"));
    assert_eq!((h.small_blind, h.big_blind, h.max_seats), (0.01, 0.02, 9));
    // Liam sits out and is never dealt in; Olga's stale `is sitting out`
    // marker does not keep her out of the hand she plays.
    assert_skipped(h, &[(4, "Liam")]);
    assert_seats(
        h,
        &[
            (1, "Ivan", "CO"),
            (2, "Judy", "BTN"),
            (3, "Ken", "SB"),
            (5, "Mia", "BB"),
            (6, "Ned", "UTG"),
            (7, "Olga", "LJ"),
            (8, "Hero", "HJ"),
        ],
    );
    // The dead blind is not an action row (see `dead_blind_is_part_of_the_posters_net_result`);
    // join, leave, timeout and connection lines add nothing.
    assert_actions(
        h,
        &[
            a(Preflop, "Ken", PostSmallBlind, 0.01),
            a(Preflop, "Mia", PostBigBlind, 0.02),
            bare(Preflop, "Ned", Fold),
            bare(Preflop, "Olga", Check),
            a(Preflop, "Hero", Raise, 0.08),
            bare(Preflop, "Ivan", Fold),
            a(Preflop, "Judy", Call, 0.08),
            bare(Preflop, "Ken", Fold),
            bare(Preflop, "Mia", Fold),
            a(Preflop, "Olga", Raise, 0.24),
            a(Preflop, "Hero", Call, 0.16),
            bare(Preflop, "Judy", Fold),
            a(Flop, "Olga", Bet, 0.3),
            a(Flop, "Hero", Call, 0.3),
            bare(Turn, "Olga", Check),
            bare(Turn, "Hero", Check),
            a(River, "Olga", Bet, 0.5),
            a(River, "Hero", Call, 0.5),
        ],
    );
    assert_results(
        h,
        &[
            ("Ivan", false, false, Some(0.0)),
            ("Judy", false, false, Some(-0.08)),
            ("Ken", false, false, Some(-0.01)),
            ("Mia", false, false, Some(-0.02)),
            ("Ned", false, false, Some(0.0)),
            ("Olga", true, true, Some(1.05)),
            ("Hero", true, false, Some(-1.04)),
        ],
    );
    assert_eq!(hole_cards(h, "Hero"), Some("Td Tc"));

    // A newcomer posts a big blind out of position; a sitting-out seat with a
    // bare summary line is skipped.
    let h = hand(&hands, "270000000102");
    assert_skipped(h, &[(1, "Ivan")]);
    assert_seats(
        h,
        &[
            (2, "Judy", "CO"),
            (3, "Ken", "BTN"),
            (5, "Mia", "SB"),
            (6, "Ned", "BB"),
            (7, "Olga", "UTG"),
            (8, "Hero", "LJ"),
            (9, "Pjotr", "HJ"),
        ],
    );
    assert_actions(
        h,
        &[
            a(Preflop, "Mia", PostSmallBlind, 0.01),
            a(Preflop, "Ned", PostBigBlind, 0.02),
            a(Preflop, "Pjotr", PostBigBlind, 0.02),
            bare(Preflop, "Olga", Fold),
            bare(Preflop, "Hero", Fold),
            bare(Preflop, "Pjotr", Check),
            a(Preflop, "Judy", Raise, 0.06),
            bare(Preflop, "Ken", Fold),
            bare(Preflop, "Mia", Fold),
            bare(Preflop, "Ned", Fold),
            bare(Preflop, "Pjotr", Fold),
        ],
    );
    assert_results(
        h,
        &[
            ("Judy", false, false, Some(0.05)),
            ("Ken", false, false, Some(0.0)),
            ("Mia", false, false, Some(-0.01)),
            ("Ned", false, false, Some(-0.02)),
            ("Olga", false, false, Some(0.0)),
            ("Hero", false, false, Some(0.0)),
            ("Pjotr", false, false, Some(-0.02)),
        ],
    );

    let mut conn = memory_db();
    let summary = import_text(&mut conn, CASH_9MAX_PLAYERS).expect("import");
    assert_eq!(counts(&summary), (2, 0, 0, 0, 0, 2), "{summary:?}");
}

/// Regression: `posts small & big blinds €0.03` was not read at all, so the
/// returning player's net result left out the 0.03 they put in, and their
/// later raise was resolved against no live blind.
#[test]
fn dead_blind_is_part_of_the_posters_net_result() {
    let hands = parse(CASH_9MAX_PLAYERS);
    let h = hand(&hands, "270000000101");
    // Olga: collected 2.10 - (0.03 dead + live, 0.22 more to raise to 0.24
    // over her live 0.02, 0.30 flop, 0.50 river).
    let olga = h.results.get("Olga").and_then(|r| r.net_result);
    assert_eq!(olga, Some(1.05));
}

/// Regression: a cash player who put nothing in had no net result at all
/// (`None`) instead of 0, unlike every other dealt-in cash player.
#[test]
fn a_cash_player_who_put_nothing_in_has_a_zero_net_result() {
    let hands = parse(CASH_6MAX_CHAT);
    let h = hand(&hands, "270000000001");
    for player in ["Alice", "Dave", "Erin"] {
        assert_eq!(h.results.get(player).and_then(|r| r.net_result), Some(0.0), "{player}");
    }
    let mut conn = memory_db();
    import_text(&mut conn, CASH_6MAX_CHAT).expect("import");
    let alice = stored_players(&conn, "270000000001")
        .into_iter()
        .find(|p| p.1 == "Alice")
        .expect("Alice stored");
    assert_eq!(alice.5, Some(0.0));
}

#[test]
fn zoom_cash_gbp_steal_with_uncalled_bet() {
    let hands = parse(ZOOM_GBP);
    let h = hand(&hands, "270000000601");
    assert_eq!((h.format, h.variant), (HandFormat::Cash, GameVariant::ZoomCash));
    assert_eq!(h.currency, "GBP");
    assert_eq!((h.small_blind, h.big_blind), (0.02, 0.05));
    assert_seats(
        h,
        &[
            (1, "Hero", "BTN"),
            (2, "Fenwick", "SB"),
            (3, "Gale", "BB"),
            (4, "Hollis", "UTG"),
            (5, "Iona", "HJ"),
            (6, "Jett", "CO"),
        ],
    );
    assert_actions(
        h,
        &[
            a(Preflop, "Fenwick", PostSmallBlind, 0.02),
            a(Preflop, "Gale", PostBigBlind, 0.05),
            bare(Preflop, "Hollis", Fold),
            bare(Preflop, "Iona", Fold),
            bare(Preflop, "Jett", Fold),
            a(Preflop, "Hero", Raise, 0.12),
            bare(Preflop, "Fenwick", Fold),
            bare(Preflop, "Gale", Fold),
        ],
    );
    assert_results(
        h,
        &[
            ("Hero", false, false, Some(0.07)),
            ("Fenwick", false, false, Some(-0.02)),
            ("Gale", false, false, Some(-0.05)),
            ("Hollis", false, false, Some(0.0)),
            ("Iona", false, false, Some(0.0)),
            ("Jett", false, false, Some(0.0)),
        ],
    );
}

/// Play-money header shape `(10/20)` with no currency is ASSUMED (no real
/// play-money hand history in the repository).
#[test]
fn play_money_cash_hand() {
    let hands = parse(PLAY_MONEY);
    let h = hand(&hands, "270000000701");
    assert_eq!((h.format, h.variant), (HandFormat::Cash, GameVariant::Cash));
    assert_eq!(h.currency, "PLAY");
    assert_eq!((h.small_blind, h.big_blind), (10.0, 20.0));
    assert_seats(h, &[(1, "PlayA", "BTN"), (3, "PlayB", "SB"), (5, "Hero", "BB")]);
    assert_actions(
        h,
        &[
            a(Preflop, "PlayB", PostSmallBlind, 10.0),
            a(Preflop, "Hero", PostBigBlind, 20.0),
            a(Preflop, "PlayA", Call, 20.0),
            a(Preflop, "PlayB", Call, 10.0),
            bare(Preflop, "Hero", Check),
            bare(Flop, "PlayB", Check),
            a(Flop, "Hero", Bet, 40.0),
            bare(Flop, "PlayA", Fold),
            bare(Flop, "PlayB", Fold),
        ],
    );
    assert_results(
        h,
        &[
            ("PlayA", false, false, Some(-20.0)),
            ("PlayB", false, false, Some(-20.0)),
            ("Hero", false, false, Some(40.0)),
        ],
    );
}

// ---------------------------------------------------------------- pots

#[test]
fn split_pot_winners_both_win_at_showdown_and_the_mucker_does_not() {
    let hands = parse(CASH_POTS);
    let h = hand(&hands, "270000000301");
    assert_seats(h, &[(1, "Quinn", "BB"), (2, "Rosa", "CO"), (4, "Tara", "BTN"), (5, "Uma", "SB")]);
    assert_actions(
        h,
        &[
            a(Preflop, "Uma", PostSmallBlind, 0.25),
            a(Preflop, "Quinn", PostBigBlind, 0.5),
            a(Preflop, "Rosa", Raise, 1.5),
            a(Preflop, "Tara", Call, 1.5),
            bare(Preflop, "Uma", Fold),
            a(Preflop, "Quinn", Call, 1.0),
            bare(Flop, "Quinn", Check),
            a(Flop, "Rosa", Bet, 2.0),
            a(Flop, "Tara", Call, 2.0),
            a(Flop, "Quinn", Call, 2.0),
            bare(Turn, "Quinn", Check),
            bare(Turn, "Rosa", Check),
            bare(Turn, "Tara", Check),
            bare(River, "Quinn", Check),
            a(River, "Rosa", Bet, 3.0),
            a(River, "Tara", Call, 3.0),
            a(River, "Quinn", Call, 3.0),
        ],
    );
    assert_results(
        h,
        &[
            ("Quinn", true, false, Some(-6.5)),
            ("Rosa", true, true, Some(3.38)),
            ("Tara", true, true, Some(3.37)),
            ("Uma", false, false, Some(-0.25)),
        ],
    );
    assert_eq!(hole_cards(h, "Quinn"), Some("Kc Qc"));
}

#[test]
fn side_pot_only_winner_wins_at_showdown_and_partial_all_in_call_is_flagged() {
    let hands = parse(CASH_POTS);
    let h = hand(&hands, "270000000302");
    assert_seats(h, &[(1, "Quinn", "BTN"), (2, "Rosa", "SB"), (4, "Tara", "BB"), (5, "Uma", "CO")]);
    assert_actions(
        h,
        &[
            a(Preflop, "Rosa", PostSmallBlind, 0.25),
            a(Preflop, "Tara", PostBigBlind, 0.5),
            a(Preflop, "Uma", Raise, 1.5),
            all_in(Preflop, "Quinn", Raise, 10.0),
            all_in(Preflop, "Rosa", Raise, 50.0),
            all_in(Preflop, "Tara", Call, 29.5),
            bare(Preflop, "Uma", Fold),
        ],
    );
    // Quinn wins only the main pot, Rosa only the side pot (plus her
    // uncalled 20): both are showdown winners.
    assert_results(
        h,
        &[
            ("Quinn", true, true, Some(21.5)),
            ("Rosa", true, true, Some(10.0)),
            ("Tara", true, false, Some(-30.0)),
            ("Uma", false, false, Some(-1.5)),
        ],
    );
}

// ---------------------------------------------------------------- tournaments

#[test]
fn mtt_hand_with_antes_bounties_and_side_pot() {
    let hands = parse(MTT_BOUNTY);
    let h = hand(&hands, "270000000401");
    assert_eq!((h.format, h.variant), (HandFormat::Tournament, GameVariant::Tournament));
    assert_eq!(h.currency, "CHIPS");
    assert_eq!(h.tournament_id.as_deref(), Some("4200000001"));
    assert_eq!(h.level.as_deref(), Some("IX"));
    assert_eq!((h.small_blind, h.big_blind, h.max_seats), (150.0, 300.0, 9));
    assert_eq!(h.hero_name.as_deref(), Some("TourneyHero"));
    // Zoe's seat says `is sitting out`, but she posts and folds: dealt in.
    assert_skipped(h, &[]);
    assert_seats(
        h,
        &[
            (1, "Wade", "HJ"),
            (2, "Xena", "CO"),
            (3, "Yuri", "BTN"),
            (4, "Zoe", "SB"),
            (5, "Abe", "BB"),
            (6, "Bea", "UTG"),
            (7, "Cal", "UTG+1"),
            (8, "TourneyHero", "UTG+2"),
            (9, "Dot", "LJ"),
        ],
    );
    let bounties: Vec<(&str, Option<f64>)> =
        h.seats.iter().map(|s| (s.player_name.as_str(), s.bounty)).collect();
    assert_eq!(
        bounties,
        vec![
            ("Wade", Some(10.0)),
            ("Xena", Some(15.0)),
            ("Yuri", Some(10.0)),
            ("Zoe", Some(10.0)),
            ("Abe", Some(12.5)),
            ("Bea", Some(22.5)),
            ("Cal", Some(10.0)),
            ("TourneyHero", Some(10.0)),
            ("Dot", Some(10.0)),
        ]
    );
    let mut expected: Vec<Act> = ["Wade", "Xena", "Yuri", "Zoe", "Abe", "Bea", "Cal", "TourneyHero", "Dot"]
        .into_iter()
        .map(|p| a(Preflop, p, PostAnte, 40.0))
        .collect();
    expected.extend([
        a(Preflop, "Zoe", PostSmallBlind, 150.0),
        a(Preflop, "Abe", PostBigBlind, 300.0),
        bare(Preflop, "Bea", Fold),
        bare(Preflop, "Cal", Fold),
        a(Preflop, "TourneyHero", Raise, 660.0),
        all_in(Preflop, "Dot", Raise, 2160.0),
        bare(Preflop, "Wade", Fold),
        bare(Preflop, "Xena", Fold),
        all_in(Preflop, "Yuri", Raise, 3060.0),
        bare(Preflop, "Zoe", Fold),
        bare(Preflop, "Abe", Fold),
        a(Preflop, "TourneyHero", Call, 2400.0),
    ]);
    assert_actions(h, &expected);
    // Dot wins only the main pot, the hero only the side pot. Tournament
    // chips are not money: no net result.
    assert_results(
        h,
        &[
            ("Wade", false, false, None),
            ("Xena", false, false, None),
            ("Yuri", true, false, None),
            ("Zoe", false, false, None),
            ("Abe", false, false, None),
            ("Bea", false, false, None),
            ("Cal", false, false, None),
            ("TourneyHero", true, true, None),
            ("Dot", true, true, None),
        ],
    );
    assert_eq!(hole_cards(h, "Yuri"), Some("Jd Jh"));
}

#[test]
fn spin_and_go_three_max_hand() {
    let hands = parse(SPIN);
    let h = hand(&hands, "270000000501");
    assert_eq!((h.format, h.variant), (HandFormat::Tournament, GameVariant::Spin));
    assert_eq!(h.buy_in.as_deref(), Some("$9.65+$0.35"));
    assert_eq!(h.level.as_deref(), Some("II"));
    assert_seats(h, &[(1, "SpinA", "BB"), (2, "Hero", "BTN"), (3, "SpinB", "SB")]);
    assert_actions(
        h,
        &[
            a(Preflop, "SpinB", PostSmallBlind, 15.0),
            a(Preflop, "SpinA", PostBigBlind, 30.0),
            a(Preflop, "Hero", Raise, 60.0),
            a(Preflop, "SpinB", Call, 45.0),
            bare(Preflop, "SpinA", Fold),
            bare(Flop, "SpinB", Check),
            a(Flop, "Hero", Bet, 60.0),
            all_in(Flop, "SpinB", Raise, 430.0),
            a(Flop, "Hero", Call, 370.0),
        ],
    );
    assert_results(
        h,
        &[
            ("SpinA", false, false, None),
            ("Hero", true, false, None),
            ("SpinB", true, true, None),
        ],
    );
    assert!(h.seats.iter().all(|s| s.bounty.is_none()));
}

// ---------------------------------------------------------------- names

/// Prefix names (`Bob`, `Bob1`, `Bob Jr`), an apostrophe, brackets and
/// spaces, unicode, and chat lines quoting other players' actions.
#[test]
fn special_character_and_prefix_names_keep_their_own_actions() {
    let hands = parse(NAMES);
    let h = hand(&hands, "270000000801");
    assert_eq!(h.hero_name.as_deref(), Some("[Pro] Kai"));
    assert_seats(
        h,
        &[
            (1, "Bob", "SB"),
            (2, "Bob1", "BB"),
            (3, "Bob Jr", "UTG"),
            (4, "O'Brien", "HJ"),
            (5, "[Pro] Kai", "CO"),
            (6, "Zoë ゆうき", "BTN"),
        ],
    );
    assert_actions(
        h,
        &[
            a(Preflop, "Bob", PostSmallBlind, 0.25),
            a(Preflop, "Bob1", PostBigBlind, 0.5),
            a(Preflop, "Bob Jr", Call, 0.5),
            bare(Preflop, "O'Brien", Fold),
            a(Preflop, "[Pro] Kai", Raise, 2.5),
            bare(Preflop, "Zoë ゆうき", Fold),
            bare(Preflop, "Bob", Fold),
            a(Preflop, "Bob1", Call, 2.0),
            a(Preflop, "Bob Jr", Call, 2.0),
            bare(Flop, "Bob1", Check),
            a(Flop, "Bob Jr", Bet, 4.0),
            a(Flop, "[Pro] Kai", Raise, 12.0),
            bare(Flop, "Bob1", Fold),
            a(Flop, "Bob Jr", Call, 8.0),
            bare(Turn, "Bob Jr", Check),
            a(Turn, "[Pro] Kai", Bet, 20.0),
            bare(Turn, "Bob Jr", Fold),
        ],
    );
    assert_results(
        h,
        &[
            ("Bob", false, false, Some(-0.25)),
            ("Bob1", false, false, Some(-2.5)),
            ("Bob Jr", false, false, Some(-14.5)),
            ("O'Brien", false, false, Some(0.0)),
            ("[Pro] Kai", false, false, Some(16.25)),
            ("Zoë ゆうき", false, false, Some(0.0)),
        ],
    );
    assert_eq!(hole_cards(h, "[Pro] Kai"), Some("Qs Qd"));

    // Heads-up: the button posts the small blind and is labelled BTN only.
    let h = hand(&hands, "270000000802");
    assert_seats(h, &[(1, "Mr (BR)", "BTN"), (2, "Bob", "BB")]);
    assert_actions(
        h,
        &[
            a(Preflop, "Mr (BR)", PostSmallBlind, 0.25),
            a(Preflop, "Bob", PostBigBlind, 0.5),
            a(Preflop, "Mr (BR)", Raise, 1.5),
            bare(Preflop, "Bob", Fold),
        ],
    );
    assert_results(
        h,
        &[("Mr (BR)", false, false, Some(0.5)), ("Bob", false, false, Some(-0.5))],
    );

    let mut conn = memory_db();
    let summary = import_text(&mut conn, NAMES).expect("import");
    assert_eq!(counts(&summary), (3, 0, 0, 0, 0, 0), "{summary:?}");
    let names: Vec<String> = {
        let mut stmt = conn.prepare("SELECT name FROM players ORDER BY name").expect("prepare");
        stmt.query_map([], |r| r.get(0)).expect("query").map(|r| r.expect("row")).collect()
    };
    assert_eq!(
        names,
        vec!["Ann: X", "Bob", "Bob Jr", "Bob1", "Mr (BR)", "O'Brien", "Zoë ゆうき", "[Pro] Kai"]
    );
}

/// Regression (hardening): a name holding `": "` split its action lines at
/// the wrong colon, so every action of `Ann: X` was dropped and the hand was
/// stored without them. Whether PokerStars allows a colon in a screen name
/// is ASSUMED, not confirmed.
#[test]
fn a_name_containing_a_colon_keeps_its_actions() {
    let hands = parse(NAMES);
    let h = hand(&hands, "270000000803");
    assert_seats(h, &[(1, "Ann: X", "BB"), (2, "Bob", "BTN")]);
    assert_actions(
        h,
        &[
            a(Preflop, "Bob", PostSmallBlind, 0.25),
            a(Preflop, "Ann: X", PostBigBlind, 0.5),
            a(Preflop, "Bob", Raise, 1.5),
            a(Preflop, "Ann: X", Call, 1.0),
            bare(Flop, "Ann: X", Check),
            a(Flop, "Bob", Bet, 2.0),
            bare(Flop, "Ann: X", Fold),
        ],
    );
    assert_results(
        h,
        &[("Ann: X", false, false, Some(-1.5)), ("Bob", false, false, Some(1.5))],
    );
}

/// Regression (hardening): a line shaped like a seat line after the seat
/// block — here every line of a player named `Seat 1: Alice ($1 in chips)`,
/// and their chat quoting a seat line — was parsed as a new seat, so the hand
/// listed Alice six times and was rejected. Seat lines are now read only in the
/// block after the table line. Such a name is ASSUMED impossible on
/// PokerStars (colon, length); the test pins that no line in the hand body
/// can add a seat.
#[test]
fn a_seat_shaped_line_in_the_hand_body_adds_no_seat() {
    let hands = parse(SEAT_SHAPED_NAME);
    let h = hand(&hands, "270000000804");
    let mallory = "Seat 1: Alice ($1 in chips)";
    assert_seats(h, &[(1, "Alice", "BTN"), (3, mallory, "BB")]);
    assert_skipped(h, &[]);
    assert_actions(
        h,
        &[
            a(Preflop, "Alice", PostSmallBlind, 0.25),
            a(Preflop, mallory, PostBigBlind, 0.5),
            a(Preflop, "Alice", Call, 0.25),
            bare(Preflop, mallory, Check),
            a(Flop, mallory, Bet, 1.0),
            bare(Flop, "Alice", Fold),
        ],
    );
    assert_results(h, &[("Alice", false, false, Some(-0.5)), (mallory, false, false, Some(0.5))]);

    let mut conn = memory_db();
    let summary = import_text(&mut conn, SEAT_SHAPED_NAME).expect("import");
    assert_eq!(counts(&summary), (1, 0, 0, 0, 0, 0), "{summary:?}");
}

// ---------------------------------------------------------------- whole corpus

/// Net results of a cash hand sum to minus its rake.
#[test]
fn cash_net_results_sum_to_minus_the_rake() {
    for (text, _) in SUPPORTED {
        for h in parse(text).iter().filter(|h| h.format == HandFormat::Cash) {
            let rake_text = h
                .raw_text
                .lines()
                .find_map(|l| l.split_once("| Rake ").map(|(_, r)| r.trim().to_string()))
                .expect("summary has a rake");
            let rake: f64 = rake_text
                .trim_start_matches(['$', '€', '£'])
                .parse()
                .unwrap_or_else(|_| panic!("rake {rake_text} of hand {}", h.hand_id));
            let total: f64 = h
                .seats
                .iter()
                .map(|s| {
                    h.results
                        .get(&s.player_name)
                        .and_then(|r| r.net_result)
                        .unwrap_or_else(|| panic!("{} has no net in {}", s.player_name, h.hand_id))
                })
                .sum();
            assert!((total + rake).abs() < 1e-9, "hand {}: nets {total}, rake {rake}", h.hand_id);
        }
    }
}

/// The whole supported corpus imports with exact counts, stores exactly the
/// parsed facts, and a second import adds nothing.
#[test]
fn supported_corpus_imports_exactly_and_reimports_as_duplicates() {
    let mut conn = memory_db();
    let mut total = ImportSummary::default();
    let mut parsed: Vec<ParsedHand> = Vec::new();
    for (text, n) in SUPPORTED {
        let hands = parse(text);
        assert_eq!(hands.len(), n);
        parsed.extend(hands);
        let s = import_text(&mut conn, text).expect("import");
        total.hands_imported += s.hands_imported;
        total.hands_skipped_duplicate += s.hands_skipped_duplicate;
        total.hands_failed += s.hands_failed;
        total.hands_rejected_invalid += s.hands_rejected_invalid;
        total.hands_with_warnings += s.hands_with_warnings;
        total.seats_not_dealt_in += s.seats_not_dealt_in;
    }
    assert_eq!(counts(&total), (14, 0, 0, 0, 0, 2), "{total:?}");

    for h in &parsed {
        let stored: Vec<(String, String, String, Option<f64>, bool)> = stored_actions(&conn, &h.hand_id);
        let want: Vec<(String, String, String, Option<f64>, bool)> = h
            .actions
            .iter()
            .map(|a| {
                (
                    a.street.as_str().to_string(),
                    a.player_name.clone(),
                    a.action_type.as_str().to_string(),
                    a.amount,
                    a.is_all_in,
                )
            })
            .collect();
        assert_eq!(stored, want, "stored actions of hand {}", h.hand_id);

        let players = stored_players(&conn, &h.hand_id);
        let mut seats: Vec<_> = h.seats.iter().collect();
        seats.sort_by_key(|s| s.seat_number);
        assert_eq!(players.len(), seats.len(), "player rows of hand {}", h.hand_id);
        for (row, seat) in players.iter().zip(seats) {
            let result = h.results.get(&seat.player_name).cloned().unwrap_or_default();
            assert_eq!(
                row,
                &(
                    seat.seat_number,
                    seat.player_name.clone(),
                    seat.position.clone(),
                    result.went_to_showdown,
                    result.won_at_showdown,
                    result.net_result,
                    seat.bounty,
                ),
                "stored player row of hand {}",
                h.hand_id
            );
        }
    }

    for (text, n) in SUPPORTED {
        let again = import_text(&mut conn, text).expect("re-import");
        assert_eq!(counts(&again).0, 0);
        assert_eq!(again.hands_skipped_duplicate, n as i64);
    }
    assert_eq!(stored_hand_ids(&conn).len(), 14);
}

// ---------------------------------------------------------------- unsupported

/// Regression: an English header over a translated body stored the hand with
/// its players and no action at all (a `no_actions` warning only), adding a
/// hand to every player's count that no stat could read.
#[test]
fn a_non_english_client_hand_is_rejected_and_counted() {
    let mut conn = memory_db();
    let summary = import_text(&mut conn, NON_ENGLISH).expect("import");
    assert_eq!(counts(&summary), (0, 0, 0, 1, 0, 0), "{summary:?}");
    assert!(stored_hand_ids(&conn).is_empty());
    assert!(problem_codes(&conn, "270000000901").contains(&"dealt_in_without_action".to_string()));
}

/// Regression: a currency code the app does not support (`CAD`), or one
/// that contradicts the amounts' symbol (`€… USD`), was stored as is. Now
/// such hands fail to parse and are counted; the rest of the file imports.
#[test]
fn unsupported_currencies_are_counted_and_never_stored() {
    let results: Vec<Result<ParsedHand, _>> =
        split_hands(OTHER_CURRENCIES).iter().map(|b| parse_hand_block(b)).collect();
    assert_eq!(results.len(), 4);
    assert!(results[0].is_err(), "INR");
    assert!(results[1].is_err(), "CAD");
    assert!(results[2].is_err(), "euro amounts labelled USD");
    assert_eq!(results[3].as_ref().expect("USD").currency, "USD");

    let mut conn = memory_db();
    let summary = import_text(&mut conn, OTHER_CURRENCIES).expect("import");
    assert_eq!(counts(&summary), (1, 0, 3, 0, 0, 0), "{summary:?}");
    assert_eq!(stored_hand_ids(&conn), vec!["270000000914".to_string()]);
}

/// Regression: a `posts` line the parser cannot read was dropped, so the
/// hand was stored without that money. The straddle line format is ASSUMED
/// (not confirmed by a real PokerStars hand history); any unreadable `posts`
/// line by a seated player is treated the same way.
#[test]
fn an_unreadable_post_line_rejects_the_hand() {
    let mut conn = memory_db();
    let summary = import_text(&mut conn, STRADDLE).expect("import");
    assert_eq!(counts(&summary), (1, 0, 0, 1, 0, 0), "{summary:?}");
    assert_eq!(stored_hand_ids(&conn), vec!["270000000922".to_string()]);
    assert!(problem_codes(&conn, "270000000921").contains(&"unrecognized_action".to_string()));
}

/// Another room's hand history has no PokerStars header: nothing is parsed,
/// nothing is stored.
#[test]
fn another_rooms_hand_history_stores_nothing() {
    assert!(split_hands(OTHER_ROOM).is_empty());
    let mut conn = memory_db();
    let summary = import_text(&mut conn, OTHER_ROOM).expect("import");
    assert_eq!(counts(&summary), (0, 0, 0, 0, 0, 0));
    assert!(stored_hand_ids(&conn).is_empty());
}
