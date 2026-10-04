//! Audit of the opponent engine's postflop and head-to-head stats (Run A,
//! task E2), of where they agree with `stats/mod.rs`, and of the pot
//! reconstruction.
//!
//! - Every `StatKey::POSTFLOP` key and every `H2hKey` is pinned, hits and
//!   opportunities, for every player of each fixture, worked out by hand from
//!   the hand history. The postflop counts are read twice — summed from
//!   `extract_postflop`'s events and from `aggregate_player`'s raw all-time
//!   counts — and so are the head-to-head counts (`h2h_events` over the
//!   hand's events, and `aggregate_player`'s `h2h`).
//! - The multiway flag is pinned with the counts: `cbet_flop.mw 1/1` is the
//!   part of `cbet_flop` whose street began with more than two players.
//! - A generated corpus with no preflop all-in and no raise between a c-bet
//!   and its answer is computed by both engines, which must agree exactly on
//!   c-bet, fold to c-bet, WTSD and W$SD.
//! - On the edge fixtures, every difference between the two engines on those
//!   four stats is listed and tagged with the documented divergence that
//!   explains it (`stat-contracts.md`, `opponent-engine.md` §9.2). An
//!   unlisted difference fails the test.
//! - `replay_pot` equals the `Total pot` line and its uncalled bets equal the
//!   `Uncalled bet` lines on every audit fixture and every hand of the C1
//!   generated corpus, except a cash dead blind: the documented known limit,
//!   pinned to its exact gap.
//!
//! Expectations are written as `"cbet_flop 1/2 cbet_flop.mw 1/1
//! h2h.fold_to_hero_cbet 0/1"` with the spec's stat keys. A key that is not
//! listed must have no opportunity at all; a `.mw` key is listed exactly when
//! the stat has a multiway opportunity.

mod common;

use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;

use rusqlite::Connection;
use velora_poker_lib::engine::{
    aggregate_player, extract_player_postflop, extract_postflop, extract_preflop, h2h_events,
    load_hands_after, load_player_hands, parse_total_pot, replay_pot, FormatKey, H2hKey, HandFacts,
    StatKey, View,
};
use velora_poker_lib::{db, import, stats};

use common::{Act, Line, Table};

const NO_OPPORTUNITY: &str = include_str!("fixtures/audit/preflop_no_opportunity.txt");
const ALLIN_OPEN: &str = include_str!("fixtures/audit/preflop_allin_open.txt");
const LIMPED_POT: &str = include_str!("fixtures/audit/preflop_limped_pot.txt");
const HEADS_UP: &str = include_str!("fixtures/audit/preflop_heads_up.txt");
const MULTIWAY: &str = include_str!("fixtures/audit/preflop_multiway.txt");
const WALK: &str = include_str!("fixtures/audit/preflop_walk.txt");
const POTS: &str = include_str!("fixtures/audit/cash_pots_usd.txt");
const MTT_ANTES: &str = include_str!("fixtures/audit/mtt_bounty_antes.txt");
const PLAYERS: &str = include_str!("fixtures/audit/cash_9max_eur_players.txt");
const NAMES: &str = include_str!("fixtures/audit/names_special.txt");
const SEAT_SHAPED: &str = include_str!("fixtures/audit/names_seat_shaped.txt");
const CHAT: &str = include_str!("fixtures/audit/cash_6max_usd_chat.txt");
const ZOOM: &str = include_str!("fixtures/audit/zoom_cash_gbp.txt");
const SPIN: &str = include_str!("fixtures/audit/spin_and_go.txt");
const PLAY_MONEY: &str = include_str!("fixtures/audit/cash_play_money.txt");
const PUSH_FOLD: &str = include_str!("fixtures/audit/engine_preflop_push_fold.txt");
const PREFLOP_SPOTS: &str = include_str!("fixtures/audit/engine_preflop_spots.txt");
const CBET_SPOTS: &str = include_str!("fixtures/audit/postflop_cbet_spots.txt");
const POSTFLOP_ALLIN: &str = include_str!("fixtures/audit/postflop_allin.txt");
const POSTFLOP_TRUNCATED: &str = include_str!("fixtures/audit/postflop_truncated.txt");
const SPOTS: &str = include_str!("fixtures/audit/engine_postflop_spots.txt");

// ---------------------------------------------------------------- expectations

