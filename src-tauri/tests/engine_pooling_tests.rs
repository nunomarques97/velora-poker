//! Opponent engine, task T6: small-sample honesty (section 4 of
//! `docs/specs/opponent-engine.md`). Shrinkage toward the per-format prior,
//! confidence on the stat-specific opportunity count, the local pool that
//! refines the priors, and its cache keyed by the import generation.

mod common;

use common::{import, player_id, Act, Table, HERO};
use velora_poker_lib::description_rules::ConfidenceTier;
use velora_poker_lib::engine::pooling::{
    Generation, PoolPasses, POOL_MIN_OPPORTUNITIES, POOL_MIN_PLAYERS, POOL_PRIOR_WEIGHT,
};
use velora_poker_lib::engine::{
    aggregate_player, confidence, confidence_tier, load_hands_after, load_player_hands, shrink,
    stat_spec, FormatKey, PoolCache, PoolTally, ShrunkStat, StatKey, View,
};
use velora_poker_lib::import;

const SIX: [&str; 6] = [HERO, "Sam", "Bob", "Vil", "Hal", "Cole"];

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-12
}

// ------------------------------------------------------------- shrinkage

#[test]
fn zero_opportunities_shows_no_player_value() {
    let prior = stat_spec(StatKey::ThreeBetIp).builtin_prior(FormatKey::Cash);
    let stat = ShrunkStat::new(StatKey::ThreeBetIp, 0, 0, prior);
    assert_eq!(stat.raw, None, "the prior is never shown as the player's number");
    assert!(close(stat.shrunk, prior), "rules see the prior, the UI sees nothing");
    assert_eq!(stat.confidence, 0.0);
    assert_eq!(stat.confidence_pct, 0);
    assert_eq!(stat.tier, ConfidenceTier::InsufficientData);
    assert!(!stat.displayable());

    // Through the aggregate: Vil only ever folds before anyone opens, so he
    // has VPIP opportunities but no 3-bet spot at all.
    let mut t = Table::cash("Pool Zero", 310_000_000, &SIX);
    let mut text = String::new();
    for i in 0..20 {
        text += &t.walk(i);
    }
    let conn = import(&text);
    let vil = player_id(&conn, "Vil");
    let agg = aggregate_player(&load_player_hands(&conn, vil).unwrap(), vil, FormatKey::Cash);
    for view in [View::AllTime, View::Recency, View::LastN] {
        let three = agg.stat(view, StatKey::ThreeBetIp, prior);
        assert_eq!((three.opportunities, three.raw), (0, None), "{view:?}");
        assert!(close(three.shrunk, prior));
    }
    let vpip = agg.stat(View::AllTime, StatKey::Vpip, 0.27);
    assert_eq!((vpip.hits, vpip.opportunities, vpip.raw), (0, 20, Some(0.0)));
}

#[test]
fn small_sample_extreme_is_pulled_toward_the_prior() {
    // 3 of 3 3-bets with k = 25 and a 7% prior: about 17%, nowhere near 100%.
    let stat = ShrunkStat::new(StatKey::ThreeBetOop, 3, 3, 0.07);
    assert_eq!(stat.raw, Some(1.0));
    assert!(close(stat.shrunk, (3.0 + 25.0 * 0.07) / 28.0));
    assert!(stat.shrunk < 0.2);
    assert!(!stat.displayable(), "3 opportunities are below n_min = 12");
}

#[test]
fn large_sample_converges_to_raw_rate() {
    let prior = 0.27;
    let mut last_gap = f64::INFINITY;
    for n in [10_u32, 100, 1_000, 10_000, 100_000] {
        let hits = n * 3 / 10;
        let stat = ShrunkStat::new(StatKey::Vpip, hits, n, prior);
        assert_eq!(stat.raw, Some(0.3));
        let gap = (stat.shrunk - 0.3).abs();
        assert!(gap < last_gap, "n = {n}: the gap must shrink");
        last_gap = gap;
    }
    assert!(last_gap < 1e-4);
    // Weighted counts (the recency view) shrink the same way.
    assert!(close(shrink(30.0, 100.0, 20.0, prior), (30.0 + 20.0 * prior) / 120.0));
}

