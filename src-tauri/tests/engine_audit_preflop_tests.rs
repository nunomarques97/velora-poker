//! Audit of the opponent engine's preflop and stack-depth stats (Run A, task
//! E1), and of where they agree with `stats/mod.rs`.
//!
//! - Every `StatKey::PREFLOP` key is pinned, hits and opportunities, for
//!   every player of each fixture, worked out by hand from the hand history.
//!   The counts are read twice — summed from `extract_preflop`'s events and
//!   from `aggregate_player`'s raw all-time counts — and both must match.
//! - A deep-stacked corpus with no all-in and no walk is computed by both
//!   engines, which must agree exactly on every stat they share.
//! - On the edge fixtures, every difference between the two engines is
//!   listed and tagged with the documented divergence that explains it
//!   (`stat-contracts.md` and `opponent-engine.md` §9.1). An unlisted
//!   difference fails the test.
//! - Recent form's VPIP counts are `extract_preflop`'s VPIP counts.
//!
//! Expectations are written as `"vpip 1/2 rfi_co 0/1"` with the spec's stat
//! keys. A key that is not listed must have no opportunity at all.

mod common;

use std::collections::BTreeMap;

use rusqlite::Connection;
use velora_poker_lib::engine::recency::{form_counts, LAST_N};
use velora_poker_lib::engine::{
    aggregate_player, extract_player_preflop, extract_preflop, load_player_hands, FormatKey,
    HandFacts, StatKey, View,
};
use velora_poker_lib::{db, import, stats};

use common::{Act, Line, Table};

const NO_OPPORTUNITY: &str = include_str!("fixtures/audit/preflop_no_opportunity.txt");
const ALLIN_OPEN: &str = include_str!("fixtures/audit/preflop_allin_open.txt");
const LIMPED_POT: &str = include_str!("fixtures/audit/preflop_limped_pot.txt");
const HEADS_UP: &str = include_str!("fixtures/audit/preflop_heads_up.txt");
const MULTIWAY: &str = include_str!("fixtures/audit/preflop_multiway.txt");
const WALK: &str = include_str!("fixtures/audit/preflop_walk.txt");
const TRUNCATED: &str = include_str!("fixtures/audit/preflop_truncated.txt");
const POTS: &str = include_str!("fixtures/audit/cash_pots_usd.txt");
const MTT_ANTES: &str = include_str!("fixtures/audit/mtt_bounty_antes.txt");
const PLAYERS: &str = include_str!("fixtures/audit/cash_9max_eur_players.txt");
const NAMES: &str = include_str!("fixtures/audit/names_special.txt");
const SEAT_SHAPED: &str = include_str!("fixtures/audit/names_seat_shaped.txt");
const CHAT: &str = include_str!("fixtures/audit/cash_6max_usd_chat.txt");
const ZOOM: &str = include_str!("fixtures/audit/zoom_cash_gbp.txt");
const SPIN: &str = include_str!("fixtures/audit/spin_and_go.txt");
const PUSH_FOLD: &str = include_str!("fixtures/audit/engine_preflop_push_fold.txt");
const SPOTS: &str = include_str!("fixtures/audit/engine_preflop_spots.txt");

// ---------------------------------------------------------------- expectations

