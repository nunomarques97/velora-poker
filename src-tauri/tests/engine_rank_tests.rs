//! Opponent engine, task T8: read ranking and the chip tag (section 7 of
//! `docs/specs/opponent-engine.md`), the `PlayerPayload.engine` contract
//! (section 13), its feature gating (section 2) and the per-player replay
//! cache.
//!
//! Ranking and tag tests use hand-built reads so every score is known; the
//! payload, gating and cache tests run on generated PokerStars hands
//! imported like a file.

mod common;

use common::{import, player_id, Act, Table, HERO};
use serde_json::Value;
use velora_poker_lib::classification::{self, PlayerClassification};
use velora_poker_lib::commands::{active_table_payloads, build_player_payload};
use velora_poker_lib::db;
use velora_poker_lib::description_rules::{ConfidenceTier, RuleCategory};
use velora_poker_lib::engine::rank::{context_multiplier, is_fact, score, spot_weight, H2H_WEIGHT, MINOR_SPOT_WEIGHT};
use velora_poker_lib::engine::{
    chip_tag, engine_payload, evaluate_rules, is_valid_tag, latest_table_hand, player_replay, rank,
    rank_reads, rule_def, stat_spec, villain_context, EngineCache, EngineContext, EngineRuleResult, Family,
    BountyContext, FormatKey, RuleInput, ShrunkStat, StackBucket, StatKey, TagSource, RULES,
};

// ------------------------------------------------------------------ helpers

/// A read with a known deviation, confidence and context multiplier (spot
/// weight 1); its score is the section-7 product.
fn read(id: &str, tag: Option<&str>, deviation: f64, confidence: f64, multiplier: f64) -> EngineRuleResult {
    EngineRuleResult {
        rule_id: id.to_string(),
        category: RuleCategory::Exploit,
        observation: format!("{id} observation"),
        advice: format!("{id} advice"),
        confidence_pct: Some((100.0 * confidence).round() as u8),
        confidence_tier: ConfidenceTier::High,
        evidence: Vec::new(),
        sample: None,
        scenario_id: "P06".into(),
        family: Family::Preflop,
        tag: tag.map(str::to_string),
        score: score(deviation, confidence, 1.0, multiplier),
        deviation,
        confidence,
        weight: 1.0,
        multiplier,
        adapted_by: Vec::new(),
    }
}

fn ids(reads: &[EngineRuleResult]) -> Vec<&str> {
    reads.iter().map(|r| r.rule_id.as_str()).collect()
}

fn context(format: FormatKey, eff: Option<f64>) -> EngineContext {
    EngineContext {
        source_hand_id: "300000000001".into(),
        variant: Some(match format {
            FormatKey::Mtt => "tournament".into(),
            FormatKey::Spin => "spin".into(),
            FormatKey::Zoom => "zoom_cash".into(),
            FormatKey::Cash => "cash".into(),
        }),
        format,
        effective_stack_bb: eff,
        stack_bb: eff,
        stack_bucket: eff.map(StackBucket::of),
        stage: None,
        level: None,
        avg_stack_bb: None,
        bounty: None,
        seat: None,
    }
}

const SIX: [&str; 6] = [HERO, "Sam", "Bob", "Vil", "Hal", "Cole"];

/// Vil opens UTG and folds to the hero's 3-bet `n` times (button = Hero).
fn fold_to_three_bet_hands(t: &mut Table, n: i64, from_minute: i64) -> String {
    let mut text = String::new();
    for i in 0..n {
        for p in SIX {
            t.stacks.insert(p.to_string(), 100.0 * t.bb);
        }
        text += &t.play(
            from_minute + i,
            &[&[
                ("Vil", Act::Raise(3.0)), ("Hal", Act::Fold), ("Cole", Act::Fold), (HERO, Act::Raise(9.0)),
                ("Sam", Act::Fold), ("Bob", Act::Fold), ("Vil", Act::Fold),
            ]],
            None,
        );
    }
    text
}

fn builtin(format: FormatKey) -> impl Fn(StatKey) -> f64 {
    move |key| stat_spec(key).builtin_prior(format)
}

fn json<T: serde::Serialize>(value: &T) -> Value {
    serde_json::to_value(value).expect("serialize")
}

fn keys(value: &Value) -> Vec<&str> {
    let mut keys: Vec<&str> = value.as_object().expect("object").keys().map(String::as_str).collect();
    keys.sort_unstable();
    keys
}

// ------------------------------------------------------------- ranking (G01)