/// `(fixture, text, hands stored, per-player engine counts)`.
type Fixture = (&'static str, &'static str, i64, &'static [(&'static str, &'static str)]);

/// Hand 101: Ana opens the button, the big blind check-folds to her c-bet.
/// Hand 102: Ana checks back the flop and faces Dov's probe and river bet
/// (no delayed c-bet spot: the turn was bet into her). Hand 103: Dov donks
/// into the raiser Caz, who has no c-bet spot and whose turn bet is no
/// barrel. Hand 104: Ana's c-bet is raised before Fin answers (no
/// fold-to-c-bet spot for Fin). Hand 105: Bru's c-bet is raised by Eli
/// before Ana answers; Eli wins at showdown.
const CBET_SPOTS_COUNTS: &[(&str, &str)] = &[
    (
        "Ana",
        "cbet_flop 2/3 cbet_flop.mw 1/1 donk_flop 0/1 donk_flop.mw 0/1 river_bet 0/1 river_raise 0/1 \
         wtsd 1/4 wtsd.mw 1/2 wsd 0/1 wsd.mw 0/1 wwsf 1/4 wwsf.mw 0/2",
    ),
    ("Bru", "cbet_flop 1/1 cbet_flop.mw 1/1 wtsd 0/1 wtsd.mw 0/1 wwsf 0/1 wwsf.mw 0/1"),
    ("Caz", "fold_to_cbet_flop 1/1 check_raise_flop 0/1 donk_flop 0/1 wtsd 0/2 wtsd.mw 0/1 wwsf 1/2 wwsf.mw 1/1"),
    (
        "Dov",
        "fold_to_cbet_flop 0/1 fold_to_cbet_flop.mw 0/1 donk_flop 1/2 donk_flop.mw 1/1 probe_turn 1/1 \
         river_bet 1/1 wtsd 0/3 wtsd.mw 0/2 wwsf 2/3 wwsf.mw 1/2",
    ),
    (
        "Eli",
        "fold_to_cbet_flop 0/1 fold_to_cbet_flop.mw 0/1 river_bet 0/1 wtsd 1/2 wtsd.mw 1/2 wsd 1/1 \
         wsd.mw 1/1 wwsf 1/2 wwsf.mw 1/2",
    ),
    ("Fin", "donk_flop 0/1 donk_flop.mw 0/1 wtsd 0/1 wtsd.mw 0/1 wwsf 0/1 wwsf.mw 0/1"),
];

/// Hand 201: Lars calls all-in preflop for less, Kye covers him with chips
/// behind and nobody left to bet: neither has a flop decision. Hand 202: an
/// all-in c-bet called all-in for less. Hand 203: the preflop raiser Lars is
/// all-in (no c-bet for anyone); Kye and Hob play on, a three-way field
/// counting the all-in Lars, and Kye wins the side pot only.
const POSTFLOP_ALLIN_COUNTS: &[(&str, &str)] = &[
    ("Gia", "cbet_flop 1/1 wtsd 1/1 wsd 1/1 wwsf 1/1"),
    ("Hob", "river_raise 0/1 river_raise.mw 0/1 wtsd 1/1 wtsd.mw 1/1 wsd 0/1 wsd.mw 0/1 wwsf 0/1 wwsf.mw 0/1"),
    ("Ina", ""),
    ("Jem", "fold_to_cbet_flop 0/1 check_raise_flop 0/1 donk_flop 0/1 wtsd 1/1 wsd 0/1 wwsf 0/1"),
    (
        "Kye",
        "check_raise_flop 0/1 check_raise_flop.mw 0/1 river_bet 1/1 river_bet.mw 1/1 wtsd 1/1 wtsd.mw 1/1 \
         wsd 1/1 wsd.mw 1/1 wwsf 1/1 wwsf.mw 1/1",
    ),
    ("Lars", ""),
];

/// The hero (`Hero`) plays against five villains. Hand 101: Hero's CO steal,
/// a three-way triple barrel; Rue folds the river, Sol wins the showdown.
/// 102: Uri's delayed c-bet. 103: checked flop and turn (delayed c-bet and
/// probe declined), river bet and raise; Uri steals against the hero's small
/// blind. 104: Pax floats Hero's c-bet in a three-way pot. 105: Uri's
/// small-blind steal against the hero, then a check-raise. 106: Uri folds to
/// the hero's 3-bet. 107: Tex calls the hero's 3-bet and folds to his c-bet.
/// 108: Pax 3-bets the hero's open. 109: Sol's three-way barrels without the
/// hero (Tex folds the turn, Uri calls it; Sol checks the river). 110: Rue
/// posts a dead blind and bets a limped flop.
const SPOTS_COUNTS: &[(&str, &str)] = &[
    (
        "Hero",
        "cbet_flop 3/3 cbet_flop.mw 2/2 cbet_turn 1/2 cbet_turn.mw 1/2 cbet_river 1/1 cbet_river.mw 1/1 \
         check_raise_flop 0/1 check_raise_flop.mw 0/1 river_bet 1/1 river_bet.mw 1/1 wtsd 1/5 wtsd.mw 1/3 \
         wsd 0/1 wsd.mw 0/1 wwsf 2/5 wwsf.mw 1/3",
    ),
    (
        "Pax",
        "fold_to_cbet_flop 0/1 fold_to_cbet_flop.mw 0/1 donk_flop 0/1 probe_turn 0/1 float_flop 1/1 \
         float_flop.mw 1/1 river_bet 1/1 wtsd 1/2 wtsd.mw 0/1 wsd 0/1 wwsf 0/2 wwsf.mw 0/1 \
         h2h.three_bet_vs_hero_open 1/3 h2h.fold_to_hero_cbet 0/1",
    ),
    (
        "Rue",
        "fold_to_cbet_flop 0/1 fold_to_cbet_flop.mw 0/1 fold_to_cbet_turn 0/1 fold_to_cbet_turn.mw 0/1 \
         fold_to_cbet_river 1/1 fold_to_cbet_river.mw 1/1 check_raise_flop 0/1 check_raise_flop.mw 0/1 \
         donk_flop 0/1 donk_flop.mw 0/1 river_bet 0/1 river_bet.mw 0/1 river_raise 0/1 river_raise.mw 0/1 \
         wtsd 0/3 wtsd.mw 0/2 wwsf 1/3 wwsf.mw 1/2 \
         h2h.three_bet_vs_hero_open 0/1 h2h.fold_to_hero_cbet 0/1 h2h.defend_vs_hero_steal 1/1",
    ),
    (
        "Sol",
        "cbet_flop 1/1 cbet_flop.mw 1/1 cbet_turn 1/1 cbet_turn.mw 1/1 cbet_river 0/1 \
         fold_to_cbet_flop 0/2 fold_to_cbet_flop.mw 0/2 fold_to_cbet_turn 0/1 fold_to_cbet_turn.mw 0/1 \
         fold_to_cbet_river 0/1 fold_to_cbet_river.mw 0/1 check_raise_flop 0/1 check_raise_flop.mw 0/1 \
         donk_flop 0/1 donk_flop.mw 0/1 river_bet 0/2 river_bet.mw 0/1 river_raise 0/1 river_raise.mw 0/1 \
         wtsd 2/3 wtsd.mw 2/3 wsd 1/2 wsd.mw 1/2 wwsf 1/3 wwsf.mw 1/3 \
         h2h.fold_to_hero_cbet 0/2 h2h.steal_vs_hero 0/2",
    ),
    (
        "Tex",
        "fold_to_cbet_flop 1/2 fold_to_cbet_flop.mw 0/1 fold_to_cbet_turn 1/1 fold_to_cbet_turn.mw 1/1 \
         check_raise_flop 0/2 check_raise_flop.mw 0/1 donk_flop 0/2 donk_flop.mw 0/1 wtsd 0/2 wtsd.mw 0/1 \
         wwsf 0/2 wwsf.mw 0/1 h2h.fold_to_hero_3bet 0/1 h2h.fold_to_hero_cbet 1/1 h2h.steal_vs_hero 0/3",
    ),
    (
        "Uri",
        "cbet_flop 0/3 fold_to_cbet_flop 0/1 fold_to_cbet_flop.mw 0/1 fold_to_cbet_turn 0/1 \
         fold_to_cbet_turn.mw 0/1 delayed_cbet 1/2 check_raise_flop 1/3 check_raise_flop.mw 0/2 \
         donk_flop 0/1 donk_flop.mw 0/1 river_bet 0/1 river_raise 1/1 wtsd 2/5 wtsd.mw 1/2 wsd 2/2 \
         wsd.mw 1/1 wwsf 4/5 wwsf.mw 1/2 h2h.fold_to_hero_3bet 1/1 h2h.steal_vs_hero 2/3",
    ),
];

/// Hand 001: the hero's CO steal is 3-bet by Carol, who c-bets into his
/// raise. Hand 002: a five-way limped pot checked to the turn; Erin mucks at
/// showdown.
const CHAT_COUNTS: &[(&str, &str)] = &[
    ("Alice", "h2h.three_bet_vs_hero_open 0/1"),
    (
        "Bob",
        "wtsd 0/1 wtsd.mw 0/1 wwsf 0/1 wwsf.mw 0/1 h2h.three_bet_vs_hero_open 0/1 h2h.defend_vs_hero_steal 0/1",
    ),
    (
        "Carol",
        "cbet_flop 1/1 wtsd 0/2 wtsd.mw 0/1 wwsf 0/2 wwsf.mw 0/1 h2h.three_bet_vs_hero_open 1/1 \
         h2h.defend_vs_hero_steal 1/1",
    ),
    ("Dave", "river_bet 0/1 river_bet.mw 0/1 wtsd 1/1 wtsd.mw 1/1 wsd 0/1 wsd.mw 0/1 wwsf 0/1 wwsf.mw 0/1"),
    ("Erin", "river_bet 0/1 river_bet.mw 0/1 wtsd 1/1 wtsd.mw 1/1 wsd 0/1 wsd.mw 0/1 wwsf 0/1 wwsf.mw 0/1"),
    (
        "Hero",
        "fold_to_cbet_flop 0/1 river_bet 0/1 river_bet.mw 0/1 wtsd 1/2 wtsd.mw 1/1 wsd 1/1 wsd.mw 1/1 \
         wwsf 2/2 wwsf.mw 1/1",
    ),
];

/// Hand 101: Olga (dead blind, then a squeeze) c-bets the hero, checks the
/// turn (the hero declines the float) and bets the river; the hero mucks.
const PLAYERS_COUNTS: &[(&str, &str)] = &[
    ("Hero", "fold_to_cbet_flop 0/1 float_flop 0/1 river_raise 0/1 wtsd 1/1 wsd 0/1 wwsf 0/1"),
    ("Ivan", "h2h.three_bet_vs_hero_open 0/1"),
    ("Judy", "h2h.three_bet_vs_hero_open 0/1"),
    ("Ken", ""),
    ("Liam", ""),
    ("Mia", ""),
    ("Ned", ""),
    ("Olga", "cbet_flop 1/1 cbet_turn 0/1 river_bet 1/1 wtsd 1/1 wsd 1/1 wwsf 1/1"),
    ("Pjotr", ""),
];

/// Hand 301: a three-way c-bet, checked turn (Tara declines the float), a
/// river bet called twice and a split pot; Quinn mucks. Hand 302: three
/// players all-in preflop, a side pot: no postflop event. Uma steals against
/// the hero's (Rosa's) small blind.
const POTS_COUNTS: &[(&str, &str)] = &[
    (
        "Quinn",
        "fold_to_cbet_flop 0/1 fold_to_cbet_flop.mw 0/1 check_raise_flop 0/1 check_raise_flop.mw 0/1 \
         donk_flop 0/1 donk_flop.mw 0/1 river_bet 0/1 river_bet.mw 0/1 river_raise 0/1 river_raise.mw 0/1 \
         wtsd 1/1 wtsd.mw 1/1 wsd 0/1 wsd.mw 0/1 wwsf 0/1 wwsf.mw 0/1 h2h.fold_to_hero_cbet 0/1",
    ),
    (
        "Rosa",
        "cbet_flop 1/1 cbet_flop.mw 1/1 cbet_turn 0/1 cbet_turn.mw 0/1 river_bet 1/1 river_bet.mw 1/1 \
         wtsd 1/1 wtsd.mw 1/1 wsd 1/1 wsd.mw 1/1 wwsf 1/1 wwsf.mw 1/1",
    ),
    (
        "Tara",
        "fold_to_cbet_flop 0/1 fold_to_cbet_flop.mw 0/1 float_flop 0/1 float_flop.mw 0/1 river_raise 0/1 \
         river_raise.mw 0/1 wtsd 1/1 wtsd.mw 1/1 wsd 1/1 wsd.mw 1/1 wwsf 1/1 wwsf.mw 1/1 \
         h2h.three_bet_vs_hero_open 0/1 h2h.fold_to_hero_cbet 0/1",
    ),
    ("Uma", "h2h.steal_vs_hero 1/1"),
];

/// A limped three-way pot: the hero bets the flop, no c-bet for anyone.
const PLAY_MONEY_COUNTS: &[(&str, &str)] = &[
    ("Hero", "wtsd 0/1 wtsd.mw 0/1 wwsf 1/1 wwsf.mw 1/1"),
    ("PlayA", "wtsd 0/1 wtsd.mw 0/1 wwsf 0/1 wwsf.mw 0/1 h2h.steal_vs_hero 0/1"),
    ("PlayB", "check_raise_flop 0/1 check_raise_flop.mw 0/1 wtsd 0/1 wtsd.mw 0/1 wwsf 0/1 wwsf.mw 0/1"),
];

/// Two blind battles: the small blind c-bets, the big blind folds.
const PREFLOP_SPOTS_COUNTS: &[(&str, &str)] = &[
    ("Ari", ""),
    ("Bel", ""),
    ("Cas", ""),
    ("Dov", ""),
    ("Eno", "cbet_flop 2/2 wtsd 0/2 wwsf 2/2"),
    ("Fyn", "fold_to_cbet_flop 2/2 wtsd 0/2 wwsf 0/2"),
];

/// Every flop of these is reached all-in preflop: no postflop event at all.
const PUSH_FOLD_COUNTS: &[(&str, &str)] =
    &[("Ace", ""), ("Bix", ""), ("Cue", ""), ("Dax", ""), ("Eli", ""), ("Fin", "")];
const MTT_ANTES_COUNTS: &[(&str, &str)] = &[
    ("Abe", ""),
    ("Bea", ""),
    ("Cal", ""),
    ("Dot", ""),
    ("TourneyHero", ""),
    ("Wade", ""),
    ("Xena", ""),
    ("Yuri", ""),
    ("Zoe", ""),
];
const ALLIN_OPEN_COUNTS: &[(&str, &str)] =
    &[("Gus", ""), ("Hana", ""), ("Ike", ""), ("Jo", ""), ("Kim", ""), ("Lou", "")];

/// Hand 801: Bob Jr donks into the isolation raiser, who raises; no c-bet
/// and no fold-to-c-bet spot. Hand 803: the hero Bob's heads-up c-bet.
/// Hand 802: Mr (BR) steals against the hero Bob.
const NAMES_COUNTS: &[(&str, &str)] = &[
    (
        "Ann: X",
        "fold_to_cbet_flop 1/1 check_raise_flop 0/1 donk_flop 0/1 wtsd 0/1 wwsf 0/1 \
         h2h.three_bet_vs_hero_open 0/1 h2h.fold_to_hero_cbet 1/1 h2h.defend_vs_hero_steal 1/1",
    ),
    ("Bob", "cbet_flop 1/1 wtsd 0/1 wwsf 1/1"),
    ("Bob Jr", "donk_flop 1/1 donk_flop.mw 1/1 wtsd 0/1 wtsd.mw 0/1 wwsf 0/1 wwsf.mw 0/1"),
    ("Bob1", "donk_flop 0/1 donk_flop.mw 0/1 wtsd 0/1 wtsd.mw 0/1 wwsf 0/1 wwsf.mw 0/1"),
    ("Mr (BR)", "h2h.steal_vs_hero 1/1"),
    ("O'Brien", ""),
    ("Zoë ゆうき", ""),
    ("[Pro] Kai", "wtsd 0/1 wtsd.mw 0/1 wwsf 1/1 wwsf.mw 1/1"),
];

const SEAT_SHAPED_COUNTS: &[(&str, &str)] =
    &[("Alice", "wtsd 0/1 wwsf 0/1"), ("Seat 1: Alice ($1 in chips)", "wtsd 0/1 wwsf 1/1")];

/// Hand 402: a heads-up limped pot, the big blind check-folds.
const HEADS_UP_COUNTS: &[(&str, &str)] =
    &[("Hal", "check_raise_flop 0/1 wtsd 0/1 wwsf 0/1"), ("Ivy", "wtsd 0/1 wwsf 1/1")];

/// The limp-reraiser and the isolating big blind are the c-bettors.
const LIMPED_POT_COUNTS: &[(&str, &str)] = &[
    ("Mo", "cbet_flop 2/2 wtsd 0/2 wwsf 2/2"),
    ("Nia", ""),
    ("Oz", ""),
    ("Pia", "fold_to_cbet_flop 1/1 wtsd 0/1 wwsf 0/1"),
    ("Quin", "fold_to_cbet_flop 1/1 wtsd 0/1 wwsf 0/1"),
    ("Rex", ""),
];

/// A four-way squeezed pot: three checks to the squeezer's c-bet, three folds.
const MULTIWAY_COUNTS: &[(&str, &str)] = &[
    (
        "Aria",
        "fold_to_cbet_flop 1/1 fold_to_cbet_flop.mw 1/1 check_raise_flop 0/1 check_raise_flop.mw 0/1 \
         donk_flop 0/1 donk_flop.mw 0/1 wtsd 0/1 wtsd.mw 0/1 wwsf 0/1 wwsf.mw 0/1",
    ),
    ("Bo", ""),
    (
        "Cy",
        "fold_to_cbet_flop 1/1 fold_to_cbet_flop.mw 1/1 check_raise_flop 0/1 check_raise_flop.mw 0/1 \
         donk_flop 0/1 donk_flop.mw 0/1 wtsd 0/1 wtsd.mw 0/1 wwsf 0/1 wwsf.mw 0/1",
    ),
    ("Di", ""),
    ("Ed", ""),
    ("Flo", "cbet_flop 1/1 cbet_flop.mw 1/1 wtsd 0/1 wtsd.mw 0/1 wwsf 1/1 wwsf.mw 1/1"),
    ("Gil", ""),
    (
        "Hu",
        "fold_to_cbet_flop 1/1 fold_to_cbet_flop.mw 1/1 check_raise_flop 0/1 check_raise_flop.mw 0/1 \
         donk_flop 0/1 donk_flop.mw 0/1 wtsd 0/1 wtsd.mw 0/1 wwsf 0/1 wwsf.mw 0/1",
    ),
    ("Io", ""),
];

/// No preflop raise: no c-bet; the checkers face a bet (check-raise spots).
const NO_OPPORTUNITY_COUNTS: &[(&str, &str)] = &[
    ("Ann", "check_raise_flop 0/1 check_raise_flop.mw 0/1 wtsd 0/1 wtsd.mw 0/1 wwsf 0/1 wwsf.mw 0/1"),
    ("Ben", "wtsd 0/1 wtsd.mw 0/1 wwsf 1/1 wwsf.mw 1/1"),
    ("Cid", ""),
    ("Dee", ""),
    ("Eve", "check_raise_flop 0/1 check_raise_flop.mw 0/1 wtsd 0/1 wtsd.mw 0/1 wwsf 0/1 wwsf.mw 0/1"),
    ("Fay", "check_raise_flop 0/1 check_raise_flop.mw 0/1 wtsd 0/1 wtsd.mw 0/1 wwsf 0/1 wwsf.mw 0/1"),
];

/// Walks: no flop for anyone.
const WALK_COUNTS: &[(&str, &str)] = &[
    ("Pat", ""),
    ("Quy", ""),
    ("Ros", ""),
    ("Sal", ""),
    ("Tom", ""),
    ("Uli", ""),
    ("Vic", ""),
    ("Wes", ""),
    ("Xia", ""),
    ("Yan", ""),
];

/// The hero's button steal is called by the small blind, whose flop
/// check-raise all-in is called; a heads-up flop after the big blind folds.
const SPIN_COUNTS: &[(&str, &str)] = &[
    ("Hero", "cbet_flop 1/1 wtsd 1/1 wsd 0/1 wwsf 0/1"),
    ("SpinA", ""),
    (
        "SpinB",
        "fold_to_cbet_flop 0/1 check_raise_flop 1/1 donk_flop 0/1 wtsd 1/1 wsd 1/1 wwsf 1/1 \
         h2h.three_bet_vs_hero_open 0/1 h2h.fold_to_hero_cbet 0/1 h2h.defend_vs_hero_steal 1/1",
    ),
];

/// The hero's Zoom button steal, folded to.
const ZOOM_COUNTS: &[(&str, &str)] = &[
    ("Fenwick", "h2h.three_bet_vs_hero_open 0/1 h2h.defend_vs_hero_steal 0/1"),
    ("Gale", "h2h.three_bet_vs_hero_open 0/1 h2h.defend_vs_hero_steal 0/1"),
    ("Hero", ""),
    ("Hollis", ""),
    ("Iona", ""),
    ("Jett", ""),
];

const FIXTURES: &[Fixture] = &[
    ("postflop_cbet_spots", CBET_SPOTS, 5, CBET_SPOTS_COUNTS),
    ("postflop_allin", POSTFLOP_ALLIN, 4, POSTFLOP_ALLIN_COUNTS),
    ("engine_postflop_spots", SPOTS, 10, SPOTS_COUNTS),
    ("cash_6max_usd_chat", CHAT, 2, CHAT_COUNTS),
    ("cash_9max_eur_players", PLAYERS, 2, PLAYERS_COUNTS),
    ("cash_pots_usd", POTS, 2, POTS_COUNTS),
    ("cash_play_money", PLAY_MONEY, 1, PLAY_MONEY_COUNTS),
    ("engine_preflop_spots", PREFLOP_SPOTS, 5, PREFLOP_SPOTS_COUNTS),
    ("engine_preflop_push_fold", PUSH_FOLD, 5, PUSH_FOLD_COUNTS),
    ("mtt_bounty_antes", MTT_ANTES, 1, MTT_ANTES_COUNTS),
    ("preflop_allin_open", ALLIN_OPEN, 2, ALLIN_OPEN_COUNTS),
    ("names_special", NAMES, 3, NAMES_COUNTS),
    ("names_seat_shaped", SEAT_SHAPED, 1, SEAT_SHAPED_COUNTS),
    ("preflop_heads_up", HEADS_UP, 3, HEADS_UP_COUNTS),
    ("preflop_limped_pot", LIMPED_POT, 2, LIMPED_POT_COUNTS),
    ("preflop_multiway", MULTIWAY, 1, MULTIWAY_COUNTS),
    ("preflop_no_opportunity", NO_OPPORTUNITY, 1, NO_OPPORTUNITY_COUNTS),
    ("preflop_walk", WALK, 2, WALK_COUNTS),
    ("spin_and_go", SPIN, 1, SPIN_COUNTS),
    ("zoom_cash_gbp", ZOOM, 1, ZOOM_COUNTS),
];

// ---------------------------------------------------------------- helpers

/// `(hits, opportunities)` per token: a postflop key (`cbet_flop`), its
/// multiway part (`cbet_flop.mw`) or a head-to-head key
/// (`h2h.fold_to_hero_cbet`). Tokens without an opportunity are absent.
type Counts = BTreeMap<String, (u32, u32)>;

fn setup_db() -> Connection {
    db::open(Path::new(":memory:")).expect("open db")
}

fn player_id(conn: &Connection, name: &str) -> Option<i64> {
    conn.query_row("SELECT id FROM players WHERE name = ?1", [name], |row| row.get(0))
        .ok()
}

fn players_with_hands(conn: &Connection) -> Vec<String> {
    let mut stmt = conn
        .prepare(
            "SELECT name FROM players p
             WHERE EXISTS (SELECT 1 FROM player_hands ph WHERE ph.player_id = p.id)
             ORDER BY name",
        )
        .unwrap();
    let rows = stmt.query_map([], |row| row.get::<_, String>(0)).unwrap();
    rows.map(Result::unwrap).collect()
}

fn h2h_token(key: H2hKey) -> String {
    format!("h2h.{}", key.as_str())
}

/// Every token in display order.
fn tokens() -> Vec<String> {
    let mut out = Vec::new();
    for key in StatKey::POSTFLOP {
        out.push(key.as_str().to_string());
        out.push(format!("{}.mw", key.as_str()));
    }
    out.extend(H2hKey::ALL.into_iter().map(h2h_token));
    out
}

fn parse(expected: &str) -> Counts {
    let known = tokens();
    let words: Vec<&str> = expected.split_whitespace().collect();
    assert!(words.len() % 2 == 0, "malformed expectation {expected:?}");
    let mut out = Counts::new();
    for pair in words.chunks(2) {
        assert!(known.iter().any(|t| t == pair[0]), "unknown token {:?}", pair[0]);
        let (n, d) = pair[1].split_once('/').expect("n/d");
        let previous = out.insert(pair[0].to_string(), (n.parse().unwrap(), d.parse().unwrap()));
        assert!(previous.is_none(), "{} listed twice in {expected:?}", pair[0]);
    }
    out
}

fn show(counts: &Counts) -> String {
    tokens()
        .iter()
        .filter_map(|t| counts.get(t).map(|(n, d)| format!("{t} {n}/{d}")))
        .collect::<Vec<_>>()
        .join(" ")
}

fn add(counts: &mut Counts, token: String, success: bool) {
    let c = counts.entry(token).or_default();
    c.0 += u32::from(success);
    c.1 += 1;
}

/// The player's counts summed from `extract_postflop`'s events and from
/// `h2h_events` over each hand's preflop and postflop events, checked
/// against `aggregate_player`'s raw all-time and head-to-head counts and
/// against `extract_player_postflop`. A player with no stored hand has none.
fn engine_counts_of(hands: &[HandFacts], id: i64, label: &str) -> Counts {
    let mut counts = Counts::new();
    let mut with_hero = 0u32;
    for hand in hands {
        let postflop = extract_postflop(hand, id);
        for event in &postflop {
            assert!(event.opportunity, "{label}: an event without an opportunity");
            assert_eq!(event.player_id, id, "{label}: an event for another player");
            let multiway = event.multiway.unwrap_or_else(|| panic!("{label}: a postflop event without multiway"));
            add(&mut counts, event.key.as_str().to_string(), event.success);
            if multiway {
                add(&mut counts, format!("{}.mw", event.key.as_str()), event.success);
            }
        }
        let mut all = extract_preflop(hand, id);
        all.extend(postflop);
        for (key, success) in h2h_events(hand, &all, id) {
            add(&mut counts, h2h_token(key), success);
        }
        with_hero += u32::from(hand.hero().is_some_and(|h| h.player_id != id));
    }

    let per_hand: usize = extract_player_postflop(hands, id).iter().map(|h| h.events.len()).sum();
    let postflop_total: u32 = StatKey::POSTFLOP.iter().map(|k| counts.get(k.as_str()).map_or(0, |c| c.1)).sum();
    assert_eq!(per_hand as u32, postflop_total, "{label}: extract_player_postflop");

    let format = hands.first().map_or(FormatKey::Cash, FormatKey::of_hand);
    let agg = aggregate_player(hands, id, format);
    for key in StatKey::POSTFLOP {
        let a = agg.counts(View::AllTime, key);
        assert_eq!(
            (a.hits, a.opportunities),
            counts.get(key.as_str()).copied().unwrap_or_default(),
            "{label}: aggregate_player and extract_postflop disagree on {}",
            key.as_str()
        );
    }
    for key in H2hKey::ALL {
        let a = agg.h2h.get(&key).copied().unwrap_or_default();
        assert_eq!(
            (a.hits, a.opportunities),
            counts.get(&h2h_token(key)).copied().unwrap_or_default(),
            "{label}: aggregate_player and h2h_events disagree on {}",
            key.as_str()
        );
    }
    assert_eq!(agg.h2h_hands, with_hero, "{label}: head-to-head hands");
    counts
}

fn engine_counts(conn: &Connection, name: &str) -> Counts {
    let Some(id) = player_id(conn, name) else {
        return Counts::new();
    };
    let hands = load_player_hands(conn, id).unwrap();
    engine_counts_of(&hands, id, name)
}

fn snapshot(conn: &Connection) -> BTreeMap<String, Counts> {
    players_with_hands(conn)
        .into_iter()
        .map(|name| {
            let c = engine_counts(conn, &name);
            (name, c)
        })
        .collect()
}

fn assert_engine_counts(conn: &Connection, fixture: &str, expected: &[(&str, &str)]) {
    for (name, want) in expected {
        assert_eq!(
            show(&engine_counts(conn, name)),
            show(&parse(want)),
            "{fixture}: engine counts of {name} (got left, expected right)"
        );
    }
    for name in players_with_hands(conn) {
        assert!(
            expected.iter().any(|(n, _)| *n == name),
            "{fixture}: player {name} has hands but no pinned expectation"
        );
    }
}

fn import_fixture(fixture: &str, text: &str, hands: i64) -> Connection {
    let mut conn = setup_db();
    let summary = import::import_text(&mut conn, text).expect("import");
    assert_eq!(
        (summary.hands_imported, summary.hands_failed, summary.hands_rejected_invalid),
        (hands, 0, 0),
        "{fixture}: import"
    );
    conn
}

/// Pins every player of a fixture, then imports it again: the duplicate
/// stores nothing and changes no count.
fn check_fixture(fixture: &Fixture) {
    let (name, text, hands, expected) = *fixture;
    let mut conn = import_fixture(name, text, hands);
    assert_engine_counts(&conn, name, expected);

    let before = snapshot(&conn);
    let again = import::import_text(&mut conn, text).expect("re-import");
    assert_eq!(
        (again.hands_imported, again.hands_skipped_duplicate),
        (0, hands),
        "{name}: duplicate import"
    );
    assert_eq!(snapshot(&conn), before, "{name}: a duplicate import changed a count");
}

fn fixture(name: &str) -> &'static Fixture {
    FIXTURES.iter().find(|f| f.0 == name).expect("fixture listed")
}

