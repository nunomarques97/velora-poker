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

/// A financially self-consistent cash hand (net results across all three
/// players sum to exactly `-rake`) used to verify `net_result` math directly,
/// independent of the other fixtures above (which were authored for
/// action/street parsing and are not rake-accurate).
const NET_RESULT_CASH_HAND: &str = r#"PokerStars Hand #300000000001: Hold'em No Limit ($0.25/$0.50 USD) - 2026/08/25 21:00:00 ET
Table 'SessionTable' 3-max Seat #1 is the button
Seat 1: Hero ($50.00 in chips)
Seat 2: Villain ($50.00 in chips)
Seat 3: Robot ($50.00 in chips)
Villain: posts small blind $0.25
Robot: posts big blind $0.50
*** HOLE CARDS ***
Dealt to Hero [Ah Ad]
Hero: raises $1.50 to $2
Villain: folds
Robot: folds
Uncalled bet ($1.50) returned to Hero
Hero collected $1.25 from pot
*** SUMMARY ***
Total pot $1.25 | Rake $0
Seat 1: Hero (button) collected ($1.25)
Seat 2: Villain (small blind) folded before Flop
Seat 3: Robot (big blind) folded before Flop
"#;

#[test]
fn computes_net_result_for_cash_hands_from_contributed_and_collected_amounts() {
    let parser = PokerStarsParser;
    let hand = parser.parse(NET_RESULT_CASH_HAND)[0]
        .as_ref()
        .expect("hand should parse")
        .clone();

    let hero = hand.results.get("Hero").expect("hero result");
    let villain = hand.results.get("Villain").expect("villain result");
    let robot = hand.results.get("Robot").expect("robot result");

    assert_eq!(hero.net_result, Some(0.75));
    assert_eq!(villain.net_result, Some(-0.25));
    assert_eq!(robot.net_result, Some(-0.50));

    let total: f64 = [hero, villain, robot]
        .iter()
        .map(|r| r.net_result.unwrap())
        .sum();
    assert!((total - 0.0).abs() < 1e-9, "zero-rake hand must net to zero across all players, got {total}");
}

#[test]
fn never_computes_net_result_for_tournament_hands() {
    let tournament_hand = r#"PokerStars Hand #300000000099: Tournament #900000001, $10+$1 Hold'em No Limit - Level I (10/20) - 2026/08/25 22:00:00 ET
Table '900000001 1' 6-max Seat #1 is the button
Seat 1: Hero (1500 in chips)
Seat 2: Villain (1500 in chips)
Villain: posts small blind 10
Hero: posts big blind 20
*** HOLE CARDS ***
Dealt to Hero [Ah Ad]
Villain: folds
Uncalled bet (0) returned to Hero
Hero collected 20 from pot
*** SUMMARY ***
Total pot 20 | Rake 0
Seat 1: Hero (big blind) collected (20)
Seat 2: Villain (small blind) folded before Flop
"#;

    let parser = PokerStarsParser;
    let hand = parser.parse(tournament_hand)[0]
        .as_ref()
        .expect("hand should parse")
        .clone();

    for (name, result) in &hand.results {
        assert_eq!(
            result.net_result, None,
            "{name} must have no net_result on a tournament hand, even though they won chips"
        );
    }
}