#[test]
fn confidence_uses_stat_specific_opportunities() {
    // VPIP k = 20, n_min = 15: 14 hidden, 15..29 medium (29/49 < 60%), 30 high.
    let vpip = stat_spec(StatKey::Vpip);
    assert_eq!(confidence_tier(14, &vpip), ConfidenceTier::InsufficientData);
    assert_eq!(confidence_tier(15, &vpip), ConfidenceTier::Medium);
    assert_eq!(confidence_tier(29, &vpip), ConfidenceTier::Medium);
    assert_eq!(confidence_tier(30, &vpip), ConfidenceTier::High);
    // Fold to 4-bet k = 10, n_min = 6: 6/16 = 37.5% medium, 15/25 = 60% high.
    let f4 = stat_spec(StatKey::FoldTo4bet);
    assert_eq!(confidence_tier(5, &f4), ConfidenceTier::InsufficientData);
    assert_eq!(confidence_tier(6, &f4), ConfidenceTier::Medium);
    assert_eq!(confidence_tier(14, &f4), ConfidenceTier::Medium);
    assert_eq!(confidence_tier(15, &f4), ConfidenceTier::High);
    // Squeeze k = 15, n_min = 8: 8/23 = 34.8% is low.
    assert_eq!(confidence_tier(8, &stat_spec(StatKey::Squeeze)), ConfidenceTier::Low);
    assert!(close(confidence(60, 20.0), 0.75));

    // 40 hands; in 10 of them Vil (UTG) opens and folds to the hero's 3-bet.
    // VPIP confidence comes from 40 dealt hands, fold to 3-bet from its 10
    // spots with its own k.
    let mut t = Table::cash("Pool Conf", 310_001_000, &SIX);
    let mut text = String::new();
    for i in 0..40 {
        text += &if i % 4 == 0 {
            t.play(
                i,
                &[&[("Vil", Act::Raise(2.5)), ("Hal", Act::Fold), ("Cole", Act::Fold), (HERO, Act::Raise(9.0)),
                    ("Sam", Act::Fold), ("Bob", Act::Fold), ("Vil", Act::Fold)]],
                None,
            )
        } else {
            t.walk(i)
        };
    }
    let conn = import(&text);
    let vil = player_id(&conn, "Vil");
    let agg = aggregate_player(&load_player_hands(&conn, vil).unwrap(), vil, FormatKey::Cash);
    let vpip = agg.stat(View::AllTime, StatKey::Vpip, 0.27);
    assert_eq!((vpip.hits, vpip.opportunities), (10, 40));
    assert!(close(vpip.confidence, confidence(40, 20.0)));
    assert_eq!(vpip.tier, ConfidenceTier::High);
    let fold3 = agg.stat(View::AllTime, StatKey::FoldTo3betOop, 0.62);
    assert_eq!((fold3.hits, fold3.opportunities), (10, 10));
    assert!(close(fold3.confidence, confidence(10, 15.0)));
    assert_eq!(fold3.confidence_pct, 40);
    assert_eq!(fold3.tier, ConfidenceTier::Medium, "not the VPIP tier of 40 hands");
}

// ------------------------------------------------------------------ pool

/// `players` distinct players with `per_player` cash VPIP opportunities
/// each, half of them hits.
fn tally(players: i64, per_player: u32) -> PoolTally {
    let mut pool = PoolTally::default();
    for p in 0..players {
        for i in 0..per_player {
            pool.add(FormatKey::Cash, StatKey::Vpip, 1_000 + p, i % 2 == 0);
        }
    }
    pool
}

#[test]
fn pool_prior_refines_builtin_when_sample_suffices() {
    let builtin = stat_spec(StatKey::Vpip).builtin_prior(FormatKey::Cash);
    assert_eq!((POOL_MIN_OPPORTUNITIES, POOL_MIN_PLAYERS, POOL_PRIOR_WEIGHT), (2000, 50, 500.0));

    // 50 players x 40 = 2,000 opportunities, 1,000 hits.
    let pool = tally(50, 40);
    assert!(pool.is_refined(FormatKey::Cash, StatKey::Vpip));
    let expected = (1000.0 + 500.0 * builtin) / (2000.0 + 500.0);
    assert!(close(pool.prior(FormatKey::Cash, StatKey::Vpip), expected));
    assert!(pool.prior(FormatKey::Cash, StatKey::Vpip) > builtin);
    // Other formats and stats keep their built-in prior.
    assert!(close(pool.prior(FormatKey::Zoom, StatKey::Vpip), stat_spec(StatKey::Vpip).builtin_prior(FormatKey::Zoom)));
    assert!(close(pool.prior(FormatKey::Cash, StatKey::Pfr), stat_spec(StatKey::Pfr).builtin_prior(FormatKey::Cash)));

    // 1,950 opportunities, or 49 players: the built-in prior stands.
    let few_opps = tally(50, 39);
    assert!(!few_opps.is_refined(FormatKey::Cash, StatKey::Vpip));
    assert!(close(few_opps.prior(FormatKey::Cash, StatKey::Vpip), builtin));
    let few_players = tally(49, 60);
    assert!(!few_players.is_refined(FormatKey::Cash, StatKey::Vpip));
    assert!(close(few_players.prior(FormatKey::Cash, StatKey::Vpip), builtin));

    // Spin has no EP open; the cash prior stands in.
    assert_eq!(stat_spec(StatKey::RfiEp).priors[3], None);
    assert!(close(stat_spec(StatKey::RfiEp).builtin_prior(FormatKey::Spin), 0.16));
}