#[test]
fn ranking_orders_by_money_relevance_score() {
    // Biggest deviation (3.0 × 0.30 = 0.90), surest read (1.0 × 0.95 =
    // 0.95) and the best product (1.5 × 0.80 = 1.20): the product wins.
    let reads = vec![
        read("a.big_deviation", Some("F3B"), 3.0, 0.30, 1.0),
        read("b.best_product", Some("STN"), 1.5, 0.80, 1.0),
        read("c.most_confident", Some("NIT"), 1.0, 0.95, 1.0),
    ];
    assert_eq!(ids(&rank(reads.clone())), ["b.best_product", "c.most_confident", "a.big_deviation"]);

    // The context multiplier counts: ×1.5 lifts the deviation read (1.35)
    // above the 1.20 product.
    let mut boosted = reads;
    boosted[0] = read("a.big_deviation", Some("RSV"), 3.0, 0.30, 1.5);
    assert_eq!(ids(&rank(boosted))[0], "a.big_deviation");

    // On real rule output, every stat read's score is deviation ×
    // confidence × spot weight × context multiplier, the weight and the
    // multiplier are section 7.2's, and the ranked list never rises; with
    // and without a table context.
    let format = FormatKey::Cash;
    let prior = builtin(format);
    let mut input = RuleInput::default();
    for (key, hits, n) in [
        (StatKey::FoldTo3betOop, 30, 30),
        (StatKey::Steal, 30, 40),
        (StatKey::Vpip, 2, 60),
        (StatKey::Squeeze, 12, 40),
        (StatKey::Wwsf, 40, 60),
    ] {
        input.stats.insert(key, ShrunkStat::new(key, hits, n, prior(key)));
    }
    for ctx in [None, Some(context(format, Some(20.0))), Some(context(format, Some(200.0)))] {
        input.context = ctx;
        let ranked = rank(evaluate_rules(&input));
        assert!(ranked.len() >= 3, "{:?}", ids(&ranked));
        for r in &ranked {
            let def = rule_def(&r.rule_id);
            assert_eq!(r.weight, spot_weight(def), "{}", r.rule_id);
            assert_eq!(r.multiplier, context_multiplier(def, input.context.as_ref()), "{}", r.rule_id);
            assert!((r.score - r.deviation * r.confidence * r.weight * r.multiplier).abs() < 1e-12, "{}", r.rule_id);
        }
        // Facts of the latest hand rank below every tendency (mock MTT
        // session report, item 5); within each group the score never rises.
        let first_fact = ranked.iter().position(|r| is_fact(&r.rule_id)).unwrap_or(ranked.len());
        assert!(ranked[first_fact..].iter().all(|r| is_fact(&r.rule_id)), "{:?}", ids(&ranked));
        for group in [&ranked[..first_fact], &ranked[first_fact..]] {
            assert!(group.windows(2).all(|w| w[0].score >= w[1].score), "{:?}", ids(&ranked));
        }
    }
}

#[test]
fn stack_and_bounty_facts_rank_below_every_tendency() {
    // `ctx.short_stack` (2.5), `ko.big_bounty` (up to 3) and
    // `ctx.deep_stack` (0.5) are facts at 100% confidence, not reads of the
    // player: a 0.3 tendency still goes first, and the facts keep their own
    // score order after the tendencies (mock MTT session report, item 5).
    assert!(is_fact("ctx.short_stack") && is_fact("ctx.deep_stack") && is_fact("ko.big_bounty"));
    for id in ["stk.call_shove.tight", "ko.calls_when_covering", "ko.hunts_bounties", "rec.tilt", "sd.tell.big_is_value"] {
        assert!(!is_fact(id), "{id}");
    }
    let short = read("ctx.short_stack", Some("12bb"), 2.5, 1.0, 1.0);
    let bounty = read("ko.big_bounty", Some("KO"), 3.0, 1.0, 1.0);
    let deep = read("ctx.deep_stack", Some("DEEP"), 0.5, 1.0, 1.0);
    let tcs = read("stk.call_shove.tight", Some("TCS"), 1.0, 0.6, 1.5);
    let weak = read("pf.limp.high", Some("LMP"), 0.5, 0.6, 1.0);
    let orders = [
        vec![short.clone(), bounty.clone(), deep.clone(), tcs.clone(), weak.clone()],
        vec![weak.clone(), deep.clone(), tcs.clone(), bounty.clone(), short.clone()],
    ];
    for order in orders {
        assert_eq!(
            ids(&rank(order)),
            ["stk.call_shove.tight", "pf.limp.high", "ko.big_bounty", "ctx.short_stack", "ctx.deep_stack"]
        );
    }
    // The hover card's top two are tendencies; a fact fills a slot only
    // when there is no second tendency.
    let ranked = rank_reads(vec![short.clone(), bounty.clone(), tcs.clone(), weak], None);
    assert_eq!(ids(&ranked.top_reads), ["stk.call_shove.tight", "pf.limp.high"]);
    let ranked = rank_reads(vec![short, bounty, tcs], None);
    assert_eq!(ids(&ranked.top_reads), ["stk.call_shove.tight", "ko.big_bounty"]);

    // On real rule output: a short-stacked knockout villain with a big
    // bounty and a tight call-vs-shove read.
    let format = FormatKey::Mtt;
    let prior = builtin(format);
    let mut input = RuleInput::default();
    input.stats.insert(StatKey::CallVsShove, ShrunkStat::new(StatKey::CallVsShove, 2, 60, prior(StatKey::CallVsShove)));
    let mut ctx = context(format, Some(9.0));
    ctx.bounty = Some(BountyContext { amount: 20.0, currency: Some("USD".into()), ratio: Some(4.0), hero_covers: Some(true) });
    input.context = Some(ctx);
    let ranked = rank(evaluate_rules(&input));
    let got = ids(&ranked);
    assert!(got.contains(&"stk.call_shove.tight") && got.contains(&"ko.big_bounty") && got.contains(&"ctx.short_stack"), "{got:?}");
    assert_eq!(got[0], "stk.call_shove.tight", "{got:?}");
    let first_fact = got.iter().position(|id| is_fact(id)).unwrap();
    assert!(got[first_fact..].iter().all(|id| is_fact(id)), "{got:?}");
}

