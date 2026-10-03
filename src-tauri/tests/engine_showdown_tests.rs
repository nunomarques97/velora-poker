//! Opponent engine, task T5: hand evaluator, showdown memory, value/bluff
//! classification and sizing tells (catalogue rows M01–M04 of
//! `docs/specs/opponent-engine.md`).
//!
//! Fixture: `engine_showdown_cash.txt` is a 6-max $0.25/$0.50 cash table
//! with 100bb stacks, seating Hero (button), Sam (SB), Bob (BB), Vic (UTG),
//! Otto (HJ) and Tess (CO). Each hand is a single raise called by Bob, who
//! then checks and calls every bet:
//! - 501–503: Vic bets small on the river with top pair good kicker, an
//!   overpair and a straight (three small-bet value lines).
//! - 504–506: Otto overbets the river with a missed flush draw, queen high
//!   and a board pair (three overbet bluffs).
//! - 507–508: Tess bets large twice (value, then an underpair): a sample
//!   below the sizing-tell minimum.
//! - 509: Vic shows after winning uncontested on the flop (no showdown).
//! - 510: Vic checks down and mucks unseen; Bob shows and wins.

use velora_poker_lib::engine::{
    board_plays, evaluate, extract_showdowns, hole_strength, load_player_hands,
    load_showdown_boards, parse_board, parse_cards, parse_total_pot, player_showdowns, replay_pot,
    showdown_record, sizing_tally, sizing_tells, Card, Category, HoleStrength, ShowdownRecord,
    ShowdownResult, SizeBucket, ValueClass, SIZING_TELL_MIN_SAMPLE,
};
use velora_poker_lib::parser::{ActionType, Street};
use velora_poker_lib::{db, import};

const SHOWDOWNS: &str = include_str!("fixtures/engine_showdown_cash.txt");
const SHOWS_AND_MUCKS: &str = include_str!("fixtures/showdown_shows_and_mucks.txt");

fn import_db(text: &str) -> rusqlite::Connection {
    let mut conn = db::open(std::path::Path::new(":memory:")).expect("open db");
    import::import_text(&mut conn, text).expect("import fixture");
    conn
}

fn setup() -> rusqlite::Connection {
    import_db(SHOWDOWNS)
}

fn player_id(conn: &rusqlite::Connection, name: &str) -> i64 {
    conn.query_row("SELECT id FROM players WHERE name = ?1", [name], |row| row.get(0))
        .unwrap_or_else(|_| panic!("player {name} not found"))
}

fn records(conn: &rusqlite::Connection, name: &str) -> Vec<ShowdownRecord> {
    player_showdowns(conn, player_id(conn, name)).expect("showdown records")
}

fn cards(text: &str) -> Vec<Card> {
    parse_cards(text).unwrap_or_else(|| panic!("bad cards {text}"))
}

fn best(text: &str) -> Category {
    evaluate(&cards(text)).expect("evaluate").category
}

fn strength(hole: &str, board: &str) -> HoleStrength {
    hole_strength(&cards(hole), &cards(board)).expect("hole strength")
}

fn hand_refs(records: &[ShowdownRecord]) -> Vec<&str> {
    records.iter().map(|r| r.hand_ref.as_str()).collect()
}

// --- M02: hand evaluator ---------------------------------------------------

#[test]
fn evaluator_handles_wheel_straight() {
    let wheel = evaluate(&cards("Ah 2c 3d 4s 5h")).unwrap();
    assert_eq!(wheel.category, Category::Straight);
    assert_eq!(wheel.ranks, vec![5], "the wheel tops at five");
    // Seven cards: the wheel is found among them, and a six-high straight
    // beats it.
    assert_eq!(best("Ah 2c 3d 4s 5h Kd Kc"), Category::Straight);
    let six_high = evaluate(&cards("2c 3d 4s 5h 6d")).unwrap();
    assert!(six_high > wheel);
    // Ace-high straight; K-A-2-3-4 does not wrap around.
    assert_eq!(evaluate(&cards("Ts Jh Qd Kc Ah")).unwrap().ranks, vec![14]);
    assert_eq!(best("Kc Ah 2d 3s 4h"), Category::HighCard);
    // A steel wheel is a straight flush.
    assert_eq!(best("Ah 2h 3h 4h 5h 9c"), Category::StraightFlush);
}

