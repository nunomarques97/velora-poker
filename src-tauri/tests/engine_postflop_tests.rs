//! Opponent engine, task T4: pot reconstruction, bet sizing and postflop
//! scenario stats (catalogue rows F01–F13 of `docs/specs/opponent-engine.md`).
//!
//! Fixture: `engine_postflop_cash.txt` is a 6-max $0.25/$0.50 cash table with
//! 100bb stacks, seating Hero (seat 1, button), Sam (SB), Bob (BB), Ugo
//! (UTG), Hal (HJ) and Cole (CO). Pot reconstruction is also checked against
//! the tournament (antes), Spin & Go, Zoom and real hand-history fixtures.

use std::collections::HashSet;

use velora_poker_lib::engine::{
    extract_postflop, load_player_hands, parse_total_pot, replay_pot, HandFacts, PotReplay,
    Relation, SizeBucket, SizedAction, StatEvent, StatKey,
};
use velora_poker_lib::parser::{ActionType, Street};
use velora_poker_lib::{db, import};

const CASH: &str = include_str!("fixtures/engine_postflop_cash.txt");

fn import_db(text: &str) -> rusqlite::Connection {
    let mut conn = db::open(std::path::Path::new(":memory:")).expect("open db");
    import::import_text(&mut conn, text).expect("import fixture");
    conn
}

fn setup() -> rusqlite::Connection {
    import_db(CASH)
}

fn player_id(conn: &rusqlite::Connection, name: &str) -> i64 {
    conn.query_row("SELECT id FROM players WHERE name = ?1", [name], |row| row.get(0))
        .unwrap_or_else(|_| panic!("player {name} not found"))
}

fn hand(conn: &rusqlite::Connection, name: &str, hand_ref: &str) -> HandFacts {
    load_player_hands(conn, player_id(conn, name))
        .expect("load hands")
        .into_iter()
        .find(|h| h.hand_ref == hand_ref)
        .unwrap_or_else(|| panic!("{name} was not dealt into hand {hand_ref}"))
}

