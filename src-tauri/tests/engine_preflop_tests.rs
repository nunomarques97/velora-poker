//! Opponent engine, task T3: hand facts and preflop / stack-depth extraction
//! (catalogue rows P01–P12 and S01–S03 of `docs/specs/opponent-engine.md`).
//!
//! Fixtures: `engine_preflop_cash.txt` is a 6-max $0.25/$0.50 cash table with
//! 100bb stacks; `engine_preflop_tournament.txt` is a 6-max tournament at
//! 100/200 with mixed stack depths (Ugo 12bb, Cole 20bb, Hal 30bb). Both seat
//! the same names: Hero (seat 1, button), Sam (SB), Bob (BB), Ugo (UTG),
//! Hal (HJ), Cole (CO).

use velora_poker_lib::engine::{
    self, extract_preflop, load_player_hands, HandFacts, Relation, StatEvent, StatKey,
};
use velora_poker_lib::parser::{ActionType, Street};
use velora_poker_lib::{db, import};

const CASH: &str = include_str!("fixtures/engine_preflop_cash.txt");
const TOURNAMENT: &str = include_str!("fixtures/engine_preflop_tournament.txt");

fn setup() -> rusqlite::Connection {
    let mut conn = db::open(std::path::Path::new(":memory:")).expect("open db");
    import::import_text(&mut conn, CASH).expect("import cash fixture");
    import::import_text(&mut conn, TOURNAMENT).expect("import tournament fixture");
    conn
}

fn player_id(conn: &rusqlite::Connection, name: &str) -> i64 {
    conn.query_row("SELECT id FROM players WHERE name = ?1", [name], |row| row.get(0))
        .unwrap_or_else(|_| panic!("player {name} not found"))
}

/// The events `name` produced in hand `hand_ref`.
fn events(conn: &rusqlite::Connection, name: &str, hand_ref: &str) -> Vec<StatEvent> {
    let id = player_id(conn, name);
    let hands = load_player_hands(conn, id).expect("load hands");
    let hand = hands
        .iter()
        .find(|h| h.hand_ref == hand_ref)
        .unwrap_or_else(|| panic!("{name} was not dealt into hand {hand_ref}"));
    extract_preflop(hand, id)
}

fn find(events: &[StatEvent], key: StatKey) -> Option<&StatEvent> {
    events.iter().find(|e| e.key == key)
}

/// Asserts an opportunity on `key` with the given success.
fn assert_spot(events: &[StatEvent], key: StatKey, success: bool) -> &StatEvent {
    let event = find(events, key)
        .unwrap_or_else(|| panic!("expected a {} opportunity in {events:#?}", key.as_str()));
    assert!(event.opportunity);
    assert_eq!(event.success, success, "{} success", key.as_str());
    event
}

fn assert_no_spot(events: &[StatEvent], key: StatKey) {
    assert!(
        find(events, key).is_none(),
        "expected no {} opportunity, got {:#?}",
        key.as_str(),
        find(events, key)
    );
}

// Hand refs, in fixture order.
const C_UTG_OPEN_CO_3BET: &str = "300000000101";
const C_CO_STEAL_SB_3BET: &str = "300000000102";
const C_HERO_STEAL_BB_3BET_HERO_4BET: &str = "300000000103";
const C_LIMP_ISO_LIMP_RERAISE: &str = "300000000104";
const C_LIMP_ISO_LIMP_CALL: &str = "300000000105";
const C_OPEN_CALL_SQUEEZE: &str = "300000000106";
const C_OPEN_CALL_NO_SQUEEZE: &str = "300000000107";
const C_LIMP_ISO_LIMP_FOLD: &str = "300000000108";
const C_SB_OPEN_BB_CALLS: &str = "300000000109";
const C_SB_OPEN_BB_3BETS: &str = "300000000110";
const C_SB_OPEN_BB_FOLDS: &str = "300000000111";
const C_LIMP_SB_COMPLETES: &str = "300000000112";
const T_OPEN_SHOVE_12BB: &str = "300000000201";
const T_RESHOVE_20BB: &str = "300000000202";
const T_FLAT_AT_20BB: &str = "300000000203";
const T_SHOVE_CALLED: &str = "300000000204";