#[test]
fn spot_weights_follow_section_seven_two() {
    // Head-to-head reads are the hero's own spots (×1.25); rare lines and
    // outcome stats rank ×0.8; every other read ×1.
    for def in RULES {
        let expected = match def.id {
            id if id.starts_with("h2h.") => H2H_WEIGHT,
            "pf.squeeze.high" | "pf.limp_reraise.seen" | "post.delayed_cbet.high" | "post.donk.high"
            | "post.float.high" | "post.probe.high" | "post.river_raise.high" | "post.fold_river_raise.high"
            | "post.wwsf_low" | "post.wwsf_high" | "post.wsd_strong" => MINOR_SPOT_WEIGHT,
            _ => 1.0,
        };
        assert_eq!(spot_weight(def), expected, "{}", def.id);
    }
    assert_eq!((H2H_WEIGHT, MINOR_SPOT_WEIGHT), (1.25, 0.8));

    // A weight moves a read past one with a bigger deviation × confidence:
    // a 4-bet read (1.0 × 0.9) under a head-to-head read (0.8 × 0.9 × 1.25).
    let mut h2h = read("h2h.3bet_vs_hero.high", Some("H2H"), 0.8, 0.9, 1.0);
    h2h.weight = spot_weight(rule_def("h2h.3bet_vs_hero.high"));
    h2h.score = score(0.8, 0.9, h2h.weight, 1.0);
    let four_bet = read("pf.4bet.high", Some("4B+"), 0.85, 0.9, 1.0);
    assert_eq!(ids(&rank(vec![four_bet, h2h])), ["h2h.3bet_vs_hero.high", "pf.4bet.high"]);
}

#[test]
fn ranking_tie_break_is_deterministic() {
    // Equal scores: the higher confidence first; equal confidence too: rule
    // id ascending. Every input order gives the same ranking.
    let a = read("pf.steal.high", Some("ST+"), 2.0, 0.50, 1.0); // 1.0, 50%
    let b = read("pf.limp.high", Some("LMP"), 1.25, 0.80, 1.0); // 1.0, 80%
    let c = read("pf.cold_call.high", Some("CC+"), 2.0, 0.50, 1.0); // 1.0, 50%
    assert_eq!(a.score, c.score);
    assert!((a.score - b.score).abs() < 1e-12);
    let expected = ["pf.limp.high", "pf.cold_call.high", "pf.steal.high"];
    let orders = [
        vec![a.clone(), b.clone(), c.clone()],
        vec![c.clone(), b.clone(), a.clone()],
        vec![b.clone(), a.clone(), c.clone()],
        vec![c.clone(), a.clone(), b.clone()],
    ];
    for order in orders {
        assert_eq!(ids(&rank(order)), expected);
    }

    // The top reads are the first two of that order.
    let ranked = rank_reads(vec![a, b, c], None);
    assert_eq!(ids(&ranked.top_reads), &expected[..2]);
    assert_eq!(ids(&ranked.reads), expected);
}

// ---------------------------------------------------------------- tag (G02)

