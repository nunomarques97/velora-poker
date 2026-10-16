//! Opponent engine, task T6: between-hands context (section 5 of
//! `docs/specs/opponent-engine.md`), the all-time / recency / last-N views,
//! head-to-head attribution (section 10) and recent form / tilt (section 6).
//!
//! Real PokerStars fixtures cover the bounty, Zoom and Spin headers; the
//! longer samples come from `common::Table`, which writes PokerStars text
//! with consistent stacks from hand to hand and is imported like a file.

mod common;

use common::{import, player_id, Act, Table, HERO};
use velora_poker_lib::engine::{
    aggregate_player, latest_table_hand, load_player_hands, recency_weight, recent_form,
    seat_relation, stat_spec, villain_context, FormFlag, FormatKey, H2hKey, Side, StackBucket,
    Stage, StatKey, View, H2H_DISPLAY_MIN, H2H_RULE_MIN,
};
use velora_poker_lib::engine::context::{initial_bounty, parse_level};
use velora_poker_lib::engine::recency::{hand_weights, LAST_N};
use velora_poker_lib::import;

const BOUNTY: &str = include_str!("fixtures/real_bounty_tournament.txt");
const ZOOM_CASH: &str = include_str!("fixtures/zoom_cash.txt");
const SPIN: &str = include_str!("fixtures/spin_three_max.txt");

const SIX: [&str; 6] = [HERO, "Sam", "Bob", "Vil", "Hal", "Cole"];

fn prior(format: FormatKey) -> impl Fn(StatKey) -> f64 {
    move |key| stat_spec(key).builtin_prior(format)
}

// ---------------------------------------------------------------- context

#[test]
fn effective_stack_uses_min_of_hero_and_villain_after_last_hand() {
    let mut t = Table::cash("Ctx Cash", 300_000_100, &SIX);
    // Button = Hero, so Vil (seat 4) is UTG. Vil opens, Hero 3-bets, Vil
    // calls; Hero bets 36bb on the flop and wins at showdown: Vil loses 45bb.
    let text = t.play(
        0,
        &[
            &[("Vil", Act::Raise(3.0)), ("Hal", Act::Fold), ("Cole", Act::Fold), (HERO, Act::Raise(9.0)),
              ("Sam", Act::Fold), ("Bob", Act::Fold), ("Vil", Act::Call)],
            &[("Vil", Act::Check), (HERO, Act::Bet(36.0)), ("Vil", Act::Call)],
            &[("Vil", Act::Check), (HERO, Act::Check)],
            &[("Vil", Act::Check), (HERO, Act::Check)],
        ],
        Some(HERO),
    );
    assert_eq!(t.stack_bb("Vil"), 55.0);
    let conn = import(&text);
    let hand = latest_table_hand(&conn, "Ctx Cash").unwrap().expect("latest hand");
    let vil = player_id(&conn, "Vil");
    let hal = player_id(&conn, "Hal");

    let ctx = villain_context(&hand, vil).expect("Vil was dealt in");
    // min(stack after: Vil 55bb, Hero 100 + 45 + 0.5 + 1 = 146.5bb).
    assert_eq!(ctx.effective_stack_bb, Some(55.0));
    assert_eq!(ctx.stack_bucket, Some(StackBucket::Mid));
    assert_eq!(ctx.source_hand_id, "300000100");
    assert_eq!(ctx.format, FormatKey::Cash);
    assert_eq!(ctx.variant.as_deref(), Some("cash"));
    assert_eq!(ctx.stage, None, "cash has no tournament stage");
    assert_eq!(ctx.bounty, None);
    // Hal folded with 100bb; the hero covers him.
    assert_eq!(villain_context(&hand, hal).unwrap().effective_stack_bb, Some(100.0));

    // Without a seated hero the biggest other stack (Hero's, as a player) caps it.
    let mut no_hero = hand.clone();
    no_hero.seats.iter_mut().for_each(|s| s.is_hero = false);
    let ctx = villain_context(&no_hero, hal).unwrap();
    assert_eq!(ctx.effective_stack_bb, Some(100.0));
    assert_eq!(ctx.seat, None, "no seat relation without the hero");
}

#[test]
fn tournament_stacks_are_the_chips_after_the_last_hand() {
    // Mock MTT session report, item 4: tournament hands store no net result
    // (chips are not money), so the context used to fall back to the stack
    // at the *start* of the last hand, one hand stale. The chips after the
    // hand come from its own lines instead.
    let mut t = Table::mtt("Ctx After", 300_000_250, &SIX, 40.0);
    t.stacks.insert("Vil".into(), 10.0 * 200.0);
    // Button = Hero, so Vil (seat 4) is UTG: he shoves 10bb, the hero calls
    // and Vil doubles up.
    let text = t.play(
        0,
        &[&[("Vil", Act::AllIn), ("Hal", Act::Fold), ("Cole", Act::Fold), (HERO, Act::Call),
            ("Sam", Act::Fold), ("Bob", Act::Fold)]],
        Some("Vil"),
    );
    assert_eq!((t.stack_bb("Vil"), t.stack_bb(HERO)), (21.5, 30.0));
    let conn = import(&text);
    let hand = latest_table_hand(&conn, "Ctx After").unwrap().unwrap();
    let vil = player_id(&conn, "Vil");
    let hal = player_id(&conn, "Hal");
    assert_eq!(hand.seat_of(vil).unwrap().closing_stack, Some(21.5 * 200.0));

    // Vil: min(21.5, Hero 30) — not his 10bb starting stack.
    let ctx = villain_context(&hand, vil).unwrap();
    assert_eq!(ctx.effective_stack_bb, Some(21.5));
    assert_eq!(ctx.stack_bucket, Some(StackBucket::Reshove));
    assert_eq!(ctx.stack_bb, Some(21.5));
    // Hal folded with 40bb: the effective stack is the hero's 30bb after
    // the hand, his own stack stays 40bb (never clamped).
    let ctx = villain_context(&hand, hal).unwrap();
    assert_eq!(ctx.effective_stack_bb, Some(30.0));
    assert_eq!(ctx.stack_bb, Some(40.0));
}