// ---------------------------------------------------------------- loader

#[test]
fn loader_returns_every_hand_with_amounts_positions_stacks_and_variant() {
    let conn = setup();
    let hero = player_id(&conn, "Hero");
    let hands = load_player_hands(&conn, hero).expect("load");
    assert_eq!(hands.len(), 16, "12 cash + 4 tournament hands");

    // Oldest first (by played_at, then row id).
    let refs: Vec<&str> = hands.iter().map(|h| h.hand_ref.as_str()).collect();
    assert_eq!(refs.first(), Some(&C_UTG_OPEN_CO_3BET));
    assert!(hands.iter().all(|h| h.played_at.is_some()));

    let cash = hands.iter().find(|h| h.hand_ref == C_HERO_STEAL_BB_3BET_HERO_4BET).unwrap();
    assert_eq!(cash.format, "cash");
    assert_eq!(cash.variant.as_deref(), Some("cash"));
    assert_eq!(cash.big_blind, Some(0.5));
    assert_eq!(cash.seats.len(), 6);
    let hero_seat = cash.hero().expect("hero flagged");
    assert_eq!(hero_seat.player_id, hero);
    assert_eq!(hero_seat.position.as_deref(), Some("BTN"));
    assert_eq!(hero_seat.starting_stack, Some(50.0));
    assert_eq!(cash.seats.iter().filter(|s| s.is_hero).count(), 1);
    // Raise amounts keep PokerStars' raise-to meaning.
    let hero_raises: Vec<f64> = cash
        .actions
        .iter()
        .filter(|a| a.player_id == hero && a.kind == ActionType::Raise)
        .map(|a| a.amount.unwrap())
        .collect();
    assert_eq!(hero_raises, vec![1.25, 11.0]);
    assert!(cash.actions.windows(2).all(|w| w[0].index < w[1].index));

    let tourney = hands.iter().find(|h| h.hand_ref == T_OPEN_SHOVE_12BB).unwrap();
    assert_eq!(tourney.format, "tournament");
    assert_eq!(tourney.variant.as_deref(), Some("tournament"));
    assert_eq!(tourney.big_blind, Some(200.0));
    let shove = tourney
        .actions
        .iter()
        .find(|a| a.street == Street::Preflop && a.kind == ActionType::Raise)
        .unwrap();
    assert_eq!(shove.amount, Some(2375.0));
    assert!(shove.is_all_in);
    assert!(tourney.actions.iter().any(|a| a.kind == ActionType::PostAnte));
}

#[test]
fn loader_ignores_hands_the_player_was_not_dealt_into() {
    let mut conn = setup();
    let other = "PokerStars Hand #300000000999: Hold'em No Limit ($0.25/$0.50 USD) - 2026/09/10 21:00:00 ET
Table 'Elsewhere' 2-max Seat #1 is the button
Seat 1: Stranger ($50 in chips)
Seat 2: Hero ($50 in chips)
Stranger: posts small blind $0.25
Hero: posts big blind $0.50
*** HOLE CARDS ***
Dealt to Hero [2c 2d]
Stranger: folds
Uncalled bet ($0.25) returned to Hero
Hero collected $0.50 from pot
*** SUMMARY ***
Total pot $0.50 | Rake $0
Seat 1: Stranger (button) (small blind) folded before Flop
Seat 2: Hero (big blind) collected ($0.50)
";
    import::import_text(&mut conn, other).expect("import");
    let ugo = player_id(&conn, "Ugo");
    let hands = load_player_hands(&conn, ugo).expect("load");
    assert_eq!(hands.len(), 16);
    assert!(hands.iter().all(|h| h.seat_of(ugo).is_some()));
    assert!(hands.iter().all(|h| h.hand_ref != "300000000999"));
}