#[test]
fn tag_precedence_tilt_then_stack_then_top_read() {
    let top = read("pf.fold_to_3bet.high", Some("F3B"), 3.0, 0.9, 1.0);
    let tilt = read("rec.tilt", Some("TILT"), 0.5, 0.5, 1.0);
    let short = read("ctx.short_stack", Some("12bb"), 2.5, 1.0, 1.0);

    // (1) Tilt beats everything, even ranked below another read and with a
    // short stack (the stack fact ranks after every tendency).
    let reads = rank(vec![top.clone(), tilt.clone(), short.clone()]);
    assert_eq!(ids(&reads), ["pf.fold_to_3bet.high", "rec.tilt", "ctx.short_stack"]);
    let tag = chip_tag(&reads, Some(&context(FormatKey::Mtt, Some(12.4)))).unwrap();
    assert_eq!((tag.text.as_str(), tag.rule_id.as_str(), tag.source), ("TILT", "rec.tilt", TagSource::Tilt));

    // (2) A tournament villain on 25bb or less shows his stack, rounded
    // down, over the top read.
    let reads = rank(vec![top.clone(), short]);
    let tag = chip_tag(&reads, Some(&context(FormatKey::Mtt, Some(12.9)))).unwrap();
    assert_eq!((tag.text.as_str(), tag.rule_id.as_str(), tag.source), ("12bb", "ctx.short_stack", TagSource::Stack));
    let reads = rank(vec![top.clone()]);
    for (eff, text) in [(20.0, "20bb"), (25.0, "25bb"), (9.5, "9bb")] {
        let tag = chip_tag(&reads, Some(&context(FormatKey::Spin, Some(eff)))).unwrap();
        assert_eq!((tag.text.as_str(), tag.rule_id.as_str()), (text, "ctx.effective_stack"), "{eff}");
    }

    // (3) Above 25bb, or in a cash game at any depth, the top read's tag.
    for ctx in [context(FormatKey::Mtt, Some(25.1)), context(FormatKey::Cash, Some(12.0)), context(FormatKey::Zoom, None)] {
        let tag = chip_tag(&reads, Some(&ctx)).unwrap();
        assert_eq!((tag.text.as_str(), tag.rule_id.as_str(), tag.source), ("F3B", "pf.fold_to_3bet.high", TagSource::Read));
    }
    assert_eq!(chip_tag(&reads, None).unwrap().text, "F3B");
    // The tag follows the ranking, not the input order.
    let second = read("pf.limp.high", Some("LMP"), 1.0, 0.5, 1.0);
    assert_eq!(chip_tag(&rank(vec![second, top]), None).unwrap().text, "F3B");

    // (4) Nothing to say: no tag segment.
    assert_eq!(chip_tag(&[], None), None);
    assert_eq!(chip_tag(&[], Some(&context(FormatKey::Cash, Some(100.0)))), None);
}

#[test]
fn short_stack_chip_shows_his_all_in_read_over_the_stack() {
    // Mock MTT session report, item 1: at 25bb or less the tag slot names
    // his shove/call tendency (the stack is already on the PokerStars
    // table); the stack tag remains only when he has no all-in read.
    let short = read("ctx.short_stack", Some("12bb"), 2.5, 1.0, 1.0);
    let steal = read("pf.steal.high", Some("ST+"), 3.0, 0.9, 1.0);
    let tilt = read("rec.tilt", Some("TILT"), 0.5, 0.5, 1.0);
    let all_in = [
        ("stk.call_shove.tight", "TCS"),
        ("stk.call_shove.wide", "CLS"),
        ("stk.open_shove.wide", "SHV"),
        ("stk.open_shove.tight", "NSV"),
        ("stk.reshove.high", "RSV"),
        ("stk.reshove.low", "RS-"),
        ("stage.late.calls_shoves", "ICM-"),
        ("stage.late.shoves_wide", "SHV"),
        ("ko.calls_when_covering", "KO+"),
        ("ko.hunts_bounties", "HNT"),
    ];
    for (id, text) in all_in {
        // Ranked below a non-all-in read and the stack fact, it still names the chip.
        let shove = read(id, Some(text), 0.4, 0.5, 1.0);
        let reads = rank(vec![short.clone(), steal.clone(), shove.clone()]);
        assert_ne!(reads[0].rule_id, id);
        for (format, eff) in [(FormatKey::Mtt, 12.9), (FormatKey::Mtt, 25.0), (FormatKey::Spin, 8.0)] {
            let tag = chip_tag(&reads, Some(&context(format, Some(eff)))).unwrap();
            assert_eq!((tag.text.as_str(), tag.rule_id.as_str(), tag.source), (text, id, TagSource::Read), "{format:?} {eff}");
        }
        // Tilt still comes first.
        let with_tilt = rank(vec![short.clone(), shove.clone(), tilt.clone()]);
        assert_eq!(chip_tag(&with_tilt, Some(&context(FormatKey::Mtt, Some(12.0)))).unwrap().source, TagSource::Tilt);
        // Above 25bb and in cash games the top read keeps the tag.
        for ctx in [context(FormatKey::Mtt, Some(25.1)), context(FormatKey::Cash, Some(12.0))] {
            assert_eq!(chip_tag(&reads, Some(&ctx)).unwrap().text, "ST+");
        }
    }
    // The best-ranked all-in read wins when he has several.
    let reads = rank(vec![
        read("stk.reshove.high", Some("RSV"), 0.5, 0.5, 1.0),
        read("stage.late.calls_shoves", Some("ICM-"), 1.0, 0.8, 1.25),
        short.clone(),
    ]);
    assert_eq!(chip_tag(&reads, Some(&context(FormatKey::Mtt, Some(18.0)))).unwrap().text, "ICM-");
    // No all-in read: the stack tag, as before.
    let reads = rank(vec![short, steal]);
    let tag = chip_tag(&reads, Some(&context(FormatKey::Mtt, Some(12.9)))).unwrap();
    assert_eq!((tag.text.as_str(), tag.source), ("12bb", TagSource::Stack));
}