#[test]
fn real_tournament_closing_stacks_conserve_the_chips() {
    // The real bounty hand: Player1 (11262) shoves over Hero's 3-bet and
    // busts him; 755 comes back uncalled, Player1 collects 21184.
    let mut conn = velora_poker_lib::db::open(std::path::Path::new(":memory:")).unwrap();
    import::import_text(&mut conn, BOUNTY).unwrap();
    let hand = latest_table_hand(&conn, "4002441239 7").unwrap().unwrap();
    let closing = |name: &str| hand.seat_of(player_id(&conn, name)).unwrap().closing_stack;
    assert_eq!(closing("Player1"), Some(11262.0 - 20.0 - 11242.0 + 755.0 + 21184.0));
    assert_eq!(closing(HERO), Some(0.0));
    assert_eq!(closing("Player2"), Some(24875.0 - 20.0 - 150.0));
    let before: f64 = hand.seats.iter().filter_map(|s| s.starting_stack).sum();
    let after: f64 = hand.seats.iter().filter_map(|s| s.closing_stack).sum();
    assert_eq!(before, after, "a tournament hand has no rake");
    // Player1's own stack after the hand, in bb (21939 / 150).
    let p1 = villain_context(&hand, player_id(&conn, "Player1")).unwrap();
    assert_eq!(p1.stack_bb, Some(146.3));
    // The hero busted (0 chips): he plays no next hand at this table, so the
    // effective stack is taken against the biggest other stack, as when he
    // is not seated, never 0bb for everyone (min(24705, 21939) / 150).
    assert_eq!(p1.effective_stack_bb, Some(146.3));
    let p2 = villain_context(&hand, player_id(&conn, "Player2")).unwrap();
    assert_eq!(p2.effective_stack_bb, Some(146.3));
    assert_eq!(p2.stack_bb, Some(164.7));
}

#[test]
fn stack_buckets_follow_section_five_boundaries() {
    assert_eq!(StackBucket::of(15.0), StackBucket::PushFold);
    assert_eq!(StackBucket::of(15.1), StackBucket::Reshove);
    assert_eq!(StackBucket::of(25.0), StackBucket::Reshove);
    assert_eq!(StackBucket::of(25.1), StackBucket::Mid);
    assert_eq!(StackBucket::of(60.0), StackBucket::Mid);
    assert_eq!(StackBucket::of(150.0), StackBucket::Standard);
    assert_eq!(StackBucket::of(150.1), StackBucket::Deep);
}

#[test]
fn short_tournament_villain_lands_in_the_push_fold_bucket() {
    let mut t = Table::mtt("Ctx Short", 300_000_200, &SIX, 40.0);
    t.stacks.insert("Vil".into(), 12.0 * 200.0);
    let text = t.walk(0);
    let conn = import(&text);
    let hand = latest_table_hand(&conn, "Ctx Short").unwrap().unwrap();
    let ctx = villain_context(&hand, player_id(&conn, "Vil")).unwrap();
    assert_eq!(ctx.effective_stack_bb, Some(12.0));
    assert_eq!(ctx.stack_bucket, Some(StackBucket::PushFold));
    assert_eq!(ctx.format, FormatKey::Mtt);
    assert_eq!(ctx.level, Some(3));
}

#[test]
fn stage_buckets_follow_average_stack_in_bb() {
    assert_eq!(Stage::of(40.0), Stage::Early);
    assert_eq!(Stage::of(39.9), Stage::Middle);
    assert_eq!(Stage::of(20.0), Stage::Middle);
    assert_eq!(Stage::of(19.9), Stage::Late);
    assert_eq!(parse_level("VI"), Some(6));
    assert_eq!(parse_level("XIV"), Some(14));
    assert_eq!(parse_level("III"), Some(3));
    assert_eq!(parse_level(""), None);

    // Average starting stack 18bb → late, whatever the level number says.
    let mut t = Table::mtt("Ctx Stage", 300_000_300, &SIX, 18.0);
    let text = t.walk(0);
    let conn = import(&text);
    let hand = latest_table_hand(&conn, "Ctx Stage").unwrap().unwrap();
    let ctx = villain_context(&hand, player_id(&conn, "Vil")).unwrap();
    assert_eq!(ctx.avg_stack_bb, Some(18.0));
    assert_eq!(ctx.stage, Some(Stage::Late));
    assert_eq!(ctx.level, Some(3));
}

