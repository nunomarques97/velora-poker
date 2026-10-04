//! Audit of `stats/mod.rs`'s preflop stats (Run A, task S1).
//!
//! Every test pins the numerator AND the denominator of all thirteen preflop
//! stats for every player of a fixture, worked out by hand from the hand
//! history. Denominators come from `compute_player_stats_with_opportunities`.
//! Numerators are not exposed, so they are recovered from the percentage and
//! its denominator (exact for any denominator under 1,000, and checked to
//! round back to the same percentage); VPIP and PFR are also recounted
//! independently from the stored actions.
//!
//! Expectations are written as `"vpip 1/2 pfr 0/2 3b 0/1"`. A stat that is not
//! listed must have no opportunity at all: denominator 0 and `None`, never
//! `Some(0.0)`.

use std::collections::BTreeMap;

use rusqlite::Connection;
use velora_poker_lib::{db, import, stats};

const NO_OPPORTUNITY: &str = include_str!("fixtures/audit/preflop_no_opportunity.txt");
const ALLIN_OPEN: &str = include_str!("fixtures/audit/preflop_allin_open.txt");
const LIMPED_POT: &str = include_str!("fixtures/audit/preflop_limped_pot.txt");
const HEADS_UP: &str = include_str!("fixtures/audit/preflop_heads_up.txt");
const MULTIWAY: &str = include_str!("fixtures/audit/preflop_multiway.txt");
const WALK: &str = include_str!("fixtures/audit/preflop_walk.txt");
const TRUNCATED: &str = include_str!("fixtures/audit/preflop_truncated.txt");
// Hand-written parser fixtures of task C2, reused for their preflop shapes.
const POTS: &str = include_str!("fixtures/audit/cash_pots_usd.txt");
const MTT_ANTES: &str = include_str!("fixtures/audit/mtt_bounty_antes.txt");
const PLAYERS: &str = include_str!("fixtures/audit/cash_9max_eur_players.txt");
const NAMES: &str = include_str!("fixtures/audit/names_special.txt");
const CHAT: &str = include_str!("fixtures/audit/cash_6max_usd_chat.txt");
const ZOOM: &str = include_str!("fixtures/audit/zoom_cash_gbp.txt");
const SPIN: &str = include_str!("fixtures/audit/spin_and_go.txt");
const PLAY_MONEY: &str = include_str!("fixtures/audit/cash_play_money.txt");
const SEAT_SHAPED: &str = include_str!("fixtures/audit/names_seat_shaped.txt");
const STRADDLE: &str = include_str!("fixtures/audit/unsupported_straddle.txt");
const CURRENCY: &str = include_str!("fixtures/audit/unsupported_currency.txt");
const NON_ENGLISH: &str = include_str!("fixtures/audit/unsupported_non_english.txt");
const OTHER_ROOM: &str = include_str!("fixtures/audit/other_room.txt");

