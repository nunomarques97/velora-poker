use velora_poker_lib::{db, import, stats};

const SHOWDOWN_HAND: &str = include_str!("fixtures/hand_3bet_showdown.txt");
const CBET_FOLD_HAND: &str = include_str!("fixtures/hand_cbet_fold.txt");
const LIMPED_HAND: &str = include_str!("fixtures/hand_limped_multiway.txt");
const FOLD_TO_3BET_HAND: &str = include_str!("fixtures/hand_fold_to_3bet.txt");
const ALLIN_PREFLOP_FOLDED_HAND: &str = include_str!("fixtures/hand_allin_preflop_folded.txt");
const ALLIN_PREFLOP_SHOWDOWN_HAND: &str = include_str!("fixtures/hand_allin_preflop_showdown.txt");
const FOUR_BET_HAND: &str = include_str!("fixtures/hand_4bet.txt");
const FOUR_BET_PILEUP_HAND: &str = include_str!("fixtures/hand_4bet_pileup.txt");
const LIMP_BEHIND_LIMP_HAND: &str = include_str!("fixtures/hand_limp_behind_limp.txt");
const WALK_HAND: &str = include_str!("fixtures/hand_walk.txt");
const COLD_CALL_HAND: &str = include_str!("fixtures/hand_cold_call.txt");
const COLD_CALL_AFTER_LIMP_HAND: &str = include_str!("fixtures/hand_cold_call_after_limp.txt");
const SQUEEZE_HAND: &str = include_str!("fixtures/hand_squeeze.txt");
const HEADS_UP_STEAL_HAND: &str = include_str!("fixtures/hand_heads_up_steal.txt");
const ISOLATION_RAISE_NOT_A_STEAL_HAND: &str =
    include_str!("fixtures/hand_isolation_raise_not_a_steal.txt");

fn setup_db() -> rusqlite::Connection {
    // The special ":memory:" filename gives an isolated in-memory database
    // per connection while still going through the real schema/init path.
    db::open(std::path::Path::new(":memory:")).expect("open db")
}

fn import_all_fixtures(conn: &mut rusqlite::Connection) {
    for text in [SHOWDOWN_HAND, CBET_FOLD_HAND, LIMPED_HAND, FOLD_TO_3BET_HAND] {
        import::import_text(conn, text).expect("import should succeed");
    }
}

fn player_id(conn: &rusqlite::Connection, name: &str) -> i64 {
    conn.query_row(
        "SELECT id FROM players WHERE name = ?1",
        [name],
        |row| row.get(0),
    )
    .unwrap_or_else(|_| panic!("player {name} not found"))
}

fn assert_close(actual: Option<f64>, expected: f64, label: &str) {
    let actual = actual.unwrap_or_else(|| panic!("{label}: expected Some({expected}), got None"));
    assert!(
        (actual - expected).abs() < 0.05,
        "{label}: expected {expected}, got {actual}"
    );
}

fn assert_no_opportunity(actual: Option<f64>, label: &str) {
    assert!(
        actual.is_none(),
        "{label}: expected None (no opportunity), got {actual:?}"
    );
}

#[test]
fn imports_all_fixture_hands_exactly_once() {
    let mut conn = setup_db();
    import_all_fixtures(&mut conn);
    assert_eq!(db::count_hands(&conn).unwrap(), 4);

    // Re-importing the same content must be a no-op (idempotent on hand id).
    import_all_fixtures(&mut conn);
    assert_eq!(db::count_hands(&conn).unwrap(), 4);
}