#[test]
fn bounty_ratio_uses_buy_in_bounty_component() {
    assert_eq!(initial_bounty("€13.50+€13.50+€3.00"), Some(13.5));
    assert_eq!(initial_bounty("$10+$1"), None, "no bounty component");

    let mut conn = velora_poker_lib::db::open(std::path::Path::new(":memory:")).unwrap();
    import::import_text(&mut conn, BOUNTY).unwrap();
    let hand = latest_table_hand(&conn, "4002441239 7").unwrap().unwrap();
    assert_eq!(hand.hand_ref, "260993449710");

    let p2 = villain_context(&hand, player_id(&conn, "Player2")).unwrap();
    assert_eq!(p2.format, FormatKey::Mtt);
    assert_eq!(p2.variant.as_deref(), Some("zoom_tournament"));
    assert_eq!(p2.level, Some(6));
    let bounty = p2.bounty.expect("Player2 carries a bounty");
    assert_eq!(bounty.amount, 20.25);
    assert_eq!(bounty.currency.as_deref(), Some("EUR"));
    assert_eq!(bounty.ratio, Some(1.5));
    assert!(!bounty.is_big());
    // Stacks after the hand: Hero busted (0) < Player2 24705.
    assert_eq!(bounty.hero_covers, Some(false));

    let p1 = villain_context(&hand, player_id(&conn, "Player1")).unwrap();
    assert_eq!(p1.bounty.unwrap().ratio, Some(1.0));
    // Average (11262 + 10507 + 24875) / 3 / 150 = 103.7bb → early.
    assert_eq!(p1.avg_stack_bb, Some(103.7));
    assert_eq!(p1.stage, Some(Stage::Early));
}

#[test]
fn seat_relation_left_right_and_distance() {
    let mut t = Table::cash("Ctx Seats", 300_000_400, &SIX);
    let text = t.walk(0);
    let conn = import(&text);
    let hand = latest_table_hand(&conn, "Ctx Seats").unwrap().unwrap();
    let rel = |name: &str| seat_relation(&hand, player_id(&conn, name)).unwrap();

    // Six occupied seats, Hero in seat 1: (n − 1) / 2 = 2.5.
    let sam = rel("Sam");
    assert_eq!((sam.distance, sam.side, sam.acts_after_hero), (1, Side::Left, true));
    assert!(sam.direct_left && !sam.direct_right);
    assert_eq!((rel("Bob").distance, rel("Bob").side), (2, Side::Left));
    assert_eq!((rel("Vil").distance, rel("Vil").side), (3, Side::Right));
    assert!(!rel("Vil").acts_after_hero);
    let cole = rel("Cole");
    assert_eq!((cole.distance, cole.side), (5, Side::Right));
    assert!(cole.direct_right && !cole.direct_left);
    assert_eq!(seat_relation(&hand, player_id(&conn, HERO)), None);

    // Three-handed (real Spin & Go hand; a Zoom table has no fixed seats,
    // see `zoom_hands_carry_no_seat_relation`): Hero in seat 1.
    let mut conn = velora_poker_lib::db::open(std::path::Path::new(":memory:")).unwrap();
    import::import_text(&mut conn, SPIN).unwrap();
    let hand = latest_table_hand(&conn, "3900000001 1").unwrap().unwrap();
    let left = seat_relation(&hand, player_id(&conn, "SpinCycle_9")).unwrap();
    assert_eq!((left.distance, left.side, left.direct_left), (1, Side::Left, true));
    let right = seat_relation(&hand, player_id(&conn, "GrindHouse")).unwrap();
    assert_eq!((right.distance, right.side, right.direct_right), (2, Side::Right, true));
}

#[test]
fn zoom_hands_carry_no_seat_relation() {
    // Section 5 (audit 2026-10): Zoom reseats every player after every
    // hand, so where a villain sat in the last hand says nothing about the
    // next one. Zoom cash and Zoom tournaments carry no seat relation.
    let mut conn = velora_poker_lib::db::open(std::path::Path::new(":memory:")).unwrap();
    import::import_text(&mut conn, ZOOM_CASH).unwrap();
    import::import_text(&mut conn, BOUNTY).unwrap();
    for (table, variant) in [("Halley", "zoom_cash"), ("4002441239 7", "zoom_tournament")] {
        let hand = latest_table_hand(&conn, table).unwrap().unwrap();
        assert_eq!(hand.variant.as_deref(), Some(variant));
        assert!(hand.hero().is_some(), "{table}: the hero is seated");
        for seat in hand.seats.iter().filter(|s| !s.is_hero) {
            assert_eq!(seat_relation(&hand, seat.player_id), None, "{variant}: seat {:?}", seat.seat);
            let ctx = villain_context(&hand, seat.player_id).unwrap();
            assert_eq!(ctx.seat, None, "{variant}: context seat");
            assert!(ctx.effective_stack_bb.is_some(), "{variant}: the rest of the context stays");
        }
    }
    // A fixed-seat table keeps it.
    let mut t = Table::cash("Ctx Fixed", 300_000_450, &SIX);
    let conn = import(&t.walk(0));
    let hand = latest_table_hand(&conn, "Ctx Fixed").unwrap().unwrap();
    assert!(villain_context(&hand, player_id(&conn, "Vil")).unwrap().seat.is_some());
}

