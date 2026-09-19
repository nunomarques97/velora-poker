use velora_poker_lib::parser::{ActionType, HandFormat, HandHistoryParser, PokerStarsParser, Street};

const SHOWDOWN_ALLIN: &str = include_str!("fixtures/tournament_showdown_allin.txt");
const UNCONTESTED_WALK: &str = include_str!("fixtures/tournament_uncontested_walk.txt");
const ALLIN_PREFLOP_DISCONNECT: &str =
    include_str!("fixtures/tournament_allin_preflop_disconnect.txt");
const ZOOM_HEADER: &str = include_str!("fixtures/tournament_zoom_header.txt");

#[test]
fn parses_tournament_header_fields() {
    let parser = PokerStarsParser;
    let hand = parser.parse(SHOWDOWN_ALLIN)[0]
        .as_ref()
        .expect("hand should parse")
        .clone();

    assert_eq!(hand.hand_id, "261851768288");
    assert_eq!(hand.format, HandFormat::Tournament);
    assert_eq!(hand.tournament_id.as_deref(), Some("4025638884"));
    assert_eq!(hand.buy_in.as_deref(), Some("44000+6000"));
    assert_eq!(hand.game_type, "Hold'em No Limit");
    assert_eq!(hand.level.as_deref(), Some("VII"));
    // The current tournament level's blinds, not cash stakes.
    assert_eq!(hand.small_blind, 60.0);
    assert_eq!(hand.big_blind, 120.0);
    assert_eq!(hand.currency, "CHIPS");
    assert_eq!(hand.played_at, "2026-08-24T13:01:03");
    assert_eq!(hand.max_seats, 9);
    assert_eq!(hand.button_seat, 4);
    assert_eq!(hand.hero_name.as_deref(), Some("TourneyHero"));
}

#[test]
fn parses_headers_with_unpadded_single_digit_hour() {
    // Real PokerStars exports don't always zero-pad a single-digit local
    // hour (e.g. "0:06:23" rather than "00:06:23") near midnight. This is
    // the exact header shape from a real Zoom tournament export.
    let header = "PokerStars Hand #260993449710: Zoom Tournament #4002441239, €13.50+€13.50+€3.00 EUR Hold'em No Limit - Level VI (75/150) - 2026/06/01 0:06:23 WET [2026/05/31 19:06:23 ET]";
    let parser = PokerStarsParser;
    let results = parser.parse(header);
    let hand = results[0]
        .as_ref()
        .expect("header with unpadded hour should still parse");

    assert_eq!(hand.format, HandFormat::Tournament);
    assert_eq!(hand.tournament_id.as_deref(), Some("4002441239"));
    // played_at must stay zero-padded/sortable even though the source wasn't.
    assert_eq!(hand.played_at, "2026-06-01T00:06:23");

    // The same fix must apply to cash headers, which share the same time
    // pattern.
    let cash_header = "PokerStars Hand #260993449711: Hold'em No Limit ($0.25/$0.50 USD) - 2026/06/01 0:06:23 ET";
    let cash_results = parser.parse(cash_header);
    let cash_hand = cash_results[0]
        .as_ref()
        .expect("cash header with unpadded hour should still parse");
    assert_eq!(cash_hand.played_at, "2026-06-01T00:06:23");
}

#[test]
fn parses_zoom_tournament_header() {
    let parser = PokerStarsParser;
    let hand = parser.parse(ZOOM_HEADER)[0]
        .as_ref()
        .expect("zoom tournament hand should parse")
        .clone();

    assert_eq!(hand.format, HandFormat::Tournament);
    assert_eq!(hand.tournament_id.as_deref(), Some("4025999001"));
    assert_eq!(hand.buy_in.as_deref(), Some("10+1"));
    assert_eq!(hand.level.as_deref(), Some("I"));
    assert_eq!(hand.small_blind, 10.0);
    assert_eq!(hand.big_blind, 20.0);
}