#[test]
fn computes_hero_statistics() {
    let mut conn = setup_db();
    import_all_fixtures(&mut conn);

    let hero_id = player_id(&conn, "Hero");
    let s = stats::compute_player_stats(&conn, hero_id).unwrap();

    assert_close(s.vpip, 100.0, "hero vpip");
    assert_close(s.pfr, 75.0, "hero pfr");
    assert_no_opportunity(s.three_bet, "hero three_bet");
    assert_close(s.fold_to_three_bet, 50.0, "hero fold_to_three_bet");
    // Phase 2, stat 2 (RFI%/Limp%): Hero is the button in all four fixtures
    // and always acts first preflop in this 3-max table, so all four hands
    // are RFI/limp opportunities for Hero — raises in 3 (showdown, cbet_fold,
    // fold_to_3bet) and limps in 1 (limped_multiway).
    assert_close(s.rfi, 75.0, "hero rfi");
    assert_close(s.limp, 25.0, "hero limp");
    // Phase 2, stat 3 (Cold Call%): Hero is always the opener (button acting
    // first in these 3-max fixtures) or the limper, and never faces a raise
    // before having voluntarily acted themselves — no cold-call opportunity
    // ever arises for Hero across these four hands.
    assert_no_opportunity(s.cold_call, "hero cold_call");
    // Phase 2, stat 4 (Squeeze%/Fold-to-Squeeze%): Hero never faces a raise
    // before acting (same reason cold_call has no opportunity), and never
    // raises into a spot with a live caller behind them, so neither stat has
    // an opportunity across these four hands.
    assert_no_opportunity(s.squeeze, "hero squeeze");
    assert_no_opportunity(s.fold_to_squeeze, "hero fold_to_squeeze");
    assert_close(s.c_bet, 100.0, "hero c_bet");
    assert_close(s.fold_to_c_bet, 0.0, "hero fold_to_c_bet");
    assert_close(s.aggression_factor, 1.0, "hero aggression_factor");
    assert_close(s.wtsd, 33.3, "hero wtsd");
    assert_close(s.wsd, 0.0, "hero wsd");
    // steal_attempt%: Hero (BTN, a steal position at this 3-max table, where
    // there's no CO label — the position set degrades to {BTN, SB} exactly
    // as position-contracts.md §4 describes) is first-in in all 4 fixtures:
    // raises in 3 (showdown, cbet_fold, fold_to_3bet), limps in 1
    // (limped_multiway) — same 3/4 split as rfi/limp above, since every
    // steal_attempt opportunity here is also an RFI/Limp opportunity.
    assert_close(s.steal_attempt, 75.0, "hero steal_attempt");
    // fold_to_steal%: Hero's own position (BTN) is never SB/BB, so this
    // opportunity never arises for Hero.
    assert_no_opportunity(s.fold_to_steal, "hero fold_to_steal");
}

#[test]
fn computes_villain_statistics() {
    let mut conn = setup_db();
    import_all_fixtures(&mut conn);

    let villain_id = player_id(&conn, "Villain");
    let s = stats::compute_player_stats(&conn, villain_id).unwrap();

    assert_close(s.vpip, 50.0, "villain vpip");
    assert_close(s.pfr, 0.0, "villain pfr");
    assert_close(s.three_bet, 0.0, "villain three_bet");
    assert_no_opportunity(s.fold_to_three_bet, "villain fold_to_three_bet");
    // Villain (SB) always acts after Hero (button), who voluntarily enters
    // every hand, so Villain never gets a genuine "first to voluntarily act"
    // spot across these four fixtures — correctly `None`, not a fabricated 0%.
    assert_no_opportunity(s.rfi, "villain rfi");
    assert_no_opportunity(s.limp, "villain limp");
    // Phase 2, stat 3 (Cold Call%): unlike RFI/Limp (gated on nobody having
    // voluntarily entered), Cold Call%'s opportunity is gated on Villain's
    // own first voluntary action, and requires a prior raise — a condition
    // Hero's every-hand open satisfies. Villain faces that open cold in three
    // of the four hands (showdown, cbet_fold, fold_to_3bet) and cold-calls in
    // exactly one (cbet_fold): 1/3 = 33.3%. The fourth hand (limped_multiway)
    // has no preflop raise at all, so it contributes no opportunity.
    assert_close(s.cold_call, 33.3, "villain cold_call");
    // Phase 2, stat 4: Villain is always the first to respond to Hero's open
    // in these fixtures, so nobody has ever called ahead of Villain's own
    // turn — every one of Villain's cold-call opportunities has
    // called_since_last_raise == false, so none of them is also a squeeze
    // opportunity. Villain also never raises, so is never the one squeezed.
    assert_no_opportunity(s.squeeze, "villain squeeze");
    assert_no_opportunity(s.fold_to_squeeze, "villain fold_to_squeeze");
    assert_no_opportunity(s.c_bet, "villain c_bet");
    assert_close(s.fold_to_c_bet, 100.0, "villain fold_to_c_bet");
    assert_close(s.aggression_factor, 1.0, "villain aggression_factor");
    assert_close(s.wtsd, 0.0, "villain wtsd");
    assert_no_opportunity(s.wsd, "villain wsd");
    // steal_attempt%: Villain never gets a genuine first-to-voluntarily-act
    // spot (same reason rfi/limp are None above), so steal_attempt has no
    // opportunity either even though Villain's own position (SB) is itself a
    // steal position.
    assert_no_opportunity(s.steal_attempt, "villain steal_attempt");
    // fold_to_steal%: Villain (SB) faces Hero's steal raise in 3 of the 4
    // hands (showdown, cbet_fold, fold_to_3bet — limped_multiway has no
    // preflop raise, so no steal attempt occurs there at all) and folds in 2
    // of those 3 (showdown, fold_to_3bet), calling in cbet_fold: 2/3 = 66.7%.
    assert_close(s.fold_to_steal, 66.7, "villain fold_to_steal");
}