#[test]
fn spin_stage_buckets_use_spin_bounds() {
    // A Spin & Go starts at 25bb (500 chips at 10/20), so the MTT bounds
    // (early from 40bb) would put its first level in `middle` and never
    // reach `early`. Section 5 (audit 2026-10): Spin early ≥ 20bb, middle
    // 12–20bb, late < 12bb.
    let mut conn = velora_poker_lib::db::open(std::path::Path::new(":memory:")).unwrap();
    import::import_text(&mut conn, SPIN).unwrap();
    let hand = latest_table_hand(&conn, "3900000001 1").unwrap().unwrap();
    let ctx = villain_context(&hand, player_id(&conn, "GrindHouse")).unwrap();
    assert_eq!(ctx.format, FormatKey::Spin);
    assert_eq!(ctx.avg_stack_bb, Some(25.0));
    assert_eq!(ctx.stage, Some(Stage::Early), "the first Spin level is early");
}

#[test]
fn variant_and_format_come_from_the_stored_hand() {
    let mut conn = velora_poker_lib::db::open(std::path::Path::new(":memory:")).unwrap();
    import::import_text(&mut conn, ZOOM_CASH).unwrap();
    import::import_text(&mut conn, SPIN).unwrap();
    let zoom = latest_table_hand(&conn, "Halley").unwrap().unwrap();
    let villain = zoom.seats.iter().find(|s| !s.is_hero).unwrap().player_id;
    let ctx = villain_context(&zoom, villain).unwrap();
    assert_eq!((ctx.format, ctx.variant.as_deref()), (FormatKey::Zoom, Some("zoom_cash")));
    assert_eq!(ctx.stage, None);

    let spin = latest_table_hand(&conn, "3900000001 1").unwrap().unwrap();
    let villain = spin.seats.iter().find(|s| !s.is_hero).unwrap().player_id;
    let ctx = villain_context(&spin, villain).unwrap();
    assert_eq!(ctx.format, FormatKey::Spin);
    assert!(ctx.stage.is_some(), "a Spin & Go is a tournament");
}

#[test]
fn context_comes_from_the_latest_completed_hand_at_the_scoped_table() {
    let mut a = Table::cash("Ctx A", 300_000_500, &SIX);
    let mut b = Table::cash("Ctx B", 300_000_600, &SIX);
    b.stacks.insert("Vil".into(), 30.0 * 0.5);
    let mut text = a.walk(0);
    text += &a.open_and_take(1, "Vil");
    // Table B played later: it must not leak into table A's context.
    text += &b.walk(5);
    let conn = import(&text);
    let vil = player_id(&conn, "Vil");

    let at_a = latest_table_hand(&conn, "Ctx A").unwrap().unwrap();
    assert_eq!(at_a.hand_ref, "300000501");
    let ctx = villain_context(&at_a, vil).unwrap();
    // 100bb − 2.5bb + (2.5 + 0.5 + 0.25 … collected) = 101.5bb after the steal.
    assert_eq!(ctx.effective_stack_bb, Some(100.0), "capped by the hero's 100bb");
    let at_b = latest_table_hand(&conn, "Ctx B").unwrap().unwrap();
    assert_eq!(villain_context(&at_b, vil).unwrap().effective_stack_bb, Some(30.0));
    assert!(latest_table_hand(&conn, "Nowhere").unwrap().is_none());
}

#[test]
fn context_serialises_to_the_spec_json_shape() {
    let mut conn = velora_poker_lib::db::open(std::path::Path::new(":memory:")).unwrap();
    import::import_text(&mut conn, BOUNTY).unwrap();
    let hand = latest_table_hand(&conn, "4002441239 7").unwrap().unwrap();
    let ctx = villain_context(&hand, player_id(&conn, "Player2")).unwrap();
    let json = serde_json::to_value(&ctx).unwrap();
    for key in ["sourceHandId", "variant", "format", "effectiveStackBb", "stackBucket", "stage",
                "level", "avgStackBb", "bounty", "seat"] {
        assert!(json.get(key).is_some(), "missing {key} in {json}");
    }
    assert_eq!(json["format"], "mtt");
    assert_eq!(json["stage"], "early");
    assert_eq!(json["bounty"]["heroCovers"], false);
    // A Zoom tournament carries no seat relation (section 5).
    assert_eq!(json["seat"], serde_json::Value::Null);

    let mut t = Table::cash("Ctx Json", 300_000_700, &SIX);
    let conn = import(&t.walk(0));
    let hand = latest_table_hand(&conn, "Ctx Json").unwrap().unwrap();
    let json = serde_json::to_value(villain_context(&hand, player_id(&conn, "Sam")).unwrap()).unwrap();
    assert_eq!(json["seat"]["actsAfterHero"], true);
    assert_eq!(json["seat"]["side"], "left");
    assert_eq!(json["seat"]["directLeft"], true);
}

// ---------------------------------------------------------- recency views