#[test]
fn stack_tag_shows_his_own_stack_not_the_hero_s() {
    // Mock MTT session report, item 4: the stack tag was the effective
    // stack, min(villain, hero), so when the hero was the short one every
    // villain without an all-in read showed the hero's own stack (`11bb` on
    // 7 of 8 chips at table 112). The tag names his own stack after the
    // last hand; the effective stack still gates it and drives the buckets.
    let short = read("ctx.short_stack", Some("9bb"), 2.5, 1.0, 1.0);
    let steal = read("pf.steal.high", Some("ST+"), 3.0, 0.9, 1.0);
    let big_bounty = read("ko.big_bounty", Some("KO"), 2.0, 1.0, 1.0);
    let hero_short = |own: f64| EngineContext { stack_bb: Some(own), ..context(FormatKey::Mtt, Some(9.0)) };

    // He is short too: his own stack, rounded down.
    let tag = chip_tag(&rank(vec![short.clone()]), Some(&hero_short(18.4))).unwrap();
    assert_eq!((tag.text.as_str(), tag.rule_id.as_str(), tag.source), ("18bb", "ctx.short_stack", TagSource::Stack));
    let tag = chip_tag(&rank(vec![steal.clone()]), Some(&hero_short(25.0))).unwrap();
    assert_eq!((tag.text.as_str(), tag.rule_id.as_str()), ("25bb", "ctx.effective_stack"));

    // He is deep: no stack tag (it would be the hero's own stack); his top
    // read names the chip, never the effective-stack fact.
    let tag = chip_tag(&rank(vec![short.clone(), steal.clone()]), Some(&hero_short(60.0))).unwrap();
    assert_eq!((tag.text.as_str(), tag.source), ("ST+", TagSource::Read));
    let tag = chip_tag(&rank(vec![short.clone(), big_bounty]), Some(&hero_short(60.0))).unwrap();
    assert_eq!(tag.text, "KO");
    assert_eq!(chip_tag(&rank(vec![short.clone()]), Some(&hero_short(25.1))), None);
    // His all-in read still wins at that effective depth.
    let shove = read("stk.call_shove.wide", Some("CLS"), 0.4, 0.5, 1.0);
    let tag = chip_tag(&rank(vec![short, steal, shove]), Some(&hero_short(60.0))).unwrap();
    assert_eq!((tag.text.as_str(), tag.source), ("CLS", TagSource::Read));
}

#[test]
fn tag_is_two_to_four_chars() {
    // Every rule's tag is valid (`NNbb` is a template filled with the stack).
    for def in RULES {
        if let Some(tag) = def.tag.filter(|t| *t != "NNbb") {
            assert!(is_valid_tag(tag), "{} tag {tag}", def.id);
        }
    }
    for valid in ["F3B", "ST-", "3B+", "TILT", "LP", "KO", "0bb", "9bb", "12bb", "25bb"] {
        assert!(is_valid_tag(valid), "{valid}");
    }
    for invalid in ["", "F", "TOOLONG", "f3b", "F 3", "123bb", "bb", "12Bb", "1.5bb", "ÉTÉ"] {
        assert!(!is_valid_tag(invalid), "{invalid}");
    }
    // Every stack tag the precedence can render is valid.
    let reads = rank(vec![read("pf.limp.high", Some("LMP"), 1.0, 0.5, 1.0)]);
    for tenths in 0..=250 {
        let eff = f64::from(tenths) / 10.0;
        let tag = chip_tag(&reads, Some(&context(FormatKey::Mtt, Some(eff)))).unwrap();
        assert!(is_valid_tag(&tag.text), "{eff}: {}", tag.text);
        assert!(tag.text.len() >= 2 && tag.text.len() <= 4, "{}", tag.text);
    }
}

// --------------------------------------------------------- payload contract