#[test]
fn computes_robot_statistics() {
    let mut conn = setup_db();
    import_all_fixtures(&mut conn);

    let robot_id = player_id(&conn, "Robot");
    let s = stats::compute_player_stats(&conn, robot_id).unwrap();

    assert_close(s.vpip, 50.0, "robot vpip");
    assert_close(s.pfr, 50.0, "robot pfr");
    assert_close(s.three_bet, 66.7, "robot three_bet");
    assert_no_opportunity(s.fold_to_three_bet, "robot fold_to_three_bet");
    // Robot (BB) acts last preflop in every fixture, always behind Hero's
    // voluntary entry — same as Villain, never a genuine RFI/limp spot here.
    assert_no_opportunity(s.rfi, "robot rfi");
    assert_no_opportunity(s.limp, "robot limp");
    // Phase 2, stat 3 (Cold Call%): Robot faces Hero's open cold in the same
    // three hands as Villain, but always declines by re-raising instead of
    // calling (3-betting in showdown/fold_to_3bet, folding in cbet_fold) —
    // 0/3 opportunities converted, a real 0.0%, not "no opportunity".
    assert_close(s.cold_call, 0.0, "robot cold_call");
    // Phase 2, stat 4: in hand_cbet_fold, Villain calls Hero's open before
    // Robot's turn — a genuine squeeze opportunity for Robot (raise_count>=1
    // AND a caller since the last raise), declined by folding rather than
    // re-raising, hence a real 0.0%, not "no opportunity". Robot is never
    // the one squeezed (their 3-bets in the other two hands are always
    // called or folded to directly, never re-raised over).
    assert_close(s.squeeze, 0.0, "robot squeeze");
    assert_no_opportunity(s.fold_to_squeeze, "robot fold_to_squeeze");
    assert_close(s.c_bet, 100.0, "robot c_bet");
    assert_no_opportunity(s.fold_to_c_bet, "robot fold_to_c_bet");
    // Robot bets/raises postflop every time but never calls, so the ratio has
    // no denominator ( requirement 2) rather than reporting the raw count.
    assert_no_opportunity(s.aggression_factor, "robot aggression_factor");
    assert_close(s.wtsd, 50.0, "robot wtsd");
    assert_close(s.wsd, 100.0, "robot wsd");
    // steal_attempt%: Robot (BB) is never first-in either, so no opportunity.
    assert_no_opportunity(s.steal_attempt, "robot steal_attempt");
    // fold_to_steal%: Robot (BB), the same "SB and BB each get an
    // independent opportunity against the same steal raise" convention
    // squeeze_opportunities already uses — faces the same 3 steal raises as
    // Villain above (regardless of whether Villain called or folded first),
    // and re-raises rather than folding in 2 of them (showdown,
    // fold_to_3bet), folding in the third (cbet_fold): 1/3 = 33.3%.
    assert_close(s.fold_to_steal, 33.3, "robot fold_to_steal");
}

///  requirement 3 follow-up correction: an all-in preflop action alone
/// does not mean the flop was dealt. `hand_allin_preflop_folded.txt` (Hero
/// shoves preflop, everyone folds — no flop, no showdown) must NOT count
/// toward `saw_flop_hands`; `hand_allin_preflop_showdown.txt` (Hero shoves
/// preflop, Villain calls all-in, the hand runs to showdown with zero
/// flop-street action rows for either player) must. Only `went_to_showdown`
/// tells them apart, since both produce identical (zero) flop action rows.
#[test]
fn all_in_preflop_only_counts_toward_saw_flop_when_the_hand_reaches_showdown() {
    let mut conn = setup_db();
    import::import_text(&mut conn, ALLIN_PREFLOP_FOLDED_HAND).expect("import should succeed");
    import::import_text(&mut conn, ALLIN_PREFLOP_SHOWDOWN_HAND).expect("import should succeed");

    // Hero: 1 hand with no flop dealt (folded to) + 1 hand that reaches
    // showdown with no flop action row of Hero's own. saw_flop_hands must be
    // 1, not 2 — so wtsd is 1/1 = 100%, not the pre-fix-regression 1/2 = 50%.
    let hero_id = player_id(&conn, "Hero");
    let hero_stats = stats::compute_player_stats(&conn, hero_id).unwrap();
    assert_close(hero_stats.wtsd, 100.0, "hero wtsd (all-in preflop, mixed outcomes)");

    // Villain: folds preflop in the first hand (never all-in), then calls
    // Hero's preflop shove and reaches showdown in the second — same
    // zero-flop-action-row shape, must also count.
    let villain_id = player_id(&conn, "Villain");
    let villain_stats = stats::compute_player_stats(&conn, villain_id).unwrap();
    assert_close(
        villain_stats.wtsd,
        100.0,
        "villain wtsd (all-in preflop, mixed outcomes)",
    );
}

