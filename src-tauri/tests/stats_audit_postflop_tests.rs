//! Audit of `stats/mod.rs`'s postflop stats (Run A, task S2).
//!
//! Every test pins the numerator AND the denominator of c-bet, fold to c-bet,
//! aggression factor, WTSD and W$SD for every player of a fixture (or of one
//! hand of it), worked out by hand from the hand history:
//!
//! - `cbet`: c-bets / `cbet_opportunities`;
//! - `fcb`: folds / `faced_cbet_opportunities`;
//! - `af`: `postflop_bets_raises` / `postflop_calls`;
//! - `wtsd`: `went_to_showdown_hands` / `saw_flop_hands`;
//! - `wsd`: showdowns won / `went_to_showdown_hands`.
//!
//! Denominators come from `compute_player_stats_with_opportunities`. The
//! c-bet, fold-to-c-bet and W$SD numerators are recovered from the percentage
//! and its denominator (exact under 1,000); the AF counts and the WTSD
//! numerator are exposed directly and checked against the reported value.
//! AF's two counts and the showdown numerators are also recounted
//! independently from the stored rows.
//!
//! Expectations are written as `"cbet 1/1 af 2/0 wtsd 0/1"`. A stat that is
//! not listed must be `0/0`: no opportunity and `None`, never `Some(0.0)`.

use std::collections::BTreeMap;

use rusqlite::Connection;
use velora_poker_lib::{db, import, stats};

const CBET_SPOTS: &str = include_str!("fixtures/audit/postflop_cbet_spots.txt");
const ALLIN: &str = include_str!("fixtures/audit/postflop_allin.txt");
const TRUNCATED: &str = include_str!("fixtures/audit/postflop_truncated.txt");
// Fixtures of tasks C2 and S1, reused for their postflop shapes.
const ALLIN_OPEN: &str = include_str!("fixtures/audit/preflop_allin_open.txt");
const NO_RAISE: &str = include_str!("fixtures/audit/preflop_no_opportunity.txt");
const LIMPED_POT: &str = include_str!("fixtures/audit/preflop_limped_pot.txt");
const HEADS_UP: &str = include_str!("fixtures/audit/preflop_heads_up.txt");
const MULTIWAY: &str = include_str!("fixtures/audit/preflop_multiway.txt");
const WALK: &str = include_str!("fixtures/audit/preflop_walk.txt");
const POTS: &str = include_str!("fixtures/audit/cash_pots_usd.txt");
const MTT_ANTES: &str = include_str!("fixtures/audit/mtt_bounty_antes.txt");
const PLAYERS: &str = include_str!("fixtures/audit/cash_9max_eur_players.txt");
const NAMES: &str = include_str!("fixtures/audit/names_special.txt");
const CHAT: &str = include_str!("fixtures/audit/cash_6max_usd_chat.txt");
const ZOOM: &str = include_str!("fixtures/audit/zoom_cash_gbp.txt");
const SPIN: &str = include_str!("fixtures/audit/spin_and_go.txt");
const PLAY_MONEY: &str = include_str!("fixtures/audit/cash_play_money.txt");
// A real tournament hand (anonymised), already used by tournament_stats_tests.
const TOURNAMENT_DONK: &str = include_str!("fixtures/tournament_showdown_allin.txt");