#[test]
fn effective_stack_is_the_smaller_of_own_and_largest_other_stack() {
    let conn = setup();
    let ugo = player_id(&conn, "Ugo");
    let sam = player_id(&conn, "Sam");
    let cole = player_id(&conn, "Cole");
    let hands = load_player_hands(&conn, ugo).unwrap();
    let hand: &HandFacts = hands.iter().find(|h| h.hand_ref == T_OPEN_SHOVE_12BB).unwrap();
    assert_eq!(hand.effective_stack_bb(ugo), Some(12.0));
    assert_eq!(hand.effective_stack_bb(cole), Some(20.0));
    // The biggest stack is capped by the second biggest (Hero, 8000).
    assert_eq!(hand.effective_stack_bb(sam), Some(40.0));
}

#[test]
fn player_preflop_events_keep_hand_identity_in_order() {
    let conn = setup();
    let ugo = player_id(&conn, "Ugo");
    let all = engine::player_preflop_events(&conn, ugo).unwrap();
    assert_eq!(all.len(), 16);
    assert_eq!(all[0].hand_ref, C_UTG_OPEN_CO_3BET);
    assert!(all.iter().all(|h| h.events.iter().all(|e| e.player_id == ugo && e.opportunity)));
}

// ---------------------------------------------------------------- P01/P02 RFI and steal

#[test]
fn rfi_counts_raise_first_in_by_position_group() {
    let conn = setup();
    // Hit: Ugo opens UTG (early group).
    let ugo = events(&conn, "Ugo", C_UTG_OPEN_CO_3BET);
    let rfi = assert_spot(&ugo, StatKey::RfiEp, true);
    assert_eq!(rfi.counterparty.creator, None);
    assert_eq!(rfi.effective_stack_bb, Some(100.0));
    assert_no_spot(&ugo, StatKey::Steal);
    // Miss: folded to Ugo, who limps.
    assert_spot(&events(&conn, "Ugo", C_LIMP_ISO_LIMP_RERAISE), StatKey::RfiEp, false);
    // Miss in the middle group: folded to Hal (HJ), who folds.
    assert_spot(&events(&conn, "Hal", C_CO_STEAL_SB_3BET), StatKey::RfiMp, false);
    // Hit in the middle group at a tournament table, 30bb deep.
    assert_spot(&events(&conn, "Hal", T_RESHOVE_20BB), StatKey::RfiMp, true);
    // Late groups: CO, BTN, SB.
    assert_spot(&events(&conn, "Cole", C_CO_STEAL_SB_3BET), StatKey::RfiCo, true);
    assert_spot(&events(&conn, "Hero", C_HERO_STEAL_BB_3BET_HERO_4BET), StatKey::RfiBtn, true);
    assert_spot(&events(&conn, "Sam", C_SB_OPEN_BB_CALLS), StatKey::RfiSb, true);
    assert_spot(&events(&conn, "Hero", C_SB_OPEN_BB_CALLS), StatKey::RfiBtn, false);
    // Not qualifying: Hal faces Ugo's open — no RFI at all.
    let hal = events(&conn, "Hal", C_UTG_OPEN_CO_3BET);
    for key in [StatKey::RfiEp, StatKey::RfiMp, StatKey::RfiCo, StatKey::RfiBtn, StatKey::RfiSb] {
        assert_no_spot(&hal, key);
    }
    // The big blind never has an RFI spot, even in a walk-like folded pot.
    assert!(events(&conn, "Bob", C_SB_OPEN_BB_CALLS)
        .iter()
        .all(|e| !matches!(e.key, StatKey::RfiEp | StatKey::RfiMp | StatKey::RfiCo | StatKey::RfiBtn | StatKey::RfiSb)));
}

#[test]
fn steal_counts_only_folded_to_late_positions() {
    let conn = setup();
    assert_spot(&events(&conn, "Cole", C_CO_STEAL_SB_3BET), StatKey::Steal, true);
    assert_spot(&events(&conn, "Sam", C_SB_OPEN_BB_FOLDS), StatKey::Steal, true);
    // Miss: folded to the CO, who folds.
    assert_spot(&events(&conn, "Cole", C_HERO_STEAL_BB_3BET_HERO_4BET), StatKey::Steal, false);
    // Not qualifying: UTG is not a steal seat.
    assert_no_spot(&events(&conn, "Ugo", C_UTG_OPEN_CO_3BET), StatKey::Steal);
}