/// Phase 2, stat 1 (4-bet%/fold-to-4-bet%): the clean, non-pileup spot.
/// Hero opens, Villain 3-bets, Robot folds facing what would be their own
/// 4-bet, Hero 4-bets back, Villain calls. Hand-verified against three real
/// hands in the live DB (hand_ids 22, 24, 27 — see the notes);
/// this fixture reproduces hand 27's shape (the opener returning to 4-bet)
/// deterministically for regression coverage.
#[test]
fn four_bet_and_fold_to_four_bet_in_a_clean_non_pileup_spot() {
    let mut conn = setup_db();
    import::import_text(&mut conn, FOUR_BET_HAND).expect("import should succeed");

    let hero_id = player_id(&conn, "Hero");
    let hero_stats = stats::compute_player_stats(&conn, hero_id).unwrap();
    // Hero is the opener: faces the 3-bet (raises again, so 0% fold-to-3bet)
    // and their own return raise is counted as a 4-bet (raise_count==2 at
    // their turn), not just a 3-bet response.
    assert_close(hero_stats.fold_to_three_bet, 0.0, "hero fold_to_three_bet");
    assert_close(hero_stats.four_bet, 100.0, "hero four_bet");

    let villain_id = player_id(&conn, "Villain");
    let villain_stats = stats::compute_player_stats(&conn, villain_id).unwrap();
    // Villain 3-bet, then called Hero's 4-bet: a clean fold-to-4-bet
    // opportunity, not folded.
    assert_close(villain_stats.three_bet, 100.0, "villain three_bet");
    assert_close(villain_stats.fold_to_four_bet, 0.0, "villain fold_to_four_bet");

    let robot_id = player_id(&conn, "Robot");
    let robot_stats = stats::compute_player_stats(&conn, robot_id).unwrap();
    // Robot never raised, but their fold happened at raise_count==2 (open +
    // 3-bet already in), so it's a real (unconverted) 4-bet opportunity —
    // four_bet% is general, not restricted to players who were 3-bettors.
    assert_close(robot_stats.four_bet, 0.0, "robot four_bet");
}

/// Phase 2, stat 1 follow-up: the documented pileup gap (stat-contracts.md,
/// "Residual pileup case"). Hero opens, Villain 3-bets, Robot 4-bets (a
/// THIRD player, not Hero) before action returns to Hero — so Hero's fold is
/// still filed under fold-to-3-bet (their `facing_3bet` flag was set by
/// Villain's 3-bet and never consumed before their own next action), even
/// though by then they are genuinely folding to a 4-bet. Hero never gets a
/// four_bet_opportunity at all (raise_count is already 3, past the `==2`
/// check, by the time it's their turn). Matches real hands 22 and 24 in the
/// live DB, hand-verified the same way.
#[test]
fn fold_to_three_bet_absorbs_the_openers_pileup_response_not_fold_to_four_bet() {
    let mut conn = setup_db();
    import::import_text(&mut conn, FOUR_BET_PILEUP_HAND).expect("import should succeed");

    let hero_id = player_id(&conn, "Hero");
    let hero_stats = stats::compute_player_stats(&conn, hero_id).unwrap();
    assert_close(hero_stats.fold_to_three_bet, 100.0, "hero fold_to_three_bet");
    assert_no_opportunity(hero_stats.four_bet, "hero four_bet");
    assert_no_opportunity(hero_stats.fold_to_four_bet, "hero fold_to_four_bet");

    let villain_id = player_id(&conn, "Villain");
    let villain_stats = stats::compute_player_stats(&conn, villain_id).unwrap();
    // Villain is the actual 3-bettor, so *they* get the clean fold-to-4-bet
    // read when Robot 4-bets them directly: they called, so 0%.
    assert_close(villain_stats.three_bet, 100.0, "villain three_bet");
    assert_close(villain_stats.fold_to_four_bet, 0.0, "villain fold_to_four_bet");

    let robot_id = player_id(&conn, "Robot");
    let robot_stats = stats::compute_player_stats(&conn, robot_id).unwrap();
    assert_close(robot_stats.four_bet, 100.0, "robot four_bet");
}