// ---------------------------------------------------------------- engine counts per fixture

#[test]
fn cbet_check_back_probe_donk_and_a_raise_before_the_answer() {
    check_fixture(fixture("postflop_cbet_spots"));
}

#[test]
fn preflop_all_in_flop_all_in_partial_call_and_side_pot() {
    check_fixture(fixture("postflop_allin"));
}

#[test]
fn barrels_delayed_cbet_float_probe_check_raise_and_hero_spots() {
    check_fixture(fixture("engine_postflop_spots"));
}

#[test]
fn split_pot_muck_and_three_way_showdowns() {
    check_fixture(fixture("cash_pots_usd"));
    check_fixture(fixture("cash_6max_usd_chat"));
    check_fixture(fixture("cash_9max_eur_players"));
}

#[test]
fn all_in_preflop_hands_give_no_postflop_event() {
    check_fixture(fixture("engine_preflop_push_fold"));
    check_fixture(fixture("mtt_bounty_antes"));
    check_fixture(fixture("preflop_allin_open"));
}

#[test]
fn limped_pots_have_no_cbet_and_raised_pots_cbet_by_the_last_raiser() {
    check_fixture(fixture("preflop_no_opportunity"));
    check_fixture(fixture("cash_play_money"));
    check_fixture(fixture("preflop_limped_pot"));
    check_fixture(fixture("preflop_heads_up"));
    check_fixture(fixture("engine_preflop_spots"));
}