#[test]
fn evaluator_flush_beats_straight() {
    let flush = evaluate(&cards("2h 7h 9h Jh Kh")).unwrap();
    let straight = evaluate(&cards("9c Td Jh Qs Kd")).unwrap();
    assert_eq!(flush.category, Category::Flush);
    assert_eq!(straight.category, Category::Straight);
    assert!(flush > straight);
    // Seven cards holding both a straight and a flush: the flush is best.
    assert_eq!(best("8h 9h Tc Jd Qh 2h 5h"), Category::Flush);
    // Flushes compare card by card.
    assert!(evaluate(&cards("Ah 3h 4h 5h 7h")).unwrap() > evaluate(&cards("Kh Qh Jh 9h 8h")).unwrap());
}

#[test]
fn evaluator_full_house_from_two_trips() {
    let value = evaluate(&cards("Kc Kd Ks 7h 7d 7c 2s")).unwrap();
    assert_eq!(value.category, Category::FullHouse);
    assert_eq!(value.ranks, vec![13, 7], "kings full of sevens");
    // Trips plus two pairs: the higher pair fills the boat.
    assert_eq!(evaluate(&cards("5c 5d 5s 9h 9d 3c 3s")).unwrap().ranks, vec![5, 9]);
    assert!(value > evaluate(&cards("Ah Kh Qh Jh 9h")).unwrap(), "a full house beats a flush");
    assert_eq!(best("9c 9d 9s 9h 2c 2d 2s"), Category::Quads);
}

#[test]
fn evaluator_board_plays() {
    let board = cards("Ts Jh Qd Kc Ah");
    // Broadway on board: low hole cards change nothing.
    assert!(board_plays(&cards("2c 3d"), &board));
    assert_eq!(strength("2c 3d", "Ts Jh Qd Kc Ah"), HoleStrength::NoPair);
    // A flush with a hole card beats the board's straight.
    assert!(!board_plays(&cards("2h 3h"), &cards("Th Jh Qh Kc Ah")));
    // Two pair on board with a better kicker in the hand is still the board.
    assert_eq!(strength("Ac 3d", "8c 8d 5s 5h Kd"), HoleStrength::NoPair);
    // A hole card pairing the top card counterfeits the board's lower pair:
    // one pair in practice, rated by its kicker.
    assert_eq!(strength("Kh 2c", "8c 8d 5s 5h Kd"), HoleStrength::TopPairWeakKicker);
    // Trips on board: an overcard kicker is not a made hand.
    assert_eq!(strength("Ac Qd", "Kc Kd Ks 5h 2c"), HoleStrength::NoPair);
    // Only a full board can play.
    assert!(!board_plays(&cards("2c 3d"), &cards("Ts Jh Qd")));
    // Malformed and repeated cards are rejected, never guessed.
    assert!(parse_cards("Ah Xx").is_none());
    assert!(parse_cards("Ah Ah").is_none());
    assert!(evaluate(&cards("Ah Kd 2c 3s")).is_none(), "four cards are not a hand");
}