/// Phase 2, stat 2 (RFI%/Limp%): the case the maintainer explicitly asked to flag —
/// a player who limps behind another limper is not "first to voluntarily
/// act" and must be excluded entirely, not counted as a 0%-RFI limp. Also
/// exercises the isolation-raise-over-limps case (mirrors the 4-bet unit's
/// "raise that isn't really the first-in decision" pattern): the BB's raise
/// here happens only after Fish has already limped, so it must not count as
/// an RFI either, for the same `entered_pot` reason. Hand-verified against
/// real DB hands 1 and 3 (see the notes); this fixture
/// reproduces that shape deterministically.
#[test]
fn limping_behind_a_limper_and_raising_over_limps_are_both_excluded_from_rfi() {
    let mut conn = setup_db();
    import::import_text(&mut conn, LIMP_BEHIND_LIMP_HAND).expect("import should succeed");

    let fish_id = player_id(&conn, "Fish");
    let fish_stats = stats::compute_player_stats(&conn, fish_id).unwrap();
    // Fish (UTG) is genuinely first to act and limps in.
    assert_close(fish_stats.rfi, 0.0, "fish rfi");
    assert_close(fish_stats.limp, 100.0, "fish limp");

    let hero_id = player_id(&conn, "Hero");
    let hero_stats = stats::compute_player_stats(&conn, hero_id).unwrap();
    // Hero (button) calls right after Fish — a limp in the colloquial sense,
    // but NOT a first-in decision, so no opportunity at all.
    assert_no_opportunity(hero_stats.rfi, "hero rfi (limped behind Fish)");
    assert_no_opportunity(hero_stats.limp, "hero limp (limped behind Fish)");

    let villain_id = player_id(&conn, "Villain");
    let villain_stats = stats::compute_player_stats(&conn, villain_id).unwrap();
    assert_no_opportunity(villain_stats.rfi, "villain rfi");
    assert_no_opportunity(villain_stats.limp, "villain limp");

    let robot_id = player_id(&conn, "Robot");
    let robot_stats = stats::compute_player_stats(&conn, robot_id).unwrap();
    // Robot (BB) raises, but only after Fish already limped — an isolation
    // raise, not an RFI, so also no opportunity.
    assert_no_opportunity(robot_stats.rfi, "robot rfi (isolation raise over limps)");
    assert_no_opportunity(robot_stats.limp, "robot limp");
}

/// Phase 2, stat 2 follow-up: the walk case the maintainer asked about. A walk is
/// NOT "no opportunity for anyone" — every player who folds before the
/// walk completes genuinely had (and declined) an RFI/limp opportunity;
/// only the player who wins by walk (no action row at all, since they were
/// never required to act) gets none. Hand-verified against real DB hand 2
/// (see the notes); this fixture reproduces it.
#[test]
fn a_walk_gives_every_folder_a_declined_opportunity_but_the_walked_to_player_none() {
    let mut conn = setup_db();
    import::import_text(&mut conn, WALK_HAND).expect("import should succeed");

    let hero_id = player_id(&conn, "Hero");
    let hero_stats = stats::compute_player_stats(&conn, hero_id).unwrap();
    assert_close(hero_stats.rfi, 0.0, "hero rfi (folded first-in)");
    assert_close(hero_stats.limp, 0.0, "hero limp (folded first-in)");

    let villain_id = player_id(&conn, "Villain");
    let villain_stats = stats::compute_player_stats(&conn, villain_id).unwrap();
    // Villain (SB) also folds before anyone has voluntarily entered — still
    // a real, declined RFI/limp opportunity, same as Hero's.
    assert_close(villain_stats.rfi, 0.0, "villain rfi (folded first-in)");
    assert_close(villain_stats.limp, 0.0, "villain limp (folded first-in)");

    let robot_id = player_id(&conn, "Robot");
    let robot_stats = stats::compute_player_stats(&conn, robot_id).unwrap();
    // Robot (BB) wins the walk uncontested — no action row at all, so no
    // opportunity, not a fabricated 0%.
    assert_no_opportunity(robot_stats.rfi, "robot rfi (won the walk, never acted)");
    assert_no_opportunity(robot_stats.limp, "robot limp (won the walk, never acted)");
}