/// Every audit fixture, then every fixture `stats_tests.rs` and
/// `tournament_stats_tests.rs` use: the invariants hold over all of them.
const ALL_FIXTURES: &[(&str, &str)] = &[
    ("postflop_cbet_spots", CBET_SPOTS),
    ("postflop_allin", ALLIN),
    ("postflop_truncated", TRUNCATED),
    ("preflop_allin_open", ALLIN_OPEN),
    ("preflop_no_opportunity", NO_RAISE),
    ("preflop_limped_pot", LIMPED_POT),
    ("preflop_heads_up", HEADS_UP),
    ("preflop_multiway", MULTIWAY),
    ("preflop_walk", WALK),
    ("preflop_truncated", include_str!("fixtures/audit/preflop_truncated.txt")),
    ("cash_pots_usd", POTS),
    ("mtt_bounty_antes", MTT_ANTES),
    ("cash_9max_eur_players", PLAYERS),
    ("names_special", NAMES),
    ("cash_6max_usd_chat", CHAT),
    ("zoom_cash_gbp", ZOOM),
    ("spin_and_go", SPIN),
    ("cash_play_money", PLAY_MONEY),
    ("names_seat_shaped", include_str!("fixtures/audit/names_seat_shaped.txt")),
    ("unsupported_straddle", include_str!("fixtures/audit/unsupported_straddle.txt")),
    ("unsupported_currency", include_str!("fixtures/audit/unsupported_currency.txt")),
    ("unsupported_non_english", include_str!("fixtures/audit/unsupported_non_english.txt")),
    ("other_room", include_str!("fixtures/audit/other_room.txt")),
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
    ("tournament_showdown_allin", TOURNAMENT_DONK),
    ("tournament_uncontested_walk", include_str!("fixtures/tournament_uncontested_walk.txt")),
    (
        "tournament_allin_preflop_disconnect",
        include_str!("fixtures/tournament_allin_preflop_disconnect.txt"),
    ),
    ("tournament_zoom_header", include_str!("fixtures/tournament_zoom_header.txt")),
];

/// The five postflop stats, in the order `Counts` stores them.
const STATS: [&str; 5] = ["cbet", "fcb", "af", "wtsd", "wsd"];
const CBET: usize = 0;
const FCB: usize = 1;
const WTSD: usize = 3;
const WSD: usize = 4;

/// `(numerator, denominator)` per stat, in `STATS` order.
type Counts = [(i64, i64); 5];

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

fn pct(n: i64, d: i64) -> Option<f64> {
    (d != 0).then(|| round1(n as f64 / d as f64 * 100.0))
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
        return [(0, 0); 5];
    };
    let (s, o) = stats::compute_player_stats_with_opportunities(conn, id).unwrap();
    let cbet = o.cbet_opportunities;
    let faced = o.faced_cbet_opportunities;
    let (bets, calls) = (o.postflop_bets_raises, o.postflop_calls);
    let (went, saw) = (o.went_to_showdown_hands, o.saw_flop_hands);

    let af = (calls != 0).then(|| round1(bets as f64 / calls as f64));
    assert_eq!(s.aggression_factor, af, "{name} af from {bets}/{calls}");
    assert_eq!(s.wtsd, pct(went, saw), "{name} wtsd from {went}/{saw}");
    let out = [
        (hits(s.c_bet, cbet, &format!("{name} cbet")), cbet),
        (hits(s.fold_to_c_bet, faced, &format!("{name} fcb")), faced),
        (bets, calls),
        (went, saw),
        (hits(s.wsd, went, &format!("{name} wsd")), went),
    ];

    // Test-local recount of the numerators whose rule is simple enough to
    // restate in SQL: postflop bets/raises and calls, and the two showdown
    // flags stored per player and hand.
    let recount = |sql: &str| -> i64 { conn.query_row(sql, [id], |row| row.get(0)).unwrap() };
    let postflop = |types: &str| {
        recount(&format!(
            "SELECT COUNT(*) FROM actions
             WHERE player_id = ?1 AND street != 'preflop' AND action_type IN ({types})"
        ))
    };
    assert_eq!(bets, postflop("'bet', 'raise'"), "{name} postflop bets/raises recount");
    assert_eq!(calls, postflop("'call'"), "{name} postflop calls recount");
    assert_eq!(
        went,
        recount("SELECT COUNT(*) FROM player_hands WHERE player_id = ?1 AND went_to_showdown = 1"),
        "{name} showdown recount"
    );
    assert_eq!(
        out[WSD].0,
        recount("SELECT COUNT(*) FROM player_hands WHERE player_id = ?1 AND won_at_showdown = 1"),
        "{name} showdown won recount"
    );
    out
}