#[test]
fn evaluator_pair_classes_for_value_bluff() {
    // Value classes.
    assert_eq!(strength("Ks Qd", "Kc 7d 2s"), HoleStrength::TopPairGoodKicker);
    assert_eq!(strength("Kd Ts", "Kc 7d 2s 4h"), HoleStrength::TopPairGoodKicker);
    assert_eq!(strength("Ah Ad", "Jc 8s 3d"), HoleStrength::Overpair);
    assert_eq!(strength("Qh Qc", "8c 8d 5s"), HoleStrength::Overpair, "a board pair stays an overpair");
    assert_eq!(strength("Kd 7c", "Kc 7d 2s"), HoleStrength::MadeHand, "two pair with both hole cards");
    assert_eq!(strength("2h 2c", "Kc 7d 2s"), HoleStrength::MadeHand, "a set");
    // Neither classes.
    assert_eq!(strength("Kd 9s", "Kc 7d 2s"), HoleStrength::TopPairWeakKicker);
    assert_eq!(strength("7s Ad", "Kc 7d 2s"), HoleStrength::SecondPairOrLower);
    assert_eq!(strength("9h 9c", "Kc 7d 2s"), HoleStrength::SecondPairOrLower);
    assert_eq!(strength("3h 3c", "Kc 7d 4s"), HoleStrength::Underpair);
    assert_eq!(strength("3h 3c", "8c 8d 5s"), HoleStrength::Underpair, "a board pair does not lift it");
    assert_eq!(strength("Kd 4c", "Kc Kh 7d 4s"), HoleStrength::MadeHand, "full house");
    assert_eq!(strength("7s 4c", "Kc Kh 7d"), HoleStrength::SecondPairOrLower);
    // Bluff class: nothing made with a hole card, missed draws included.
    assert_eq!(strength("6h 5h", "Ah Kh 2c 9s Jd"), HoleStrength::NoPair);
    assert_eq!(strength("7s 6s", "Kc Kd 4h 2d Ts"), HoleStrength::NoPair);
    // The spec's mapping.
    assert_eq!(ValueClass::from_strength(HoleStrength::TopPairGoodKicker), ValueClass::Value);
    assert_eq!(ValueClass::from_strength(HoleStrength::Overpair), ValueClass::Value);
    assert_eq!(ValueClass::from_strength(HoleStrength::MadeHand), ValueClass::Value);
    assert_eq!(ValueClass::from_strength(HoleStrength::TopPairWeakKicker), ValueClass::Neither);
    assert_eq!(ValueClass::from_strength(HoleStrength::SecondPairOrLower), ValueClass::Neither);
    assert_eq!(ValueClass::from_strength(HoleStrength::Underpair), ValueClass::Neither);
    assert_eq!(ValueClass::from_strength(HoleStrength::NoPair), ValueClass::Bluff);
}

// --- M01: showdown records -------------------------------------------------

#[test]
fn fixture_pots_reconcile_with_total_pot_lines() {
    let conn = setup();
    let hands = load_player_hands(&conn, player_id(&conn, "Bob")).unwrap();
    assert_eq!(hands.len(), 10);
    for block in SHOWDOWNS.split("PokerStars Hand #").filter(|b| !b.trim().is_empty()) {
        let hand_ref = block.split(':').next().unwrap();
        let hand = hands.iter().find(|h| h.hand_ref == hand_ref).unwrap();
        let total = parse_total_pot(block).unwrap();
        assert!((replay_pot(hand).total - total).abs() < 1e-6, "hand {hand_ref} pot");
    }
}

#[test]
fn showdown_record_captures_cards_board_line_and_result() {
    let conn = setup();
    let vic = records(&conn, "Vic");
    assert_eq!(hand_refs(&vic), ["300000000501", "300000000502", "300000000503"]);

    let first = &vic[0];
    assert_eq!(first.cards, "Ks Qd");
    assert_eq!(first.board, "Kc 7d 2s 4h 9c");
    assert_eq!(first.category, Category::Pair);
    assert_eq!(first.result, ShowdownResult::Won);
    assert_eq!(first.played_at.as_deref().map(|p| p.is_empty()), Some(false));
    let line: Vec<(Street, ActionType, Option<SizeBucket>)> =
        first.line.iter().map(|s| (s.street, s.action, s.size_bucket)).collect();
    assert_eq!(
        line,
        [
            (Street::Preflop, ActionType::Raise, Some(SizeBucket::Large)),
            (Street::Flop, ActionType::Bet, Some(SizeBucket::Small)),
            (Street::Turn, ActionType::Check, None),
            (Street::River, ActionType::Bet, Some(SizeBucket::Small)),
        ]
    );
    let river = first.line.last().unwrap().pot_fraction.unwrap();
    assert!((river - 1.5 / 5.25).abs() < 1e-9, "river fraction {river}");
    assert_eq!(vic[2].category, Category::Straight);

    // A losing record: Bob called down with jacks against aces.
    let bob = records(&conn, "Bob");
    let lost = bob.iter().find(|r| r.hand_ref == "300000000502").unwrap();
    assert_eq!((lost.cards.as_str(), lost.result), ("Jh Td", ShowdownResult::Lost));
    assert!(lost.line.iter().all(|s| matches!(s.action, ActionType::Call | ActionType::Check)));
    assert!(lost.last_aggression.is_none(), "calling down is no aggression");
    let won = bob.iter().find(|r| r.hand_ref == "300000000504").unwrap();
    assert_eq!((won.category, won.result), (Category::Pair, ShowdownResult::Won));

    // The other fixture: a river bettor who showed, against a caller whose
    // summary revealed his cards.
    let other = import_db(SHOWS_AND_MUCKS);
    let bjorn = records(&other, "Barrel_Bjorn");
    assert_eq!(hand_refs(&bjorn), ["262100000201"]);
    assert_eq!(bjorn[0].board, "Ks 8d 5c 2h Qd");
    let station = records(&other, "CallStation62");
    assert_eq!((station[0].cards.as_str(), station[0].result), ("Kd Jc", ShowdownResult::Lost));
}