/// `(fixture, text, hands stored, per-player engine counts)`.
type Fixture = (&'static str, &'static str, i64, &'static [(&'static str, &'static str)]);

/// A limped pot with no raise: the first limper's RFI/limp spot, then an
/// isolation spot for everyone behind (completing and checking included).
const NO_OPPORTUNITY_COUNTS: &[(&str, &str)] = &[
    ("Ann", "vpip 1/1 pfr 0/1 rfi_ep 0/1 limp 1/1"),
    ("Ben", "vpip 1/1 pfr 0/1 iso_raise 0/1"),
    ("Cid", "vpip 0/1 pfr 0/1 iso_raise 0/1"),
    ("Dee", "vpip 0/1 pfr 0/1 iso_raise 0/1"),
    ("Eve", "vpip 1/1 pfr 0/1 iso_raise 0/1"),
    ("Fay", "vpip 0/1 pfr 0/1 iso_raise 0/1"),
];

/// Hand 201: CO shoves 100bb first in (an RFI and a steal for him), the 24bb
/// big blind calls all-in for less. Everyone behind the shove faces
/// `call_vs_shove`, never a 3-bet, cold-call or steal-defence spot.
/// Hand 202: the small blind shoves first in; the big blind folds (no
/// `bb_defend_vs_sb`: an all-in open is a shove).
const ALLIN_OPEN_COUNTS: &[(&str, &str)] = &[
    ("Gus", "vpip 0/2 pfr 0/2 rfi_ep 0/1 limp 0/1 call_vs_shove 0/1"),
    ("Hana", "vpip 0/2 pfr 0/2 rfi_ep 0/1 rfi_mp 0/1 limp 0/2"),
    ("Ike", "vpip 1/2 pfr 1/2 rfi_mp 0/1 rfi_co 1/1 steal 1/1 limp 0/2"),
    ("Jo", "vpip 0/2 pfr 0/2 rfi_co 0/1 steal 0/1 limp 0/1 call_vs_shove 0/1"),
    ("Kim", "vpip 0/2 pfr 0/2 rfi_btn 0/1 steal 0/1 limp 0/1 call_vs_shove 0/1"),
    ("Lou", "vpip 2/2 pfr 1/2 rfi_sb 1/1 steal 1/1 limp 0/1 call_vs_shove 1/1"),
];

/// Hand 301: UTG limps, HJ limps behind, CO isolates, BTN cold-calls, the
/// blinds fold (squeeze spots), UTG limp-reraises, CO folds (fold to 3-bet
/// IP: the isolation raiser is the opener). Hand 302: BTN limps, SB
/// completes, BB raises (isolation), BTN limp-calls.
const LIMPED_POT_COUNTS: &[(&str, &str)] = &[
    (
        "Mo",
        "vpip 2/2 pfr 2/2 rfi_ep 0/1 limp 1/1 limp_fold 0/1 limp_call 0/1 limp_reraise 1/1 iso_raise 1/1",
    ),
    ("Nia", "vpip 1/2 pfr 0/2 rfi_ep 0/1 limp 0/1 iso_raise 0/1"),
    ("Oz", "vpip 1/2 pfr 1/2 rfi_mp 0/1 fold_to_3bet_ip 1/1 four_bet 0/1 limp 0/1 iso_raise 1/1"),
    ("Pia", "vpip 1/2 pfr 0/2 rfi_co 0/1 steal 0/1 three_bet_ip 0/1 cold_call 1/1 limp 0/1"),
    (
        "Quin",
        "vpip 1/2 pfr 0/2 rfi_btn 0/1 steal 0/1 three_bet_oop 0/1 squeeze 0/1 cold_call 0/1 limp 1/1 \
         limp_fold 0/1 limp_call 1/1 limp_reraise 0/1",
    ),
    ("Rex", "vpip 1/2 pfr 0/2 three_bet_oop 0/1 squeeze 0/1 cold_call 0/1 iso_raise 0/1"),
];

/// Heads-up the button is labelled BTN and acts last postflop. Hand 401:
/// open (steal), 3-bet OOP, 4-bet, fold. Hand 402: the button completes, the
/// big blind checks (an isolation spot). Hand 403: a walk: the big blind has
/// no decision and no event.
const HEADS_UP_COUNTS: &[(&str, &str)] = &[
    (
        "Hal",
        "vpip 1/3 pfr 1/3 rfi_btn 1/2 steal 1/2 fold_to_3bet_ip 0/1 four_bet 1/1 limp 0/2 iso_raise 0/1",
    ),
    (
        "Ivy",
        "vpip 2/2 pfr 1/2 rfi_btn 0/1 steal 0/1 fold_to_steal_bb 0/1 three_bet_oop 1/1 fold_to_4bet 1/1 \
         cold_call 0/1 limp 1/1",
    ),
];

/// 9-max: UTG opens, two cold calls, CO squeezes. Everyone behind the
/// squeeze faces two raises: no spot but VPIP/PFR. The opener calls.
const MULTIWAY_COUNTS: &[(&str, &str)] = &[
    ("Aria", "vpip 1/1 pfr 1/1 rfi_ep 1/1 limp 0/1 fold_to_3bet_oop 0/1 four_bet 0/1"),
    ("Bo", "vpip 0/1 pfr 0/1 three_bet_ip 0/1 cold_call 0/1"),
    ("Cy", "vpip 1/1 pfr 0/1 three_bet_ip 0/1 cold_call 1/1"),
    ("Di", "vpip 1/1 pfr 0/1 three_bet_ip 0/1 squeeze 0/1 cold_call 1/1"),
    ("Ed", "vpip 0/1 pfr 0/1 three_bet_ip 0/1 squeeze 0/1 cold_call 0/1"),
    ("Flo", "vpip 1/1 pfr 1/1 three_bet_ip 1/1 squeeze 1/1 cold_call 0/1"),
    ("Gil", "vpip 0/1 pfr 0/1"),
    ("Hu", "vpip 1/1 pfr 0/1"),
    ("Io", "vpip 0/1 pfr 0/1"),
];

/// A cash walk and a tournament walk with antes: every folder has a declined
/// first-in spot; the big blind has no decision, hence no VPIP/PFR spot.
const WALK_COUNTS: &[(&str, &str)] = &[
    ("Pat", "vpip 0/1 pfr 0/1 rfi_ep 0/1 limp 0/1"),
    ("Quy", "vpip 0/1 pfr 0/1 rfi_mp 0/1 limp 0/1"),
    ("Ros", "vpip 0/1 pfr 0/1 rfi_co 0/1 steal 0/1 limp 0/1"),
    ("Sal", "vpip 0/1 pfr 0/1 rfi_btn 0/1 steal 0/1 limp 0/1"),
    ("Tom", "vpip 0/1 pfr 0/1 rfi_sb 0/1 steal 0/1 limp 0/1"),
    ("Uli", ""),
    ("Vic", "vpip 0/1 pfr 0/1 rfi_co 0/1 steal 0/1 limp 0/1"),
    ("Wes", "vpip 0/1 pfr 0/1 rfi_btn 0/1 steal 0/1 limp 0/1"),
    ("Xia", "vpip 0/1 pfr 0/1 rfi_sb 0/1 steal 0/1 limp 0/1"),
    ("Yan", ""),
];

/// Hand 301: CO steal, BTN cold-calls, SB folds with a caller in between (no
/// fold-to-steal spot), BB calls (squeeze declined). Hand 302: CO opens, the
/// 20bb BTN 3-bets all-in (a 3-bet IP and a reshove), SB re-raises all-in
/// (a declined `call_vs_shove`), BB calls the shove, the opener folds facing
/// two raises: neither a fold to 3-bet nor a `call_vs_shove` (a caller came
/// first).
const POTS_COUNTS: &[(&str, &str)] = &[
    (
        "Quinn",
        "vpip 2/2 pfr 1/2 three_bet_ip 1/1 three_bet_oop 0/1 squeeze 0/1 cold_call 1/2 reshove 1/1",
    ),
    ("Rosa", "vpip 2/2 pfr 2/2 rfi_co 1/1 steal 1/1 limp 0/1 call_vs_shove 0/1"),
    ("Tara", "vpip 2/2 pfr 0/2 three_bet_ip 0/1 cold_call 1/1 call_vs_shove 1/1"),
    (
        "Uma",
        "vpip 1/2 pfr 1/2 rfi_co 1/1 steal 1/1 three_bet_oop 0/1 squeeze 0/1 cold_call 0/1 limp 0/1",
    ),
];

/// Nine antes, 300 big blind. UTG+2 opens at 37bb, the 7bb LJ 3-bets all-in
/// (no event at all: at 15bb or less only the push/fold keys count, and an
/// all-in over an open is not an open shove), the 10bb BTN re-shoves (a
/// declined `call_vs_shove`), the opener calls. Zoe is marked sitting out
/// but posts and folds: dealt in.
const MTT_ANTES_COUNTS: &[(&str, &str)] = &[
    ("Abe", "vpip 0/1 pfr 0/1 call_vs_shove 0/1"),
    ("Bea", "vpip 0/1 pfr 0/1 rfi_ep 0/1 limp 0/1"),
    ("Cal", "vpip 0/1 pfr 0/1 rfi_ep 0/1 limp 0/1"),
    ("Dot", ""),
    ("TourneyHero", "vpip 1/1 pfr 1/1 rfi_ep 1/1 limp 0/1 call_vs_shove 1/1"),
    ("Wade", "vpip 0/1 pfr 0/1 call_vs_shove 0/1"),
    ("Xena", "vpip 0/1 pfr 0/1 call_vs_shove 0/1"),
    ("Yuri", "call_vs_shove 0/1"),
    ("Zoe", "vpip 0/1 pfr 0/1 call_vs_shove 0/1"),
];

/// Hand 101: Olga posts a dead blind from LJ (not stored as an action), her
/// check is a declined first-in spot, her raise later a squeeze; Liam sits
/// out and is not dealt in. Hand 102: Pjotr posts a big blind out of
/// position from HJ: his check is a declined first-in spot and, not being in
/// a blind seat, he has no fold-to-steal spot facing the CO steal.
const PLAYERS_COUNTS: &[(&str, &str)] = &[
    ("Hero", "vpip 1/2 pfr 1/2 rfi_mp 1/2 fold_to_3bet_ip 0/1 four_bet 0/1 limp 0/2"),
    ("Ivan", "vpip 0/1 pfr 0/1 three_bet_ip 0/1 cold_call 0/1"),
    ("Judy", "vpip 2/2 pfr 1/2 rfi_co 1/1 steal 1/1 three_bet_ip 0/1 cold_call 1/1 limp 0/1"),
    ("Ken", "vpip 0/2 pfr 0/2 three_bet_ip 0/1 three_bet_oop 0/1 squeeze 0/1 cold_call 0/2"),
    ("Liam", ""),
    ("Mia", "vpip 0/2 pfr 0/2 fold_to_steal_sb 1/1 three_bet_oop 0/2 squeeze 0/1 cold_call 0/2"),
    (
        "Ned",
        "vpip 0/2 pfr 0/2 rfi_ep 0/1 fold_to_steal_bb 1/1 three_bet_oop 0/1 cold_call 0/1 limp 0/1",
    ),
    (
        "Olga",
        "vpip 1/2 pfr 1/2 rfi_ep 0/1 rfi_mp 0/1 three_bet_oop 1/1 squeeze 1/1 cold_call 0/1 limp 0/2",
    ),
    ("Pjotr", "vpip 0/1 pfr 0/1 rfi_mp 0/1 three_bet_oop 0/1 cold_call 0/1 limp 0/1"),
];

/// Prefix names, chat lines quoting another player's action, apostrophes,
/// brackets, unicode, a name holding `": "`. Hand 801: limp, isolation
/// raise, limp-call. Hands 802 and 803 are heads-up steals.
const NAMES_COUNTS: &[(&str, &str)] = &[
    ("Ann: X", "vpip 1/1 pfr 0/1 fold_to_steal_bb 0/1 three_bet_oop 0/1 cold_call 1/1"),
    (
        "Bob",
        "vpip 1/3 pfr 1/3 rfi_btn 1/1 steal 1/1 fold_to_steal_bb 1/1 three_bet_oop 0/2 cold_call 0/2 limp 0/1",
    ),
    ("Bob Jr", "vpip 1/1 pfr 0/1 rfi_ep 0/1 limp 1/1 limp_fold 0/1 limp_call 1/1 limp_reraise 0/1"),
    ("Bob1", "vpip 1/1 pfr 0/1 three_bet_oop 0/1 cold_call 1/1"),
    ("Mr (BR)", "vpip 1/1 pfr 1/1 rfi_btn 1/1 steal 1/1 limp 0/1"),
    ("O'Brien", "vpip 0/1 pfr 0/1 iso_raise 0/1"),
    ("Zoë ゆうき", "vpip 0/1 pfr 0/1 three_bet_ip 0/1 cold_call 0/1"),
    ("[Pro] Kai", "vpip 1/1 pfr 1/1 iso_raise 1/1"),
];

/// A player named like a seat line, heads-up: the button completes, the big
/// blind checks.
const SEAT_SHAPED_COUNTS: &[(&str, &str)] = &[
    ("Alice", "vpip 1/1 pfr 0/1 rfi_btn 0/1 steal 0/1 limp 1/1"),
    ("Seat 1: Alice ($1 in chips)", "vpip 0/1 pfr 0/1 iso_raise 0/1"),
];

const CHAT_COUNTS: &[(&str, &str)] = &[
    ("Alice", "vpip 0/2 pfr 0/2 three_bet_ip 0/1 cold_call 0/1 iso_raise 0/1"),
    (
        "Bob",
        "vpip 1/2 pfr 0/2 fold_to_steal_sb 1/1 three_bet_oop 0/1 cold_call 0/1 iso_raise 0/1",
    ),
    (
        "Carol",
        "vpip 2/2 pfr 1/2 fold_to_steal_bb 0/1 three_bet_oop 1/1 cold_call 0/1 iso_raise 0/1",
    ),
    ("Dave", "vpip 0/2 pfr 0/2 rfi_ep 0/1 limp 0/1 iso_raise 0/1"),
    ("Erin", "vpip 1/2 pfr 0/2 rfi_ep 0/1 rfi_mp 0/1 limp 1/2"),
    (
        "Hero",
        "vpip 2/2 pfr 1/2 rfi_co 1/1 steal 1/1 fold_to_3bet_ip 0/1 four_bet 0/1 limp 0/1 iso_raise 0/1",
    ),
];

const ZOOM_COUNTS: &[(&str, &str)] = &[
    ("Fenwick", "vpip 0/1 pfr 0/1 fold_to_steal_sb 1/1 three_bet_oop 0/1 cold_call 0/1"),
    ("Gale", "vpip 0/1 pfr 0/1 fold_to_steal_bb 1/1 three_bet_oop 0/1 cold_call 0/1"),
    ("Hero", "vpip 1/1 pfr 1/1 rfi_btn 1/1 steal 1/1 limp 0/1"),
    ("Hollis", "vpip 0/1 pfr 0/1 rfi_ep 0/1 limp 0/1"),
    ("Iona", "vpip 0/1 pfr 0/1 rfi_mp 0/1 limp 0/1"),
    ("Jett", "vpip 0/1 pfr 0/1 rfi_co 0/1 steal 0/1 limp 0/1"),
];

/// 3-max Spin & Go at 16-16.3bb: deep by one big blind. The small blind
/// calls the button's steal; the big blind behind the caller has a squeeze
/// and a reshove spot but no fold-to-steal one (a caller came first).
const SPIN_COUNTS: &[(&str, &str)] = &[
    ("Hero", "vpip 1/1 pfr 1/1 rfi_btn 1/1 steal 1/1 limp 0/1"),
    ("SpinA", "vpip 0/1 pfr 0/1 three_bet_oop 0/1 squeeze 0/1 cold_call 0/1 reshove 0/1"),
    (
        "SpinB",
        "vpip 1/1 pfr 0/1 fold_to_steal_sb 0/1 three_bet_oop 0/1 cold_call 1/1 reshove 0/1",
    ),
];

/// 100/200, antes 25. Ace has exactly 15bb (push/fold), Bix 15.5bb (deep,
/// in the reshove band), Cue 12bb, Dax 25bb (top of the band), Eli 26bb and
/// Fin 100bb (both 26bb effective). Hand 101: Cue open-shoves, Eli calls.
/// Hand 102: Ace min-raises first in (a declined open shove, no RFI), Bix
/// 3-bets all-in from the SB (3-bet, reshove), Ace calls all-in for less.
/// Hand 103: Ace folds first in, Bix opens, Dax flats, Eli squeezes all-in;
/// Bix's fold is a fold to 3-bet with no 4-bet spot (the 3-bet is all-in).
/// Hand 104: Cue limps first in (a declined open shove, no limp spot), Dax
/// isolates. Hand 105: Ace raises to 1600 without "all-in": committing at
/// least half the stack is an open shove.
const PUSH_FOLD_COUNTS: &[(&str, &str)] = &[
    ("Ace", "open_shove 1/3 call_vs_shove 1/1"),
    (
        "Bix",
        "vpip 2/5 pfr 2/5 rfi_mp 1/1 limp 0/1 fold_to_steal_sb 0/1 three_bet_ip 0/1 three_bet_oop 1/2 \
         fold_to_3bet_ip 1/1 cold_call 0/3 call_vs_shove 0/1 reshove 1/3",
    ),
    ("Cue", "open_shove 1/2 call_vs_shove 0/1"),
    (
        "Dax",
        "vpip 2/5 pfr 1/5 rfi_ep 0/1 limp 0/1 three_bet_ip 0/2 cold_call 1/2 iso_raise 1/1 \
         call_vs_shove 0/2 reshove 0/2",
    ),
    (
        "Eli",
        "vpip 2/5 pfr 1/5 rfi_mp 0/1 limp 0/1 three_bet_ip 0/1 three_bet_oop 1/2 squeeze 1/1 \
         cold_call 0/3 call_vs_shove 1/1",
    ),
    (
        "Fin",
        "vpip 0/5 pfr 0/5 rfi_co 0/1 steal 0/1 limp 0/1 three_bet_ip 0/1 three_bet_oop 0/1 \
         cold_call 0/2 call_vs_shove 0/1",
    ),
];

/// 100bb cash. Hands 201-203 and 205: SB steals and the BB calls, 3-bets
/// (then folds to the SB's 4-bet), folds, and 3-bets then calls the 4-bet.
/// Hand 204: UTG limps, HJ isolates, CO
/// flats, BTN squeezes; the blinds face two raises (no spot), the limper
/// limp-folds, the isolation raiser folds to the 3-bet, the caller has no
/// spot left.
const SPOTS_COUNTS: &[(&str, &str)] = &[
    ("Ari", "vpip 1/5 pfr 0/5 rfi_ep 0/5 limp 1/5 limp_fold 1/1 limp_call 0/1 limp_reraise 0/1"),
    ("Bel", "vpip 1/5 pfr 1/5 rfi_mp 0/4 limp 0/4 iso_raise 1/1 fold_to_3bet_oop 1/1 four_bet 0/1"),
    ("Cas", "vpip 1/5 pfr 0/5 rfi_co 0/4 steal 0/4 limp 0/4 three_bet_ip 0/1 cold_call 1/1"),
    (
        "Dov",
        "vpip 1/5 pfr 1/5 rfi_btn 0/4 steal 0/4 limp 0/4 three_bet_ip 1/1 squeeze 1/1 cold_call 0/1",
    ),
    ("Eno", "vpip 4/5 pfr 4/5 rfi_sb 4/4 steal 4/4 limp 0/4 fold_to_3bet_oop 0/2 four_bet 2/2"),
    (
        "Fyn",
        "vpip 3/5 pfr 2/5 fold_to_steal_bb 1/4 bb_defend_vs_sb 3/4 three_bet_ip 2/4 cold_call 1/4 fold_to_4bet 1/2",
    ),
];

const FIXTURES: &[Fixture] = &[
    ("preflop_no_opportunity", NO_OPPORTUNITY, 1, NO_OPPORTUNITY_COUNTS),
    ("preflop_allin_open", ALLIN_OPEN, 2, ALLIN_OPEN_COUNTS),
    ("preflop_limped_pot", LIMPED_POT, 2, LIMPED_POT_COUNTS),
    ("preflop_heads_up", HEADS_UP, 3, HEADS_UP_COUNTS),
    ("preflop_multiway", MULTIWAY, 1, MULTIWAY_COUNTS),
    ("preflop_walk", WALK, 2, WALK_COUNTS),
    ("cash_pots_usd", POTS, 2, POTS_COUNTS),
    ("mtt_bounty_antes", MTT_ANTES, 1, MTT_ANTES_COUNTS),
    ("cash_9max_eur_players", PLAYERS, 2, PLAYERS_COUNTS),
    ("names_special", NAMES, 3, NAMES_COUNTS),
    ("names_seat_shaped", SEAT_SHAPED, 1, SEAT_SHAPED_COUNTS),
    ("cash_6max_usd_chat", CHAT, 2, CHAT_COUNTS),
    ("zoom_cash_gbp", ZOOM, 1, ZOOM_COUNTS),
    ("spin_and_go", SPIN, 1, SPIN_COUNTS),
    ("engine_preflop_push_fold", PUSH_FOLD, 5, PUSH_FOLD_COUNTS),
    ("engine_preflop_spots", SPOTS, 5, SPOTS_COUNTS),
];

// ---------------------------------------------------------------- helpers

/// `(hits, opportunities)` per preflop key; keys without an opportunity are
/// absent.
type Counts = BTreeMap<StatKey, (u32, u32)>;

fn setup_db() -> Connection {
    db::open(std::path::Path::new(":memory:")).expect("open db")
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

fn key_of(name: &str) -> StatKey {
    StatKey::PREFLOP
        .into_iter()
        .find(|k| k.as_str() == name)
        .unwrap_or_else(|| panic!("unknown preflop stat key {name:?}"))
}

fn parse(expected: &str) -> Counts {
    let tokens: Vec<&str> = expected.split_whitespace().collect();
    assert!(tokens.len() % 2 == 0, "malformed expectation {expected:?}");
    let mut out = Counts::new();
    for pair in tokens.chunks(2) {
        let (n, d) = pair[1].split_once('/').expect("n/d");
        let previous = out.insert(key_of(pair[0]), (n.parse().unwrap(), d.parse().unwrap()));
        assert!(previous.is_none(), "{} listed twice in {expected:?}", pair[0]);
    }
    out
}

fn show(counts: &Counts) -> String {
    StatKey::PREFLOP
        .iter()
        .filter_map(|k| counts.get(k).map(|(n, d)| format!("{} {n}/{d}", k.as_str())))
        .collect::<Vec<_>>()
        .join(" ")
}

/// The player's counts summed from `extract_preflop`'s events, checked
/// against `aggregate_player`'s raw all-time counts and against
/// `extract_player_preflop`. A player with no stored hand has none.
fn engine_counts_of(hands: &[HandFacts], id: i64, label: &str) -> Counts {
    let mut events = Counts::new();
    for hand in hands {
        for event in extract_preflop(hand, id) {
            assert!(event.opportunity, "{label}: an event without an opportunity");
            assert_eq!(event.player_id, id, "{label}: an event for another player");
            let c = events.entry(event.key).or_default();
            c.0 += u32::from(event.success);
            c.1 += 1;
        }
    }

    // Every spot facing one raise is IP or OOP: no 3-bet spot is lost to an
    // undefined relation.
    let opps = |key: StatKey| events.get(&key).map_or(0, |c| c.1);
    assert_eq!(
        opps(StatKey::ColdCall),
        opps(StatKey::ThreeBetIp) + opps(StatKey::ThreeBetOop),
        "{label}: 3-bet spots without IP/OOP"
    );

    let per_hand: u32 = extract_player_preflop(hands, id).iter().map(|h| h.events.len() as u32).sum();
    assert_eq!(per_hand, events.values().map(|c| c.1).sum::<u32>(), "{label}: extract_player_preflop");

    let format = hands.first().map_or(FormatKey::Cash, FormatKey::of_hand);
    let agg = aggregate_player(hands, id, format);
    for key in StatKey::PREFLOP {
        let a = agg.counts(View::AllTime, key);
        assert_eq!(
            (a.hits, a.opportunities),
            events.get(&key).copied().unwrap_or_default(),
            "{label}: aggregate_player and extract_preflop disagree on {}",
            key.as_str()
        );
    }
    events
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
fn check_fixture(fixture: &(&str, &str, i64, &[(&str, &str)])) {
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
fn no_opportunity_limped_pot_gives_isolation_spots_and_nothing_else() {
    check_fixture(fixture("preflop_no_opportunity"));
}

#[test]
fn all_in_open_is_a_shove_for_everyone_behind_and_a_partial_call_is_a_call_vs_shove() {
    check_fixture(fixture("preflop_allin_open"));
}

#[test]
fn limped_pot_isolation_limp_reraise_and_limp_call() {
    check_fixture(fixture("preflop_limped_pot"));
}

#[test]
fn heads_up_open_three_bet_four_bet_completion_and_walk() {
    check_fixture(fixture("preflop_heads_up"));
}

#[test]
fn multiway_squeeze_leaves_no_spot_to_the_players_behind_it() {
    check_fixture(fixture("preflop_multiway"));
}

#[test]
fn walk_gives_the_big_blind_no_decision_in_cash_and_with_antes() {
    check_fixture(fixture("preflop_walk"));
}

#[test]
fn all_in_three_bet_reshove_and_re_shove_over_a_shove() {
    check_fixture(fixture("cash_pots_usd"));
}

#[test]
fn antes_short_stacks_and_a_sitting_out_player_who_posts() {
    check_fixture(fixture("mtt_bounty_antes"));
}

#[test]
fn dead_blind_out_of_position_big_blind_and_sitting_out_players() {
    check_fixture(fixture("cash_9max_eur_players"));
}

#[test]
fn similar_and_special_character_names_keep_their_own_counts() {
    check_fixture(fixture("names_special"));
    check_fixture(fixture("names_seat_shaped"));
}

#[test]
fn six_max_zoom_and_spin_formats() {
    check_fixture(fixture("cash_6max_usd_chat"));
    check_fixture(fixture("zoom_cash_gbp"));
    check_fixture(fixture("spin_and_go"));
}

#[test]
fn push_fold_split_at_fifteen_big_blinds_and_reshove_band() {
    check_fixture(fixture("engine_preflop_push_fold"));
}

#[test]
fn blind_battle_limp_fold_and_spots_behind_a_squeeze() {
    check_fixture(fixture("engine_preflop_spots"));
}

#[test]
fn every_preflop_key_is_pinned_with_hits_and_misses() {
    let mut totals: BTreeMap<StatKey, (u32, u32)> = BTreeMap::new();
    for (_, _, _, expected) in FIXTURES {
        for (_, want) in *expected {
            for (key, (n, d)) in parse(want) {
                let t = totals.entry(key).or_default();
                t.0 += n;
                t.1 += d;
            }
        }
    }
    assert_eq!(StatKey::PREFLOP.len(), 27);
    for key in StatKey::PREFLOP {
        let (hits, opportunities) = totals.get(&key).copied().unwrap_or_default();
        assert!(hits > 0, "no fixture pins a hit of {}", key.as_str());
        assert!(hits < opportunities, "no fixture pins a miss of {}", key.as_str());
    }
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
        assert!(engine_counts(&conn, name).is_empty(), "{name} counted from a cut hand");
    }

    // The complete file then counts exactly as pinned, and a late cut copy
    // changes nothing.
    import::import_text(&mut conn, ALLIN_OPEN).unwrap();
    assert_engine_counts(&conn, "preflop_allin_open after cut copies", ALLIN_OPEN_COUNTS);
    let before = snapshot(&conn);
    let late = import::import_text(&mut conn, TRUNCATED).unwrap();
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

/// The stats both engines compute, in `stats/mod.rs`'s terms. The engine
/// side of each is the sum of its keys in [`engine_side`].
const SHARED: [&str; 12] = ["vpip", "pfr", "3b", "f3b", "4b", "f4b", "rfi", "limp", "cc", "sqz", "steal", "fts"];

/// `(hits, opportunities)` per [`SHARED`] stat.
type Shared = [(i64, i64); 12];

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
        return [(0, 0); 12];
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
        (s.steal_attempt, o.steal_attempt_opportunities),
        (s.fold_to_steal, o.fold_to_steal_opportunities),
    ];
    let mut out = [(0, 0); 12];
    for (i, (pct, den)) in pairs.into_iter().enumerate() {
        out[i] = (hits(pct, den, &format!("{name} {}", SHARED[i])), den);
    }
    out
}

/// The engine's keys behind each [`SHARED`] stat: summed where the engine
/// splits a stat by position group, blind or IP/OOP.
fn engine_keys(stat: &str) -> &'static [StatKey] {
    match stat {
        "vpip" => &[StatKey::Vpip],
        "pfr" => &[StatKey::Pfr],
        "3b" => &[StatKey::ThreeBetIp, StatKey::ThreeBetOop],
        "f3b" => &[StatKey::FoldTo3betIp, StatKey::FoldTo3betOop],
        "4b" => &[StatKey::FourBet],
        "f4b" => &[StatKey::FoldTo4bet],
        "rfi" => &[StatKey::RfiEp, StatKey::RfiMp, StatKey::RfiCo, StatKey::RfiBtn, StatKey::RfiSb],
        "limp" => &[StatKey::Limp],
        "cc" => &[StatKey::ColdCall],
        "sqz" => &[StatKey::Squeeze],
        "steal" => &[StatKey::Steal],
        "fts" => &[StatKey::FoldToStealSb, StatKey::FoldToStealBb],
        other => panic!("unknown shared stat {other}"),
    }
}

fn engine_side(counts: &Counts) -> Shared {
    let mut out = [(0, 0); 12];
    for (i, stat) in SHARED.iter().enumerate() {
        for key in engine_keys(stat) {
            let (n, d) = counts.get(key).copied().unwrap_or_default();
            out[i].0 += i64::from(n);
            out[i].1 += i64::from(d);
        }
    }
    out
}

/// Why `stats/mod.rs` and the engine count a spot differently. Each is a
/// documented difference of definition (`stat-contracts.md`, "Opponent
/// engine divergences"; `opponent-engine.md` §9.1), not a defect.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Divergence {
    /// stats/mod.rs puts every dealt-in hand in the VPIP/PFR denominator;
    /// the engine needs a decision, and a big blind's walk has none.
    WalkBigBlind,
    /// At 15bb effective or less the engine counts only `open_shove`,
    /// `call_vs_shove` and `reshove`; stats/mod.rs reads no stack.
    PushFold,
    /// The engine treats an all-in open as a shove: the players behind face
    /// `call_vs_shove`, not a 3-bet, cold-call or fold-to-steal spot.
    AllInOpen,
    /// stats/mod.rs files the opener's answer to open, 3-bet and 4-bet as a
    /// fold-to-3-bet spot; the engine counts it only facing the 3-bet alone.
    ThreeBetPileup,
    /// stats/mod.rs gives a limper facing the first raise a 3-bet spot; the
    /// engine's 3-bet needs no money of the player's own in the pot (the
    /// open-limper gets `limp_fold`/`limp_call`/`limp_reraise` instead).
    LimperFacingRaise,
    /// stats/mod.rs gives a player with no voluntary money in a cold-call
    /// (and, after a caller, a squeeze) spot facing any number of raises;
    /// the engine only facing the open.
    ColdCallBeyondOpen,
    /// stats/mod.rs gives anyone facing two raises a 4-bet spot; the engine
    /// only the opener answering the 3-bet.
    FourBetNotOpener,
    /// stats/mod.rs gives a blind facing a steal a fold-to-steal spot even
    /// when someone called the steal first; the engine requires no caller.
    CallerBeforeSteal,
    /// Facing an all-in 3-bet, the opener has no `four_bet` spot in the
    /// engine; stats/mod.rs counts one.
    AllInThreeBet,
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
        for (i, stat) in SHARED.iter().enumerate() {
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
        // Hand 201: everyone behind Ike's shove; hand 202: Gus behind Lou's.
        "preflop_allin_open" => &[
            ("Gus", "3b", "0/1", "0/0", &[AllInOpen]),
            ("Gus", "cc", "0/1", "0/0", &[AllInOpen]),
            ("Gus", "fts", "1/1", "0/0", &[AllInOpen]),
            ("Jo", "3b", "0/1", "0/0", &[AllInOpen]),
            ("Jo", "cc", "0/1", "0/0", &[AllInOpen]),
            ("Kim", "3b", "0/1", "0/0", &[AllInOpen]),
            ("Kim", "cc", "0/1", "0/0", &[AllInOpen]),
            ("Kim", "fts", "1/1", "0/0", &[AllInOpen]),
            ("Lou", "3b", "0/1", "0/0", &[AllInOpen]),
            ("Lou", "cc", "1/1", "0/0", &[AllInOpen]),
            ("Lou", "fts", "0/1", "0/0", &[AllInOpen]),
        ],
        // Mo, Quin and Rex limped before facing the first raise; Nia (a
        // limper) and Pia (a caller) then face Mo's re-raise.
        "preflop_limped_pot" => &[
            ("Mo", "3b", "1/1", "0/0", &[LimperFacingRaise]),
            ("Nia", "4b", "0/1", "0/0", &[FourBetNotOpener]),
            ("Pia", "4b", "0/1", "0/0", &[FourBetNotOpener]),
            ("Quin", "3b", "0/2", "0/1", &[LimperFacingRaise]),
            ("Rex", "3b", "0/2", "0/1", &[LimperFacingRaise]),
        ],
        // Hand 403 is Ivy's walk.
        "preflop_heads_up" => &[
            ("Ivy", "vpip", "2/3", "2/2", &[WalkBigBlind]),
            ("Ivy", "pfr", "1/3", "1/2", &[WalkBigBlind]),
        ],
        // The callers Cy and Di face the squeeze with money in; Gil, Hu and
        // Io face it cold (Io after Hu's call: a squeeze spot in stats/mod.rs).
        "preflop_multiway" => &[
            ("Cy", "4b", "0/1", "0/0", &[FourBetNotOpener]),
            ("Di", "4b", "0/1", "0/0", &[FourBetNotOpener]),
            ("Gil", "4b", "0/1", "0/0", &[FourBetNotOpener]),
            ("Gil", "cc", "0/1", "0/0", &[ColdCallBeyondOpen]),
            ("Hu", "4b", "0/1", "0/0", &[FourBetNotOpener]),
            ("Hu", "cc", "1/1", "0/0", &[ColdCallBeyondOpen]),
            ("Io", "4b", "0/1", "0/0", &[FourBetNotOpener]),
            ("Io", "cc", "0/1", "0/0", &[ColdCallBeyondOpen]),
            ("Io", "sqz", "0/1", "0/0", &[ColdCallBeyondOpen]),
        ],
        "preflop_walk" => &[
            ("Uli", "vpip", "0/1", "0/0", &[WalkBigBlind]),
            ("Uli", "pfr", "0/1", "0/0", &[WalkBigBlind]),
            ("Yan", "vpip", "0/1", "0/0", &[WalkBigBlind]),
            ("Yan", "pfr", "0/1", "0/0", &[WalkBigBlind]),
        ],
        // Hand 301: Tara called Rosa's steal before the blinds acted.
        // Hand 302: Rosa and Tara face two and three raises cold; Uma folds
        // after the 3-bet and the 4-bet.
        "cash_pots_usd" => &[
            ("Quinn", "fts", "0/1", "0/0", &[CallerBeforeSteal]),
            ("Rosa", "4b", "1/1", "0/0", &[FourBetNotOpener]),
            ("Rosa", "cc", "0/1", "0/0", &[ColdCallBeyondOpen]),
            ("Tara", "cc", "2/2", "1/1", &[ColdCallBeyondOpen]),
            ("Uma", "f3b", "1/1", "0/0", &[ThreeBetPileup]),
            ("Uma", "fts", "1/1", "0/0", &[CallerBeforeSteal]),
        ],
        // Dot (7bb) and Yuri (10bb) are push/fold; everyone else behind
        // Dot's 3-bet faces two or three raises cold.
        "mtt_bounty_antes" => &[
            ("Abe", "cc", "0/1", "0/0", &[ColdCallBeyondOpen]),
            ("Dot", "vpip", "1/1", "0/0", &[PushFold]),
            ("Dot", "pfr", "1/1", "0/0", &[PushFold]),
            ("Dot", "3b", "1/1", "0/0", &[PushFold]),
            ("Dot", "cc", "0/1", "0/0", &[PushFold]),
            ("TourneyHero", "f3b", "0/1", "0/0", &[ThreeBetPileup]),
            ("Wade", "4b", "0/1", "0/0", &[FourBetNotOpener]),
            ("Wade", "cc", "0/1", "0/0", &[ColdCallBeyondOpen]),
            ("Xena", "4b", "0/1", "0/0", &[FourBetNotOpener]),
            ("Xena", "cc", "0/1", "0/0", &[ColdCallBeyondOpen]),
            ("Yuri", "vpip", "1/1", "0/0", &[PushFold]),
            ("Yuri", "pfr", "1/1", "0/0", &[PushFold]),
            ("Yuri", "4b", "1/1", "0/0", &[PushFold, FourBetNotOpener]),
            ("Yuri", "cc", "0/1", "0/0", &[PushFold, ColdCallBeyondOpen]),
            ("Zoe", "cc", "0/1", "0/0", &[ColdCallBeyondOpen]),
        ],
        // Hand 101: Judy called Hero's open, then faces Olga's squeeze.
        "cash_9max_eur_players" => &[("Judy", "4b", "0/1", "0/0", &[FourBetNotOpener])],
        // Hand 801: Bob Jr limped, then faces the isolation raise.
        "names_special" => &[("Bob Jr", "3b", "0/1", "0/0", &[LimperFacingRaise])],
        // SpinB called the button's steal before SpinA acted.
        "spin_and_go" => &[("SpinA", "fts", "1/1", "0/0", &[CallerBeforeSteal])],
        // Ace (15bb) and Cue (12bb) are push/fold in every hand. Hand 101 is
        // Cue's all-in open; in hand 103 Bix's open faces an all-in squeeze
        // that Dax (a caller) and Fin (cold) face too.
        "engine_preflop_push_fold" => &[
            ("Ace", "vpip", "2/5", "0/0", &[PushFold]),
            ("Ace", "pfr", "2/5", "0/0", &[PushFold]),
            ("Ace", "3b", "0/2", "0/0", &[PushFold, AllInOpen]),
            ("Ace", "f3b", "0/1", "0/0", &[PushFold]),
            ("Ace", "4b", "0/1", "0/0", &[PushFold]),
            ("Ace", "rfi", "2/3", "0/0", &[PushFold]),
            ("Ace", "limp", "0/3", "0/0", &[PushFold]),
            ("Ace", "cc", "0/2", "0/0", &[PushFold, AllInOpen]),
            ("Ace", "sqz", "0/1", "0/0", &[PushFold, AllInOpen]),
            ("Ace", "steal", "1/1", "0/0", &[PushFold]),
            ("Bix", "3b", "1/4", "1/3", &[AllInOpen]),
            ("Bix", "4b", "0/1", "0/0", &[AllInThreeBet]),
            ("Bix", "cc", "0/4", "0/3", &[AllInOpen]),
            ("Bix", "sqz", "0/1", "0/0", &[AllInOpen]),
            ("Cue", "vpip", "2/5", "0/0", &[PushFold]),
            ("Cue", "pfr", "1/5", "0/0", &[PushFold]),
            ("Cue", "3b", "0/3", "0/0", &[PushFold, LimperFacingRaise]),
            ("Cue", "4b", "0/1", "0/0", &[PushFold, FourBetNotOpener]),
            ("Cue", "rfi", "1/2", "0/0", &[PushFold]),
            ("Cue", "limp", "1/2", "0/0", &[PushFold]),
            ("Cue", "cc", "0/3", "0/0", &[PushFold, ColdCallBeyondOpen]),
            ("Dax", "3b", "0/3", "0/2", &[AllInOpen]),
            ("Dax", "4b", "0/1", "0/0", &[FourBetNotOpener]),
            ("Dax", "cc", "1/3", "1/2", &[AllInOpen]),
            ("Eli", "3b", "1/4", "1/3", &[AllInOpen]),
            ("Eli", "cc", "1/4", "0/3", &[AllInOpen]),
            ("Fin", "3b", "0/3", "0/2", &[AllInOpen]),
            ("Fin", "4b", "0/1", "0/0", &[FourBetNotOpener]),
            ("Fin", "cc", "0/4", "0/2", &[AllInOpen, ColdCallBeyondOpen]),
            ("Fin", "sqz", "0/1", "0/0", &[AllInOpen]),
        ],
        // Hand 204: the limper Ari and the caller Cas face the squeeze with
        // money in, the blinds face it cold.
        "engine_preflop_spots" => &[
            ("Ari", "4b", "0/1", "0/0", &[FourBetNotOpener]),
            ("Cas", "4b", "0/1", "0/0", &[FourBetNotOpener]),
            ("Eno", "4b", "2/3", "2/2", &[FourBetNotOpener]),
            ("Eno", "cc", "0/1", "0/0", &[ColdCallBeyondOpen]),
            ("Fyn", "4b", "0/1", "0/0", &[FourBetNotOpener]),
            ("Fyn", "cc", "1/5", "1/4", &[ColdCallBeyondOpen]),
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

/// One hand of the clean corpus: every stack 100bb, no all-in, no walk, and
/// no spot where the two engines differ by definition. So: limped pots are
/// never raised; a steal is folded to the big blind; only the big blind
/// re-raises, which nobody behind it can face cold; and a squeeze is always
/// 4-bet by the opener, so the callers answer three raises (no 4-bet spot in
/// either engine) instead of two.
fn clean_hand(t: &mut Table, rng: &mut Rng, minute: i64) -> String {
    let start = 100.0 * t.bb;
    for stack in t.stacks.values_mut() {
        *stack = start;
    }
    let order = seat_order(t);
    let n = order.len();
    // Preflop acting order: UTG first, the blinds last.
    let pre: Vec<String> = order[2..].iter().chain(order[..2].iter()).cloned().collect();
    let bb = n - 1;
    let steal = |i: usize| i + 4 >= n && i < bb;

    let mut lines: Vec<(usize, Act)> = Vec::new();
    let first = rng.below(bb);
    lines.extend((0..first).map(|i| (i, Act::Fold)));
    // Players still in at the end of preflop; the writer shows at most four.
    let mut live: Vec<usize> = vec![first];
    if rng.one_in(4) {
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
        let mut callers = Vec::new();
        for i in first + 1..bb {
            if !steal(first) && callers.len() < 2 && rng.one_in(3) {
                lines.push((i, Act::Call));
                callers.push(i);
            } else {
                lines.push((i, Act::Fold));
            }
        }
        live.extend(&callers);
        match rng.below(3) {
            0 => lines.push((bb, Act::Fold)),
            1 => {
                lines.push((bb, Act::Call));
                live.push(bb);
            }
            _ => {
                lines.push((bb, Act::Raise(if callers.is_empty() { 9.0 } else { 12.0 })));
                let answer = if callers.is_empty() { rng.below(3) } else { 2 };
                live = vec![first, bb];
                match answer {
                    0 => {
                        lines.push((first, Act::Fold));
                        live = vec![bb];
                    }
                    1 => lines.push((first, Act::Call)),
                    _ => {
                        lines.push((first, Act::Raise(22.0)));
                        lines.extend(callers.iter().map(|&c| (c, Act::Fold)));
                        if rng.one_in(2) {
                            lines.push((bb, Act::Fold));
                            live = vec![first];
                        } else {
                            lines.push((bb, Act::Call));
                        }
                    }
                }
            }
        }
    }

    // Postflop the small blind acts first: checked down to a showdown.
    live.sort_by_key(|&i| (i + 2) % n);
    let preflop: Vec<Line> = lines.iter().map(|&(i, act)| (pre[i].as_str(), act)).collect();
    let checks: Vec<Line> = live.iter().map(|&i| (pre[i].as_str(), Act::Check)).collect();
    let text = if live.len() > 1 {
        let winner = pre[live[0]].clone();
        t.play(minute, &[&preflop, &checks, &checks, &checks], Some(&winner))
    } else {
        t.play(minute, &[&preflop], None)
    };
    next_button(t);
    text
}

/// A table of `players`, `hands` clean hands one minute apart.
fn clean_table(name: &str, first_id: u64, players: &[&str], hands: usize, seed: u64) -> String {
    let mut t = Table::cash(name, first_id, players);
    t.max = if players.len() > 6 { 9 } else { 6 };
    let mut rng = Rng(seed);
    (0..hands).map(|i| clean_hand(&mut t, &mut rng, i as i64)).collect()
}

const SIX_MAX: [&str; 6] = ["Hero", "Sam", "Tess", "Ugo", "Vera", "Walt"];
const NINE_MAX: [&str; 9] = ["Abby", "Bram", "Cleo", "Dirk", "Edda", "Finn", "Gwen", "Hugo", "Ines"];
const SHORT_HANDED: [&str; 3] = ["Jago", "Kira", "Lars"];

/// 300 6-max hands, 180 9-max hands and 120 three-handed hands.
fn clean_corpus() -> (Connection, i64) {
    let text = [
        clean_table("Clean Six", 281_000_000_001, &SIX_MAX, 300, 0x9E37_79B9_7F4A_7C15),
        clean_table("Clean Nine", 282_000_000_001, &NINE_MAX, 180, 0x2545_F491_4F6C_DD1D),
        clean_table("Clean Three", 283_000_000_001, &SHORT_HANDED, 120, 0x1405_7B7E_F767_814F),
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
fn clean_corpus_engines_agree_exactly_on_every_shared_stat() {
    let (conn, hands) = clean_corpus();
    assert_eq!(hands, 600);

    let mut totals = [(0i64, 0i64); 12];
    let mut split: BTreeMap<StatKey, (u32, u32)> = BTreeMap::new();
    for name in corpus_players() {
        let counts = engine_counts(&conn, name);
        let engine = engine_side(&counts);
        let stats = stats_side(&conn, name);
        for (i, stat) in SHARED.iter().enumerate() {
            assert_eq!(
                fraction(stats[i]),
                fraction(engine[i]),
                "{name} {stat}: stats/mod.rs (left) and the engine (right) disagree"
            );
            totals[i].0 += engine[i].0;
            totals[i].1 += engine[i].1;
        }
        // Every hand is a decision above 15bb: VPIP's denominator is the
        // hand count in both engines.
        let dealt: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM player_hands WHERE player_id = ?1",
                [player_id(&conn, name).unwrap()],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(engine[0].1, dealt, "{name}: VPIP opportunities");

        // IP/OOP is defined for every 3-bet and fold-to-3-bet spot: the
        // remainder the split leaves uncounted is zero. Every spot facing one
        // raise is a cold-call spot, and with no all-in 3-bet every answer
        // to a 3-bet is a 4-bet spot.
        let opps = |key: StatKey| counts.get(&key).map_or(0, |c| c.1);
        assert_eq!(
            opps(StatKey::ColdCall) - opps(StatKey::ThreeBetIp) - opps(StatKey::ThreeBetOop),
            0,
            "{name}: 3-bet spots without IP/OOP"
        );
        assert_eq!(
            opps(StatKey::FourBet) - opps(StatKey::FoldTo3betIp) - opps(StatKey::FoldTo3betOop),
            0,
            "{name}: fold-to-3-bet spots without IP/OOP"
        );
        for (key, c) in counts {
            let t = split.entry(key).or_default();
            t.0 += c.0;
            t.1 += c.1;
        }
    }

    // The agreement means something: every shared stat has hits and misses,
    // and every split key behind it has spots.
    for (i, stat) in SHARED.iter().enumerate() {
        let (hits, opps) = totals[i];
        assert!(hits > 0 && hits < opps, "clean corpus {stat}: {hits}/{opps} pins no hit or no miss");
        for key in engine_keys(stat) {
            assert!(split.get(key).is_some_and(|c| c.1 > 0), "clean corpus: no {} spot", key.as_str());
        }
    }
    // No spot outside the clean shapes: a limp is never raised (players
    // behind a limper only decline isolation spots), no stack is short.
    assert_eq!(split.get(&StatKey::IsoRaise).map(|c| c.0), Some(0), "clean corpus has an isolation raise");
    for key in [
        StatKey::LimpFold,
        StatKey::LimpCall,
        StatKey::LimpReraise,
        StatKey::OpenShove,
        StatKey::CallVsShove,
        StatKey::Reshove,
    ] {
        assert!(!split.contains_key(&key), "clean corpus has a {} spot", key.as_str());
    }
}

// ---------------------------------------------------------------- recent form

/// `form_counts`' window (the last `window_hands`) and baseline, recounted
/// from `extract_preflop`'s VPIP events over the same hands.
fn assert_form_matches_events(conn: &Connection, name: &str, window_hands: usize) {
    let id = player_id(conn, name).unwrap();
    let hands = load_player_hands(conn, id).unwrap();
    let vpip = |slice: &[HandFacts]| {
        slice.iter().fold((0u32, 0u32), |(n, x), hand| {
            match extract_preflop(hand, id).iter().find(|e| e.key == StatKey::Vpip) {
                Some(e) => (n + 1, x + u32::from(e.success)),
                None => (n, x),
            }
        })
    };
    let split = hands.len() - window_hands;
    let (n, x) = vpip(&hands[split..]);
    let (base_n, base_x) = vpip(&hands[..split]);
    let form = form_counts(&hands, id).unwrap_or_else(|| panic!("{name}: no recent-form counts"));
    assert_eq!(
        (form.window, form.window_hits, form.baseline, form.baseline_hits),
        (n, x, base_n, base_x),
        "{name}: recent form's VPIP counts are not extract_preflop's"
    );
    let all = engine_counts(conn, name)[&StatKey::Vpip];
    assert_eq!(
        (form.window + form.baseline, form.window_hits + form.baseline_hits),
        (all.1, all.0),
        "{name}: recent form misses VPIP spots"
    );
}

#[test]
fn recent_form_vpip_counts_equal_extract_preflop_counts() {
    // The clean corpus: one minute apart, so the window is the last 12.
    let (conn, _) = clean_corpus();
    for name in corpus_players() {
        assert_form_matches_events(&conn, name, LAST_N);
    }

    // Walks (no VPIP spot for the big blind) and a break: the last 11 hands
    // come two hours after the rest, so the 60-minute cut, not the last-12
    // cut, closes the window.
    let mut t = Table::cash("Form Table", 284_000_000_001, &SIX_MAX);
    let mut rng = Rng(0xD1B5_4A32_D192_ED03);
    let mut text = String::new();
    for i in 0..80i64 {
        let minute = if i < 69 { i } else { 120 + i };
        if i % 5 == 0 {
            text.push_str(&t.walk(minute));
            next_button(&mut t);
        } else {
            text.push_str(&clean_hand(&mut t, &mut rng, minute));
        }
    }
    let conn = common::import(&text);
    let stored: i64 = conn
        .query_row("SELECT COUNT(*) FROM hands", [], |row| row.get(0))
        .unwrap();
    assert_eq!(stored, 80);
    for name in SIX_MAX {
        assert_form_matches_events(&conn, name, 11);
        let id = common::player_id(&conn, name);
        let hands = load_player_hands(&conn, id).unwrap();
        let vpip = engine_counts(&conn, name)[&StatKey::Vpip];
        assert!(vpip.1 < hands.len() as u32, "{name}: a walk gave the big blind a VPIP spot");
    }
}