#[test]
fn engine_payload_matches_spec_json_contract() {
    let mut t = Table::cash("Rank Cash", 300_008_000, &SIX);
    let conn = import(&fold_to_three_bet_hands(&mut t, 30, 0));
    let vil = player_id(&conn, "Vil");
    let hand = latest_table_hand(&conn, "Rank Cash").unwrap().unwrap();
    let ctx = villain_context(&hand, vil).unwrap();
    let replay = player_replay(&conn, vil, Some(ctx.format)).unwrap();
    let payload = engine_payload(&replay, &builtin(ctx.format), Some(ctx));

    // Ranked, top two, tag of the top read.
    assert!(payload.reads.len() >= 2, "{:?}", ids(&payload.reads));
    assert!(payload.reads.windows(2).all(|w| w[0].score >= w[1].score));
    assert_eq!(payload.top_reads, payload.reads[..2].to_vec());
    let tag = payload.tag.as_ref().expect("a tag");
    assert_eq!(Some(&tag.text), payload.reads[0].tag.as_ref());
    assert_eq!(tag.rule_id, payload.reads[0].rule_id);

    let v = json(&payload);
    assert_eq!(
        keys(&v),
        ["autoNotes", "context", "headToHead", "reads", "recentForm", "showdowns", "sizingTells", "tag", "topReads", "version"]
    );
    assert_eq!(v["version"], 1);
    assert_eq!(keys(&v["tag"]), ["ruleId", "source", "text"]);
    assert_eq!(v["tag"]["source"], "read");
    assert_eq!(
        keys(&v["topReads"][0]),
        ["advice", "category", "confidencePct", "confidenceTier", "evidence", "family", "observation", "ruleId", "sample", "scenarioId", "score", "tag"]
    );
    // The chip's sample (section 13): the count and unit the read's confidence rests on.
    assert_eq!(keys(&v["topReads"][0]["sample"]), ["count", "unit"]);
    assert_eq!(keys(&v["reads"][0]["evidence"][0]), ["hits", "opportunities", "shrunk", "statName", "value"]);
    assert_eq!(
        keys(&v["context"]),
        ["avgStackBb", "bounty", "effectiveStackBb", "format", "level", "seat", "sourceHandId", "stackBb", "stackBucket", "stage", "variant"]
    );
    assert_eq!(v["context"]["sourceHandId"], hand.hand_ref.as_str());
    assert_eq!(v["context"]["format"], "cash");
    assert_eq!(keys(&v["context"]["seat"]), ["actsAfterHero", "directLeft", "directRight", "distance", "side"]);
    // Head-to-head: 30 of his opens met the hero's 3-bet.
    let h2h = &v["headToHead"];
    assert_eq!(keys(h2h), ["hands", "stats"]);
    let fold = h2h["stats"].as_array().unwrap().iter().find(|s| s["key"] == "fold_to_hero_3bet").expect("h2h stat");
    assert_eq!(keys(fold), ["hits", "key", "opportunities", "rawPct", "shrunkPct"]);
    assert_eq!((fold["hits"].as_u64(), fold["opportunities"].as_u64()), (Some(30), Some(30)));
    // Nothing shown, no recent-form sample, no auto-note: empty, never invented.
    assert_eq!(v["showdowns"], Value::Array(vec![]));
    assert_eq!(v["sizingTells"], Value::Array(vec![]));
    assert_eq!(v["recentForm"], Value::Null);
    assert_eq!(v["autoNotes"], Value::Array(vec![]));
    // Engine evidence never carries the prior as the player's value.
    for r in &payload.reads {
        for e in &r.evidence {
            assert!(e.opportunities > 0 && e.value.is_some(), "{} {}", r.rule_id, e.stat_name);
        }
    }
}

#[test]
fn engine_payload_carries_showdowns_newest_first() {
    let mut conn = db::open(std::path::Path::new(":memory:")).unwrap();
    velora_poker_lib::import::import_text(&mut conn, include_str!("fixtures/engine_showdown_cash.txt")).unwrap();
    let vic = player_id(&conn, "Vic");
    let replay = player_replay(&conn, vic, None).unwrap();
    let v = json(&engine_payload(&replay, &builtin(replay.format), None));

    assert_eq!(v["context"], Value::Null);
    let shown = v["showdowns"].as_array().unwrap();
    let refs: Vec<&str> = shown.iter().map(|s| s["handId"].as_str().unwrap()).collect();
    assert_eq!(refs, ["300000000503", "300000000502", "300000000501"]);
    let first = shown.last().unwrap();
    assert_eq!(keys(first), ["board", "cards", "category", "handId", "lastAggression", "line", "playedAt", "result"]);
    assert_eq!((first["cards"].as_str(), first["board"].as_str()), (Some("Ks Qd"), Some("Kc 7d 2s 4h 9c")));
    assert_eq!((first["category"].as_str(), first["result"].as_str()), (Some("pair"), Some("won")));
    let river = first["line"].as_array().unwrap().last().unwrap().clone();
    assert_eq!(keys(&river), ["action", "isAllIn", "potFraction", "sizeBucket", "street"]);
    assert_eq!((river["street"].as_str(), river["action"].as_str(), river["sizeBucket"].as_str()), (Some("river"), Some("bet"), Some("small")));
    assert_eq!(river["potFraction"].as_f64(), Some(0.29));
    assert_eq!(keys(&first["lastAggression"]), ["class", "potFraction", "sizeBucket", "street"]);
    assert_eq!(first["lastAggression"]["street"], "river");
}

// ----------------------------------------------------- gating and payloads

#[cfg(not(feature = "strategic-analysis"))]
#[test]
fn default_build_engine_payload_is_null() {
    let mut t = Table::cash("Rank Default", 300_008_100, &SIX);
    let conn = import(&fold_to_three_bet_hands(&mut t, 30, 0));
    let rules = classification::list_rules(&conn).unwrap();
    let mut cache = EngineCache::new();
    let vil = player_id(&conn, "Vil");

    let page = build_player_payload(&conn, &rules, &mut cache, None, vil, "Vil".into(), 30, None, None, true).unwrap();
    let overlay = active_table_payloads(&conn, &mut cache, Some("Rank Default"), None).unwrap();
    assert_eq!(overlay.len(), 6);
    for payload in overlay.iter().chain([&page]) {
        let v = json(payload);
        assert_eq!(v["engine"], Value::Null, "{}", payload.name);
        assert_eq!(v["descriptions"], Value::Array(vec![]), "{}", payload.name);
    }
    // Nothing was computed: no replay, no cache entry.
    assert_eq!(cache.counts().replays + cache.counts().hits, 0);
}