/// The postflop events `name` produced in hand `hand_ref`.
fn events(conn: &rusqlite::Connection, name: &str, hand_ref: &str) -> Vec<StatEvent> {
    extract_postflop(&hand(conn, name, hand_ref), player_id(conn, name))
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

fn assert_close(actual: f64, expected: f64, what: &str) {
    assert!((actual - expected).abs() < 1e-3, "{what}: expected {expected}, got {actual}");
}

// Hand refs, in fixture order.
const HU_CBET_FOLD: &str = "300000000301";
const MW_CBET_HERO_FLOATS: &str = "300000000302";
const HERO_TRIPLE_BARREL: &str = "300000000303";
const DELAYED_CBET_SHOWDOWN: &str = "300000000304";
const PROBE_AFTER_CHECKBACK: &str = "300000000305";
const DONK_THEN_RAISED: &str = "300000000306";
const FLOP_CHECK_RAISE: &str = "300000000307";
const PREFLOP_ALL_IN: &str = "300000000308";
const RIVER_OVERBET_RAISED: &str = "300000000309";
const LIMPED_MULTIWAY: &str = "300000000310";
const FLOP_ALL_IN_VS_HERO: &str = "300000000311";
const FLOAT_CHECKED_BEHIND: &str = "300000000312";
const HERO_CHECKS_TWICE: &str = "300000000313";
const RIVER_BARREL_GIVEN_UP: &str = "300000000314";
const FOLD_TO_TURN_BARREL: &str = "300000000315";
const RIVER_BARREL_CALLED: &str = "300000000316";

// ---- F13: pot reconstruction and sizing

/// Every distinct hand in a database, with its raw text.
fn all_hands(conn: &rusqlite::Connection) -> Vec<(HandFacts, String)> {
    let players: Vec<i64> = conn
        .prepare("SELECT id FROM players ORDER BY id")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .map(|r| r.unwrap())
        .collect();
    let mut seen = HashSet::new();
    let mut hands = Vec::new();
    for player in players {
        for hand in load_player_hands(conn, player).unwrap() {
            if seen.insert(hand.id) {
                let raw: String = conn
                    .query_row("SELECT raw_text FROM hands WHERE id = ?1", [hand.id], |row| {
                        row.get(0)
                    })
                    .unwrap();
                hands.push((hand, raw));
            }
        }
    }
    hands
}

/// The amounts on a hand history's `Uncalled bet (x) returned to P` lines.
fn uncalled_lines(raw: &str) -> Vec<f64> {
    let mut amounts: Vec<f64> = raw
        .lines()
        .filter_map(|l| l.trim().strip_prefix("Uncalled bet ("))
        .map(|rest| {
            let token = rest.split(')').next().unwrap();
            token.trim_start_matches(['$', '€', '£']).replace(',', "").parse().unwrap()
        })
        .collect();
    amounts.sort_by(f64::total_cmp);
    amounts
}

#[test]
fn pot_reconstruction_matches_total_pot_line() {
    let fixtures = [
        ("engine_postflop_cash", CASH),
        ("engine_preflop_cash", include_str!("fixtures/engine_preflop_cash.txt")),
        ("engine_preflop_tournament", include_str!("fixtures/engine_preflop_tournament.txt")),
        ("real_bounty_tournament", include_str!("fixtures/real_bounty_tournament.txt")),
        ("real_positions_by_table_size", include_str!("fixtures/real_positions_by_table_size.txt")),
        ("spin_three_max", include_str!("fixtures/spin_three_max.txt")),
        ("zoom_cash", include_str!("fixtures/zoom_cash.txt")),
        ("tournament_showdown_allin", include_str!("fixtures/tournament_showdown_allin.txt")),
        ("showdown_shows_and_mucks", include_str!("fixtures/showdown_shows_and_mucks.txt")),
    ];
    let (mut checked, mut with_antes, mut with_uncalled, mut with_postflop_raise) = (0, 0, 0, 0);
    for (name, text) in fixtures {
        let conn = import_db(text);
        let hands = all_hands(&conn);
        assert!(!hands.is_empty(), "{name}: no hands imported");
        for (hand, raw) in hands {
            let replay = replay_pot(&hand);
            let total = parse_total_pot(&raw)
                .unwrap_or_else(|| panic!("{name} {}: no Total pot line", hand.hand_ref));
            assert_close(replay.total, total, &format!("{name} {} final pot", hand.hand_ref));

            let mut derived: Vec<f64> = replay.uncalled.iter().map(|u| u.amount).collect();
            derived.sort_by(f64::total_cmp);
            let logged = uncalled_lines(&raw);
            assert_eq!(derived.len(), logged.len(), "{name} {} uncalled returns", hand.hand_ref);
            for (d, l) in derived.iter().zip(&logged) {
                assert_close(*d, *l, &format!("{name} {} uncalled bet", hand.hand_ref));
            }

            checked += 1;
            with_antes += hand.actions.iter().any(|a| a.kind == ActionType::PostAnte) as usize;
            with_uncalled += !logged.is_empty() as usize;
            with_postflop_raise += hand
                .actions
                .iter()
                .any(|a| a.street != Street::Preflop && a.kind == ActionType::Raise)
                as usize;
        }
    }
    assert!(checked >= 40, "only {checked} hands checked");
    assert!(with_antes >= 3, "only {with_antes} hands with antes");
    assert!(with_uncalled >= 15, "only {with_uncalled} hands with an uncalled bet");
    assert!(with_postflop_raise >= 4, "only {with_postflop_raise} hands with a postflop raise");
}

fn cash_replay(conn: &rusqlite::Connection, hand_ref: &str) -> PotReplay {
    replay_pot(&hand(conn, "Hero", hand_ref))
}

/// The sized action `name` took as their `nth` (0-based) `kind` on `street`.
fn sized<'a>(
    conn: &rusqlite::Connection,
    replay: &'a PotReplay,
    name: &str,
    street: Street,
    kind: ActionType,
) -> &'a SizedAction {
    let id = player_id(conn, name);
    replay
        .actions
        .iter()
        .find(|a| a.player_id == id && a.street == street && a.kind == kind)
        .unwrap_or_else(|| panic!("no {kind:?} by {name} on {street:?}"))
}