#[test]
fn isolation_raise_is_not_a_steal() {
    let conn = setup();
    // Ugo limps UTG, Cole raises from the CO: an isolation raise, not a
    // steal and not an RFI.
    let cole = events(&conn, "Cole", C_LIMP_ISO_LIMP_RERAISE);
    let iso = assert_spot(&cole, StatKey::IsoRaise, true);
    assert_eq!(iso.counterparty.creator, Some(player_id(&conn, "Ugo")));
    assert_no_spot(&cole, StatKey::Steal);
    assert_no_spot(&cole, StatKey::RfiCo);
    // The blinds facing that raise face no steal either.
    assert_no_spot(&events(&conn, "Sam", C_LIMP_ISO_LIMP_RERAISE), StatKey::FoldToStealSb);
    assert_no_spot(&events(&conn, "Bob", C_LIMP_ISO_LIMP_RERAISE), StatKey::FoldToStealBb);
}

// ---------------------------------------------------------------- P03/P04 blind defence

#[test]
fn fold_to_steal_is_split_per_blind() {
    let conn = setup();
    let hero = player_id(&conn, "Hero");
    // Hero steals from the BTN: Sam (SB) folds — hit; Bob (BB) 3-bets — miss.
    let sam = events(&conn, "Sam", C_HERO_STEAL_BB_3BET_HERO_4BET);
    let spot = assert_spot(&sam, StatKey::FoldToStealSb, true);
    assert_eq!(spot.counterparty.creator, Some(hero));
    assert!(spot.counterparty.hero_created);
    assert_no_spot(&sam, StatKey::FoldToStealBb);
    let bob = events(&conn, "Bob", C_HERO_STEAL_BB_3BET_HERO_4BET);
    assert_spot(&bob, StatKey::FoldToStealBb, false);
    assert_no_spot(&bob, StatKey::FoldToStealSb);
    // SB steals: the BB's answer is the BB stat.
    assert_spot(&events(&conn, "Bob", C_SB_OPEN_BB_FOLDS), StatKey::FoldToStealBb, true);
    // Not qualifying: Cole steals, Sam 3-bets, so Bob faces two raises.
    assert_no_spot(&events(&conn, "Bob", C_CO_STEAL_SB_3BET), StatKey::FoldToStealBb);
    // Not qualifying: an UTG open is not a steal.
    assert_no_spot(&events(&conn, "Sam", C_UTG_OPEN_CO_3BET), StatKey::FoldToStealSb);
}

#[test]
fn bb_defend_vs_sb_counts_only_sb_opens() {
    let conn = setup();
    assert_spot(&events(&conn, "Bob", C_SB_OPEN_BB_CALLS), StatKey::BbDefendVsSb, true);
    assert_spot(&events(&conn, "Bob", C_SB_OPEN_BB_3BETS), StatKey::BbDefendVsSb, true);
    assert_spot(&events(&conn, "Bob", C_SB_OPEN_BB_FOLDS), StatKey::BbDefendVsSb, false);
    // Not qualifying: the steal came from the button.
    assert_no_spot(&events(&conn, "Bob", C_HERO_STEAL_BB_3BET_HERO_4BET), StatKey::BbDefendVsSb);
}

// ---------------------------------------------------------------- P05/P06/P07 3-bet, 4-bet