#[test]
fn pool_excludes_the_hero_and_splits_by_format() {
    let mut t = Table::cash("Pool Hero", 310_002_000, &SIX);
    let mut text = String::new();
    for i in 0..10 {
        text += &t.open_and_take(i, HERO);
    }
    let conn = import(&text);
    let hero = player_id(&conn, HERO);
    let mut pool = PoolTally::default();
    for hand in load_hands_after(&conn, 0, 100).unwrap() {
        pool.add_hand(&hand);
    }
    assert_eq!(pool.hands(), 10);
    let vpip = pool.entry(FormatKey::Cash, StatKey::Vpip).unwrap();
    assert_eq!(vpip.opportunities, 50, "five non-hero players per hand");
    assert_eq!(vpip.players.len(), 5);
    assert!(!vpip.players.contains(&hero));
    assert_eq!(vpip.hits, 0, "every villain folded to the hero's opens");
    assert!(pool.entry(FormatKey::Mtt, StatKey::Vpip).is_none());
}

fn passes(full: u32, incremental: u32) -> PoolPasses {
    PoolPasses { full, incremental }
}

#[test]
fn pool_cache_invalidated_on_import() {
    let mut t = Table::cash("Pool Cache", 310_003_000, &SIX);
    let first: String = (0..6).map(|i| t.open_and_take(i, "Vil")).collect();
    let second: String = (6..10).map(|i| t.walk(i)).collect();

    let mut conn = velora_poker_lib::db::open(std::path::Path::new(":memory:")).unwrap();
    let mut cache = PoolCache::new();
    assert_eq!(cache.tally(&conn).unwrap().hands(), 0);
    assert_eq!(cache.passes(), passes(1, 0));

    import::import_text(&mut conn, &first).unwrap();
    assert_eq!(cache.tally(&conn).unwrap().hands(), 6);
    assert_eq!(cache.passes(), passes(1, 1), "an import walks only the new hands");

    // Overlay refreshes with no new import walk nothing.
    for _ in 0..12 {
        cache.prior(&conn, FormatKey::Cash, StatKey::Vpip).unwrap();
    }
    assert_eq!(cache.passes(), passes(1, 1));

    // Re-importing the same file adds no hand and moves no generation.
    let before = Generation::read(&conn).unwrap();
    import::import_text(&mut conn, &first).unwrap();
    assert_eq!(Generation::read(&conn).unwrap(), before);
    cache.tally(&conn).unwrap();
    assert_eq!(cache.passes(), passes(1, 1));

    import::import_text(&mut conn, &second).unwrap();
    let after = Generation::read(&conn).unwrap();
    assert_eq!((after.imports - before.imports, after.rebuilds), (4, before.rebuilds));
    let incremental = cache.tally(&conn).unwrap().clone();
    assert_eq!(incremental.hands(), 10);
    assert_eq!(cache.passes(), passes(1, 2));

    // The incremental tally equals a full walk of the same database.
    let mut fresh = PoolCache::new();
    assert_eq!(fresh.tally(&conn).unwrap(), &incremental);
    let vil = player_id(&conn, "Vil");
    let vpip = incremental.entry(FormatKey::Cash, StatKey::Vpip).unwrap();
    assert!(vpip.players.contains(&vil));
    assert_eq!(vpip.hits, 6);

    // A changed stored hand (repair, backfill) forces a full walk.
    conn.execute("UPDATE hands SET variant = 'zoom_cash' WHERE hand_id = '310003000'", []).unwrap();
    let rebuilt = cache.tally(&conn).unwrap();
    assert_eq!(rebuilt.hands(), 10);
    assert!(rebuilt.entry(FormatKey::Zoom, StatKey::Vpip).is_some());
    assert_eq!(cache.passes(), passes(2, 2));

    // An explicit invalidation also walks everything once.
    cache.invalidate();
    cache.tally(&conn).unwrap();
    cache.tally(&conn).unwrap();
    assert_eq!(cache.passes(), passes(3, 2));
}

#[test]
fn engine_generation_migration_is_idempotent() {
    let dir = std::env::temp_dir().join(format!("velora-pool-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("pool.db");
    let _ = std::fs::remove_file(&path);

    let mut t = Table::cash("Pool Migrate", 310_004_000, &SIX);
    let text: String = (0..3).map(|i| t.walk(i)).collect();
    let generation = {
        let mut conn = velora_poker_lib::db::open(&path).unwrap();
        import::import_text(&mut conn, &text).unwrap();
        Generation::read(&conn).unwrap()
    };
    assert_eq!(generation.imports, 3);

    // Reopening runs the migrations again: counters and triggers survive once.
    let conn = velora_poker_lib::db::open(&path).unwrap();
    assert_eq!(Generation::read(&conn).unwrap(), generation);
    let triggers: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'trigger' AND name LIKE 'engine_gen_%'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(triggers, 9);
    let rows: i64 = conn.query_row("SELECT COUNT(*) FROM engine_state", [], |r| r.get(0)).unwrap();
    assert_eq!(rows, 2);
    drop(conn);
    let _ = std::fs::remove_dir_all(&dir);
}