#[test]
fn multiway_heads_up_and_formats() {
    check_fixture(fixture("preflop_multiway"));
    check_fixture(fixture("spin_and_go"));
    check_fixture(fixture("zoom_cash_gbp"));
    check_fixture(fixture("preflop_walk"));
}

#[test]
fn similar_and_special_character_names_keep_their_own_counts() {
    check_fixture(fixture("names_special"));
    check_fixture(fixture("names_seat_shaped"));
}

#[test]
fn every_postflop_and_head_to_head_key_is_pinned_with_hits_and_misses() {
    let mut totals = Counts::new();
    for (_, _, _, expected) in FIXTURES {
        for (_, want) in *expected {
            for (token, (n, d)) in parse(want) {
                let t = totals.entry(token).or_default();
                t.0 += n;
                t.1 += d;
            }
        }
    }
    assert_eq!(StatKey::POSTFLOP.len(), 16);
    assert_eq!(H2hKey::ALL.len(), 5);
    let keys = StatKey::POSTFLOP
        .iter()
        .map(|k| k.as_str().to_string())
        .chain(H2hKey::ALL.into_iter().map(h2h_token));
    for token in keys {
        let (hits, opportunities) = totals.get(&token).copied().unwrap_or_default();
        assert!(hits > 0, "no fixture pins a hit of {token}");
        assert!(hits < opportunities, "no fixture pins a miss of {token}");
    }
    // Both sides of the multiway flag are pinned on the c-bet chain and the
    // showdown keys.
    for key in [StatKey::CbetFlop, StatKey::FoldToCbetFlop, StatKey::CbetTurn, StatKey::Wtsd, StatKey::Wsd] {
        let all = totals[key.as_str()].1;
        let mw = totals.get(&format!("{}.mw", key.as_str())).map_or(0, |c| c.1);
        assert!(mw > 0 && mw < all, "{}: {mw} multiway of {all} pins no heads-up and multiway side", key.as_str());
    }
}