#[test]
fn three_bet_attributes_ip_vs_oop() {
    let conn = setup();
    let ugo = player_id(&conn, "Ugo");
    let cole = player_id(&conn, "Cole");
    let sam = player_id(&conn, "Sam");
    let hero = player_id(&conn, "Hero");

    // CO 3-bets an UTG open: in position.
    let ev = events(&conn, "Cole", C_UTG_OPEN_CO_3BET);
    let spot = assert_spot(&ev, StatKey::ThreeBetIp, true);
    assert_eq!(spot.relation, Some(Relation::InPosition));
    assert_eq!(spot.counterparty.creator, Some(ugo));
    assert!(!spot.counterparty.hero_created);
    assert_no_spot(&ev, StatKey::ThreeBetOop);

    // SB 3-bets a CO open: out of position.
    let ev = events(&conn, "Sam", C_CO_STEAL_SB_3BET);
    let spot = assert_spot(&ev, StatKey::ThreeBetOop, true);
    assert_eq!(spot.relation, Some(Relation::OutOfPosition));
    assert_eq!(spot.counterparty.creator, Some(cole));
    assert_no_spot(&ev, StatKey::ThreeBetIp);

    // BB 3-bets the hero's BTN open: out of position, and the hero created it.
    let spot =
        assert_spot(&events(&conn, "Bob", C_HERO_STEAL_BB_3BET_HERO_4BET), StatKey::ThreeBetOop, true)
            .clone();
    assert_eq!(spot.counterparty.creator, Some(hero));
    assert!(spot.counterparty.hero_created);
    assert_eq!(spot.counterparty.hero_position.as_deref(), Some("BTN"));

    // BB 3-bets an SB open: the BB acts after the SB postflop — in position.
    let spot = assert_spot(&events(&conn, "Bob", C_SB_OPEN_BB_3BETS), StatKey::ThreeBetIp, true).clone();
    assert_eq!(spot.counterparty.creator, Some(sam));

    // Misses: the HJ and the BTN fold or call behind an open.
    assert_spot(&events(&conn, "Hal", C_UTG_OPEN_CO_3BET), StatKey::ThreeBetIp, false);
    assert_spot(&events(&conn, "Hero", C_CO_STEAL_SB_3BET), StatKey::ThreeBetIp, false);
    // Not qualifying: the button faces an open and a 3-bet.
    let ev = events(&conn, "Hero", C_UTG_OPEN_CO_3BET);
    assert_no_spot(&ev, StatKey::ThreeBetIp);
    assert_no_spot(&ev, StatKey::ThreeBetOop);
    // Not qualifying: a limper re-raising is a limp-reraise, not a 3-bet.
    let ev = events(&conn, "Ugo", C_LIMP_ISO_LIMP_RERAISE);
    assert_no_spot(&ev, StatKey::ThreeBetIp);
    assert_no_spot(&ev, StatKey::ThreeBetOop);
}

#[test]
fn fold_to_three_bet_attributes_ip_vs_oop() {
    let conn = setup();
    // UTG opener folds to a CO 3-bet: out of position.
    let ev = events(&conn, "Ugo", C_UTG_OPEN_CO_3BET);
    let spot = assert_spot(&ev, StatKey::FoldTo3betOop, true);
    assert_eq!(spot.counterparty.creator, Some(player_id(&conn, "Cole")));
    assert_no_spot(&ev, StatKey::FoldTo3betIp);
    // CO opener calls an SB 3-bet: in position, miss.
    let ev = events(&conn, "Cole", C_CO_STEAL_SB_3BET);
    assert_spot(&ev, StatKey::FoldTo3betIp, false);
    assert_no_spot(&ev, StatKey::FoldTo3betOop);
    // SB opener folds to the BB's 3-bet: out of position.
    assert_spot(&events(&conn, "Sam", C_SB_OPEN_BB_3BETS), StatKey::FoldTo3betOop, true);
    // Hero 4-bets the BB's 3-bet: in position, miss.
    assert_spot(&events(&conn, "Hero", C_HERO_STEAL_BB_3BET_HERO_4BET), StatKey::FoldTo3betIp, false);
    // Not qualifying: an opener nobody re-raised.
    let ev = events(&conn, "Sam", C_SB_OPEN_BB_CALLS);
    assert_no_spot(&ev, StatKey::FoldTo3betIp);
    assert_no_spot(&ev, StatKey::FoldTo3betOop);
}