#[test]
fn pot_is_tracked_street_by_street_with_uncalled_returns() {
    let conn = setup();
    let replay = cash_replay(&conn, HERO_TRIPLE_BARREL);
    assert_close(replay.pot_at_start(Street::Flop).unwrap(), 2.75, "flop pot");
    assert_close(replay.pot_at_start(Street::Turn).unwrap(), 5.75, "turn pot");
    assert_close(replay.pot_at_start(Street::River).unwrap(), 15.75, "river pot");
    assert_eq!(replay.uncalled.len(), 1);
    assert_eq!(replay.uncalled[0].street, Street::River);
    assert_eq!(replay.uncalled[0].player_id, player_id(&conn, "Hero"));
    assert_close(replay.uncalled[0].amount, 18.0, "uncalled river bet");
    assert_close(replay.contributed[&player_id(&conn, "Hero")], 7.75, "Hero net put in");
    assert_close(replay.total, 15.75, "final pot");

    // A raise adds only what the raiser had not put in yet on the street.
    let replay = cash_replay(&conn, DONK_THEN_RAISED);
    let raise = sized(&conn, &replay, "Ugo", Street::Flop, ActionType::Raise);
    assert_close(raise.put_in, 6.0, "raise put in");
    assert_close(raise.pot_before, 5.25, "pot before the raise");
    assert_close(raise.to_call, 2.0, "to call before the raise");
    assert_close(replay.uncalled[0].amount, 4.0, "unmatched part of the raise");

    // A preflop all-in that is called in full returns nothing.
    let replay = cash_replay(&conn, PREFLOP_ALL_IN);
    assert!(replay.uncalled.is_empty());
    assert!(replay.pot_at_start(Street::Flop).is_none(), "no flop action, no flop entry");
    assert_close(replay.total, 100.75, "all-in pot");
}

#[test]
fn sizing_buckets_follow_pot_fraction() {
    // Section 9 boundaries; an all-in overrides the fraction.
    assert_eq!(SizeBucket::classify(0.0, false), SizeBucket::Small);
    assert_eq!(SizeBucket::classify(0.3999, false), SizeBucket::Small);
    assert_eq!(SizeBucket::classify(0.40, false), SizeBucket::Medium);
    assert_eq!(SizeBucket::classify(0.7499, false), SizeBucket::Medium);
    assert_eq!(SizeBucket::classify(0.75, false), SizeBucket::Large);
    assert_eq!(SizeBucket::classify(1.0999, false), SizeBucket::Large);
    assert_eq!(SizeBucket::classify(1.10, false), SizeBucket::Overbet);
    assert_eq!(SizeBucket::classify(0.2, true), SizeBucket::AllIn);
    assert_eq!(SizeBucket::AllIn.as_str(), "allin");

    let conn = setup();
    let check = |hand_ref: &str, name: &str, street: Street, kind: ActionType, f: f64, b: SizeBucket| {
        let replay = cash_replay(&conn, hand_ref);
        let action = sized(&conn, &replay, name, street, kind);
        assert_close(action.fraction.unwrap(), f, &format!("{hand_ref} {name} fraction"));
        assert_eq!(action.bucket, Some(b), "{hand_ref} {name} bucket");
    };
    // Bets: amount / pot before the bet.
    check(RIVER_OVERBET_RAISED, "Hal", Street::Flop, ActionType::Bet, 1.0 / 3.25, SizeBucket::Small);
    check(HU_CBET_FOLD, "Ugo", Street::Flop, ActionType::Bet, 2.0 / 3.25, SizeBucket::Medium);
    check(HERO_TRIPLE_BARREL, "Hero", Street::Flop, ActionType::Bet, 1.5 / 2.75, SizeBucket::Medium);
    check(HERO_TRIPLE_BARREL, "Hero", Street::Turn, ActionType::Bet, 5.0 / 5.75, SizeBucket::Large);
    check(HERO_TRIPLE_BARREL, "Hero", Street::River, ActionType::Bet, 18.0 / 15.75, SizeBucket::Overbet);
    check(RIVER_OVERBET_RAISED, "Bob", Street::River, ActionType::Bet, 6.0 / 5.25, SizeBucket::Overbet);
    // Raises: (raise-to − facing bet) / (pot before + amount to call).
    check(DONK_THEN_RAISED, "Ugo", Street::Flop, ActionType::Raise, 4.0 / 7.25, SizeBucket::Medium);
    check(RIVER_OVERBET_RAISED, "Hal", Street::River, ActionType::Raise, 12.0 / 17.25, SizeBucket::Medium);
    check(FLOP_ALL_IN_VS_HERO, "Hero", Street::Flop, ActionType::Raise, 7.0 / 8.75, SizeBucket::Large);
    // An all-in re-raise is `allin` whatever its fraction.
    check(FLOP_ALL_IN_VS_HERO, "Ugo", Street::Flop, ActionType::Raise, 39.0 / 22.75, SizeBucket::AllIn);

    // Calls, checks, folds and posts carry no size.
    let replay = cash_replay(&conn, HERO_TRIPLE_BARREL);
    let call = sized(&conn, &replay, "Bob", Street::Flop, ActionType::Call);
    assert_eq!((call.fraction, call.bucket), (None, None));
}