#[cfg(feature = "strategic-analysis")]
#[test]
fn strategic_build_payload_carries_engine_ranked_reads() {
    let mut t = Table::cash("Rank Strategic", 300_008_200, &SIX);
    let conn = import(&fold_to_three_bet_hands(&mut t, 30, 0));
    let rules = classification::list_rules(&conn).unwrap();
    let mut cache = EngineCache::new();
    let hand = latest_table_hand(&conn, "Rank Strategic").unwrap().unwrap();

    let overlay = active_table_payloads(&conn, &mut cache, Some("Rank Strategic"), None).unwrap();
    let vil = overlay.iter().find(|p| p.name == "Vil").expect("Vil seated");
    let engine = vil.engine.as_ref().expect("engine payload in the strategic build");
    // `descriptions` is the engine's ranked reads in the RuleResult contract.
    let description_ids: Vec<&str> = vil.descriptions.iter().map(|d| d.rule_id.as_str()).collect();
    assert_eq!(description_ids, ids(&engine.reads));
    assert!(description_ids.contains(&"pf.fold_to_3bet.high"));
    assert_eq!(json(&vil.descriptions[0]), json(&engine.reads[0].to_rule_result()));
    // The overlay's context is the roster's own latest hand at this table.
    let ctx = engine.context.as_ref().expect("table context");
    assert_eq!(ctx.source_hand_id, hand.hand_ref);
    assert!(ctx.seat.is_some());
    assert_eq!(engine.tag.as_ref().map(|t| t.rule_id.as_str()), Some(engine.reads[0].rule_id.as_str()));

    // Outside a table (Players list) there is no context, but still reads.
    let page = build_player_payload(&conn, &rules, &mut cache, None, vil.id.parse().unwrap(), "Vil".into(), 30, None, None, true).unwrap();
    let engine = page.engine.as_ref().unwrap();
    assert!(engine.context.is_none());
    assert!(ids(&engine.reads).contains(&"pf.fold_to_3bet.high"));
    assert!(
        engine.reads.iter().all(|r| !r.advice.contains(" your left)") && !r.advice.contains(" your right)")),
        "no seat advice without a table"
    );
}

/// Mock-session report item 3: the hero's own chip carried reads about the
/// hero (`3B+`, `RS-`, `KO`) and its hover card gave advice against himself.
/// The overlay roster keeps the hero's plain stats and drops his reads; the
/// Players list still reviews him.
#[cfg(feature = "strategic-analysis")]
#[test]
fn overlay_roster_carries_no_reads_about_the_hero() {
    let mut t = Table::cash("Rank Hero", 300_008_250, &SIX);
    let conn = import(&fold_to_three_bet_hands(&mut t, 30, 0));
    let rules = classification::list_rules(&conn).unwrap();
    let mut cache = EngineCache::new();
    let hero_id = player_id(&conn, HERO);

    // He does have reads: he 3-bets every hand.
    let page = build_player_payload(&conn, &rules, &mut cache, None, hero_id, HERO.into(), 30, None, None, true).unwrap();
    assert!(page.engine.as_ref().is_some_and(|e| e.tag.is_some() && !e.reads.is_empty()));
    assert!(!page.descriptions.is_empty());

    let overlay = active_table_payloads(&conn, &mut cache, Some("Rank Hero"), None).unwrap();
    assert_eq!(overlay.len(), 6);
    let hero = overlay.iter().find(|p| p.name == HERO).expect("hero seated");
    assert_eq!(hero.seat_offset, Some(0));
    let v = json(hero);
    assert_eq!(v["engine"], Value::Null, "no chip tag, no hover card");
    assert_eq!(v["descriptions"], Value::Array(vec![]), "no reads in the drawer");
    // Plain stats stay on his chip.
    assert_eq!(hero.hands, 30);
    assert!(hero.stats.vpip.is_some_and(|vpip| vpip > 90.0), "{:?}", hero.stats.vpip);
    // Villains keep theirs.
    let vil = overlay.iter().find(|p| p.name == "Vil").expect("Vil seated");
    assert!(vil.engine.as_ref().is_some_and(|e| e.tag.is_some()));
}