#[test]
fn four_bet_and_fold_to_four_bet_count_correctly() {
    let conn = setup();
    let hero = player_id(&conn, "Hero");
    // Hit: Hero 4-bets.
    assert_spot(&events(&conn, "Hero", C_HERO_STEAL_BB_3BET_HERO_4BET), StatKey::FourBet, true);
    // Miss: Ugo folds to the 3-bet, Cole calls it.
    assert_spot(&events(&conn, "Ugo", C_UTG_OPEN_CO_3BET), StatKey::FourBet, false);
    assert_spot(&events(&conn, "Cole", C_CO_STEAL_SB_3BET), StatKey::FourBet, false);
    // Fold to 4-bet: Bob 3-bet and folds to Hero's 4-bet.
    let bob = events(&conn, "Bob", C_HERO_STEAL_BB_3BET_HERO_4BET);
    let spot = assert_spot(&bob, StatKey::FoldTo4bet, true);
    assert_eq!(spot.counterparty.creator, Some(hero));
    assert!(spot.counterparty.hero_created);
    // Not qualifying: a 3-bettor nobody 4-bet.
    assert_no_spot(&events(&conn, "Cole", C_UTG_OPEN_CO_3BET), StatKey::FoldTo4bet);
    // Not qualifying: an all-in 3-bet leaves no 4-bet to make.
    assert_no_spot(&events(&conn, "Hal", T_RESHOVE_20BB), StatKey::FourBet);
}

// ---------------------------------------------------------------- P08/P11 squeeze, cold call

#[test]
fn squeeze_requires_open_and_caller() {
    let conn = setup();
    let cole = events(&conn, "Cole", C_OPEN_CALL_SQUEEZE);
    let spot = assert_spot(&cole, StatKey::Squeeze, true);
    assert_eq!(spot.counterparty.creator, Some(player_id(&conn, "Ugo")));
    assert_spot(&cole, StatKey::ThreeBetIp, true);
    // Misses: Cole folds and Hero flats behind an open and a caller.
    assert_spot(&events(&conn, "Cole", C_OPEN_CALL_NO_SQUEEZE), StatKey::Squeeze, false);
    assert_spot(&events(&conn, "Hero", C_OPEN_CALL_NO_SQUEEZE), StatKey::Squeeze, false);
    // Not qualifying: the caller himself faced the open with no caller yet.
    assert_no_spot(&events(&conn, "Hal", C_OPEN_CALL_SQUEEZE), StatKey::Squeeze);
    // Not qualifying: a limper before the raise is not a caller of it.
    assert_no_spot(&events(&conn, "Hero", C_LIMP_ISO_LIMP_FOLD), StatKey::Squeeze);
}

#[test]
fn cold_call_excludes_blind_completion_and_limpers() {
    let conn = setup();
    // Hits: Hal flats Ugo's open; Hero flats behind.
    assert_spot(&events(&conn, "Hal", C_OPEN_CALL_SQUEEZE), StatKey::ColdCall, true);
    assert_spot(&events(&conn, "Hero", C_OPEN_CALL_NO_SQUEEZE), StatKey::ColdCall, true);
    // Miss: Cole 3-bets instead.
    assert_spot(&events(&conn, "Cole", C_UTG_OPEN_CO_3BET), StatKey::ColdCall, false);
    // Not qualifying: a limper calling the isolation raise is a limp-call.
    let ugo = events(&conn, "Ugo", C_LIMP_ISO_LIMP_CALL);
    assert_no_spot(&ugo, StatKey::ColdCall);
    assert_spot(&ugo, StatKey::LimpCall, true);
    // Not qualifying: an opener calling a 3-bet already had money in.
    assert_no_spot(&events(&conn, "Cole", C_CO_STEAL_SB_3BET), StatKey::ColdCall);
    // Not qualifying: the SB completing a limped pot faces no raise; it is a
    // missed isolation, not a cold call and not an open-limp.
    let sam = events(&conn, "Sam", C_LIMP_SB_COMPLETES);
    assert_no_spot(&sam, StatKey::ColdCall);
    assert_no_spot(&sam, StatKey::Limp);
    assert_spot(&sam, StatKey::IsoRaise, false);
    assert_spot(&sam, StatKey::Vpip, true);
}

// ---------------------------------------------------------------- P09/P10 limp, iso, limp follow-ups