/// Phase 2, stat 3 (Cold Call%): the two "declined" opportunities excluded
/// from RFI/Limp (facing an open cold, facing a 3-bet cold) both feed this
/// stat instead. Hero cold-calls Fish's open (raise_count==1 at Hero's
/// turn, Hero's first voluntary action) and Robot cold-calls Villain's
/// 3-bet (raise_count==2, Robot's first voluntary action — their forced BB
/// post doesn't count) in the same hand; Villain instead declines their own
/// cold-call opportunity by 3-betting rather than calling. Hand-verified
/// against real DB hands (see the notes); this fixture
/// reproduces both shapes deterministically.
#[test]
fn cold_call_credits_the_first_voluntary_call_of_a_raise_whether_open_or_3bet() {
    let mut conn = setup_db();
    import::import_text(&mut conn, COLD_CALL_HAND).expect("import should succeed");

    let hero_id = player_id(&conn, "Hero");
    let hero_stats = stats::compute_player_stats(&conn, hero_id).unwrap();
    assert_close(hero_stats.cold_call, 100.0, "hero cold_call (called Fish's open cold)");

    let villain_id = player_id(&conn, "Villain");
    let villain_stats = stats::compute_player_stats(&conn, villain_id).unwrap();
    // Villain had the same cold-call opportunity Hero did one seat later
    // (raise_count == 1, never having voluntarily acted) but chose to 3-bet
    // instead of calling it — a real, declined opportunity, not a fold-away.
    assert_close(villain_stats.cold_call, 0.0, "villain cold_call (3-bet instead of calling)");

    let robot_id = player_id(&conn, "Robot");
    let robot_stats = stats::compute_player_stats(&conn, robot_id).unwrap();
    assert_close(robot_stats.cold_call, 100.0, "robot cold_call (called Villain's 3-bet cold)");

    let fish_id = player_id(&conn, "Fish");
    let fish_stats = stats::compute_player_stats(&conn, fish_id).unwrap();
    // Fish is the opener: by the time they act again they're facing the
    // 3-bet as the original raiser (fold_to_three_bet territory), never as
    // a first-voluntary-action cold call.
    assert_no_opportunity(fish_stats.cold_call, "fish cold_call (opener, not a cold call)");
}

/// Phase 2, stat 3 follow-up: the excluded case the maintainer explicitly asked to
/// flag — a player who limps first and is later raised over does NOT get a
/// cold-call opportunity when they call that raise, because their first
/// voluntary action was already the limp. Mirrors the RFI/Limp unit's
/// limp-behind-a-limp fixture shape, but with an isolation raise the limper
/// then calls instead of folds. Hand-verified against real DB hands (see
/// the notes).
#[test]
fn calling_a_raise_after_already_limping_is_excluded_from_cold_call() {
    let mut conn = setup_db();
    import::import_text(&mut conn, COLD_CALL_AFTER_LIMP_HAND).expect("import should succeed");

    let fish_id = player_id(&conn, "Fish");
    let fish_stats = stats::compute_player_stats(&conn, fish_id).unwrap();
    // Fish limped in, then called Robot's isolation raise — a call of live
    // preflop aggression, but not their FIRST voluntary action, so it is
    // correctly excluded from Cold Call% entirely (no opportunity, not a
    // fabricated 0%).
    assert_no_opportunity(fish_stats.cold_call, "fish cold_call (limped first, excluded)");
    assert_close(fish_stats.limp, 100.0, "fish limp (the earlier, genuine first-in limp)");

    let robot_id = player_id(&conn, "Robot");
    let robot_stats = stats::compute_player_stats(&conn, robot_id).unwrap();
    // Robot's own action here is a raise (the isolation raise itself, at
    // raise_count == 0), never a call of a prior raise, so no cold-call
    // opportunity either.
    assert_no_opportunity(robot_stats.cold_call, "robot cold_call (raised, not facing a raise)");
}