#[test]
fn recency_weights_halve_every_half_life() {
    assert_eq!(recency_weight(0, 100.0), 1.0);
    assert!((recency_weight(100, 100.0) - 0.5).abs() < 1e-12);
    assert!((recency_weight(200, 100.0) - 0.25).abs() < 1e-12);
    assert!((recency_weight(60, FormatKey::Spin.half_life()) - 0.5).abs() < 1e-12);
    assert_eq!(FormatKey::Cash.half_life(), 150.0);
    assert_eq!(FormatKey::Zoom.half_life(), 200.0);
    let w = hand_weights(3, 100.0);
    assert_eq!(w[2], 1.0, "the latest hand weighs 1");
    assert!(w[0] < w[1] && w[1] < w[2]);

    // 150 folds, then 30 opens: recent hands pull the recency view up.
    let mut t = Table::cash("Rec", 300_001_000, &SIX);
    let mut text = String::new();
    for i in 0..150 {
        text += &t.walk(i);
    }
    for i in 150..180 {
        text += &t.open_and_take(i, "Vil");
    }
    let conn = import(&text);
    let vil = player_id(&conn, "Vil");
    let hands = load_player_hands(&conn, vil).unwrap();
    let agg = aggregate_player(&hands, vil, FormatKey::Cash);
    let p = stat_spec(StatKey::Vpip).builtin_prior(FormatKey::Cash);
    let all = agg.stat(View::AllTime, StatKey::Vpip, p);
    let rec = agg.stat(View::Recency, StatKey::Vpip, p);
    assert_eq!((all.hits, all.opportunities), (30, 180));
    assert!(rec.shrunk > all.shrunk + 0.05, "recency {} vs all-time {}", rec.shrunk, all.shrunk);
    // Confidence stays on the raw stat-specific count in every view.
    assert_eq!(rec.confidence, all.confidence);
    assert_eq!(rec.raw, all.raw);
    let opps_w = agg.counts(View::Recency, StatKey::Vpip).opportunities_w;
    let expected: f64 = hand_weights(180, 150.0).iter().sum();
    assert!((opps_w - expected).abs() < 1e-9);
}

#[test]
fn recency_half_life_counts_the_stats_own_opportunities() {
    // Matrix validation (section 18): counted in hands, a 150-hand half-life
    // kept about 216 hands of a 1,800-hand cash sample, so a spot he meets
    // once in 30 hands (a flop c-bet) kept a handful of weighted
    // observations: FitOrFoldFiona's 83% fold to 64 c-bets shrank to 56%
    // and showed no read. The age of an opportunity is now the number of
    // newer opportunities at the same stat.
    let mut t = Table::cash("Spot", 300_003_000, &SIX);
    let mut text = String::new();
    // One hand in four Vil is under the gun (an early-seat spot); in the
    // others he is in the small blind and folded to (a steal spot). Every
    // hand is a VPIP spot. Stacks reset so nobody goes broke.
    for i in 0..600 {
        for stack in t.stacks.values_mut() {
            *stack = 50.0;
        }
        t.button = if i % 4 == 0 { 1 } else { 3 };
        text += &if i % 8 == 1 { t.open_and_take(i, "Vil") } else { t.walk(i) };
    }
    let conn = import(&text);
    let vil = player_id(&conn, "Vil");
    let agg = aggregate_player(&load_player_hands(&conn, vil).unwrap(), vil, FormatKey::Cash);
    let ep = agg.counts(View::Recency, StatKey::RfiEp);
    assert!(ep.opportunities >= 20 && ep.opportunities * 3 < agg.hands, "{} early-seat spots in {} hands", ep.opportunities, agg.hands);
    let expected: f64 = (0..ep.opportunities as usize).map(|age| recency_weight(age, 150.0)).sum();
    assert!((ep.opportunities_w - expected).abs() < 1e-9, "weighted {} vs {expected}", ep.opportunities_w);
    let vpip = agg.counts(View::Recency, StatKey::Vpip);
    let expected: f64 = (0..vpip.opportunities as usize).map(|age| recency_weight(age, 150.0)).sum();
    assert!((vpip.opportunities_w - expected).abs() < 1e-9, "weighted {} vs {expected}", vpip.opportunities_w);
}

#[test]
fn last_n_window_uses_latest_hands() {
    let mut t = Table::cash("LastN", 300_002_000, &SIX);
    let mut text = String::new();
    for i in 0..20 {
        text += &t.open_and_take(i, "Vil");
    }
    for i in 20..20 + LAST_N as i64 - 3 {
        text += &t.walk(i);
    }
    for i in 0..3 {
        text += &t.open_and_take(40 + i, "Vil");
    }
    let conn = import(&text);
    let vil = player_id(&conn, "Vil");
    let agg = aggregate_player(&load_player_hands(&conn, vil).unwrap(), vil, FormatKey::Cash);
    let last = agg.counts(View::LastN, StatKey::Vpip);
    assert_eq!((last.hits, last.opportunities), (3, LAST_N as u32));
    let all = agg.counts(View::AllTime, StatKey::Vpip);
    assert_eq!((all.hits, all.opportunities), (23, 20 + LAST_N as u32));
}

// ------------------------------------------------------------ recent form

/// MTT 100/200, Vil UTG with 50bb: `baseline` hands where he opens every
/// fourth, optionally a hand where he loses down to 18bb, then `window`
/// hands one minute apart in which he opens `window_opens` times.
fn form_sample(baseline: i64, big_loss: bool, window: i64, window_opens: i64) -> (rusqlite::Connection, i64) {
    let mut t = Table::mtt("Form", 300_003_000, &SIX, 100.0);
    t.stacks.insert("Vil".into(), 50.0 * 200.0);
    let mut text = String::new();
    for i in 0..baseline {
        text += &if i % 4 == 0 { t.open_and_take(i, "Vil") } else { t.walk(i) };
    }
    if big_loss {
        // Called down to 18bb behind: a lost pot of 30+bb and over half his stack.
        let street = (t.stack_bb("Vil") - 18.0 - 9.0) / 2.0;
        text += &t.play(
            120,
            &[
                &[("Vil", Act::Raise(3.0)), ("Hal", Act::Fold), ("Cole", Act::Fold), (HERO, Act::Raise(9.0)),
                  ("Sam", Act::Fold), ("Bob", Act::Fold), ("Vil", Act::Call)],
                &[("Vil", Act::Check), (HERO, Act::Bet(street)), ("Vil", Act::Call)],
                &[("Vil", Act::Check), (HERO, Act::Bet(street)), ("Vil", Act::Call)],
                &[("Vil", Act::Check), (HERO, Act::Check)],
            ],
            Some(HERO),
        );
        assert!((t.stack_bb("Vil") - 18.0).abs() < 1e-9);
    }
    for j in 0..window {
        let minute = 121 + j;
        text += &if j < window_opens { t.open_and_take(minute, "Vil") } else { t.walk(minute) };
    }
    let conn = import(&text);
    let vil = player_id(&conn, "Vil");
    (conn, vil)
}