#[test]
fn limp_and_iso_raise_count_correctly() {
    let conn = setup();
    // Limp: hit when folded to Ugo who calls; miss when he raises first in.
    let limp = events(&conn, "Ugo", C_LIMP_ISO_LIMP_RERAISE);
    assert_spot(&limp, StatKey::Limp, true);
    assert_spot(&events(&conn, "Ugo", C_UTG_OPEN_CO_3BET), StatKey::Limp, false);
    // Not qualifying: the BB never has an open-limp spot.
    assert_no_spot(&events(&conn, "Bob", C_SB_OPEN_BB_CALLS), StatKey::Limp);
    // Iso: hits for Cole and Hero, misses for those who fold behind a limp.
    assert_spot(&events(&conn, "Hero", C_LIMP_ISO_LIMP_CALL), StatKey::IsoRaise, true);
    assert_spot(&events(&conn, "Hal", C_LIMP_ISO_LIMP_FOLD), StatKey::IsoRaise, true);
    assert_spot(&events(&conn, "Hal", C_LIMP_ISO_LIMP_RERAISE), StatKey::IsoRaise, false);
    assert_spot(&events(&conn, "Cole", C_LIMP_ISO_LIMP_CALL), StatKey::IsoRaise, false);
    // Not qualifying: the limper himself, and players facing the iso-raise.
    assert_no_spot(&limp, StatKey::IsoRaise);
    assert_no_spot(&events(&conn, "Hero", C_LIMP_ISO_LIMP_RERAISE), StatKey::IsoRaise);
}

#[test]
fn limp_fold_call_reraise_split_correctly() {
    let conn = setup();
    let cases = [
        (C_LIMP_ISO_LIMP_FOLD, (true, false, false)),
        (C_LIMP_ISO_LIMP_CALL, (false, true, false)),
        (C_LIMP_ISO_LIMP_RERAISE, (false, false, true)),
    ];
    for (hand, (fold, call, reraise)) in cases {
        let ugo = events(&conn, "Ugo", hand);
        assert_spot(&ugo, StatKey::LimpFold, fold);
        assert_spot(&ugo, StatKey::LimpCall, call);
        assert_spot(&ugo, StatKey::LimpReraise, reraise);
    }
    let raiser = assert_spot(&events(&conn, "Ugo", C_LIMP_ISO_LIMP_CALL), StatKey::LimpCall, true)
        .counterparty
        .clone();
    assert_eq!(raiser.creator, Some(player_id(&conn, "Hero")));
    assert!(raiser.hero_created);
    // Not qualifying: a raise-first-in opener never limped.
    let ev = events(&conn, "Ugo", C_UTG_OPEN_CO_3BET);
    for key in [StatKey::LimpFold, StatKey::LimpCall, StatKey::LimpReraise] {
        assert_no_spot(&ev, key);
    }
}

// ---------------------------------------------------------------- P12 VPIP / PFR

#[test]
fn vpip_and_pfr_count_voluntary_money_and_raises() {
    let conn = setup();
    let hero = events(&conn, "Hero", C_HERO_STEAL_BB_3BET_HERO_4BET);
    assert_spot(&hero, StatKey::Vpip, true);
    assert_spot(&hero, StatKey::Pfr, true);
    let hal = events(&conn, "Hal", C_OPEN_CALL_SQUEEZE);
    assert_spot(&hal, StatKey::Vpip, true);
    assert_spot(&hal, StatKey::Pfr, false);
    let folded = events(&conn, "Hero", C_UTG_OPEN_CO_3BET);
    assert_spot(&folded, StatKey::Vpip, false);
    assert_spot(&folded, StatKey::Pfr, false);
    // Not qualifying: a 12bb stack is push/fold, not a deep VPIP sample.
    assert_no_spot(&events(&conn, "Ugo", T_OPEN_SHOVE_12BB), StatKey::Vpip);
}

// ---------------------------------------------------------------- S01/S02/S03 stack depth

#[test]
fn open_shove_at_12bb_counts_as_push_fold_not_rfi() {
    let conn = setup();
    let ugo = events(&conn, "Ugo", T_OPEN_SHOVE_12BB);
    let shove = assert_spot(&ugo, StatKey::OpenShove, true);
    assert_eq!(shove.effective_stack_bb, Some(12.0));
    for key in [StatKey::RfiEp, StatKey::Steal, StatKey::Limp, StatKey::Pfr] {
        assert_no_spot(&ugo, key);
    }
    // Miss: folded to Ugo at 12bb, who folds.
    assert_spot(&events(&conn, "Ugo", T_RESHOVE_20BB), StatKey::OpenShove, false);
    // Not qualifying: a deep stack folded to is an RFI spot, never a shove one.
    let hal = events(&conn, "Hal", T_RESHOVE_20BB);
    assert_no_spot(&hal, StatKey::OpenShove);
    assert_spot(&hal, StatKey::RfiMp, true);
}