#[test]
fn unshown_hands_add_no_showdown_record() {
    let conn = setup();
    let vic = records(&conn, "Vic");
    // 509: shown after an uncontested win; 510: mucked unseen at showdown.
    assert!(!hand_refs(&vic).contains(&"300000000509"));
    assert!(!hand_refs(&vic).contains(&"300000000510"));
    // Bob mucked unseen in 501, 503 and 507.
    let bob = hand_refs(&records(&conn, "Bob")).join(",");
    assert_eq!(bob, "300000000502,300000000504,300000000505,300000000506,300000000508,300000000510");
    // Players who never reached showdown and the hero have none.
    assert!(records(&conn, "Sam").is_empty());
    assert!(records(&conn, "Hero").is_empty(), "the hero's dealt cards are not a shown hand");
    // The shown-without-showdown hand of the other fixture adds nothing.
    let other = import_db(SHOWS_AND_MUCKS);
    assert!(records(&other, "Mr.Overbet").is_empty());
    // Without a board the record is not fabricated.
    let id = player_id(&conn, "Vic");
    let hands = load_player_hands(&conn, id).unwrap();
    assert!(extract_showdowns(&hands, &Default::default(), id).is_empty());
    let boards = load_showdown_boards(&conn, id).unwrap();
    assert_eq!(boards.len(), 3, "boards load only for shown showdowns");
}

#[test]
fn showdown_board_is_parsed_from_the_summary_line() {
    assert_eq!(parse_board("Total pot $3 | Rake $0\nBoard [Kc 7d 2s]\n").unwrap(), cards("Kc 7d 2s"));
    assert!(parse_board("*** SUMMARY ***\nTotal pot $3 | Rake $0\n").is_none());
    assert!(parse_board("Board [Kc 7d Zz]").is_none());
    // A board run twice has no single board line.
    assert!(parse_board("FIRST Board [Kc 7d 2s 4h 9c]\nSECOND Board [Kc 7d 2s 5h 3c]").is_none());
}

#[test]
fn equal_shown_hands_that_both_win_are_a_split() {
    let conn = setup();
    let vic = player_id(&conn, "Vic");
    let mut hand = load_player_hands(&conn, vic)
        .unwrap()
        .into_iter()
        .find(|h| h.hand_ref == "300000000510")
        .unwrap();
    // Give Vic an ace with the same kickers as Bob's and a share of the pot.
    for seat in hand.seats.iter_mut().filter(|s| s.player_id == vic) {
        seat.hole_cards = Some("As 9d".into());
        seat.won_at_showdown = true;
    }
    let board = cards("Ac 8h 5s 4d 2h");
    assert_eq!(showdown_record(&hand, &board, vic).unwrap().result, ShowdownResult::Split);
    let bob = player_id(&conn, "Bob");
    assert_eq!(showdown_record(&hand, &board, bob).unwrap().result, ShowdownResult::Split);
}

// --- M03: value/bluff classification of the last aggression ---------------