/// The same hands without the hero's `Dealt to` lines: nobody is the hero,
/// so no head-to-head count, while every postflop count stays the same.
#[test]
fn head_to_head_needs_the_hero_in_the_hand() {
    let hero_present = import_fixture("engine_postflop_spots", SPOTS, 10);
    let without: String = SPOTS.lines().filter(|l| !l.starts_with("Dealt to ")).map(|l| format!("{l}\n")).collect();
    let hero_absent = import_fixture("engine_postflop_spots without the hero", &without, 10);
    let mut h2h_seen = 0;
    for (name, _) in SPOTS_COUNTS {
        let present = engine_counts(&hero_present, name);
        let absent = engine_counts(&hero_absent, name);
        let postflop = |c: &Counts| -> Counts {
            c.iter().filter(|(t, _)| !t.starts_with("h2h.")).map(|(t, v)| (t.clone(), *v)).collect()
        };
        assert_eq!(show(&postflop(&absent)), show(&postflop(&present)), "{name}: postflop counts");
        assert!(absent.keys().all(|t| !t.starts_with("h2h.")), "{name}: h2h without the hero: {}", show(&absent));
        h2h_seen += present.keys().filter(|t| t.starts_with("h2h.")).count();

        let id = player_id(&hero_absent, name).unwrap();
        let agg = aggregate_player(&load_player_hands(&hero_absent, id).unwrap(), id, FormatKey::Cash);
        assert_eq!(agg.h2h_hands, 0, "{name}: head-to-head hands without the hero");
        let id = player_id(&hero_present, name).unwrap();
        let agg = aggregate_player(&load_player_hands(&hero_present, id).unwrap(), id, FormatKey::Cash);
        let expected = if *name == "Hero" { 0 } else { 10 };
        assert_eq!(agg.h2h_hands, expected, "{name}: head-to-head hands with the hero");
    }
    // Pax 2, Rue 3, Sol 2, Tex 3, Uri 2.
    assert_eq!(h2h_seen, 12, "head-to-head tokens pinned with the hero");
}