#[test]
fn parses_seats_and_starting_stacks() {
    let parser = PokerStarsParser;
    let hand = parser.parse(SHOWDOWN_ALLIN)[0].as_ref().unwrap().clone();

    assert_eq!(hand.seats.len(), 9);
    let hero_seat = hand
        .seats
        .iter()
        .find(|s| s.player_name == "TourneyHero")
        .expect("TourneyHero seat");
    assert_eq!(hero_seat.seat_number, 2);
    assert_eq!(hero_seat.starting_stack, 1500.0);

    // Non-ASCII player names must round-trip correctly.
    let unicode_name = hand
        .seats
        .iter()
        .find(|s| s.player_name == "Opponént10")
        .expect("unicode-named seat");
    assert_eq!(unicode_name.starting_stack, 1254.0);
}

#[test]
fn parses_antes_for_every_seated_player() {
    let parser = PokerStarsParser;
    let hand = parser.parse(SHOWDOWN_ALLIN)[0].as_ref().unwrap().clone();

    let antes: Vec<_> = hand
        .actions
        .iter()
        .filter(|a| a.action_type == ActionType::PostAnte)
        .collect();
    assert_eq!(antes.len(), 9);
    assert!(antes.iter().all(|a| a.amount == Some(12.0)));
}

#[test]
fn parses_small_and_big_blind_posts() {
    let parser = PokerStarsParser;
    let hand = parser.parse(SHOWDOWN_ALLIN)[0].as_ref().unwrap().clone();

    let sb = hand
        .actions
        .iter()
        .find(|a| a.action_type == ActionType::PostSmallBlind)
        .expect("small blind action");
    assert_eq!(sb.player_name, "Opponént10");
    assert_eq!(sb.amount, Some(60.0));

    let bb = hand
        .actions
        .iter()
        .find(|a| a.action_type == ActionType::PostBigBlind)
        .expect("big blind action");
    assert_eq!(bb.player_name, "Opponent11");
    assert_eq!(bb.amount, Some(120.0));
}

#[test]
fn parses_preflop_fold_call_and_raise_actions() {
    let parser = PokerStarsParser;
    let hand = parser.parse(SHOWDOWN_ALLIN)[0].as_ref().unwrap().clone();

    let preflop: Vec<_> = hand
        .actions
        .iter()
        .filter(|a| a.street == Street::Preflop)
        .collect();

    let raise = preflop
        .iter()
        .find(|a| a.player_name == "TourneyHero" && a.action_type == ActionType::Raise)
        .expect("TourneyHero preflop raise");
    assert_eq!(raise.amount, Some(540.0));

    // opponent--12, Opponent14, Opponent8, Opponent9, Opponént10, Opponent11, Opponent7.
    let folds = preflop
        .iter()
        .filter(|a| a.action_type == ActionType::Fold)
        .count();
    assert_eq!(folds, 7);

    let calls = preflop
        .iter()
        .filter(|a| a.action_type == ActionType::Call)
        .count();
    assert_eq!(calls, 3);
}

#[test]
fn parses_flop_turn_river_streets() {
    let parser = PokerStarsParser;
    let hand = parser.parse(SHOWDOWN_ALLIN)[0].as_ref().unwrap().clone();

    let flop_bets: Vec<_> = hand
        .actions
        .iter()
        .filter(|a| a.street == Street::Flop && a.action_type == ActionType::Bet)
        .collect();
    assert_eq!(flop_bets.len(), 1);
    assert_eq!(flop_bets[0].player_name, "Opponent13");
    assert!(flop_bets[0].is_all_in);

    let flop_calls: Vec<_> = hand
        .actions
        .iter()
        .filter(|a| a.street == Street::Flop && a.action_type == ActionType::Call)
        .collect();
    assert_eq!(flop_calls.len(), 1);
    assert!(flop_calls[0].is_all_in);

    // No further actions on turn/river: both players are already all-in.
    assert!(hand.actions.iter().all(|a| a.street != Street::Turn));
    assert!(hand.actions.iter().all(|a| a.street != Street::River));
}