// ---- F01–F04: c-bets and barrels

#[test]
fn cbet_flop_distinguishes_heads_up_from_multiway() {
    let conn = setup();
    let hu = events(&conn, "Ugo", HU_CBET_FOLD);
    let cbet = assert_spot(&hu, StatKey::CbetFlop, true);
    assert_eq!(cbet.multiway, Some(false));
    assert_eq!(cbet.relation, Some(Relation::InPosition), "UTG acts after the BB");
    assert_eq!(cbet.counterparty.creator, None);

    let mw = events(&conn, "Hal", MW_CBET_HERO_FLOATS);
    let cbet = assert_spot(&mw, StatKey::CbetFlop, true);
    assert_eq!(cbet.multiway, Some(true), "three players saw the flop");
    assert_eq!(cbet.relation, Some(Relation::OutOfPosition), "CO and BTN act after the HJ");

    // Miss: the raiser checked the flop.
    assert_spot(&events(&conn, "Cole", DELAYED_CBET_SHOWDOWN), StatKey::CbetFlop, false);
    // Non-qualifying: donked into, a caller, a limped pot.
    assert_no_spot(&events(&conn, "Ugo", DONK_THEN_RAISED), StatKey::CbetFlop);
    assert_no_spot(&events(&conn, "Bob", HU_CBET_FOLD), StatKey::CbetFlop);
    assert_no_spot(&events(&conn, "Bob", LIMPED_MULTIWAY), StatKey::CbetFlop);
}

#[test]
fn cbet_turn_requires_flop_cbet() {
    let conn = setup();
    let barrel = events(&conn, "Hal", FOLD_TO_TURN_BARREL);
    assert_spot(&barrel, StatKey::CbetTurn, true);
    assert_eq!(find(&barrel, StatKey::CbetTurn).unwrap().multiway, Some(false));
    assert_spot(&events(&conn, "Hal", MW_CBET_HERO_FLOATS), StatKey::CbetTurn, false);
    assert_spot(&events(&conn, "Hal", RIVER_OVERBET_RAISED), StatKey::CbetTurn, false);
    // The flop was checked: a turn bet is a delayed c-bet, not a barrel.
    assert_no_spot(&events(&conn, "Cole", DELAYED_CBET_SHOWDOWN), StatKey::CbetTurn);
    // The c-bettor was check-raised and gave up: no turn at all.
    assert_no_spot(&events(&conn, "Cole", FLOP_CHECK_RAISE), StatKey::CbetTurn);
}

#[test]
fn cbet_river_requires_turn_barrel() {
    let conn = setup();
    assert_spot(&events(&conn, "Hero", HERO_TRIPLE_BARREL), StatKey::CbetRiver, true);
    assert_spot(&events(&conn, "Cole", RIVER_BARREL_GIVEN_UP), StatKey::CbetRiver, false);
    // Checked the turn: the river bet chain is broken.
    assert_no_spot(&events(&conn, "Hal", RIVER_OVERBET_RAISED), StatKey::CbetRiver);
    assert_no_spot(&events(&conn, "Ugo", FLOAT_CHECKED_BEHIND), StatKey::CbetRiver);
}

// ---- F02, F05: folding to c-bets and barrels

#[test]
fn fold_to_cbet_flop_counts_only_raiser_bets() {
    let conn = setup();
    let fold = events(&conn, "Bob", HU_CBET_FOLD);
    let spot = assert_spot(&fold, StatKey::FoldToCbetFlop, true);
    assert_eq!(spot.counterparty.creator, Some(player_id(&conn, "Ugo")));
    assert_eq!(spot.relation, Some(Relation::OutOfPosition));
    assert_eq!(spot.multiway, Some(false));

    let cole = events(&conn, "Cole", MW_CBET_HERO_FLOATS);
    assert_eq!(assert_spot(&cole, StatKey::FoldToCbetFlop, true).multiway, Some(true));
    let hero = events(&conn, "Hero", MW_CBET_HERO_FLOATS);
    let spot = assert_spot(&hero, StatKey::FoldToCbetFlop, false);
    assert_eq!(spot.relation, Some(Relation::InPosition));
    // Raising the c-bet is not folding to it.
    assert_spot(&events(&conn, "Bob", FLOP_CHECK_RAISE), StatKey::FoldToCbetFlop, false);
    // A bet from a player who did not raise preflop is not a c-bet.
    assert_no_spot(&events(&conn, "Hal", LIMPED_MULTIWAY), StatKey::FoldToCbetFlop);
    assert_no_spot(&events(&conn, "Hal", PROBE_AFTER_CHECKBACK), StatKey::FoldToCbetFlop);
}