fn form(conn: &rusqlite::Connection, vil: i64) -> Option<velora_poker_lib::engine::RecentForm> {
    let hands = load_player_hands(conn, vil).unwrap();
    recent_form(&hands, vil, stat_spec(StatKey::Vpip).builtin_prior(FormatKey::Mtt))
}

#[test]
fn tilt_fires_on_vpip_spike_after_big_loss() {
    let (conn, vil) = form_sample(48, true, 12, 9);
    // The loss is only visible in consecutive stacks: tournaments store no net.
    let nets: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM player_hands WHERE player_id = ?1 AND net_result IS NOT NULL",
            [vil],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(nets, 0);

    let form = form(&conn, vil).expect("window and baseline are large enough");
    assert_eq!((form.window, form.window_hits), (12, 9));
    assert_eq!(form.baseline_opportunities, 49);
    assert!(form.after_big_loss);
    assert!(form.z >= 2.5, "z = {}", form.z);
    assert_eq!(form.flag, Some(FormFlag::Tilt));
    assert_eq!(FormFlag::Tilt.rule_id(), "rec.tilt");

    // The same spike without the lost pot is only `looser`.
    let (conn, vil) = form_sample(49, false, 12, 9);
    let form = self::form(&conn, vil).unwrap();
    assert!(!form.after_big_loss);
    assert_eq!(form.flag, Some(FormFlag::Looser));
}

#[test]
fn tilt_does_not_fire_on_ordinary_variance() {
    // 5 of 12 against a ~26% baseline: z ≈ 1.3.
    let (conn, vil) = form_sample(48, true, 12, 5);
    let form = form(&conn, vil).unwrap();
    assert!(form.after_big_loss);
    assert!(form.z < 2.5);
    assert_eq!(form.flag, None);

    // Spec example: p0 = 0.25, n = 12 → 6 hits gives z = 2.0, no flag.
    let z = (6.0 - 12.0 * 0.25) / (12.0_f64 * 0.25 * 0.75).sqrt();
    assert!((z - 2.0).abs() < 1e-9);
}

#[test]
fn tilt_detects_a_villain_tightening_up() {
    // Baseline 75% VPIP, window 0 of 12.
    let mut t = Table::mtt("Tight", 300_004_000, &SIX, 100.0);
    let mut text = String::new();
    for i in 0..48 {
        text += &if i % 4 != 0 { t.open_and_take(i, "Vil") } else { t.walk(i) };
    }
    for j in 0..12 {
        text += &t.walk(121 + j);
    }
    let conn = import(&text);
    let vil = player_id(&conn, "Vil");
    assert_eq!(form(&conn, vil).unwrap().flag, Some(FormFlag::Tighter));
}

#[test]
fn tilt_needs_minimum_baseline() {
    // 12 of 12 after a big loss, but only 30 baseline hands.
    let (conn, vil) = form_sample(30, true, 12, 12);
    assert_eq!(form(&conn, vil), None);
    // Nine hands after the loss: the loss hand itself sits within the 60
    // minutes, so the window holds 10 hands, exactly the minimum.
    let (conn, vil) = form_sample(48, true, 9, 9);
    assert_eq!(form(&conn, vil).unwrap().window, 10);
    let (conn, vil) = form_sample(48, false, 9, 9);
    assert_eq!(form(&conn, vil), None, "9 window hands are not enough");
}

// ---------------------------------------------------------- head-to-head

/// Hero seat 1, Sam 2, Vil 3, Ugo 4, Hal 5, Cole 6 with the hero on the
/// button: Sam posts the small blind, Vil the big blind.
fn alpha(first_id: u64) -> Table {
    let mut t = Table::cash("H2H Alpha", first_id, &[HERO, "Sam", "Vil", "Ugo", "Hal", "Cole"]);
    t.button = 1;
    t
}

const EARLY_FOLDS: [(&str, Act); 3] = [("Ugo", Act::Fold), ("Hal", Act::Fold), ("Cole", Act::Fold)];

fn h2h_agg(text: &str) -> velora_poker_lib::engine::PlayerAggregate {
    let conn = import(text);
    let vil = player_id(&conn, "Vil");
    aggregate_player(&load_player_hands(&conn, vil).unwrap(), vil, FormatKey::Cash)
}

