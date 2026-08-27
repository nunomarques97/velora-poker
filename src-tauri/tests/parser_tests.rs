use velora_poker_lib::parser::{
    split_hands, ActionType, HandFormat, HandHistoryParser, ParseError, PokerStarsParser, Street,
};

const SHOWDOWN_HAND: &str = include_str!("fixtures/hand_3bet_showdown.txt");
const CBET_FOLD_HAND: &str = include_str!("fixtures/hand_cbet_fold.txt");

#[test]
fn parses_header_fields() {
    let parser = PokerStarsParser;
    let results = parser.parse(SHOWDOWN_HAND);
    assert_eq!(results.len(), 1);
    let hand = results[0].as_ref().expect("hand should parse");

    assert_eq!(hand.hand_id, "200000000001");
    assert_eq!(hand.site, "pokerstars");
    assert_eq!(hand.format, HandFormat::Cash);
    assert_eq!(hand.table_name, "Atlas III");
    assert_eq!(hand.max_seats, 3);
    assert_eq!(hand.button_seat, 1);
    assert_eq!(hand.small_blind, 0.25);
    assert_eq!(hand.big_blind, 0.50);
    assert_eq!(hand.currency, "USD");
    assert_eq!(hand.played_at, "2026-08-20T21:15:03");
    assert_eq!(hand.hero_name.as_deref(), Some("Hero"));
    assert_eq!(hand.seats.len(), 3);
}

#[test]
fn classifies_actions_by_street_and_type() {
    let parser = PokerStarsParser;
    let hand = parser.parse(SHOWDOWN_HAND)[0].as_ref().unwrap().clone();

    let preflop_raises: Vec<_> = hand
        .actions
        .iter()
        .filter(|a| a.street == Street::Preflop && a.action_type == ActionType::Raise)
        .collect();
    // Hero opens, Robot 3-bets.
    assert_eq!(preflop_raises.len(), 2);

    let flop_bets: Vec<_> = hand
        .actions
        .iter()
        .filter(|a| a.street == Street::Flop && a.action_type == ActionType::Bet)
        .collect();
    assert_eq!(flop_bets.len(), 1);
    assert_eq!(flop_bets[0].player_name, "Robot");

    let blinds: Vec<_> = hand
        .actions
        .iter()
        .filter(|a| {
            matches!(
                a.action_type,
                ActionType::PostSmallBlind | ActionType::PostBigBlind
            )
        })
        .collect();
    assert_eq!(blinds.len(), 2);
}

#[test]
fn detects_showdown_results() {
    let parser = PokerStarsParser;
    let hand = parser.parse(SHOWDOWN_HAND)[0].as_ref().unwrap().clone();

    let robot = hand.results.get("Robot").expect("robot result");
    assert!(robot.went_to_showdown);
    assert!(robot.won_at_showdown);

    let hero = hand.results.get("Hero").expect("hero result");
    assert!(hero.went_to_showdown);
    assert!(!hero.won_at_showdown);

    let villain = hand.results.get("Villain").expect("villain result");
    assert!(!villain.went_to_showdown);
    assert!(!villain.won_at_showdown);
}

#[test]
fn detects_uncontested_pot_without_showdown() {
    let parser = PokerStarsParser;
    let hand = parser.parse(CBET_FOLD_HAND)[0].as_ref().unwrap().clone();

    for result in hand.results.values() {
        assert!(!result.went_to_showdown);
        assert!(!result.won_at_showdown);
    }
}

#[test]
fn splits_multiple_hands_from_one_file() {
    let combined = format!("{}\n\n{}", SHOWDOWN_HAND.trim(), CBET_FOLD_HAND.trim());
    let blocks = split_hands(&combined);
    assert_eq!(blocks.len(), 2);

    let parser = PokerStarsParser;
    let results = parser.parse(&combined);
    assert_eq!(results.len(), 2);
    assert!(results.iter().all(|r| r.is_ok()));
}

#[test]
fn parses_tournament_headers_as_supported() {
    // Tournament hand histories are now supported (see tournament_parser_tests.rs
    // for full coverage); this just confirms the dispatch no longer rejects them.
    let tournament_header = "PokerStars Hand #200000000099: Tournament #123456, $5+$0.50 USD Hold'em No Limit - Level I (10/20) - 2026/08/20 21:15:03 ET";
    let parser = PokerStarsParser;
    let results = parser.parse(tournament_header);
    assert_eq!(results.len(), 1);
    let hand = results[0].as_ref().expect("tournament header should parse");
    assert_eq!(hand.format, HandFormat::Tournament);
    assert_eq!(hand.tournament_id.as_deref(), Some("123456"));
}

#[test]
fn rejects_unrecognized_tournament_header_format() {
    // Contains "Tournament #" (so it's dispatched to the tournament parser)
    // but doesn't match the recognized structure at all — must fail
    // explicitly rather than silently produce a garbage/empty hand.
    let malformed = "PokerStars Hand #200000000098: Tournament #123456 something completely different - 2026/08/20 21:15:03 ET";
    let parser = PokerStarsParser;
    let results = parser.parse(malformed);
    assert_eq!(results.len(), 1);
    match &results[0] {
        Err(ParseError::UnsupportedFormat(_)) => {}
        other => panic!("expected UnsupportedFormat, got {other:?}"),
    }
}