#[test]
fn fold_to_barrels_count_per_street() {
    let conn = setup();
    let bob = events(&conn, "Bob", HERO_TRIPLE_BARREL);
    assert_spot(&bob, StatKey::FoldToCbetFlop, false);
    assert_spot(&bob, StatKey::FoldToCbetTurn, false);
    assert_spot(&bob, StatKey::FoldToCbetRiver, true);

    let bob = events(&conn, "Bob", RIVER_BARREL_CALLED);
    assert_spot(&bob, StatKey::FoldToCbetTurn, false);
    assert_spot(&bob, StatKey::FoldToCbetRiver, false);
    assert_spot(&events(&conn, "Hal", RIVER_BARREL_CALLED), StatKey::CbetRiver, true);

    let bob = events(&conn, "Bob", FOLD_TO_TURN_BARREL);
    assert_spot(&bob, StatKey::FoldToCbetTurn, true);
    assert_no_spot(&bob, StatKey::FoldToCbetRiver);

    let bob = events(&conn, "Bob", RIVER_BARREL_GIVEN_UP);
    assert_spot(&bob, StatKey::FoldToCbetTurn, false);
    assert_no_spot(&bob, StatKey::FoldToCbetRiver);
    // The raiser checked the turn: no barrel to face.
    assert_no_spot(&events(&conn, "Hero", MW_CBET_HERO_FLOATS), StatKey::FoldToCbetTurn);
    // A delayed c-bet is not a turn barrel.
    assert_no_spot(&events(&conn, "Bob", DELAYED_CBET_SHOWDOWN), StatKey::FoldToCbetTurn);
}

// ---- F06–F10: delayed c-bet, check-raise, donk, float, probe

#[test]
fn delayed_cbet_requires_checked_flop() {
    let conn = setup();
    assert_spot(&events(&conn, "Cole", DELAYED_CBET_SHOWDOWN), StatKey::DelayedCbet, true);
    let hero = events(&conn, "Hero", HERO_CHECKS_TWICE);
    let spot = assert_spot(&hero, StatKey::DelayedCbet, false);
    assert_eq!(spot.relation, Some(Relation::InPosition));
    // The caller bet the turn first.
    assert_no_spot(&events(&conn, "Hal", PROBE_AFTER_CHECKBACK), StatKey::DelayedCbet);
    // The flop was c-bet.
    assert_no_spot(&events(&conn, "Hal", FOLD_TO_TURN_BARREL), StatKey::DelayedCbet);
}

#[test]
fn check_raise_requires_prior_check() {
    let conn = setup();
    let bob = events(&conn, "Bob", FLOP_CHECK_RAISE);
    let spot = assert_spot(&bob, StatKey::CheckRaiseFlop, true);
    assert_eq!(spot.counterparty.creator, Some(player_id(&conn, "Cole")));
    assert_spot(&events(&conn, "Bob", HU_CBET_FOLD), StatKey::CheckRaiseFlop, false);
    let sam = events(&conn, "Sam", LIMPED_MULTIWAY);
    let spot = assert_spot(&sam, StatKey::CheckRaiseFlop, false);
    assert_eq!(spot.counterparty.creator, Some(player_id(&conn, "Bob")));
    assert_eq!(spot.multiway, Some(true));
    // Led out instead of checking; facing the bet without checking first.
    assert_no_spot(&events(&conn, "Bob", DONK_THEN_RAISED), StatKey::CheckRaiseFlop);
    assert_no_spot(&events(&conn, "Cole", MW_CBET_HERO_FLOATS), StatKey::CheckRaiseFlop);
}