/// Every audit fixture, then every fixture `stats_tests.rs` and
/// `tournament_stats_tests.rs` use: the invariants hold over all of them.
const ALL_FIXTURES: &[(&str, &str)] = &[
    ("preflop_no_opportunity", NO_OPPORTUNITY),
    ("preflop_allin_open", ALLIN_OPEN),
    ("preflop_limped_pot", LIMPED_POT),
    ("preflop_heads_up", HEADS_UP),
    ("preflop_multiway", MULTIWAY),
    ("preflop_walk", WALK),
    ("preflop_truncated", TRUNCATED),
    ("cash_pots_usd", POTS),
    ("mtt_bounty_antes", MTT_ANTES),
    ("cash_9max_eur_players", PLAYERS),
    ("names_special", NAMES),
    ("cash_6max_usd_chat", CHAT),
    ("zoom_cash_gbp", ZOOM),
    ("spin_and_go", SPIN),
    ("cash_play_money", PLAY_MONEY),
    ("names_seat_shaped", SEAT_SHAPED),
    ("unsupported_straddle", STRADDLE),
    ("unsupported_currency", CURRENCY),
    ("unsupported_non_english", NON_ENGLISH),
    ("other_room", OTHER_ROOM),
    ("hand_3bet_showdown", include_str!("fixtures/hand_3bet_showdown.txt")),
    ("hand_cbet_fold", include_str!("fixtures/hand_cbet_fold.txt")),
    ("hand_limped_multiway", include_str!("fixtures/hand_limped_multiway.txt")),
    ("hand_fold_to_3bet", include_str!("fixtures/hand_fold_to_3bet.txt")),
    ("hand_allin_preflop_folded", include_str!("fixtures/hand_allin_preflop_folded.txt")),
    ("hand_allin_preflop_showdown", include_str!("fixtures/hand_allin_preflop_showdown.txt")),
    ("hand_4bet", include_str!("fixtures/hand_4bet.txt")),
    ("hand_4bet_pileup", include_str!("fixtures/hand_4bet_pileup.txt")),
    ("hand_limp_behind_limp", include_str!("fixtures/hand_limp_behind_limp.txt")),
    ("hand_walk", include_str!("fixtures/hand_walk.txt")),
    ("hand_cold_call", include_str!("fixtures/hand_cold_call.txt")),
    ("hand_cold_call_after_limp", include_str!("fixtures/hand_cold_call_after_limp.txt")),
    ("hand_squeeze", include_str!("fixtures/hand_squeeze.txt")),
    ("hand_heads_up_steal", include_str!("fixtures/hand_heads_up_steal.txt")),
    (
        "hand_isolation_raise_not_a_steal",
        include_str!("fixtures/hand_isolation_raise_not_a_steal.txt"),
    ),
    ("tournament_showdown_allin", include_str!("fixtures/tournament_showdown_allin.txt")),
    ("tournament_uncontested_walk", include_str!("fixtures/tournament_uncontested_walk.txt")),
    (
        "tournament_allin_preflop_disconnect",
        include_str!("fixtures/tournament_allin_preflop_disconnect.txt"),
    ),
    ("tournament_zoom_header", include_str!("fixtures/tournament_zoom_header.txt")),
];

/// The thirteen preflop stats, in the order `Counts` stores them.
const STATS: [&str; 13] = [
    "vpip", "pfr", "3b", "f3b", "4b", "f4b", "rfi", "limp", "cc", "sqz", "fsqz", "steal", "fts",
];
const VPIP: usize = 0;
const PFR: usize = 1;
const RFI: usize = 6;
const LIMP: usize = 7;

/// `(numerator, denominator)` per stat, in `STATS` order.
type Counts = [(i64, i64); 13];

fn setup_db() -> Connection {
    db::open(std::path::Path::new(":memory:")).expect("open db")
}

fn player_id(conn: &Connection, name: &str) -> Option<i64> {
    conn.query_row("SELECT id FROM players WHERE name = ?1", [name], |row| row.get(0))
        .ok()
}

fn round1(value: f64) -> f64 {
    (value * 10.0).round() / 10.0
}

/// Recovers the numerator behind a one-decimal percentage. Exact while the
/// denominator is under 1,000: two numerators then differ by at least 0.1%.
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