#[test]
fn h2h_three_bet_counts_only_hero_opens() {
    let mut t = alpha(300_005_000);
    let mut text = String::new();
    let mut m = 0;
    let mut deal = |t: &mut Table, lines: Vec<(&str, Act)>| {
        m += 1;
        t.play(m, &[&lines], None)
    };
    let pre = |rest: &[(&'static str, Act)]| -> Vec<(&'static str, Act)> {
        EARLY_FOLDS.iter().copied().chain(rest.iter().copied()).collect()
    };
    // Hero opens, Vil 3-bets: hit (and a defended steal).
    text += &deal(&mut t, pre(&[(HERO, Act::Raise(2.5)), ("Sam", Act::Fold), ("Vil", Act::Raise(9.0)), (HERO, Act::Fold)]));
    // Hero opens, Vil folds: miss.
    text += &deal(&mut t, pre(&[(HERO, Act::Raise(2.5)), ("Sam", Act::Fold), ("Vil", Act::Fold)]));
    // Hero opens, Sam calls, Vil squeezes: not a 3-bet facing only the hero's raise.
    text += &deal(&mut t, pre(&[(HERO, Act::Raise(2.5)), ("Sam", Act::Call), ("Vil", Act::Raise(12.0)), (HERO, Act::Fold), ("Sam", Act::Fold)]));
    // Cole opens, Vil 3-bets: not the hero's open.
    text += &deal(&mut t, vec![("Ugo", Act::Fold), ("Hal", Act::Fold), ("Cole", Act::Raise(2.5)), (HERO, Act::Fold),
                               ("Sam", Act::Fold), ("Vil", Act::Raise(9.0)), ("Cole", Act::Fold)]);
    // Hero limps, Vil raises: an isolation raise, not a 3-bet.
    text += &deal(&mut t, pre(&[(HERO, Act::Call), ("Sam", Act::Fold), ("Vil", Act::Raise(4.0)), (HERO, Act::Fold)]));

    let agg = h2h_agg(&text);
    let three = agg.h2h[&H2hKey::ThreeBetVsHeroOpen];
    assert_eq!((three.hits, three.opportunities), (1, 2));
    let defend = agg.h2h[&H2hKey::DefendVsHeroSteal];
    assert_eq!((defend.hits, defend.opportunities), (1, 2));
    // Overall, Vil had more 3-bet opportunities than the head-to-head view.
    let overall = agg.counts(View::AllTime, StatKey::ThreeBetOop);
    assert!(overall.opportunities > three.opportunities);
    assert_eq!(agg.h2h_hands, 5);
}

#[test]
fn h2h_fold_to_hero_cbet_counts_only_hero_cbets() {
    let mut t = alpha(300_006_000);
    let mut text = String::new();
    let hero_open: Vec<(&str, Act)> = EARLY_FOLDS.iter().copied()
        .chain([(HERO, Act::Raise(2.5)), ("Sam", Act::Fold), ("Vil", Act::Call)]).collect();
    // Hero c-bets, Vil folds.
    text += &t.play(1, &[&hero_open, &[("Vil", Act::Check), (HERO, Act::Bet(2.0)), ("Vil", Act::Fold)]], None);
    // Hero c-bets, Vil calls; both check down.
    text += &t.play(
        2,
        &[&hero_open, &[("Vil", Act::Check), (HERO, Act::Bet(2.0)), ("Vil", Act::Call)],
          &[("Vil", Act::Check), (HERO, Act::Check)], &[("Vil", Act::Check), (HERO, Act::Check)]],
        Some(HERO),
    );
    // Hero checks back the flop: no c-bet faced.
    text += &t.play(
        3,
        &[&hero_open, &[("Vil", Act::Check), (HERO, Act::Check)],
          &[("Vil", Act::Bet(2.0)), (HERO, Act::Fold)]],
        None,
    );
    // Cole opens and c-bets into Vil: not the hero's.
    let cole_open: Vec<(&str, Act)> = vec![("Ugo", Act::Fold), ("Hal", Act::Fold), ("Cole", Act::Raise(2.5)),
                                           (HERO, Act::Fold), ("Sam", Act::Fold), ("Vil", Act::Call)];
    text += &t.play(4, &[&cole_open, &[("Vil", Act::Check), ("Cole", Act::Bet(2.0)), ("Vil", Act::Fold)]], None);

    let agg = h2h_agg(&text);
    let cbet = agg.h2h[&H2hKey::FoldToHeroCbet];
    assert_eq!((cbet.hits, cbet.opportunities), (1, 2));
    let overall = agg.counts(View::AllTime, StatKey::FoldToCbetFlop);
    assert_eq!((overall.hits, overall.opportunities), (2, 3));
}

#[test]
fn h2h_steal_battles_attribute_to_hero() {
    // Vil seat 1, Hero seat 2: with Vil on the button the hero is the small blind.
    let mut t = Table::cash("H2H Beta", 300_007_000, &["Vil", HERO, "Bob", "Ugo", "Hal", "Cole"]);
    let mut text = String::new();
    let folds = [("Ugo", Act::Fold), ("Hal", Act::Fold), ("Cole", Act::Fold)];
    t.button = 1;
    // Vil steals into the hero's small blind; hero 3-bets, Vil folds.
    let mut lines: Vec<(&str, Act)> = folds.to_vec();
    lines.extend([("Vil", Act::Raise(2.5)), (HERO, Act::Raise(9.0)), ("Bob", Act::Fold), ("Vil", Act::Fold)]);
    text += &t.play(1, &[&lines], None);
    // Vil folds his button: a missed steal against the hero.
    let mut lines: Vec<(&str, Act)> = folds.to_vec();
    lines.extend([("Vil", Act::Fold), (HERO, Act::Fold)]);
    text += &t.play(2, &[&lines], None);
    // Vil in the small blind steals against the hero's big blind.
    t.button = 6;
    text += &t.play(3, &[&[("Bob", Act::Fold), ("Ugo", Act::Fold), ("Hal", Act::Fold), ("Cole", Act::Fold),
                          ("Vil", Act::Raise(2.5)), (HERO, Act::Fold)]], None);
    // Hero on the button, Vil in the cutoff: a steal, but the hero is not in a blind.
    t.button = 2;
    text += &t.play(4, &[&[("Ugo", Act::Fold), ("Hal", Act::Fold), ("Cole", Act::Fold), ("Vil", Act::Raise(2.5)),
                          (HERO, Act::Fold), ("Bob", Act::Fold), ("Ugo", Act::Fold)]], None);
    // Hand order matters: Ugo posts the big blind in hand 4 and folds after the steal.

    let agg = h2h_agg(&text);
    let steal = agg.h2h[&H2hKey::StealVsHero];
    assert_eq!((steal.hits, steal.opportunities), (2, 3));
    let fold3 = agg.h2h[&H2hKey::FoldToHeroThreeBet];
    assert_eq!((fold3.hits, fold3.opportunities), (1, 1));
    let overall = agg.counts(View::AllTime, StatKey::Steal);
    assert_eq!((overall.hits, overall.opportunities), (3, 4));
}

#[test]
fn h2h_reads_need_minimum_sample() {
    let build = |n: i64| {
        let mut t = alpha(300_008_000);
        let mut text = String::new();
        for i in 0..n {
            let mut lines: Vec<(&str, Act)> = EARLY_FOLDS.to_vec();
            lines.extend([(HERO, Act::Raise(2.5)), ("Sam", Act::Fold), ("Vil", Act::Raise(9.0)), (HERO, Act::Fold)]);
            text += &t.play(i, &[&lines], None);
        }
        h2h_agg(&text)
    };
    let p = prior(FormatKey::Cash);
    let seven = build(i64::from(H2H_DISPLAY_MIN) - 1);
    assert_eq!(seven.h2h[&H2hKey::ThreeBetVsHeroOpen].opportunities, 7);
    assert!(seven.head_to_head(&p).is_none(), "below 8 opportunities nothing is displayed");

    let eight = build(i64::from(H2H_DISPLAY_MIN));
    let view = eight.head_to_head(&p).expect("8 opportunities are displayed");
    assert_eq!(view.hands, 8);
    let stat = view.stats.iter().find(|s| s.key == H2hKey::ThreeBetVsHeroOpen).unwrap();
    assert_eq!((stat.hits, stat.opportunities, stat.raw_pct), (8, 8, Some(100.0)));
    assert!(!eight.h2h_stat(H2hKey::ThreeBetVsHeroOpen, &p).rule_eligible());

    let ten = build(i64::from(H2H_RULE_MIN));
    assert!(ten.h2h_stat(H2hKey::ThreeBetVsHeroOpen, &p).rule_eligible());
    let json = serde_json::to_value(ten.head_to_head(&p).unwrap()).unwrap();
    assert_eq!(json["stats"][0]["key"], "three_bet_vs_hero_open");
    assert!(json["stats"][0].get("shrunkPct").is_some());
}

#[test]
fn h2h_value_shrinks_toward_villain_overall() {
    // Vil 3-bets Cole's opens every time (8/8) and the hero's once in 4.
    let mut t = alpha(300_009_000);
    let mut text = String::new();
    for i in 0..8 {
        text += &t.play(i, &[&[("Ugo", Act::Fold), ("Hal", Act::Fold), ("Cole", Act::Raise(2.5)), (HERO, Act::Fold),
                               ("Sam", Act::Fold), ("Vil", Act::Raise(9.0)), ("Cole", Act::Fold)]], None);
    }
    for i in 0..4 {
        let vil = if i == 0 { Act::Raise(9.0) } else { Act::Fold };
        let mut lines: Vec<(&str, Act)> = EARLY_FOLDS.to_vec();
        lines.extend([(HERO, Act::Raise(2.5)), ("Sam", Act::Fold), ("Vil", vil)]);
        if i == 0 {
            lines.push((HERO, Act::Fold));
        }
        text += &t.play(10 + i, &[&lines], None);
    }
    let agg = h2h_agg(&text);
    let p = prior(FormatKey::Cash);
    let stat = agg.h2h_stat(H2hKey::ThreeBetVsHeroOpen, &p);
    assert_eq!((stat.hits, stat.opportunities), (1, 4));
    // Villain overall on three_bet_ip + three_bet_oop: 9 of 12, k = 25.
    let overall = agg.counts(View::AllTime, StatKey::ThreeBetOop);
    assert_eq!((overall.hits, overall.opportunities), (9, 12));
    let mean_prior = (p(StatKey::ThreeBetIp) + p(StatKey::ThreeBetOop)) / 2.0;
    let villain = (9.0 + 25.0 * mean_prior) / (12.0 + 25.0);
    assert!((stat.villain_shrunk - villain).abs() < 1e-12);
    assert!(stat.villain_shrunk > mean_prior, "his own 3-bet rate, not the population's");
    let expected = (1.0 + 25.0 * villain) / (4.0 + 25.0);
    assert!((stat.shrunk - expected).abs() < 1e-12);
}