/// Phase 2, stat 4 (Squeeze%/Fold-to-Squeeze%): a squeeze is specifically a
/// re-raise that comes after at least one live caller — narrower than "any
/// raise while raise_count >= 1", which Cold Call% already tracks. Fish
/// opens, Hero cold-calls (the caller a squeeze requires), Villain re-raises
/// over both of them (the squeeze itself), Robot folds facing the same
/// raise_count >= 1 spot Cold Call% covers (but with no caller since
/// Villain's raise, so it's not a squeeze opportunity for Robot), and Fish —
/// the original opener — folds to the squeeze. Hand-verified against real DB
/// hands (see the notes).
#[test]
fn squeeze_credits_a_re_raise_after_a_live_caller_and_the_opener_folds_to_it() {
    let mut conn = setup_db();
    import::import_text(&mut conn, SQUEEZE_HAND).expect("import should succeed");

    let villain_id = player_id(&conn, "Villain");
    let villain_stats = stats::compute_player_stats(&conn, villain_id).unwrap();
    assert_close(villain_stats.squeeze, 100.0, "villain squeeze (re-raised after Hero's cold call)");
    assert_no_opportunity(villain_stats.fold_to_squeeze, "villain fold_to_squeeze (never re-raised over)");

    let fish_id = player_id(&conn, "Fish");
    let fish_stats = stats::compute_player_stats(&conn, fish_id).unwrap();
    assert_close(
        fish_stats.fold_to_squeeze,
        100.0,
        "fish fold_to_squeeze (opener folds to Villain's squeeze)",
    );
    assert_no_opportunity(fish_stats.squeeze, "fish squeeze (opener, never faces a raise before acting)");

    let hero_id = player_id(&conn, "Hero");
    let hero_stats = stats::compute_player_stats(&conn, hero_id).unwrap();
    // Hero is the cold caller a squeeze requires, not the squeezer — this
    // action feeds Cold Call%, not Squeeze%.
    assert_close(hero_stats.cold_call, 100.0, "hero cold_call (the caller Villain's squeeze needs)");
    assert_no_opportunity(hero_stats.squeeze, "hero squeeze (called, did not re-raise)");

    let robot_id = player_id(&conn, "Robot");
    let robot_stats = stats::compute_player_stats(&conn, robot_id).unwrap();
    // Robot faces the same raise_count >= 1 spot Cold Call% tracks (0% there,
    // declined by folding), but no caller has intervened since Villain's
    // raise by the time it's Robot's turn, so it's not a squeeze opportunity.
    assert_close(robot_stats.cold_call, 0.0, "robot cold_call (declined by folding)");
    assert_no_opportunity(robot_stats.squeeze, "robot squeeze (no caller since Villain's raise)");
}

/// Squeeze% follow-up: the excluded case the maintainer explicitly asked to flag — a
/// direct re-raise with no caller in between is not a squeeze, for either the
/// re-raiser or the original raiser, even though it satisfies the same
/// raise_count >= 1 gate Cold Call%/Squeeze% share. Reuses hand_3bet_showdown
/// (Hero opens, Villain folds directly, Robot 3-bets with no caller ever
/// having entered between Hero's open and Robot's 3-bet, Hero calls) — no new
/// fixture needed, since that hand already is exactly this shape.
#[test]
fn a_direct_re_raise_with_no_caller_in_between_is_not_a_squeeze() {
    let mut conn = setup_db();
    import::import_text(&mut conn, SHOWDOWN_HAND).expect("import should succeed");

    let robot_id = player_id(&conn, "Robot");
    let robot_stats = stats::compute_player_stats(&conn, robot_id).unwrap();
    // Robot's 3-bet satisfies Cold Call%'s raise_count >= 1 gate (a real,
    // declined-by-raising opportunity, hence 0.0% not None), but Villain
    // folded rather than calling Hero's open, so no live caller ever existed
    // between the two raises — not a squeeze opportunity at all.
    assert_close(robot_stats.cold_call, 0.0, "robot cold_call (direct 3-bet, still a raise-facing spot)");
    assert_no_opportunity(robot_stats.squeeze, "robot squeeze (no caller before the 3-bet)");

    let hero_id = player_id(&conn, "Hero");
    let hero_stats = stats::compute_player_stats(&conn, hero_id).unwrap();
    // Hero (the opener) faces Robot's direct 3-bet — a real fold_to_three_bet
    // opportunity elsewhere, but never a fold_to_squeeze one, since Robot's
    // raise was never preceded by a caller.
    assert_no_opportunity(hero_stats.fold_to_squeeze, "hero fold_to_squeeze (direct 3-bet, not a squeeze)");
}