/// Counts for one player; a player with no row at all has none.
fn counts(conn: &Connection, name: &str) -> Counts {
    let Some(id) = player_id(conn, name) else {
        return [(0, 0); 13];
    };
    let (s, o) = stats::compute_player_stats_with_opportunities(conn, id).unwrap();
    let pairs = [
        (s.vpip, o.hands),
        (s.pfr, o.hands),
        (s.three_bet, o.three_bet_opportunities),
        (s.fold_to_three_bet, o.faced_3bet_opportunities),
        (s.four_bet, o.four_bet_opportunities),
        (s.fold_to_four_bet, o.faced_4bet_opportunities),
        (s.rfi, o.rfi_limp_opportunities),
        (s.limp, o.rfi_limp_opportunities),
        (s.cold_call, o.cold_call_opportunities),
        (s.squeeze, o.squeeze_opportunities),
        (s.fold_to_squeeze, o.faced_squeeze_opportunities),
        (s.steal_attempt, o.steal_attempt_opportunities),
        (s.fold_to_steal, o.fold_to_steal_opportunities),
    ];
    let mut out = [(0, 0); 13];
    for (i, (pct, den)) in pairs.into_iter().enumerate() {
        out[i] = (hits(pct, den, &format!("{name} {}", STATS[i])), den);
    }

    // Test-local recount of the two numerators whose rule is simple enough to
    // restate in SQL: a preflop call/bet/raise (VPIP), a preflop raise (PFR).
    let recount = |types: &str| -> i64 {
        conn.query_row(
            &format!(
                "SELECT COUNT(DISTINCT hand_id) FROM actions
                 WHERE player_id = ?1 AND street = 'preflop' AND action_type IN ({types})"
            ),
            [id],
            |row| row.get(0),
        )
        .unwrap()
    };
    assert_eq!(out[VPIP].0, recount("'call', 'bet', 'raise'"), "{name} vpip recount");
    assert_eq!(out[PFR].0, recount("'raise'"), "{name} pfr recount");
    out
}

fn parse(expected: &str) -> Counts {
    let mut out = [(0, 0); 13];
    let tokens: Vec<&str> = expected.split_whitespace().collect();
    assert!(tokens.len() % 2 == 0, "malformed expectation {expected:?}");
    for pair in tokens.chunks(2) {
        let i = STATS
            .iter()
            .position(|s| *s == pair[0])
            .unwrap_or_else(|| panic!("unknown stat {:?}", pair[0]));
        let (n, d) = pair[1].split_once('/').expect("n/d");
        out[i] = (n.parse().unwrap(), d.parse().unwrap());
    }
    out
}