#[test]
fn call_vs_shove_counts_only_all_in_raises() {
    let conn = setup();
    let ugo = player_id(&conn, "Ugo");
    // Hit: Hero calls Ugo's shove.
    let spot = assert_spot(&events(&conn, "Hero", T_SHOVE_CALLED), StatKey::CallVsShove, true).clone();
    assert_eq!(spot.counterparty.creator, Some(ugo));
    // Misses: everyone who folded to the shove first.
    assert_spot(&events(&conn, "Hal", T_OPEN_SHOVE_12BB), StatKey::CallVsShove, false);
    assert_spot(&events(&conn, "Bob", T_OPEN_SHOVE_12BB), StatKey::CallVsShove, false);
    // A shove facing an all-in 3-bet answers it too.
    assert_spot(&events(&conn, "Hal", T_RESHOVE_20BB), StatKey::CallVsShove, false);
    // Not qualifying: once someone called, the players behind face a call.
    assert_no_spot(&events(&conn, "Sam", T_SHOVE_CALLED), StatKey::CallVsShove);
    // Not qualifying: a normal open is not a shove.
    assert_no_spot(&events(&conn, "Cole", T_FLAT_AT_20BB), StatKey::CallVsShove);
    // A shove is not an open: no 3-bet or cold-call spot behind it.
    let hal = events(&conn, "Hal", T_OPEN_SHOVE_12BB);
    for key in [StatKey::ThreeBetIp, StatKey::ThreeBetOop, StatKey::ColdCall] {
        assert_no_spot(&hal, key);
    }
}

#[test]
fn reshove_at_20bb_over_open_counts() {
    let conn = setup();
    let hal = player_id(&conn, "Hal");
    let cole = events(&conn, "Cole", T_RESHOVE_20BB);
    let spot = assert_spot(&cole, StatKey::Reshove, true);
    assert_eq!(spot.effective_stack_bb, Some(20.0));
    assert_eq!(spot.counterparty.creator, Some(hal));
    // Above 15bb the same jam is also an in-position 3-bet.
    assert_spot(&cole, StatKey::ThreeBetIp, true);
    // Miss: Cole flats the same open at 20bb.
    assert_spot(&events(&conn, "Cole", T_FLAT_AT_20BB), StatKey::Reshove, false);
    // Not qualifying: Hero is 40bb deep; Ugo (12bb) never faced an open.
    assert_no_spot(&events(&conn, "Hero", T_FLAT_AT_20BB), StatKey::Reshove);
    assert_no_spot(&events(&conn, "Ugo", T_FLAT_AT_20BB), StatKey::Reshove);
    // Not qualifying: the 100bb cash table.
    assert_no_spot(&events(&conn, "Cole", C_UTG_OPEN_CO_3BET), StatKey::Reshove);
}

// ---------------------------------------------------------------- catalogue coverage

#[test]
fn every_preflop_stat_key_is_extracted_from_the_fixtures() {
    let conn = setup();
    let mut seen = std::collections::HashSet::new();
    for name in ["Hero", "Sam", "Bob", "Ugo", "Hal", "Cole"] {
        for hand in engine::player_preflop_events(&conn, player_id(&conn, name)).unwrap() {
            seen.extend(hand.events.iter().map(|e| e.key));
        }
    }
    for key in StatKey::PREFLOP {
        assert!(seen.contains(&key), "no fixture produced {}", key.as_str());
    }
    let mut keys: Vec<&str> = StatKey::PREFLOP.iter().map(|k| k.as_str()).collect();
    keys.sort();
    keys.dedup();
    assert_eq!(keys.len(), StatKey::PREFLOP.len(), "stat keys are unique");
}