#[test]
fn last_aggression_classified_value_bluff_neither() {
    let conn = setup();
    let class_of = |name: &str, hand_ref: &str| {
        let aggression = records(&conn, name)
            .into_iter()
            .find(|r| r.hand_ref == hand_ref)
            .unwrap_or_else(|| panic!("{name} has no record in {hand_ref}"))
            .last_aggression
            .unwrap_or_else(|| panic!("{name} made no aggression in {hand_ref}"));
        (aggression.street, aggression.size_bucket, aggression.class)
    };
    // Value: top pair good kicker, an overpair, a straight.
    assert_eq!(class_of("Vic", "300000000501"), (Street::River, SizeBucket::Small, ValueClass::Value));
    assert_eq!(class_of("Vic", "300000000502"), (Street::River, SizeBucket::Small, ValueClass::Value));
    assert_eq!(class_of("Vic", "300000000503"), (Street::River, SizeBucket::Small, ValueClass::Value));
    // Bluff: a missed flush draw, queen high, a board-only pair.
    for hand_ref in ["300000000504", "300000000505", "300000000506"] {
        assert_eq!(class_of("Otto", hand_ref), (Street::River, SizeBucket::Overbet, ValueClass::Bluff));
    }
    // Neither: eights under a queen-jack board.
    assert_eq!(class_of("Tess", "300000000508"), (Street::River, SizeBucket::Large, ValueClass::Neither));
    assert_eq!(class_of("Tess", "300000000507"), (Street::River, SizeBucket::Large, ValueClass::Value));
}

#[test]
fn last_aggression_is_judged_on_the_board_of_its_street() {
    let conn = setup();
    let vic = player_id(&conn, "Vic");
    let hand = load_player_hands(&conn, vic)
        .unwrap()
        .into_iter()
        .find(|h| h.hand_ref == "300000000510")
        .unwrap();
    // Pretend Vic showed Jd Td: his only bet was on the flop (Ac 8h 5s),
    // where jack-ten made nothing.
    let mut hand = hand;
    for seat in hand.seats.iter_mut().filter(|s| s.player_id == vic) {
        seat.hole_cards = Some("Jd Td".into());
    }
    let record = showdown_record(&hand, &cards("Ac 8h 5s 4d 2h"), vic).unwrap();
    let aggression = record.last_aggression.unwrap();
    assert_eq!((aggression.street, aggression.class), (Street::Flop, ValueClass::Bluff));
    // A turn-paired jack would not change the flop verdict.
    let record = showdown_record(&hand, &cards("Ac 8h 5s Jc 2h"), vic).unwrap();
    assert_eq!(record.last_aggression.unwrap().class, ValueClass::Bluff);
    assert_eq!(record.category, Category::Pair);
}

// --- M04: sizing tells -----------------------------------------------------

#[test]
fn small_bet_value_line_counts_as_value_tell() {
    let conn = setup();
    let tells = sizing_tells(&records(&conn, "Vic"));
    assert_eq!(tells.len(), 1);
    let small = tells[0];
    assert_eq!(small.bucket, SizeBucket::Small);
    assert_eq!((small.value, small.bluff, small.neither, small.n), (3, 0, 0, 3));
}

#[test]
fn overbet_bluff_line_counts_as_bluff_tell() {
    let conn = setup();
    let tells = sizing_tells(&records(&conn, "Otto"));
    assert_eq!(tells.len(), 1);
    let overbet = tells[0];
    assert_eq!(overbet.bucket, SizeBucket::Overbet);
    assert_eq!((overbet.value, overbet.bluff, overbet.neither, overbet.n), (0, 3, 0, 3));
}

#[test]
fn below_minimum_sizing_sample_emits_nothing() {
    let conn = setup();
    let tess = records(&conn, "Tess");
    // The counts exist, but two bets are below the minimum and emit nothing.
    let tally = sizing_tally(&tess);
    assert_eq!(tally.len(), 1);
    assert_eq!((tally[0].bucket, tally[0].value, tally[0].neither, tally[0].n), (SizeBucket::Large, 1, 1, 2));
    assert!(tally[0].n < SIZING_TELL_MIN_SAMPLE);
    assert!(sizing_tells(&tess).is_empty());
    // A player who never bet into a showdown has no tells at all.
    assert!(sizing_tally(&records(&conn, "Bob")).is_empty());
    // Two of Otto's three bluffs are not enough either.
    assert!(sizing_tells(&records(&conn, "Otto")[..2]).is_empty());
}