#[test]
fn detects_all_in_flags_on_bet_and_raise() {
    let parser = PokerStarsParser;

    let allin_bet_hand = parser.parse(SHOWDOWN_ALLIN)[0].as_ref().unwrap().clone();
    assert!(allin_bet_hand
        .actions
        .iter()
        .any(|a| a.action_type == ActionType::Bet && a.is_all_in));
    assert!(allin_bet_hand
        .actions
        .iter()
        .any(|a| a.action_type == ActionType::Call && a.is_all_in));

    let allin_raise_hand = parser.parse(ALLIN_PREFLOP_DISCONNECT)[0]
        .as_ref()
        .unwrap()
        .clone();
    let raise = allin_raise_hand
        .actions
        .iter()
        .find(|a| a.player_name == "TourneyHero" && a.action_type == ActionType::Raise)
        .expect("all-in raise");
    assert!(raise.is_all_in);
    assert_eq!(raise.amount, Some(4950.0));
}

#[test]
fn detects_tournament_showdown_results() {
    let parser = PokerStarsParser;
    let hand = parser.parse(SHOWDOWN_ALLIN)[0].as_ref().unwrap().clone();

    let winner = hand.results.get("Opponent13").expect("winner result");
    assert!(winner.went_to_showdown);
    assert!(winner.won_at_showdown);

    let loser = hand.results.get("TourneyHero").expect("loser result");
    assert!(loser.went_to_showdown);
    assert!(!loser.won_at_showdown);
}

#[test]
fn detects_folded_players_including_didnt_bet_suffix() {
    let parser = PokerStarsParser;
    let hand = parser.parse(SHOWDOWN_ALLIN)[0].as_ref().unwrap().clone();

    // "folded before Flop (didn't bet)" must still be recognized as folded.
    let opp14 = hand.results.get("Opponent14").expect("Opponent14 result");
    assert!(!opp14.went_to_showdown);
    assert!(!opp14.won_at_showdown);

    let opp7 = hand.results.get("Opponent7").expect("Opponent7 result");
    assert!(!opp7.went_to_showdown);
}

#[test]
fn detects_uncontested_walk_without_showdown() {
    let parser = PokerStarsParser;
    let hand = parser.parse(UNCONTESTED_WALK)[0].as_ref().unwrap().clone();

    // No "*** SHOW DOWN ***" section: pot won preflop, uncontested.
    for result in hand.results.values() {
        assert!(!result.went_to_showdown);
        assert!(!result.won_at_showdown);
    }

    // Opponent4, Opponent5, Opponent6, TourneyHero, Opponent1, Opponent2.
    let folds = hand
        .actions
        .iter()
        .filter(|a| a.action_type == ActionType::Fold)
        .count();
    assert_eq!(folds, 6);
}

#[test]
fn parses_all_in_raise_uncontested_hand() {
    let parser = PokerStarsParser;
    let hand = parser.parse(ALLIN_PREFLOP_DISCONNECT)[0]
        .as_ref()
        .unwrap()
        .clone();

    assert_eq!(hand.format, HandFormat::Tournament);
    assert_eq!(hand.level.as_deref(), Some("VIII"));

    // "Opponent1 is disconnected", "Uncalled bet ... returned to ...", and
    // "doesn't show hand" must not be mis-parsed as actions or crash the
    // parser: 7 antes + 2 blinds + 8 preflop actions (1 fold, 1 call, 1
    // all-in raise, then 5 more folds around the table) = 17.
    assert_eq!(hand.actions.len(), 17);
    let folds = hand
        .actions
        .iter()
        .filter(|a| a.action_type == ActionType::Fold)
        .count();
    assert_eq!(folds, 6);
    let calls = hand
        .actions
        .iter()
        .filter(|a| a.action_type == ActionType::Call)
        .count();
    assert_eq!(calls, 1);
}

#[test]
fn splits_multiple_tournament_hands_from_one_file() {
    // tournament_uncontested_walk.txt and tournament_allin_preflop_disconnect.txt
    // are two consecutive hands from the same real export file; concatenate them
    // back together the way PokerStars writes them to one file.
    let combined = format!(
        "{}\n\n\n{}",
        UNCONTESTED_WALK.trim(),
        ALLIN_PREFLOP_DISCONNECT.trim()
    );
    let parser = PokerStarsParser;
    let results = parser.parse(&combined);
    assert_eq!(results.len(), 2);
    assert!(results.iter().all(|r| r.is_ok()));
    assert_eq!(results[0].as_ref().unwrap().hand_id, "261851769364");
    assert_eq!(results[1].as_ref().unwrap().hand_id, "261851776358");
}