/// steal_attempt%/fold_to_steal% at heads-up (position-contracts.md §2/§4):
/// `labels_for(2)` returns only `BTN`/`BB`, collapsing the small blind into
/// the button. `BTN` still matches the steal-position set by plain string
/// membership, no special-casing needed — Hero's button open counts as a
/// steal attempt exactly as it would at any other ring size, and Villain's
/// (BB) fold is a fold-to-steal.
#[test]
fn steal_attempt_and_fold_to_steal_at_heads_up() {
    let mut conn = setup_db();
    import::import_text(&mut conn, HEADS_UP_STEAL_HAND).expect("import should succeed");

    let hero_id = player_id(&conn, "Hero");
    let hero_stats = stats::compute_player_stats(&conn, hero_id).unwrap();
    assert_close(hero_stats.steal_attempt, 100.0, "hero steal_attempt (heads-up button open)");
    assert_no_opportunity(hero_stats.fold_to_steal, "hero fold_to_steal (button, never SB/BB)");

    let villain_id = player_id(&conn, "Villain");
    let villain_stats = stats::compute_player_stats(&conn, villain_id).unwrap();
    assert_no_opportunity(villain_stats.steal_attempt, "villain steal_attempt (BB, never first-in)");
    assert_close(villain_stats.fold_to_steal, 100.0, "villain fold_to_steal (BB folds to the button open)");
}

/// position-contracts.md §4's isolation-raise exclusion, applied to
/// fold_to_steal% specifically: a raise from a steal position (BTN here)
/// after a limp is an isolation raise, not a steal, because the raiser's own
/// `entered_pot` was already true when they acted — the same reason it's
/// excluded from `steal_attempt_opportunities`. The blinds folding to it must
/// NOT be credited with a fold_to_steal opportunity. Fish (CO) limps first,
/// Hero (BTN) isolates, Villain (SB) and Robot (BB) both fold — regression
/// coverage for a bug caught during Phase 0 wiring verification: an earlier
/// version of this gate checked only "raise_count == 1 and the raiser is in
/// a steal position," which wrongly counted this shape (the live-DB
/// verification run came back 56,027 fold_to_steal opportunities against the
/// design doc's 50,355 simulated figure until this fixture exposed why).
#[test]
fn a_raise_over_a_limp_from_a_steal_position_is_not_a_steal_for_fold_to_steal() {
    let mut conn = setup_db();
    import::import_text(&mut conn, ISOLATION_RAISE_NOT_A_STEAL_HAND).expect("import should succeed");

    let hero_id = player_id(&conn, "Hero");
    let hero_stats = stats::compute_player_stats(&conn, hero_id).unwrap();
    // Hero (BTN) raised after Fish's limp, not first-in, so no steal_attempt
    // opportunity either — same exclusion, the other side of it.
    assert_no_opportunity(hero_stats.steal_attempt, "hero steal_attempt (isolation raise, not first-in)");

    let villain_id = player_id(&conn, "Villain");
    let villain_stats = stats::compute_player_stats(&conn, villain_id).unwrap();
    assert_no_opportunity(
        villain_stats.fold_to_steal,
        "villain fold_to_steal (folded to an isolation raise, not a steal)",
    );

    let robot_id = player_id(&conn, "Robot");
    let robot_stats = stats::compute_player_stats(&conn, robot_id).unwrap();
    assert_no_opportunity(
        robot_stats.fold_to_steal,
        "robot fold_to_steal (folded to an isolation raise, not a steal)",
    );
}

#[test]
fn player_with_no_hands_reports_no_opportunity_for_every_stat() {
    let conn = setup_db();
    // No import performed; any player id is guaranteed to have zero hands, so
    // every stat's denominator is zero and must report "no opportunity"
    // (`None`) rather than a fabricated 0.0 ( requirement 1).
    let s = stats::compute_player_stats(&conn, 999).unwrap();
    assert_no_opportunity(s.vpip, "vpip");
    assert_no_opportunity(s.pfr, "pfr");
    assert_no_opportunity(s.three_bet, "three_bet");
    assert_no_opportunity(s.fold_to_three_bet, "fold_to_three_bet");
    assert_no_opportunity(s.four_bet, "four_bet");
    assert_no_opportunity(s.fold_to_four_bet, "fold_to_four_bet");
    assert_no_opportunity(s.rfi, "rfi");
    assert_no_opportunity(s.limp, "limp");
    assert_no_opportunity(s.cold_call, "cold_call");
    assert_no_opportunity(s.squeeze, "squeeze");
    assert_no_opportunity(s.fold_to_squeeze, "fold_to_squeeze");
    assert_no_opportunity(s.c_bet, "c_bet");
    assert_no_opportunity(s.fold_to_c_bet, "fold_to_c_bet");
    assert_no_opportunity(s.aggression_factor, "aggression_factor");
    assert_no_opportunity(s.wtsd, "wtsd");
    assert_no_opportunity(s.wsd, "wsd");
    assert_no_opportunity(s.steal_attempt, "steal_attempt");
    assert_no_opportunity(s.fold_to_steal, "fold_to_steal");
}