#[test]
fn engine_respects_min_hands_for_archetype_colour() {
    // 24 hands of a maniac's profile: below the 25-hand gate, so whatever
    // the build, the chip colour comes from no archetype.
    let mut t = Table::cash("Rank Gate", 300_008_300, &SIX);
    let mut text = String::new();
    for i in 0..24 {
        for p in SIX {
            t.stacks.insert(p.to_string(), 100.0 * t.bb);
        }
        text += &t.play(
            i,
            &[&[
                ("Vil", Act::Raise(4.0)), ("Hal", Act::Fold), ("Cole", Act::Fold), (HERO, Act::Fold),
                ("Sam", Act::Fold), ("Bob", Act::Fold),
            ]],
            None,
        );
    }
    let conn = import(&text);
    let rules = classification::list_rules(&conn).unwrap();
    let mut cache = EngineCache::new();
    let vil = player_id(&conn, "Vil");
    let payload = build_player_payload(&conn, &rules, &mut cache, None, vil, "Vil".into(), 24, None, None, true).unwrap();
    assert_eq!(payload.classification.classification, PlayerClassification::Unknown);
    assert!(!payload.classification.is_override);

    // The engine adds a tag segment, never an archetype or a colour.
    let replay = player_replay(&conn, vil, None).unwrap();
    let engine = json(&engine_payload(&replay, &builtin(replay.format), None));
    let text = engine.to_string();
    for forbidden in ["\"classification\"", "\"color\"", "\"archetype\"", "\"label\""] {
        assert!(!text.contains(forbidden), "engine payload carries {forbidden}");
    }
}

#[test]
fn overlay_hot_path_loads_no_manual_note() {
    let mut t = Table::cash("Rank Notes", 300_008_400, &SIX);
    let conn = import(&fold_to_three_bet_hands(&mut t, 12, 0));
    let rules = classification::list_rules(&conn).unwrap();
    let mut cache = EngineCache::new();
    let vil = player_id(&conn, "Vil");
    db::set_player_note(&conn, vil, "opens light from UTG").unwrap();

    let overlay = active_table_payloads(&conn, &mut cache, Some("Rank Notes"), None).unwrap();
    assert_eq!(overlay.len(), 6);
    assert!(overlay.iter().all(|p| p.note.is_none()));
    // The note exists and every other caller still loads it.
    let page = build_player_payload(&conn, &rules, &mut cache, None, vil, "Vil".into(), 12, None, None, true).unwrap();
    assert_eq!(page.note.as_deref(), Some("opens light from UTG"));
    assert_eq!(overlay.iter().any(|p| p.engine.is_some()), cfg!(feature = "strategic-analysis"));
}

// ------------------------------------------------------------------- cache

#[test]
fn engine_cache_replays_only_when_the_player_has_a_new_hand() {
    let mut t = Table::cash("Rank Cache", 300_008_500, &SIX);
    let mut conn = import(&fold_to_three_bet_hands(&mut t, 20, 0));
    let vil = player_id(&conn, "Vil");
    let mut cache = EngineCache::new();

    let first = cache.payload(&conn, vil, None).unwrap();
    assert_eq!((cache.counts().replays, cache.counts().hits), (1, 0));
    // A refresh with nothing new reuses the replay and gives the same payload.
    let again = cache.payload(&conn, vil, None).unwrap();
    assert_eq!((cache.counts().replays, cache.counts().hits), (1, 1));
    assert_eq!(first, again);

    // A hand at another table without him: still reused.
    let mut other = Table::cash("Rank Elsewhere", 300_008_600, &[HERO, "Ann", "Ben"]);
    velora_poker_lib::import::import_text(&mut conn, &other.walk(100)).unwrap();
    cache.payload(&conn, vil, None).unwrap();
    assert_eq!((cache.counts().replays, cache.counts().hits), (1, 2));

    // His new hand invalidates it: replayed once, then reused again.
    velora_poker_lib::import::import_text(&mut conn, &fold_to_three_bet_hands(&mut t, 1, 200)).unwrap();
    let updated = cache.payload(&conn, vil, None).unwrap();
    assert_eq!((cache.counts().replays, cache.counts().hits), (2, 2));
    let opps = |p: &velora_poker_lib::engine::EnginePayload| {
        p.reads.iter().find(|r| r.rule_id == "pf.fold_to_3bet.high").map(|r| r.evidence[0].opportunities)
    };
    assert_eq!((opps(&first), opps(&updated)), (Some(20), Some(21)));
    cache.payload(&conn, vil, None).unwrap();
    assert_eq!((cache.counts().replays, cache.counts().hits), (2, 3));

    // A rebuild of stored hands (repair, reparse backfill) invalidates it too.
    conn.execute("UPDATE engine_state SET value = value + 1 WHERE key = 'rebuild_generation'", []).unwrap();
    cache.payload(&conn, vil, None).unwrap();
    assert_eq!(cache.counts().replays, 3);

    // A table context in another format replays for that format's half-life.
    let mut ctx = villain_context(&latest_table_hand(&conn, "Rank Cache").unwrap().unwrap(), vil).unwrap();
    ctx.format = FormatKey::Zoom;
    let zoom = cache.payload(&conn, vil, Some(ctx.clone())).unwrap();
    assert_eq!(cache.counts().replays, 4);
    assert_eq!(zoom.context.as_ref().map(|c| c.format), Some(FormatKey::Zoom));
    cache.payload(&conn, vil, Some(ctx)).unwrap();
    assert_eq!(cache.counts().replays, 4);
}