#[test]
fn truncated_hand_contributes_no_partial_counts() {
    // Two copies of hand 272000000105 cut before its summary: one on the
    // turn, one after the showdown lines.
    let players = ["Ana", "Bru", "Caz", "Dov", "Eli", "Fin"];
    let mut conn = setup_db();
    let cut = import::import_text(&mut conn, POSTFLOP_TRUNCATED).unwrap();
    assert_eq!(cut.hands_imported, 0, "a truncated hand was stored");
    assert_eq!(cut.hands_failed + cut.hands_rejected_invalid, 2, "{cut:?}");
    for name in players {
        assert!(engine_counts(&conn, name).is_empty(), "{name} counted from a cut hand");
    }

    // The complete file then counts exactly as pinned, and a late cut copy
    // changes nothing.
    import::import_text(&mut conn, CBET_SPOTS).unwrap();
    assert_engine_counts(&conn, "postflop_cbet_spots after cut copies", CBET_SPOTS_COUNTS);
    let before = snapshot(&conn);
    let late = import::import_text(&mut conn, POSTFLOP_TRUNCATED).unwrap();
    assert_eq!(late.hands_imported, 0);
    assert_eq!(snapshot(&conn), before, "a late cut copy changed a count");
}

#[test]
fn duplicate_import_of_every_fixture_together_changes_no_count() {
    let mut conn = setup_db();
    for (_, text, _, _) in FIXTURES {
        import::import_text(&mut conn, text).unwrap();
    }
    let before = snapshot(&conn);
    for (name, text, hands, _) in FIXTURES {
        let again = import::import_text(&mut conn, text).unwrap();
        assert_eq!((again.hands_imported, again.hands_skipped_duplicate), (0, *hands), "{name}");
    }
    assert_eq!(snapshot(&conn), before);
}

// ---------------------------------------------------------------- cross-check with stats/mod.rs

/// The postflop stats both engines compute, in `stats/mod.rs`'s terms, with
/// the engine key behind each.
const SHARED: [(&str, StatKey); 4] = [
    ("cbet", StatKey::CbetFlop),
    ("fcb", StatKey::FoldToCbetFlop),
    ("wtsd", StatKey::Wtsd),
    ("wsd", StatKey::Wsd),
];

/// `(hits, opportunities)` per [`SHARED`] stat.
type Shared = [(i64, i64); 4];

fn round1(value: f64) -> f64 {
    (value * 10.0).round() / 10.0
}

/// Recovers the numerator behind `stats/mod.rs`'s one-decimal percentage:
/// exact while the denominator is under 1,000.
fn hits(pct: Option<f64>, den: i64, label: &str) -> i64 {
    if den == 0 {
        assert!(pct.is_none(), "{label}: no opportunity must be None, got {pct:?}");
        return 0;
    }
    assert!(den < 1000, "{label}: denominator {den} too large to recover the numerator");
    let pct = pct.unwrap_or_else(|| panic!("{label}: {den} opportunities but None"));
    let n = (pct * den as f64 / 100.0).round() as i64;
    assert!(
        (round1(n as f64 / den as f64 * 100.0) - pct).abs() < 1e-9,
        "{label}: {pct}% is not a whole number of {den} opportunities"
    );
    n
}

/// `stats/mod.rs`'s counts for one player.
fn stats_side(conn: &Connection, name: &str) -> Shared {
    let Some(id) = player_id(conn, name) else {
        return [(0, 0); 4];
    };
    let (s, o) = stats::compute_player_stats_with_opportunities(conn, id).unwrap();
    let pairs = [
        (s.c_bet, o.cbet_opportunities),
        (s.fold_to_c_bet, o.faced_cbet_opportunities),
        (s.wtsd, o.saw_flop_hands),
        (s.wsd, o.went_to_showdown_hands),
    ];
    let mut out = [(0, 0); 4];
    for (i, (pct, den)) in pairs.into_iter().enumerate() {
        out[i] = (hits(pct, den, &format!("{name} {}", SHARED[i].0)), den);
    }
    out
}

fn engine_side(counts: &Counts) -> Shared {
    let mut out = [(0, 0); 4];
    for (i, (_, key)) in SHARED.iter().enumerate() {
        let (n, d) = counts.get(key.as_str()).copied().unwrap_or_default();
        out[i] = (i64::from(n), i64::from(d));
    }
    out
}

/// Why `stats/mod.rs` and the engine count a postflop spot differently. Each
/// is a documented difference of definition (`stat-contracts.md`, "Opponent
/// engine divergences (postflop)"; `opponent-engine.md` §9.2), not a defect.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Divergence {
    /// A player all-in preflop who reaches showdown: stats/mod.rs counts the
    /// hand in WTSD (a showdown means the flop was seen) and W$SD; the engine
    /// counts only hands with a postflop decision.
    PreflopAllIn,
    /// A player who covers a preflop all-in, with chips behind and nobody
    /// left to bet against: same as above, without being all-in.
    CoveredCaller,
    /// A raise between the c-bet and the player's answer: stats/mod.rs files
    /// the answer as a fold-to-c-bet spot; the engine does not.
    RaiseBeforeAnswer,
}
use Divergence::*;