fn parse(expected: &str) -> Counts {
    let mut out = [(0, 0); 5];
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
        .filter(|(_, (n, d))| *n != 0 || *d != 0)
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

/// One hand of a fixture file, by its PokerStars hand id.
fn hand(text: &str, id: u64) -> String {
    let marker = format!("PokerStars Hand #{id}:");
    let start = text.find(&marker).unwrap_or_else(|| panic!("hand {id} not in fixture"));
    let rest = &text[start..];
    let end = rest[marker.len()..]
        .find("PokerStars Hand #")
        .map_or(rest.len(), |i| i + marker.len());
    rest[..end].to_string()
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
fn no_flop_reports_none_never_zero() {
    // Walks (cash, and a tournament with antes) and a Zoom steal: nobody sees
    // a flop, so every postflop stat of every player has no opportunity.
    check_fixture(
        "preflop_walk",
        WALK,
        2,
        &[
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
        ],
    );
    check_fixture(
        "zoom_cash_gbp",
        ZOOM,
        1,
        &[("Hollis", ""), ("Iona", ""), ("Jett", ""), ("Hero", ""), ("Fenwick", ""), ("Gale", "")],
    );

    // The same, read field by field for the raiser who took the pot preflop.
    let mut conn = setup_db();
    import::import_text(&mut conn, ZOOM).unwrap();
    let s = stats::compute_player_stats(&conn, player_id(&conn, "Hero").unwrap()).unwrap();
    assert_eq!(s.pfr, Some(100.0));
    for (label, value) in [
        ("c_bet", s.c_bet),
        ("fold_to_c_bet", s.fold_to_c_bet),
        ("aggression_factor", s.aggression_factor),
        ("wtsd", s.wtsd),
        ("wsd", s.wsd),
    ] {
        assert_eq!(value, None, "Hero {label}");
    }
}

/// Suspected case (a), confirmed and fixed: a preflop raiser who is all-in
/// preflop never acts on the flop, so they have no c-bet opportunity. Before
/// the fix the WTSD all-in branch also gave them one (0/1).
#[test]
fn preflop_raiser_all_in_preflop_who_reaches_showdown_has_no_cbet_opportunity() {
    // Hand 201: Ike shoves first in, Lou calls all-in for less: both see the
    // flop with no action row, Lou wins. Hand 202: Lou shoves, folded to.
    check_fixture(
        "preflop_allin_open",
        ALLIN_OPEN,
        2,
        &[
            ("Gus", ""),
            ("Hana", ""),
            ("Ike", "wtsd 1/1 wsd 0/1"),
            ("Jo", ""),
            ("Kim", ""),
            ("Lou", "wtsd 1/1 wsd 1/1"),
        ],
    );
    // Multiway: Lars shoves, two deeper players call and play on. Hob's flop
    // bet is not a c-bet (Lars raised last), so Kye has no fold-to-c-bet spot.
    // Lars wins the main pot only, Kye the side pot only.
    check_fixture(
        "postflop_allin #203",
        &hand(ALLIN, 272000000203),
        1,
        &[
            ("Gia", ""),
            ("Hob", "af 1/1 wtsd 1/1 wsd 0/1"),
            ("Ina", ""),
            ("Jem", ""),
            ("Kye", "af 1/1 wtsd 1/1 wsd 1/1"),
            ("Lars", "wtsd 1/1 wsd 1/1"),
        ],
    );
}

/// Suspected case (b), confirmed and fixed: a raiser whose first flop action
/// faces a donk bet never got to "bet first when the action reached them",
/// so it is not a c-bet opportunity. Before the fix it counted as 0/1.
#[test]
fn raiser_facing_a_donk_bet_before_acting_has_no_cbet_opportunity() {
    // Multiway: the SB donks into the button raiser, who calls, then bets the
    // turn. The donk is not a c-bet, so the BB who folds to it has no
    // fold-to-c-bet spot either.
    check_fixture(
        "postflop_cbet_spots #103",
        &hand(CBET_SPOTS, 272000000103),
        1,
        &[
            ("Ana", ""),
            ("Bru", ""),
            ("Caz", "af 1/1 wtsd 0/1"),
            ("Dov", "af 1/0 wtsd 0/1"),
            ("Eli", "wtsd 0/1"),
            ("Fin", ""),
        ],
    );
    // Hand 801: Bob Jr donks into [Pro] Kai, who raises (no c-bet spot), and
    // Bob1, who checked before the donk, has no fold-to-c-bet spot. Hand 803:
    // Bob's heads-up c-bet, folded to by `Ann: X`.
    check_fixture(
        "names_special",
        NAMES,
        3,
        &[
            ("Bob", "cbet 1/1 af 1/0 wtsd 0/1"),
            ("Bob1", "wtsd 0/1"),
            ("Bob Jr", "af 1/1 wtsd 0/1"),
            ("O'Brien", ""),
            ("[Pro] Kai", "af 2/0 wtsd 0/1"),
            ("Zoë ゆうき", ""),
            ("Mr (BR)", ""),
            ("Ann: X", "fcb 1/1 wtsd 0/1"),
        ],
    );
    // A real tournament hand: Opponent13 donks all-in into TourneyHero, the
    // preflop raiser, who calls. `tournament_stats_tests` pinned this as a
    // 0% c-bet before the fix; it is now no opportunity.
    check_fixture(
        "tournament_showdown_allin",
        TOURNAMENT_DONK,
        1,
        &[
            ("TourneyHero", "af 0/1 wtsd 1/1 wsd 0/1"),
            ("Opponent13", "af 1/0 wtsd 1/1 wsd 1/1"),
            ("Opponent7", ""),
            ("Opponent8", ""),
            ("Opponent9", ""),
            ("Opponént10", ""),
            ("Opponent11", ""),
            ("opponent--12", ""),
            ("Opponent14", ""),
        ],
    );
}

#[test]
fn raiser_who_checks_and_then_faces_a_bet_missed_the_cbet() {
    // The CO raiser checks behind on the flop (an opportunity, not taken);
    // the big blind bets turn and river. A bet after the raiser checked is
    // not a c-bet, so the raiser's turn call is no fold-to-c-bet spot.
    check_fixture(
        "postflop_cbet_spots #102",
        &hand(CBET_SPOTS, 272000000102),
        1,
        &[
            ("Ana", "cbet 0/1 af 0/1 wtsd 0/1"),
            ("Bru", ""),
            ("Caz", ""),
            ("Dov", "af 2/0 wtsd 0/1"),
            ("Eli", ""),
            ("Fin", ""),
        ],
    );
}

/// The documented behaviour, pinned and left for a Sponsor decision: the
/// contract's denominator is "an action of their own after the c-bet", so a
/// raise between the c-bet and the player's answer does not remove the spot.
#[test]
fn raise_between_the_cbet_and_the_response_is_still_filed_as_facing_the_cbet() {
    // Hand 104: the BB checks, UTG c-bets, the button raises, the BB folds
    // (counted as a fold to the c-bet, though she faced the raise) and the
    // c-bettor folds (his own bet: no spot).
    check_fixture(
        "postflop_cbet_spots #104",
        &hand(CBET_SPOTS, 272000000104),
        1,
        &[
            ("Ana", "cbet 1/1 af 1/0 wtsd 0/1"),
            ("Bru", ""),
            ("Caz", ""),
            ("Dov", "fcb 0/1 af 1/0 wtsd 0/1"),
            ("Eli", ""),
            ("Fin", "fcb 1/1 wtsd 0/1"),
        ],
    );
    // Hand 105: the same shape, the BB calls the raise (a faced c-bet not
    // folded) and wins nothing at a two-way showdown.
    check_fixture(
        "postflop_cbet_spots #105",
        &hand(CBET_SPOTS, 272000000105),
        1,
        &[
            ("Ana", "fcb 0/1 af 0/1 wtsd 1/1 wsd 0/1"),
            ("Bru", "cbet 1/1 af 1/0 wtsd 0/1"),
            ("Caz", ""),
            ("Dov", ""),
            ("Eli", "fcb 0/1 af 1/0 wtsd 1/1 wsd 1/1"),
            ("Fin", ""),
        ],
    );
}

#[test]
fn flop_all_in_cbet_and_partial_all_in_call() {
    // The raiser c-bets all-in, the short big blind calls all-in for less.
    check_fixture(
        "postflop_allin #202",
        &hand(ALLIN, 272000000202),
        1,
        &[
            ("Gia", "cbet 1/1 af 1/0 wtsd 1/1 wsd 1/1"),
            ("Hob", ""),
            ("Ina", ""),
            ("Jem", "fcb 0/1 af 0/1 wtsd 1/1 wsd 0/1"),
            ("Kye", ""),
            ("Lars", ""),
        ],
    );
    // Spin & Go: the c-bet is check-raised all-in and called.
    check_fixture(
        "spin_and_go",
        SPIN,
        1,
        &[
            ("Hero", "cbet 1/1 af 1/1 wtsd 1/1 wsd 0/1"),
            ("SpinB", "fcb 0/1 af 1/0 wtsd 1/1 wsd 1/1"),
            ("SpinA", ""),
        ],
    );
}

/// Defect found by this audit and fixed: a player who calls a preflop all-in
/// with chips behind and nobody left to bet against never acts on the flop
/// and is not all-in, so they went to showdown without "seeing the flop":
/// WTSD's numerator held a hand its denominator did not (1/0 here; 200%
/// over a player's two showdowns in the whole `postflop_allin` file).
#[test]
fn covered_caller_of_a_preflop_all_in_saw_the_flop() {
    // Kye raises, Lars calls all-in for less, everyone else folds: Kye, the
    // preflop raiser, sees the flop and the showdown with no action left.
    check_fixture(
        "postflop_allin #201",
        &hand(ALLIN, 272000000201),
        1,
        &[
            ("Gia", ""),
            ("Hob", ""),
            ("Ina", ""),
            ("Jem", ""),
            ("Kye", "wtsd 1/1 wsd 1/1"),
            ("Lars", "wtsd 1/1 wsd 0/1"),
        ],
    );
    // MTT: Yuri's all-in 4-bet (the last raise: no c-bet spot, case (a)) and
    // Dot's all-in 3-bet are called by TourneyHero, who has chips behind.
    // Dot wins the main pot and TourneyHero the side pot only.
    check_fixture(
        "mtt_bounty_antes",
        MTT_ANTES,
        1,
        &[
            ("Wade", ""),
            ("Xena", ""),
            ("Yuri", "wtsd 1/1 wsd 0/1"),
            ("Zoe", ""),
            ("Abe", ""),
            ("Bea", ""),
            ("Cal", ""),
            ("TourneyHero", "wtsd 1/1 wsd 1/1"),
            ("Dot", "wtsd 1/1 wsd 1/1"),
        ],
    );
}

#[test]
fn all_in_preflop_folded_to_versus_called() {
    // Hand 204: Ina shoves and everyone folds: no flop, so nothing at all,
    // although she was all-in and the last raiser.
    check_fixture(
        "postflop_allin #204",
        &hand(ALLIN, 272000000204),
        1,
        &[("Gia", ""), ("Hob", ""), ("Ina", ""), ("Jem", ""), ("Kye", ""), ("Lars", "")],
    );
    // The whole file: Lars's two called all-ins (one call, one shove) both
    // count as flops seen; Kye's two showdowns (a covered call and a side
    // pot) both count; Ina's folded-to shove counts nowhere.
    check_fixture(
        "postflop_allin",
        ALLIN,
        4,
        &[
            ("Gia", "cbet 1/1 af 1/0 wtsd 1/1 wsd 1/1"),
            ("Hob", "af 1/1 wtsd 1/1 wsd 0/1"),
            ("Ina", ""),
            ("Jem", "fcb 0/1 af 0/1 wtsd 1/1 wsd 0/1"),
            ("Kye", "af 1/1 wtsd 2/2 wsd 2/2"),
            ("Lars", "wtsd 2/2 wsd 1/2"),
        ],
    );
}

#[test]
fn limped_pot_has_no_cbet_and_no_fold_to_cbet() {
    // No preflop raise: a flop bet is never a c-bet, and folding to it is
    // never a fold to a c-bet. AF and WTSD still count.
    check_fixture(
        "preflop_no_opportunity",
        NO_RAISE,
        1,
        &[
            ("Ann", "wtsd 0/1"),
            ("Ben", "af 1/0 wtsd 0/1"),
            ("Cid", ""),
            ("Dee", ""),
            ("Eve", "wtsd 0/1"),
            ("Fay", "wtsd 0/1"),
        ],
    );
    // Heads-up: the button completes, the big blind checks, then folds the
    // flop to the button's bet.
    check_fixture(
        "preflop_heads_up",
        HEADS_UP,
        3,
        &[("Hal", "wtsd 0/1"), ("Ivy", "af 1/0 wtsd 0/1")],
    );
    check_fixture(
        "cash_play_money",
        PLAY_MONEY,
        1,
        &[("Hero", "af 1/0 wtsd 0/1"), ("PlayA", "wtsd 0/1"), ("PlayB", "wtsd 0/1")],
    );
}

#[test]
fn heads_up_and_multiway_cbets_and_responses() {
    // Heads-up: the button c-bets, the big blind check-folds.
    check_fixture(
        "postflop_cbet_spots #101",
        &hand(CBET_SPOTS, 272000000101),
        1,
        &[
            ("Ana", "cbet 1/1 af 1/0 wtsd 0/1"),
            ("Bru", ""),
            ("Caz", "fcb 1/1 wtsd 0/1"),
            ("Dov", ""),
            ("Eli", ""),
            ("Fin", ""),
        ],
    );
    // Four-way flop: three players check to the squeezer, who c-bets; all
    // three fold, so each has a fold-to-c-bet spot.
    check_fixture(
        "preflop_multiway",
        MULTIWAY,
        1,
        &[
            ("Aria", "fcb 1/1 wtsd 0/1"),
            ("Bo", ""),
            ("Cy", "fcb 1/1 wtsd 0/1"),
            ("Di", ""),
            ("Ed", ""),
            ("Flo", "cbet 1/1 af 1/0 wtsd 0/1"),
            ("Gil", ""),
            ("Hu", "fcb 1/1 wtsd 0/1"),
            ("Io", ""),
        ],
    );
    // The limp-reraiser and a big blind who raises a limped pot are each the
    // last preflop raiser, and each c-bets.
    check_fixture(
        "preflop_limped_pot",
        LIMPED_POT,
        2,
        &[
            ("Mo", "cbet 2/2 af 2/0 wtsd 0/2"),
            ("Nia", ""),
            ("Oz", ""),
            ("Pia", "fcb 1/1 wtsd 0/1"),
            ("Quin", "fcb 1/1 wtsd 0/1"),
            ("Rex", ""),
        ],
    );
    // The whole c-bet file, every spot together.
    check_fixture(
        "postflop_cbet_spots",
        CBET_SPOTS,
        5,
        &[
            ("Ana", "cbet 2/3 fcb 0/1 af 2/2 wtsd 1/4 wsd 0/1"),
            ("Bru", "cbet 1/1 af 1/0 wtsd 0/1"),
            ("Caz", "fcb 1/1 af 1/1 wtsd 0/2"),
            ("Dov", "fcb 0/1 af 4/0 wtsd 0/3"),
            ("Eli", "fcb 0/1 af 1/0 wtsd 1/2 wsd 1/1"),
            ("Fin", "fcb 1/1 wtsd 0/1"),
        ],
    );
}

#[test]
fn split_pot_side_pot_and_muck_at_showdown() {
    // Hand 301: Rosa c-bets into two callers; at showdown Rosa and Tara split
    // the pot (both won) and Quinn mucks (went, lost). Hand 302: three
    // preflop all-ins; Rosa, the last raiser, wins only the side pot and has
    // no c-bet spot (case (a)); Quinn wins the main pot; Tara loses.
    check_fixture(
        "cash_pots_usd",
        POTS,
        2,
        &[
            ("Quinn", "fcb 0/1 af 0/2 wtsd 2/2 wsd 1/2"),
            ("Rosa", "cbet 1/1 af 2/0 wtsd 2/2 wsd 2/2"),
            ("Tara", "fcb 0/1 af 0/2 wtsd 2/2 wsd 1/2"),
            ("Uma", ""),
        ],
    );
    // Hand 1: Carol's c-bet is raised by Hero, who wins. Hand 2: limped pot,
    // Dave shows and loses, Erin mucks, Hero wins.
    check_fixture(
        "cash_6max_usd_chat",
        CHAT,
        2,
        &[
            ("Alice", ""),
            ("Bob", "wtsd 0/1"),
            ("Carol", "cbet 1/1 af 1/0 wtsd 0/2"),
            ("Dave", "af 1/0 wtsd 1/1 wsd 0/1"),
            ("Erin", "af 0/1 wtsd 1/1 wsd 0/1"),
            ("Hero", "fcb 0/1 af 1/1 wtsd 1/2 wsd 1/1"),
        ],
    );
    // Olga squeezes from a dead blind, c-bets and wins; Hero mucks.
    check_fixture(
        "cash_9max_eur_players",
        PLAYERS,
        2,
        &[
            ("Ivan", ""),
            ("Judy", ""),
            ("Ken", ""),
            ("Liam", ""),
            ("Mia", ""),
            ("Ned", ""),
            ("Olga", "cbet 1/1 af 2/0 wtsd 1/1 wsd 1/1"),
            ("Hero", "fcb 0/1 af 0/2 wtsd 1/1 wsd 0/1"),
            ("Pjotr", ""),
        ],
    );
}

#[test]
fn truncated_hand_contributes_no_partial_counts() {
    // Two copies of hand 272000000105 cut before its summary: one on the
    // turn, one after the showdown lines.
    let players = ["Ana", "Bru", "Caz", "Dov", "Eli", "Fin"];
    let mut conn = setup_db();
    let cut = import::import_text(&mut conn, TRUNCATED).unwrap();
    assert_eq!(cut.hands_imported, 0, "a truncated hand was stored");
    assert_eq!(cut.hands_failed + cut.hands_rejected_invalid, 2, "{cut:?}");
    for name in players {
        assert_eq!(counts(&conn, name), [(0, 0); 5], "{name} counted from a cut hand");
    }

    // The complete file then counts exactly as on a clean database, and a
    // late cut copy changes nothing.
    import::import_text(&mut conn, CBET_SPOTS).unwrap();
    let mut clean = setup_db();
    import::import_text(&mut clean, CBET_SPOTS).unwrap();
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
        let (s, o) = stats::compute_player_stats_with_opportunities(conn, id).unwrap();
        let at = format!("{fixture}: {name} ({})", show(&c));
        for i in [CBET, FCB, WTSD, WSD] {
            assert!(c[i].0 <= c[i].1, "{} numerator above its denominator for {at}", STATS[i]);
        }
        assert!(o.cbet_opportunities <= o.saw_flop_hands, "c-bet spots above flops seen for {at}");
        assert!(
            o.faced_cbet_opportunities <= o.saw_flop_hands,
            "faced c-bets above flops seen for {at}"
        );
        // A c-bet opportunity needs the last preflop raise, so a PFR hand.
        let pfr_hands = hits(s.pfr, o.hands, &format!("{name} pfr"));
        assert!(o.cbet_opportunities <= pfr_hands, "c-bet spots above PFR hands for {at}");
        assert!(o.saw_flop_hands <= o.hands, "flops seen above hands for {at}");
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