fn show(c: &Counts) -> String {
    STATS
        .iter()
        .zip(c)
        .filter(|(_, (_, d))| *d != 0)
        .map(|(s, (n, d))| format!("{s} {n}/{d}"))
        .collect::<Vec<_>>()
        .join(" ")
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

fn snapshot(conn: &Connection) -> BTreeMap<String, Counts> {
    players_with_hands(conn)
        .into_iter()
        .map(|name| {
            let c = counts(conn, &name);
            (name, c)
        })
        .collect()
}

fn assert_counts(conn: &Connection, fixture: &str, expected: &[(&str, &str)]) {
    for (name, want) in expected {
        let got = counts(conn, name);
        assert_eq!(
            show(&got),
            show(&parse(want)),
            "{fixture}: counts of {name} (got left, expected right)"
        );
    }
    for name in players_with_hands(conn) {
        assert!(
            expected.iter().any(|(n, _)| *n == name),
            "{fixture}: player {name} has hands but no pinned expectation"
        );
    }
}

/// Imports a fixture into a fresh database, pins every player, then imports
/// it again: the second import stores nothing and changes no count.
fn check_fixture(fixture: &str, text: &str, hands: i64, expected: &[(&str, &str)]) {
    let mut conn = setup_db();
    let first = import::import_text(&mut conn, text).expect("import");
    assert_eq!(
        (first.hands_imported, first.hands_failed, first.hands_rejected_invalid),
        (hands, 0, 0),
        "{fixture}: first import"
    );
    assert_counts(&conn, fixture, expected);

    let before = snapshot(&conn);
    let second = import::import_text(&mut conn, text).expect("re-import");
    assert_eq!(
        (second.hands_imported, second.hands_skipped_duplicate),
        (0, hands),
        "{fixture}: duplicate import"
    );
    assert_eq!(snapshot(&conn), before, "{fixture}: a duplicate import changed a count");
}

#[test]
fn no_opportunity_hand_reports_none_never_zero() {
    // A limped pot with no raise: only the first limper (UTG) had a
    // first-in decision. Everyone else has VPIP/PFR and nothing more.
    check_fixture(
        "preflop_no_opportunity",
        NO_OPPORTUNITY,
        1,
        &[
            ("Ann", "vpip 1/1 pfr 0/1 rfi 0/1 limp 1/1"),
            ("Ben", "vpip 1/1 pfr 0/1"),
            ("Cid", "vpip 0/1 pfr 0/1"),
            ("Dee", "vpip 0/1 pfr 0/1"),
            ("Eve", "vpip 1/1 pfr 0/1"),
            ("Fay", "vpip 0/1 pfr 0/1"),
        ],
    );

    // The same, read field by field: the big blind who checks her option has
    // no opportunity for any preflop stat, so every one of them is None.
    let mut conn = setup_db();
    import::import_text(&mut conn, NO_OPPORTUNITY).unwrap();
    let s = stats::compute_player_stats(&conn, player_id(&conn, "Fay").unwrap()).unwrap();
    assert_eq!(s.vpip, Some(0.0));
    assert_eq!(s.pfr, Some(0.0));
    for (label, value) in [
        ("three_bet", s.three_bet),
        ("fold_to_three_bet", s.fold_to_three_bet),
        ("four_bet", s.four_bet),
        ("fold_to_four_bet", s.fold_to_four_bet),
        ("rfi", s.rfi),
        ("limp", s.limp),
        ("cold_call", s.cold_call),
        ("squeeze", s.squeeze),
        ("fold_to_squeeze", s.fold_to_squeeze),
        ("steal_attempt", s.steal_attempt),
        ("fold_to_steal", s.fold_to_steal),
    ] {
        assert_eq!(value, None, "Fay {label}");
    }
}

#[test]
fn all_in_open_counts_as_a_raise_first_in_and_a_partial_call_as_a_cold_call() {
    // Hand 1: CO shoves first in (RFI and steal), the short big blind calls
    // all-in for less (a cold call, and a fold-to-steal spot not folded).
    // Hand 2: the small blind shoves first in (steal) and the big blind folds.
    check_fixture(
        "preflop_allin_open",
        ALLIN_OPEN,
        2,
        &[
            ("Gus", "vpip 0/2 pfr 0/2 3b 0/1 cc 0/1 rfi 0/1 limp 0/1 fts 1/1"),
            ("Hana", "vpip 0/2 pfr 0/2 rfi 0/2 limp 0/2"),
            ("Ike", "vpip 1/2 pfr 1/2 rfi 1/2 limp 0/2 steal 1/1"),
            ("Jo", "vpip 0/2 pfr 0/2 3b 0/1 cc 0/1 rfi 0/1 limp 0/1 steal 0/1"),
            ("Kim", "vpip 0/2 pfr 0/2 3b 0/1 cc 0/1 fts 1/1 rfi 0/1 limp 0/1 steal 0/1"),
            ("Lou", "vpip 2/2 pfr 1/2 3b 0/1 cc 1/1 fts 0/1 rfi 1/1 limp 0/1 steal 1/1"),
        ],
    );
}

#[test]
fn all_in_three_bet_four_bet_and_partial_call_with_the_documented_pileup() {
    // Hand 301: CO opens (steal), BTN cold-calls, SB folds (squeeze spot,
    // fold to steal), BB calls. Hand 302: CO opens, BTN 3-bets all-in, SB
    // 4-bets all-in, BB calls all-in for less (cold call at the fourth raise
    // level), the opener folds. The opener faced a 4-bet by then, but the
    // fold is filed under Fold-to-3-bet: the documented pileup case. The
    // all-in 3-bettor never acts again, so has no Fold-to-4-bet spot.
    check_fixture(
        "cash_pots_usd",
        POTS,
        2,
        &[
            ("Quinn", "vpip 2/2 pfr 1/2 3b 1/2 cc 1/2 sqz 0/1 fts 0/1"),
            ("Rosa", "vpip 2/2 pfr 2/2 rfi 1/1 limp 0/1 steal 1/1 4b 1/1 cc 0/1"),
            ("Tara", "vpip 2/2 pfr 0/2 3b 0/1 cc 2/2"),
            (
                "Uma",
                "vpip 1/2 pfr 1/2 3b 0/1 f3b 1/1 rfi 1/1 limp 0/1 cc 0/1 sqz 0/1 steal 1/1 fts 1/1",
            ),
        ],
    );
}

#[test]
fn limped_pot_isolation_raise_and_limp_reraise() {
    // Hand 301: UTG limps, HJ limps behind, CO raises over the limps (an
    // isolation raise: no RFI, but it makes CO the opener), BTN cold-calls,
    // the blinds fold, UTG limp-reraises (a 3-bet, not a squeeze: UTG had
    // already put money in), HJ folds, CO folds (fold to 3-bet and fold to
    // squeeze, since BTN had called CO's raise), BTN calls.
    // Hand 302: BTN limps first in, SB completes, BB raises, BTN calls, SB folds.
    check_fixture(
        "preflop_limped_pot",
        LIMPED_POT,
        2,
        &[
            ("Mo", "vpip 2/2 pfr 2/2 rfi 0/1 limp 1/1 3b 1/1"),
            ("Nia", "vpip 1/2 pfr 0/2 4b 0/1 rfi 0/1 limp 0/1"),
            ("Oz", "vpip 1/2 pfr 1/2 f3b 1/1 fsqz 1/1 4b 0/1 rfi 0/1 limp 0/1"),
            ("Pia", "vpip 1/2 pfr 0/2 3b 0/1 cc 1/1 4b 0/1 rfi 0/1 limp 0/1 steal 0/1"),
            ("Quin", "vpip 1/2 pfr 0/2 3b 0/2 cc 0/1 sqz 0/1 rfi 0/1 limp 1/1 steal 0/1"),
            ("Rex", "vpip 1/2 pfr 0/2 3b 0/2 cc 0/1 sqz 0/1"),
        ],
    );
}

#[test]
fn heads_up_button_is_the_small_blind_and_its_open_is_a_steal() {
    // Heads-up the button posts the small blind and is labelled BTN (a steal
    // seat); the other player is BB. Hand 401: open, 3-bet, 4-bet, fold.
    // Hand 402: the button completes, the big blind checks. Hand 403: the
    // button folds, a walk: the big blind has no decision at all.
    check_fixture(
        "preflop_heads_up",
        HEADS_UP,
        3,
        &[
            ("Hal", "vpip 1/3 pfr 1/3 rfi 1/2 limp 0/2 steal 1/2 f3b 0/1 4b 1/1"),
            (
                "Ivy",
                "vpip 2/3 pfr 1/3 3b 1/1 cc 0/1 fts 0/1 f4b 1/1 rfi 0/1 limp 1/1 steal 0/1",
            ),
        ],
    );
}

#[test]
fn multiway_open_cold_calls_squeeze_and_responses() {
    // UTG opens, two cold calls, CO squeezes, SB cold-calls the squeeze, and
    // the BB then has a squeeze spot of the 3-bet. The opener calls (not a
    // fold to 3-bet, not a fold to squeeze).
    check_fixture(
        "preflop_multiway",
        MULTIWAY,
        1,
        &[
            ("Aria", "vpip 1/1 pfr 1/1 rfi 1/1 limp 0/1 f3b 0/1 fsqz 0/1 4b 0/1"),
            ("Bo", "vpip 0/1 pfr 0/1 3b 0/1 cc 0/1"),
            ("Cy", "vpip 1/1 pfr 0/1 3b 0/1 cc 1/1 4b 0/1"),
            ("Di", "vpip 1/1 pfr 0/1 3b 0/1 cc 1/1 sqz 0/1 4b 0/1"),
            ("Ed", "vpip 0/1 pfr 0/1 3b 0/1 cc 0/1 sqz 0/1"),
            ("Flo", "vpip 1/1 pfr 1/1 3b 1/1 cc 0/1 sqz 1/1"),
            ("Gil", "vpip 0/1 pfr 0/1 4b 0/1 cc 0/1"),
            ("Hu", "vpip 1/1 pfr 0/1 4b 0/1 cc 1/1"),
            ("Io", "vpip 0/1 pfr 0/1 4b 0/1 cc 0/1 sqz 0/1"),
        ],
    );
}

#[test]
fn tournament_blinds_and_antes_are_never_voluntary() {
    // Nine antes and both blinds posted; UTG+2 opens, LJ 3-bets all-in, BTN
    // re-raises all-in for less than a full raise (still a raise: a 4-bet),
    // the opener calls (the pileup: filed as a fold-to-3-bet spot, not
    // folded). The small blind is marked sitting out but posts and folds, so
    // she is dealt in and counted.
    check_fixture(
        "mtt_bounty_antes",
        MTT_ANTES,
        1,
        &[
            ("Bea", "vpip 0/1 pfr 0/1 rfi 0/1 limp 0/1"),
            ("Cal", "vpip 0/1 pfr 0/1 rfi 0/1 limp 0/1"),
            ("TourneyHero", "vpip 1/1 pfr 1/1 rfi 1/1 limp 0/1 f3b 0/1"),
            ("Dot", "vpip 1/1 pfr 1/1 3b 1/1 cc 0/1"),
            ("Wade", "vpip 0/1 pfr 0/1 4b 0/1 cc 0/1"),
            ("Xena", "vpip 0/1 pfr 0/1 4b 0/1 cc 0/1"),
            ("Yuri", "vpip 1/1 pfr 1/1 4b 1/1 cc 0/1"),
            ("Zoe", "vpip 0/1 pfr 0/1 cc 0/1"),
            ("Abe", "vpip 0/1 pfr 0/1 cc 0/1"),
        ],
    );
}

#[test]
fn walks_give_every_folder_a_declined_opportunity_and_the_big_blind_none() {
    // A cash walk and a tournament walk with antes.
    check_fixture(
        "preflop_walk",
        WALK,
        2,
        &[
            ("Pat", "vpip 0/1 pfr 0/1 rfi 0/1 limp 0/1"),
            ("Quy", "vpip 0/1 pfr 0/1 rfi 0/1 limp 0/1"),
            ("Ros", "vpip 0/1 pfr 0/1 rfi 0/1 limp 0/1 steal 0/1"),
            ("Sal", "vpip 0/1 pfr 0/1 rfi 0/1 limp 0/1 steal 0/1"),
            ("Tom", "vpip 0/1 pfr 0/1 rfi 0/1 limp 0/1 steal 0/1"),
            ("Uli", "vpip 0/1 pfr 0/1"),
            ("Vic", "vpip 0/1 pfr 0/1 rfi 0/1 limp 0/1 steal 0/1"),
            ("Wes", "vpip 0/1 pfr 0/1 rfi 0/1 limp 0/1 steal 0/1"),
            ("Xia", "vpip 0/1 pfr 0/1 rfi 0/1 limp 0/1 steal 0/1"),
            ("Yan", "vpip 0/1 pfr 0/1"),
        ],
    );
}

#[test]
fn dead_blind_sitting_out_returning_and_joining_players() {
    // Hand 101: Olga returns and posts a dead blind (small blind dead, big
    // blind live) from LJ. Her check while nobody has entered is a declined
    // first-in decision; her later raise over an open and a call is a squeeze.
    // Liam sits out and is never dealt in. Hand 102: Pjotr joins and posts a
    // big blind out of position from HJ. His check is a declined first-in
    // decision too, and since he is not in a blind seat, the CO steal is not a
    // fold-to-steal spot for him. Ivan sits out and is not dealt in.
    check_fixture(
        "cash_9max_eur_players",
        PLAYERS,
        2,
        &[
            ("Ivan", "vpip 0/1 pfr 0/1 3b 0/1 cc 0/1"),
            ("Judy", "vpip 2/2 pfr 1/2 3b 0/1 cc 1/1 4b 0/1 rfi 1/1 limp 0/1 steal 1/1"),
            ("Ken", "vpip 0/2 pfr 0/2 3b 0/2 cc 0/2 sqz 0/1"),
            ("Liam", ""),
            ("Mia", "vpip 0/2 pfr 0/2 3b 0/2 cc 0/2 sqz 0/1 fts 1/1"),
            ("Ned", "vpip 0/2 pfr 0/2 rfi 0/1 limp 0/1 3b 0/1 cc 0/1 fts 1/1"),
            ("Olga", "vpip 1/2 pfr 1/2 rfi 0/2 limp 0/2 3b 1/1 cc 0/1 sqz 1/1"),
            ("Hero", "vpip 1/2 pfr 1/2 rfi 1/2 limp 0/2 f3b 0/1 fsqz 0/1 4b 0/1"),
            ("Pjotr", "vpip 0/1 pfr 0/1 rfi 0/1 limp 0/1 3b 0/1 cc 0/1"),
        ],
    );
}

#[test]
fn similar_and_special_character_names_keep_their_own_counts() {
    // `Bob`, `Bob1` and `Bob Jr` share a prefix; chat lines quote one name's
    // action from another seat. Apostrophes, brackets, unicode, `Mr (BR)`
    // heads-up and a name holding `": "`.
    check_fixture(
        "names_special",
        NAMES,
        3,
        &[
            ("Bob", "vpip 1/3 pfr 1/3 3b 0/2 cc 0/2 fts 1/1 rfi 1/1 limp 0/1 steal 1/1"),
            ("Bob1", "vpip 1/1 pfr 0/1 3b 0/1 cc 1/1"),
            ("Bob Jr", "vpip 1/1 pfr 0/1 rfi 0/1 limp 1/1 3b 0/1"),
            ("O'Brien", "vpip 0/1 pfr 0/1"),
            ("[Pro] Kai", "vpip 1/1 pfr 1/1"),
            ("Zoë ゆうき", "vpip 0/1 pfr 0/1 3b 0/1 cc 0/1"),
            ("Mr (BR)", "vpip 1/1 pfr 1/1 rfi 1/1 limp 0/1 steal 1/1"),
            ("Ann: X", "vpip 1/1 pfr 0/1 3b 0/1 cc 1/1 fts 0/1"),
        ],
    );
}

#[test]
fn six_max_zoom_and_spin_formats() {
    check_fixture(
        "cash_6max_usd_chat",
        CHAT,
        2,
        &[
            ("Alice", "vpip 0/2 pfr 0/2 3b 0/1 cc 0/1"),
            ("Bob", "vpip 1/2 pfr 0/2 3b 0/1 cc 0/1 fts 1/1"),
            ("Carol", "vpip 2/2 pfr 1/2 3b 1/1 cc 0/1 fts 0/1"),
            ("Dave", "vpip 0/2 pfr 0/2 rfi 0/1 limp 0/1"),
            ("Erin", "vpip 1/2 pfr 0/2 rfi 0/2 limp 1/2"),
            ("Hero", "vpip 2/2 pfr 1/2 rfi 1/1 limp 0/1 steal 1/1 f3b 0/1 4b 0/1"),
        ],
    );
    check_fixture(
        "zoom_cash_gbp",
        ZOOM,
        1,
        &[
            ("Hollis", "vpip 0/1 pfr 0/1 rfi 0/1 limp 0/1"),
            ("Iona", "vpip 0/1 pfr 0/1 rfi 0/1 limp 0/1"),
            ("Jett", "vpip 0/1 pfr 0/1 rfi 0/1 limp 0/1 steal 0/1"),
            ("Hero", "vpip 1/1 pfr 1/1 rfi 1/1 limp 0/1 steal 1/1"),
            ("Fenwick", "vpip 0/1 pfr 0/1 3b 0/1 cc 0/1 fts 1/1"),
            ("Gale", "vpip 0/1 pfr 0/1 3b 0/1 cc 0/1 fts 1/1"),
        ],
    );
    // 3-max: no CO label, so BTN and SB are the steal seats. The big blind
    // faces the button's steal after the small blind called it: still a
    // fold-to-steal spot (exactly one raise), and a squeeze spot.
    check_fixture(
        "spin_and_go",
        SPIN,
        1,
        &[
            ("Hero", "vpip 1/1 pfr 1/1 rfi 1/1 limp 0/1 steal 1/1"),
            ("SpinB", "vpip 1/1 pfr 0/1 3b 0/1 cc 1/1 fts 0/1"),
            ("SpinA", "vpip 0/1 pfr 0/1 3b 0/1 cc 0/1 sqz 0/1 fts 1/1"),
        ],
    );
}

#[test]
fn truncated_hand_contributes_no_partial_counts() {
    // Two copies of hand 271000000201 cut before its summary: one mid
    // preflop, one after the showdown.
    let players = ["Gus", "Hana", "Ike", "Jo", "Kim", "Lou"];
    let mut conn = setup_db();
    let cut = import::import_text(&mut conn, TRUNCATED).unwrap();
    assert_eq!(cut.hands_imported, 0, "a truncated hand was stored");
    assert_eq!(cut.hands_failed + cut.hands_rejected_invalid, 2, "{cut:?}");
    for name in players {
        assert_eq!(counts(&conn, name), [(0, 0); 13], "{name} counted from a cut hand");
    }

    // The complete file then counts exactly as on a clean database, and a
    // late cut copy changes nothing.
    import::import_text(&mut conn, ALLIN_OPEN).unwrap();
    let mut clean = setup_db();
    import::import_text(&mut clean, ALLIN_OPEN).unwrap();
    for name in players {
        assert_eq!(counts(&conn, name), counts(&clean, name), "{name}");
    }
    let before = snapshot(&conn);
    let late = import::import_text(&mut conn, TRUNCATED).unwrap();
    assert_eq!(late.hands_imported, 0);
    assert_eq!(snapshot(&conn), before, "a late cut copy changed a count");
}

#[test]
fn duplicate_import_of_every_fixture_leaves_every_count_unchanged() {
    let mut conn = setup_db();
    let mut stored = 0;
    for (_, text) in ALL_FIXTURES {
        stored += import::import_text(&mut conn, text).unwrap().hands_imported;
    }
    let before = snapshot(&conn);
    assert!(before.len() > 50, "only {} players", before.len());
    let mut duplicates = 0;
    for (name, text) in ALL_FIXTURES {
        let again = import::import_text(&mut conn, text).unwrap();
        assert_eq!(again.hands_imported, 0, "{name}: stored again");
        duplicates += again.hands_skipped_duplicate;
    }
    // Cut copies and hand ids shared between fixtures may add to this.
    assert!(duplicates >= stored, "{duplicates} duplicates for {stored} stored hands");
    assert_eq!(snapshot(&conn), before);
}

fn assert_invariants(conn: &Connection, fixture: &str) {
    for name in players_with_hands(conn) {
        let id = player_id(conn, &name).unwrap();
        let c = counts(conn, &name);
        let (_, o) = stats::compute_player_stats_with_opportunities(conn, id).unwrap();
        let at = format!("{fixture}: {name} ({})", show(&c));
        assert!(c[PFR].0 <= c[VPIP].0, "PFR > VPIP for {at}");
        assert!(
            c[RFI].0 + c[LIMP].0 <= o.rfi_limp_opportunities,
            "RFI + limp hits exceed their opportunities for {at}"
        );
        assert!(
            o.squeeze_opportunities <= o.cold_call_opportunities,
            "squeeze opportunities exceed cold-call opportunities for {at}"
        );
        assert!(
            o.steal_attempt_opportunities <= o.rfi_limp_opportunities,
            "steal opportunities exceed RFI opportunities for {at}"
        );
        for (stat, (n, d)) in STATS.iter().zip(c) {
            assert!(n <= d, "{stat} numerator above its denominator for {at}");
        }
    }
}

#[test]
fn invariants_hold_on_every_fixture_alone_and_together() {
    let mut all = setup_db();
    for (fixture, text) in ALL_FIXTURES {
        let mut conn = setup_db();
        import::import_text(&mut conn, text).unwrap();
        assert_invariants(&conn, fixture);
        import::import_text(&mut all, text).unwrap();
    }
    assert_invariants(&all, "all fixtures");
}