/// One player's difference on one shared stat over a whole fixture:
/// `(player, stat, stats/mod.rs "n/d", engine "n/d", causes)`.
type Diff = (&'static str, &'static str, &'static str, &'static str, &'static [Divergence]);

fn fraction((n, d): (i64, i64)) -> String {
    format!("{n}/{d}")
}

/// Every `(player, stat)` where the engines differ, as `Diff`-shaped text.
fn differences(conn: &Connection) -> Vec<(String, &'static str, String, String)> {
    let mut out = Vec::new();
    for name in players_with_hands(conn) {
        let s = stats_side(conn, &name);
        let e = engine_side(&engine_counts(conn, &name));
        for (i, (stat, _)) in SHARED.iter().enumerate() {
            if s[i] != e[i] {
                out.push((name.clone(), *stat, fraction(s[i]), fraction(e[i])));
            }
        }
    }
    out
}

/// `None` when the differences are exactly `expected`, else a report.
fn divergence_report(fixture: &str, text: &str, hands: i64, expected: &[Diff]) -> Option<String> {
    let conn = import_fixture(fixture, text, hands);
    let got = differences(&conn);
    let want: Vec<(String, &str, String, String)> = expected
        .iter()
        .map(|(p, stat, s, e, causes)| {
            assert!(!causes.is_empty(), "{fixture}: {p} {stat} has no documented cause");
            (p.to_string(), *stat, s.to_string(), e.to_string())
        })
        .collect();
    let render = |list: &[(String, &str, String, String)]| {
        list.iter()
            .map(|(p, stat, s, e)| format!("    ({p:?}, {stat:?}, {s:?}, {e:?}, &[]),"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    (got != want).then(|| {
        format!(
            "{fixture}: differences between stats/mod.rs and the engine\n  got:\n{}\n  expected:\n{}",
            render(&got),
            render(&want)
        )
    })
}

#[test]
fn edge_fixture_differences_are_all_documented_divergences() {
    let reports: Vec<String> = FIXTURES
        .iter()
        .filter_map(|(fixture, text, hands, _)| {
            divergence_report(fixture, text, *hands, divergences_of(fixture))
        })
        .collect();
    assert!(reports.is_empty(), "{}", reports.join("\n"));
}

/// Every difference per fixture, traced by hand. A fixture not listed has
/// none.
fn divergences_of(fixture: &str) -> &'static [Diff] {
    match fixture {
        // Hand 104: Dov raises Ana's c-bet before Fin folds. Hand 105: Eli
        // raises Bru's c-bet before Ana calls.
        "postflop_cbet_spots" => &[
            ("Ana", "fcb", "0/1", "0/0", &[RaiseBeforeAnswer]),
            ("Fin", "fcb", "1/1", "0/0", &[RaiseBeforeAnswer]),
        ],
        // Hand 201: Lars calls all-in for less, Kye covers him. Hand 203:
        // Lars is all-in and wins the main pot.
        "postflop_allin" => &[
            ("Kye", "wtsd", "2/2", "1/1", &[CoveredCaller]),
            ("Kye", "wsd", "2/2", "1/1", &[CoveredCaller]),
            ("Lars", "wtsd", "2/2", "0/0", &[PreflopAllIn]),
            ("Lars", "wsd", "1/2", "0/0", &[PreflopAllIn]),
        ],
        // Hand 302: all three players all-in preflop.
        "cash_pots_usd" => &[
            ("Quinn", "wtsd", "2/2", "1/1", &[PreflopAllIn]),
            ("Quinn", "wsd", "1/2", "0/1", &[PreflopAllIn]),
            ("Rosa", "wtsd", "2/2", "1/1", &[PreflopAllIn]),
            ("Rosa", "wsd", "2/2", "1/1", &[PreflopAllIn]),
            ("Tara", "wtsd", "2/2", "1/1", &[PreflopAllIn]),
            ("Tara", "wsd", "1/2", "1/1", &[PreflopAllIn]),
        ],
        // Hand 101: Cue shoves, Eli covers him. Hand 102: Bix shoves, Ace
        // calls all-in.
        "engine_preflop_push_fold" => &[
            ("Ace", "wtsd", "1/1", "0/0", &[PreflopAllIn]),
            ("Ace", "wsd", "1/1", "0/0", &[PreflopAllIn]),
            ("Bix", "wtsd", "1/1", "0/0", &[PreflopAllIn]),
            ("Bix", "wsd", "0/1", "0/0", &[PreflopAllIn]),
            ("Cue", "wtsd", "1/1", "0/0", &[PreflopAllIn]),
            ("Cue", "wsd", "1/1", "0/0", &[PreflopAllIn]),
            ("Eli", "wtsd", "1/1", "0/0", &[CoveredCaller]),
            ("Eli", "wsd", "0/1", "0/0", &[CoveredCaller]),
        ],
        // Dot and Yuri are all-in; the hero covers both and wins a side pot.
        "mtt_bounty_antes" => &[
            ("Dot", "wtsd", "1/1", "0/0", &[PreflopAllIn]),
            ("Dot", "wsd", "1/1", "0/0", &[PreflopAllIn]),
            ("TourneyHero", "wtsd", "1/1", "0/0", &[CoveredCaller]),
            ("TourneyHero", "wsd", "1/1", "0/0", &[CoveredCaller]),
            ("Yuri", "wtsd", "1/1", "0/0", &[PreflopAllIn]),
            ("Yuri", "wsd", "0/1", "0/0", &[PreflopAllIn]),
        ],
        // Hand 201: Ike's all-in open, Lou's all-in call.
        "preflop_allin_open" => &[
            ("Ike", "wtsd", "1/1", "0/0", &[PreflopAllIn]),
            ("Ike", "wsd", "0/1", "0/0", &[PreflopAllIn]),
            ("Lou", "wtsd", "1/1", "0/0", &[PreflopAllIn]),
            ("Lou", "wsd", "1/1", "0/0", &[PreflopAllIn]),
        ],
        _ => &[],
    }
}

// ---------------------------------------------------------------- clean corpus

/// A small xorshift generator: the corpus is the same on every run.
struct Rng(u64);

impl Rng {
    fn below(&mut self, n: usize) -> usize {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 % n as u64) as usize
    }

    fn one_in(&mut self, n: usize) -> bool {
        self.below(n) == 0
    }
}

/// The seated names clockwise from the small blind (the button last).
fn seat_order(t: &Table) -> Vec<String> {
    let mut seats = t.seats.clone();
    seats.sort();
    let split = seats.iter().position(|(s, _)| *s > t.button).unwrap_or(seats.len());
    seats.rotate_left(split);
    seats.into_iter().map(|(_, n)| n).collect()
}

/// Moves the button one seat clockwise.
fn next_button(t: &mut Table) {
    let next = t.seats.iter().map(|(s, _)| *s).filter(|s| *s > t.button).min();
    t.button = next.unwrap_or(1);
}

/// One betting round among `live` (in acting order, indices into the
/// preflop order), at most one raise and only heads-up. Returns the actions
/// and the players still in.
fn betting_round(rng: &mut Rng, live: &[usize]) -> (Vec<(usize, Act)>, Vec<usize>) {
    let heads_up = live.len() == 2;
    let mut lines = Vec::new();
    let mut folded: Vec<usize> = Vec::new();
    let mut level = 0.0;
    let mut put: BTreeMap<usize, f64> = BTreeMap::new();
    let mut raised = false;
    let mut pending: Vec<usize> = live.to_vec();
    while !pending.is_empty() {
        let p = pending.remove(0);
        let in_hand = live.len() - folded.len();
        if in_hand < 2 {
            break;
        }
        let mine = put.get(&p).copied().unwrap_or(0.0);
        // Everyone else still in, in acting order after `p`.
        let behind = |folded: &[usize]| -> Vec<usize> {
            let at = live.iter().position(|x| *x == p).unwrap();
            (1..live.len()).map(|k| live[(at + k) % live.len()]).filter(|x| !folded.contains(x)).collect()
        };
        if level == mine {
            if rng.one_in(3) {
                level = [1.0, 2.0, 3.0][rng.below(3)];
                put.insert(p, level);
                lines.push((p, Act::Bet(level)));
                pending = behind(&folded);
            } else {
                lines.push((p, Act::Check));
            }
        } else {
            match rng.below(8) {
                0..=2 => {
                    lines.push((p, Act::Fold));
                    folded.push(p);
                }
                3 if heads_up && !raised => {
                    raised = true;
                    level *= 3.0;
                    put.insert(p, level);
                    lines.push((p, Act::Raise(level)));
                    pending = behind(&folded);
                }
                _ => {
                    put.insert(p, level);
                    lines.push((p, Act::Call));
                }
            }
        }
    }
    let still: Vec<usize> = live.iter().copied().filter(|x| !folded.contains(x)).collect();
    (lines, still)
}

/// One hand of the clean corpus: every stack 100bb, no all-in. Preflop is a
/// limped pot, a single raise with callers, or a big-blind 3-bet called by
/// the opener; at most four players see the flop. Postflop, raises happen
/// only heads-up, so a raise never separates a c-bet from an answer to it.
fn postflop_hand(t: &mut Table, rng: &mut Rng, minute: i64) -> String {
    let start = 100.0 * t.bb;
    for stack in t.stacks.values_mut() {
        *stack = start;
    }
    let order = seat_order(t);
    let n = order.len();
    // Preflop acting order: UTG first, the blinds last.
    let pre: Vec<String> = order[2..].iter().chain(order[..2].iter()).cloned().collect();
    let (sb, bb) = (n - 2, n - 1);

    let mut lines: Vec<(usize, Act)> = Vec::new();
    let first = rng.below(bb);
    lines.extend((0..first).map(|i| (i, Act::Fold)));
    let mut live: Vec<usize> = vec![first];
    if rng.one_in(5) {
        lines.push((first, Act::Call));
        for i in first + 1..bb {
            if live.len() < 3 && rng.one_in(2) {
                lines.push((i, Act::Call));
                live.push(i);
            } else {
                lines.push((i, Act::Fold));
            }
        }
        lines.push((bb, Act::Check));
        live.push(bb);
    } else {
        lines.push((first, Act::Raise(2.5)));
        for i in first + 1..bb {
            if live.len() < 3 && rng.one_in(3) {
                lines.push((i, Act::Call));
                live.push(i);
            } else {
                lines.push((i, Act::Fold));
            }
        }
        match rng.below(4) {
            0 => lines.push((bb, Act::Fold)),
            1 if live.len() == 1 && first != sb => {
                lines.push((bb, Act::Raise(9.0)));
                lines.push((first, Act::Call));
                live.push(bb);
            }
            _ => {
                lines.push((bb, Act::Call));
                live.push(bb);
            }
        }
    }

    // Postflop the small blind acts first.
    live.sort_by_key(|&i| (i + 2) % n);
    let mut streets: Vec<Vec<(usize, Act)>> = vec![lines];
    for _ in 0..3 {
        if live.len() < 2 {
            break;
        }
        let (actions, still) = betting_round(rng, &live);
        streets.push(actions);
        live = still;
    }
    let named: Vec<Vec<Line>> = streets
        .iter()
        .map(|s| s.iter().map(|&(i, act)| (pre[i].as_str(), act)).collect())
        .collect();
    let refs: Vec<&[Line]> = named.iter().map(|s| s.as_slice()).collect();
    let winner = (live.len() > 1).then(|| pre[live[rng.below(live.len())]].clone());
    let text = t.play(minute, &refs, winner.as_deref());
    next_button(t);
    text
}

/// A table of `players`, `hands` clean hands one minute apart.
fn clean_table(name: &str, first_id: u64, players: &[&str], hands: usize, seed: u64) -> String {
    let mut t = Table::cash(name, first_id, players);
    t.max = if players.len() > 6 { 9 } else { 6 };
    let mut rng = Rng(seed);
    (0..hands).map(|i| postflop_hand(&mut t, &mut rng, i as i64)).collect()
}

const SIX_MAX: [&str; 6] = ["Hero", "Sam", "Tess", "Ugo", "Vera", "Walt"];
const NINE_MAX: [&str; 9] = ["Abby", "Bram", "Cleo", "Dirk", "Edda", "Finn", "Gwen", "Hugo", "Ines"];
const SHORT_HANDED: [&str; 3] = ["Jago", "Kira", "Lars"];

/// 300 6-max hands, 180 9-max hands and 120 three-handed hands.
fn clean_corpus() -> (Connection, i64) {
    let text = [
        clean_table("Postflop Six", 291_000_000_001, &SIX_MAX, 300, 0x9E37_79B9_7F4A_7C15),
        clean_table("Postflop Nine", 292_000_000_001, &NINE_MAX, 180, 0x2545_F491_4F6C_DD1D),
        clean_table("Postflop Three", 293_000_000_001, &SHORT_HANDED, 120, 0x1405_7B7E_F767_814F),
    ]
    .concat();
    let mut conn = setup_db();
    let summary = import::import_text(&mut conn, &text).expect("import clean corpus");
    assert_eq!(
        (summary.hands_failed, summary.hands_rejected_invalid, summary.hands_skipped_duplicate),
        (0, 0, 0),
        "{summary:?}"
    );
    (conn, summary.hands_imported)
}

fn corpus_players() -> impl Iterator<Item = &'static str> {
    SIX_MAX.into_iter().chain(NINE_MAX).chain(SHORT_HANDED)
}

#[test]
fn clean_corpus_engines_agree_exactly_on_cbet_fold_to_cbet_wtsd_and_wsd() {
    let (conn, hands) = clean_corpus();
    assert_eq!(hands, 600);
    let all_in: i64 = conn.query_row("SELECT COUNT(*) FROM actions WHERE is_all_in != 0", [], |r| r.get(0)).unwrap();
    assert_eq!(all_in, 0, "the clean corpus has an all-in");

    let mut totals = [(0i64, 0i64); 4];
    let mut multiway = [0i64; 4];
    for name in corpus_players() {
        let counts = engine_counts(&conn, name);
        let engine = engine_side(&counts);
        let stats = stats_side(&conn, name);
        for (i, (stat, key)) in SHARED.iter().enumerate() {
            assert_eq!(
                fraction(stats[i]),
                fraction(engine[i]),
                "{name} {stat}: stats/mod.rs (left) and the engine (right) disagree"
            );
            totals[i].0 += engine[i].0;
            totals[i].1 += engine[i].1;
            multiway[i] += counts.get(&format!("{}.mw", key.as_str())).map_or(0, |c| i64::from(c.1));
        }
    }

    // The agreement means something: every shared stat has hits and misses,
    // heads-up and multiway.
    for (i, (stat, _)) in SHARED.iter().enumerate() {
        let (hits, opps) = totals[i];
        assert!(hits > 0 && hits < opps, "clean corpus {stat}: {hits}/{opps} pins no hit or no miss");
        assert!(multiway[i] > 0 && multiway[i] < opps, "clean corpus {stat}: {} multiway of {opps}", multiway[i]);
    }
}

// ---------------------------------------------------------------- pot reconstruction

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

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-6
}