#[test]
fn donk_requires_oop_before_aggressor() {
    let conn = setup();
    let bob = events(&conn, "Bob", DONK_THEN_RAISED);
    let spot = assert_spot(&bob, StatKey::DonkFlop, true);
    assert_eq!(spot.counterparty.creator, Some(player_id(&conn, "Ugo")));
    assert_eq!(spot.relation, Some(Relation::OutOfPosition));
    assert_spot(&events(&conn, "Bob", HU_CBET_FOLD), StatKey::DonkFlop, false);
    // In position to the raiser, or no preflop raiser at all.
    assert_no_spot(&events(&conn, "Cole", MW_CBET_HERO_FLOATS), StatKey::DonkFlop);
    assert_no_spot(&events(&conn, "Bob", LIMPED_MULTIWAY), StatKey::DonkFlop);
}

#[test]
fn float_requires_ip_flop_call_and_turn_check() {
    let conn = setup();
    let hero = events(&conn, "Hero", MW_CBET_HERO_FLOATS);
    let spot = assert_spot(&hero, StatKey::FloatFlop, true);
    assert_eq!(spot.counterparty.creator, Some(player_id(&conn, "Hal")));
    assert_eq!(spot.relation, Some(Relation::InPosition));
    assert_spot(&events(&conn, "Cole", FLOAT_CHECKED_BEHIND), StatKey::FloatFlop, false);
    // Called out of position, or the raiser barrelled the turn.
    assert_no_spot(&events(&conn, "Bob", HERO_TRIPLE_BARREL), StatKey::FloatFlop);
    assert_no_spot(&events(&conn, "Bob", RIVER_BARREL_GIVEN_UP), StatKey::FloatFlop);
}

#[test]
fn probe_requires_raiser_flop_checkback() {
    let conn = setup();
    let bob = events(&conn, "Bob", PROBE_AFTER_CHECKBACK);
    let spot = assert_spot(&bob, StatKey::ProbeTurn, true);
    assert_eq!(spot.counterparty.creator, Some(player_id(&conn, "Hal")));
    assert_spot(&events(&conn, "Bob", HERO_CHECKS_TWICE), StatKey::ProbeTurn, false);
    assert_spot(&events(&conn, "Bob", DELAYED_CBET_SHOWDOWN), StatKey::ProbeTurn, false);
    // The raiser c-bet the flop.
    assert_no_spot(&events(&conn, "Bob", RIVER_OVERBET_RAISED), StatKey::ProbeTurn);
    assert_no_spot(&events(&conn, "Bob", FOLD_TO_TURN_BARREL), StatKey::ProbeTurn);
}

// ---- F11: river aggression

#[test]
fn river_bet_and_raise_count_correctly() {
    let conn = setup();
    let bob = events(&conn, "Bob", RIVER_OVERBET_RAISED);
    assert_spot(&bob, StatKey::RiverBet, true);
    assert_no_spot(&bob, StatKey::RiverRaise);
    let hal = events(&conn, "Hal", RIVER_OVERBET_RAISED);
    let spot = assert_spot(&hal, StatKey::RiverRaise, true);
    assert_eq!(spot.counterparty.creator, Some(player_id(&conn, "Bob")));
    assert_no_spot(&hal, StatKey::RiverBet);

    let bob = events(&conn, "Bob", HERO_TRIPLE_BARREL);
    assert_spot(&bob, StatKey::RiverBet, false);
    assert_spot(&bob, StatKey::RiverRaise, false);
    assert_spot(&events(&conn, "Hero", HERO_TRIPLE_BARREL), StatKey::RiverBet, true);

    let hero = events(&conn, "Hero", HERO_CHECKS_TWICE);
    assert_no_spot(&hero, StatKey::RiverBet);
    assert_spot(&hero, StatKey::RiverRaise, false);
    assert_spot(&events(&conn, "Bob", HERO_CHECKS_TWICE), StatKey::RiverBet, true);
    // All in on the flop: no river decision.
    let ugo = events(&conn, "Ugo", FLOP_ALL_IN_VS_HERO);
    assert_no_spot(&ugo, StatKey::RiverBet);
    assert_no_spot(&ugo, StatKey::RiverRaise);
}

// ---- F12: showdown tendencies