/// Every stored hand's `(hand ref, Total pot − replayed pot)`, after checking
/// that the derived uncalled bets equal the `Uncalled bet` lines.
fn pot_gaps(conn: &Connection, label: &str) -> Vec<(String, f64)> {
    let mut gaps = Vec::new();
    for hand in load_hands_after(conn, 0, i64::MAX).unwrap() {
        let raw: String = conn
            .query_row("SELECT raw_text FROM hands WHERE id = ?1", [hand.id], |row| row.get(0))
            .unwrap();
        let replay = replay_pot(&hand);
        let total = parse_total_pot(&raw).unwrap_or_else(|| panic!("{label} {}: no Total pot line", hand.hand_ref));
        let mut derived: Vec<f64> = replay.uncalled.iter().map(|u| u.amount).collect();
        derived.sort_by(f64::total_cmp);
        let logged = uncalled_lines(&raw);
        assert!(
            derived.len() == logged.len() && derived.iter().zip(&logged).all(|(d, l)| close(*d, *l)),
            "{label} {}: uncalled bets {derived:?}, logged {logged:?}",
            hand.hand_ref
        );
        gaps.push((hand.hand_ref.clone(), total - replay.total));
    }
    gaps
}

/// The dead blind (`posts small & big blinds`) is not a stored action: a
/// hand with one replays short by it, less its live big-blind part when the
/// poster later raises (a raise-to is counted from zero). The documented
/// known limit; every other audit hand replays exactly.
#[test]
fn pot_reconstruction_matches_every_audit_fixture_but_the_dead_blind_gap() {
    let dead_blind_gaps = [("270000000101", 0.01), ("273000000110", 0.75)];
    let mut checked = 0;
    for (name, text, hands, _) in FIXTURES {
        let conn = import_fixture(name, text, *hands);
        for (hand_ref, gap) in pot_gaps(&conn, name) {
            let raw: String = conn
                .query_row("SELECT raw_text FROM hands WHERE hand_id = ?1", [&hand_ref], |row| row.get(0))
                .unwrap();
            let expected = dead_blind_gaps.iter().find(|(h, _)| *h == hand_ref).map_or(0.0, |(_, g)| *g);
            assert_eq!(raw.contains("posts small & big blinds"), expected > 0.0, "{name} {hand_ref}: dead blind");
            assert!(close(gap, expected), "{name} {hand_ref}: Total pot − replay = {gap}, expected {expected}");
            checked += 1;
        }
    }
    assert_eq!(checked, 54, "audit hands checked");
}

#[test]
fn pot_reconstruction_matches_every_c1_generated_hand() {
    // The C1 corpus of `parser_corpus_tests.rs`: same generator, seed and
    // sizes, into a temporary folder.
    let dir = tempfile::tempdir().expect("temp dir");
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().expect("repo root").to_path_buf();
    let mut conn = setup_db();
    for (format, tables, hands) in [("cash", 3, 120), ("zoom", 1, 300), ("mtt", 2, 150), ("spin", 2, 150)] {
        let out = dir.path().join(format);
        let output = Command::new("node")
            .current_dir(&root)
            .arg("scripts/sim/generate.mjs")
            .args(["--out", out.to_str().expect("utf-8 path")])
            .args(["--format", format, "--seed", "parser-corpus"])
            .args(["--tables", &tables.to_string(), "--hands", &hands.to_string()])
            .args(["--pace", "0", "--manifest"])
            .output()
            .expect("run node scripts/sim/generate.mjs (is node on PATH?)");
        assert!(output.status.success(), "generator failed: {}", String::from_utf8_lossy(&output.stderr));
        let summary = import::import_directory(&mut conn, &out).expect("import corpus");
        assert_eq!((summary.hands_failed, summary.hands_rejected_invalid), (0, 0), "{format}: {summary:?}");
    }
    let gaps = pot_gaps(&conn, "C1");
    assert_eq!(gaps.len(), 1260, "C1 hands checked");
    let off: Vec<&(String, f64)> = gaps.iter().filter(|(_, g)| !close(*g, 0.0)).collect();
    assert!(off.is_empty(), "replayed pot differs from Total pot on {} C1 hands, first {:?}", off.len(), off.first());
}

#[test]
fn pot_reconstruction_matches_the_clean_corpus() {
    let (conn, hands) = clean_corpus();
    let gaps = pot_gaps(&conn, "clean corpus");
    assert_eq!(gaps.len() as i64, hands);
    assert!(gaps.iter().all(|(_, g)| close(*g, 0.0)), "clean corpus pot gaps");
}