#[test]
fn wtsd_wsd_wwsf_count_showdowns_and_pots_won() {
    let conn = setup();
    let bob = events(&conn, "Bob", DELAYED_CBET_SHOWDOWN);
    assert_spot(&bob, StatKey::Wtsd, true);
    assert_spot(&bob, StatKey::Wsd, true);
    assert_spot(&bob, StatKey::Wwsf, true);
    let cole = events(&conn, "Cole", DELAYED_CBET_SHOWDOWN);
    assert_spot(&cole, StatKey::Wtsd, true);
    assert_spot(&cole, StatKey::Wsd, false);
    assert_spot(&cole, StatKey::Wwsf, false);

    // Won without showdown; folded on the flop.
    let ugo = events(&conn, "Ugo", HU_CBET_FOLD);
    assert_spot(&ugo, StatKey::Wtsd, false);
    assert_no_spot(&ugo, StatKey::Wsd);
    assert_spot(&ugo, StatKey::Wwsf, true);
    let bob = events(&conn, "Bob", HU_CBET_FOLD);
    assert_spot(&bob, StatKey::Wtsd, false);
    assert_spot(&bob, StatKey::Wwsf, false);

    let ugo = events(&conn, "Ugo", LIMPED_MULTIWAY);
    assert_eq!(assert_spot(&ugo, StatKey::Wtsd, false).multiway, Some(true));
    assert_spot(&ugo, StatKey::Wwsf, false);
    // All in on the flop still saw the flop with a decision.
    assert_spot(&events(&conn, "Ugo", FLOP_ALL_IN_VS_HERO), StatKey::Wtsd, true);
}

#[test]
fn preflop_all_in_creates_no_postflop_opportunities() {
    let conn = setup();
    // Both all-in players reached a showdown, and the others folded preflop:
    // nobody made a postflop decision.
    for name in ["Ugo", "Hal", "Bob", "Hero"] {
        let found = events(&conn, name, PREFLOP_ALL_IN);
        assert!(found.is_empty(), "{name}: expected no postflop events, got {found:#?}");
    }
    // A flop all-in is a postflop decision.
    assert_spot(&events(&conn, "Ugo", FLOP_ALL_IN_VS_HERO), StatKey::CbetFlop, true);
}

// ---- Hero counterparty attribution

#[test]
fn postflop_events_carry_hero_counterparty() {
    let conn = setup();
    let hero_id = player_id(&conn, "Hero");

    let bob = events(&conn, "Bob", HERO_TRIPLE_BARREL);
    for key in [StatKey::FoldToCbetFlop, StatKey::FoldToCbetTurn, StatKey::FoldToCbetRiver] {
        let spot = find(&bob, key).unwrap();
        assert_eq!(spot.counterparty.creator, Some(hero_id));
        assert!(spot.counterparty.hero_created, "{} created by the hero", key.as_str());
        assert!(spot.counterparty.hero_in_pot);
        assert_eq!(spot.counterparty.hero_position.as_deref(), Some("BTN"));
    }

    // The hero folded preflop: dealt in, but not in the pot.
    let bob = events(&conn, "Bob", HU_CBET_FOLD);
    let spot = find(&bob, StatKey::FoldToCbetFlop).unwrap();
    assert!(!spot.counterparty.hero_created);
    assert!(spot.counterparty.hero_in_hand);
    assert!(!spot.counterparty.hero_in_pot);

    // The hero was in the multiway pot without creating the spot.
    let cole = events(&conn, "Cole", MW_CBET_HERO_FLOATS);
    let spot = find(&cole, StatKey::FoldToCbetFlop).unwrap();
    assert!(!spot.counterparty.hero_created);
    assert!(spot.counterparty.hero_in_pot);
    let ugo = events(&conn, "Ugo", FLOP_ALL_IN_VS_HERO);
    assert!(find(&ugo, StatKey::CbetFlop).unwrap().counterparty.hero_in_pot);

    // The hero's own events never count the hero as a counterparty.
    let hero = events(&conn, "Hero", MW_CBET_HERO_FLOATS);
    assert!(hero.iter().all(|e| !e.counterparty.hero_in_pot && !e.counterparty.hero_created));
}

#[test]
fn every_postflop_key_is_exercised_by_the_fixture() {
    let conn = setup();
    let mut seen = HashSet::new();
    for name in ["Hero", "Sam", "Bob", "Ugo", "Hal", "Cole"] {
        let id = player_id(&conn, name);
        for hand in load_player_hands(&conn, id).unwrap() {
            for event in extract_postflop(&hand, id) {
                assert!(StatKey::POSTFLOP.contains(&event.key));
                seen.insert((event.key, event.success));
            }
        }
    }
    for key in StatKey::POSTFLOP {
        assert!(seen.contains(&(key, true)), "no hit for {}", key.as_str());
        assert!(seen.contains(&(key, false)), "no miss for {}", key.as_str());
    }
}
