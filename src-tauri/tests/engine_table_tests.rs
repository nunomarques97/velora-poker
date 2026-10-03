//! Opponent engine, task T9: table quality, multi-table villains and the
//! side-panel snapshot (catalogue rows Q01, Q02 and G04, sections 12 and 13
//! of `docs/specs/opponent-engine.md`).
//!
//! Generated fixture tables (`common::Table`, $0.25/$0.50 cash, 100bb):
//! - 'Soft I': four fish limp, call and check every hand down to showdown;
//!   Shark folds.
//! - 'Tough I': one regular raises first in and takes the blinds each hand.
//! - 'Mixed I': a few walks, seating Reg1, Fish1 and Shark again, so Shark
//!   sits at all three tables, Reg1 and Fish1 at two.

mod common;

use common::{import, player_id, Act, Table, HERO};
use rusqlite::Connection;
use serde_json::Value;
use velora_poker_lib::commands::{side_panel_snapshot, PanelTable, SidePanelSnapshot};
use velora_poker_lib::db;
use velora_poker_lib::engine::{
    multi_table_index, table_quality, villain_softness, EngineCache, FormatKey, QualityLabel,
    VillainQuality,
};

const SOFT: [&str; 6] = [HERO, "Fish1", "Fish2", "Fish3", "Fish4", "Shark"];
const TOUGH: [&str; 6] = [HERO, "Reg1", "Reg2", "Reg3", "Reg4", "Shark"];
const MIXED: [&str; 6] = [HERO, "Reg1", "Fish1", "Shark", "Ann", "Ben"];

/// Cash priors of VPIP, PFR and WTSD (section 4.3).
fn villain(hands: u32, vpip: f64, pfr: f64, wtsd: f64) -> VillainQuality {
    VillainQuality { hands, vpip, pfr, wtsd, prior_vpip: 0.27, prior_pfr: 0.20, prior_wtsd: 0.27 }
}

fn reset(t: &mut Table, players: &[&str]) {
    for p in players {
        t.stacks.insert(p.to_string(), 100.0 * t.bb);
    }
}

/// Seats (button first): Hero, Fish1 (SB), Fish2 (BB), Fish3, Fish4, Shark.
fn soft_hands(t: &mut Table, n: i64, from_minute: i64) -> String {
    let fish = ["Fish1", "Fish2", "Fish3", "Fish4"];
    let check_round: Vec<(&str, Act)> = fish.iter().map(|f| (*f, Act::Check)).collect();
    let mut text = String::new();
    for i in 0..n {
        reset(t, &SOFT);
        text += &t.play(
            from_minute + i,
            &[
                &[
                    ("Fish3", Act::Call), ("Fish4", Act::Call), ("Shark", Act::Fold), (HERO, Act::Fold),
                    ("Fish1", Act::Call), ("Fish2", Act::Check),
                ],
                &check_round,
                &check_round,
                &check_round,
            ],
            Some(fish[i as usize % 4]),
        );
    }
    text
}

/// Seats (button first): Hero, Reg1 (SB), Reg2 (BB), Reg3, Reg4, Shark.
fn tough_hands(t: &mut Table, n: i64, from_minute: i64) -> String {
    let openers = ["Reg3", "Reg4", "Shark", "Reg1"];
    let mut text = String::new();
    for i in 0..n {
        reset(t, &TOUGH);
        text += &t.open_and_take(from_minute + i, openers[i as usize % 4]);
    }
    text
}

fn three_tables() -> Connection {
    let mut soft = Table::cash("Soft I", 300_009_100, &SOFT);
    let mut tough = Table::cash("Tough I", 300_009_300, &TOUGH);
    let mut mixed = Table::cash("Mixed I", 300_009_500, &MIXED);
    let mut text = soft_hands(&mut soft, 40, 0);
    text += &tough_hands(&mut tough, 40, 100);
    for i in 0..3 {
        reset(&mut mixed, &MIXED);
        text += &mixed.walk(200 + i);
    }
    import(&text)
}

fn panel(id: u32, name: &str) -> PanelTable {
    PanelTable { table_id: id, table_name: Some(name.into()), since: None }
}

fn tables() -> Vec<PanelTable> {
    vec![panel(1, "Soft I"), panel(2, "Tough I"), panel(3, "Mixed I")]
}

fn json<T: serde::Serialize>(value: &T) -> Value {
    serde_json::to_value(value).expect("serialize")
}

fn keys(v: &Value) -> Vec<String> {
    let mut keys: Vec<String> = v.as_object().expect("object").keys().cloned().collect();
    keys.sort();
    keys
}

/// `name → otherTableIds` of one table of the snapshot.
fn others(snapshot: &SidePanelSnapshot, table_id: u32) -> Vec<(String, Vec<u32>)> {
    let table = snapshot.tables.iter().find(|t| t.table_id == table_id).expect("table");
    let mut rows: Vec<(String, Vec<u32>)> =
        table.villains.iter().map(|v| (v.name.clone(), v.other_table_ids.clone())).collect();
    rows.sort();
    rows
}

fn rows(expected: &[(&str, &[u32])]) -> Vec<(String, Vec<u32>)> {
    expected.iter().map(|(n, ids)| (n.to_string(), ids.to_vec())).collect()
}

/// The quality of one table straight from the engine (both builds): its
/// latest hand's non-hero roster, each villain's cached replay.
fn engine_quality(conn: &Connection, table: &str) -> Option<velora_poker_lib::engine::TableQuality> {
    let mut cache = EngineCache::new();
    let roster = db::list_active_table_players_with_seats(conn, Some(table), None).unwrap();
    let villains: Vec<VillainQuality> = roster
        .iter()
        .filter(|r| !r.is_hero)
        .map(|r| cache.villain_quality(conn, r.id, Some(FormatKey::Cash)).unwrap())
        .collect();
    table_quality(&villains)
}

// -------------------------------------------------------------- quality

#[test]
fn soft_table_scores_above_tough_table() {
    // Pure: five loose-passive, showdown-bound villains against five tight
    // ones, all well sampled.
    let soft = table_quality(&[villain(200, 0.45, 0.08, 0.38); 5]).expect("soft quality");
    let tough = table_quality(&[villain(200, 0.20, 0.18, 0.22); 5]).expect("tough quality");
    assert!(soft.score > tough.score, "{soft:?} vs {tough:?}");
    assert_eq!(soft.label, QualityLabel::Soft);
    assert_eq!(tough.label, QualityLabel::Tough);
    assert_eq!(soft.basis_hands, 1000);

    // Fixture tables, from the villains' replayed and shrunk stats.
    let conn = three_tables();
    let soft = engine_quality(&conn, "Soft I").expect("soft table sampled");
    let tough = engine_quality(&conn, "Tough I").expect("tough table sampled");
    assert!(soft.score > tough.score, "{soft:?} vs {tough:?}");
    assert_eq!(soft.label, QualityLabel::Soft, "{soft:?}");
    assert!(tough.score < 50, "{tough:?}");
}

#[test]
fn quality_labels_fish_soft_regs_and_nits_tough_unknowns_neutral() {
    // Pro review (D107): soft_i = clamp((.35 L + .45 P + .20 S) / 1.25).
    // Fish: loose, passive, showdown-bound.
    let fish = table_quality(&[villain(200, 0.45, 0.08, 0.38); 5]).unwrap();
    assert_eq!(fish.label, QualityLabel::Soft, "{fish:?}");
    assert!(fish.score >= 85, "{fish:?}");

    // Solid 6-max regulars, 23/19 with a normal WTSD: L = −.5, P = −.375,
    // S = 0 → −.275 × 200/220 = −.25 → 37.5. D106's (.5 L + .3 P + .2 S)/2
    // gave −.18 × .91 → 42, "average": a table of five regs was not tough.
    let regs = table_quality(&[villain(200, 0.23, 0.19, 0.27); 5]).unwrap();
    assert_eq!(regs.label, QualityLabel::Tough, "{regs:?}");
    assert!((37..=38).contains(&regs.score), "{regs:?}");
    // Nits (13/11, rarely at showdown) are tough to get paid by.
    let nits = table_quality(&[villain(200, 0.13, 0.11, 0.22); 5]).unwrap();
    assert_eq!(nits.label, QualityLabel::Tough, "{nits:?}");
    assert!(nits.score < regs.score, "{nits:?} vs {regs:?}");
    // A mix of regs and nits is tough as well.
    let mut mixed = vec![villain(150, 0.23, 0.19, 0.27); 3];
    mixed.extend([villain(150, 0.13, 0.11, 0.22); 2]);
    assert_eq!(table_quality(&mixed).unwrap().label, QualityLabel::Tough);

    // A loose but aggressive regular (30/26) is not soft: his VPIP–PFR gap
    // is narrower than the pool's.
    let lag_reg = villain_softness(&villain(200, 0.30, 0.26, 0.26));
    assert!(lag_reg < 0.0, "{lag_reg}");
    // A maniac (55/45) still is: his looseness dominates.
    assert!(villain_softness(&villain(200, 0.55, 0.45, 0.30)) > 0.5);

    // Unknowns: a fresh table of five strangers with 5–10 hands each, whose
    // shrunk stats sit a little either side of the priors, scores neutral.
    let unknowns = [
        villain(10, 0.33, 0.17, 0.30),
        villain(8, 0.22, 0.18, 0.25),
        villain(6, 0.30, 0.15, 0.29),
        villain(6, 0.25, 0.21, 0.26),
        villain(5, 0.29, 0.19, 0.28),
    ];
    let q = table_quality(&unknowns).expect("35 hands, one villain at 10");
    assert_eq!(q.label, QualityLabel::Average, "{q:?}");
    assert!((45..=55).contains(&q.score), "{q:?}");
}

#[test]
fn unknown_players_pull_quality_toward_neutral() {
    let fish = villain(200, 0.50, 0.06, 0.40);
    let alone = table_quality(&[fish]).expect("quality");
    let unknown = villain(2, 0.27, 0.20, 0.27);
    let strangers = villain(3, 0.60, 0.02, 0.50);
    let diluted = table_quality(&[fish, unknown, strangers, villain(0, 0.27, 0.20, 0.27)]).expect("quality");
    assert!(alone.score > diluted.score, "{alone:?} vs {diluted:?}");
    assert!(diluted.score > 50, "still on the soft side: {diluted:?}");
    // A wild stranger with three hands weighs 3/23 of a regular.
    let s = villain_softness(&strangers);
    assert_eq!(s, 1.0);
    // Every villain at the prior is exactly neutral.
    let neutral = table_quality(&[villain(100, 0.27, 0.20, 0.27); 6]).unwrap();
    assert_eq!((neutral.score, neutral.label), (50, QualityLabel::Average));
}

#[test]
fn quality_null_below_sample() {
    // 29 hands in total.
    assert_eq!(table_quality(&[villain(20, 0.5, 0.1, 0.4), villain(9, 0.5, 0.1, 0.4)]), None);
    // 40 hands, but no villain with 10.
    assert_eq!(table_quality(&[villain(8, 0.5, 0.1, 0.4); 5]), None);
    // No villains at all.
    assert_eq!(table_quality(&[]), None);
    // Exactly 30 hands with one villain at 10: scored.
    let q = table_quality(&[villain(10, 0.5, 0.1, 0.4), villain(20, 0.5, 0.1, 0.4)]).expect("scored");
    assert_eq!(q.basis_hands, 30);
}

// ---------------------------------------------------------- multi-table

#[test]
fn multi_table_detection_lists_other_tables() {
    // No tracked table.
    let none = multi_table_index(&[]);
    assert_eq!(none.other_tables(1, 42), Vec::<u32>::new());
    assert_eq!(none.multi_tabling().count(), 0);

    // Two tables: Shark (7) at both, the hero (1) at both, others at one.
    let two = multi_table_index(&[
        (4, vec![(1, true), (7, false), (8, false)]),
        (9, vec![(1, true), (7, false), (5, false)]),
    ]);
    assert_eq!(two.other_tables(4, 7), [9]);
    assert_eq!(two.other_tables(9, 7), [4]);
    assert_eq!(two.other_tables(4, 8), Vec::<u32>::new());
    assert_eq!(two.other_tables(4, 1), Vec::<u32>::new(), "the hero is never flagged");
    assert_eq!(two.multi_tabling().map(|(p, _)| p).collect::<Vec<_>>(), [7]);

    // Three tables: Shark at all three, Reg (8) at two, listed ascending.
    let three = multi_table_index(&[
        (9, vec![(1, true), (7, false), (8, false)]),
        (3, vec![(1, true), (7, false)]),
        (5, vec![(1, true), (7, false), (8, false), (6, false)]),
    ]);
    assert_eq!(three.other_tables(5, 7), [3, 9]);
    assert_eq!(three.other_tables(3, 7), [5, 9]);
    assert_eq!(three.other_tables(9, 8), [5]);
    assert_eq!(three.other_tables(5, 6), Vec::<u32>::new());

    // Fixture tables through the snapshot (both builds).
    let conn = three_tables();
    let mut cache = EngineCache::new();
    let snapshot = side_panel_snapshot(&conn, &mut cache, &tables()).unwrap();
    assert_eq!(
        others(&snapshot, 1),
        rows(&[("Fish1", &[3]), ("Fish2", &[]), ("Fish3", &[]), ("Fish4", &[]), ("Shark", &[2, 3])])
    );
    assert_eq!(
        others(&snapshot, 2),
        rows(&[("Reg1", &[3]), ("Reg2", &[]), ("Reg3", &[]), ("Reg4", &[]), ("Shark", &[1, 3])])
    );
    assert_eq!(
        others(&snapshot, 3),
        rows(&[("Ann", &[]), ("Ben", &[]), ("Fish1", &[1]), ("Reg1", &[2]), ("Shark", &[1, 2])])
    );
}

// ------------------------------------------------------------- snapshot

#[test]
fn side_panel_snapshot_follows_the_spec_contract() {
    let conn = three_tables();
    let mut cache = EngineCache::new();
    let snapshot = side_panel_snapshot(&conn, &mut cache, &tables()).unwrap();
    let v = json(&snapshot);
    assert_eq!(keys(&v), ["generatedAt", "tables"]);
    let tables = v["tables"].as_array().unwrap();
    assert_eq!(tables.iter().map(|t| t["tableId"].as_u64().unwrap()).collect::<Vec<_>>(), [1, 2, 3]);
    let soft = &tables[0];
    assert_eq!(keys(soft), ["maxPlayers", "playerCount", "quality", "tableId", "tableName", "villains"]);
    assert_eq!((soft["tableName"].as_str(), soft["maxPlayers"].as_i64()), (Some("Soft I"), Some(6)));
    // The hero is counted at the table but is not a villain.
    assert_eq!(soft["playerCount"], 6);
    let villains = soft["villains"].as_array().unwrap();
    assert_eq!(villains.len(), 5);
    assert!(villains.iter().all(|p| p["name"] != HERO));
    let fish3 = villains.iter().find(|p| p["name"] == "Fish3").unwrap();
    assert_eq!(keys(fish3), ["hands", "name", "otherTableIds", "playerId", "seat", "tag", "topRead"]);
    assert_eq!(fish3["playerId"], player_id(&conn, "Fish3").to_string());
    assert_eq!((fish3["seat"].as_i64(), fish3["hands"].as_i64()), (Some(4), Some(40)));
}

#[test]
fn side_panel_scopes_each_table_to_its_own_roster() {
    let conn = three_tables();
    let mut cache = EngineCache::new();
    // An unparsed title next to another table renders empty, never another
    // table's players.
    let two = side_panel_snapshot(
        &conn,
        &mut cache,
        &[panel(1, "Soft I"), PanelTable { table_id: 2, table_name: None, since: None }],
    )
    .unwrap();
    assert_eq!(two.tables[0].villains.len(), 5);
    let unparsed = &two.tables[1];
    assert_eq!((unparsed.player_count, unparsed.max_players, unparsed.villains.len()), (0, None, 0));
    assert!(unparsed.quality.is_none());
    // Alone, it keeps the global fallback: the latest hand anywhere (Mixed I).
    let one = side_panel_snapshot(&conn, &mut cache, &[PanelTable { table_id: 2, table_name: None, since: None }])
        .unwrap();
    let mut names: Vec<&str> = one.tables[0].villains.iter().map(|v| v.name.as_str()).collect();
    names.sort_unstable();
    assert_eq!(names, ["Ann", "Ben", "Fish1", "Reg1", "Shark"]);
    // The `since` floor: a sitting that started after every stored hand has
    // no roster yet.
    let later = PanelTable { table_id: 1, table_name: Some("Soft I".into()), since: Some("2027-01-01T00:00:00".into()) };
    let fresh = side_panel_snapshot(&conn, &mut cache, &[later]).unwrap();
    assert_eq!((fresh.tables[0].player_count, fresh.tables[0].villains.len()), (0, 0));
    // No tracked table at all.
    assert!(side_panel_snapshot(&conn, &mut cache, &[]).unwrap().tables.is_empty());
}

#[cfg(not(feature = "strategic-analysis"))]
#[test]
fn default_build_side_panel_has_no_quality_or_reads() {
    let conn = three_tables();
    let mut cache = EngineCache::new();
    let snapshot = side_panel_snapshot(&conn, &mut cache, &tables()).unwrap();
    let v = json(&snapshot);
    for table in v["tables"].as_array().unwrap() {
        assert_eq!(table["quality"], Value::Null, "{}", table["tableName"]);
        let villains = table["villains"].as_array().unwrap();
        assert_eq!(villains.len(), 5);
        for villain in villains {
            assert_eq!(villain["tag"], Value::Null);
            assert_eq!(villain["topRead"], Value::Null);
            assert!(villain["hands"].as_i64().unwrap() > 0);
            assert!(villain["otherTableIds"].is_array());
        }
    }
    // Tables, villains, hands and multi-table flags are all there.
    assert_eq!(others(&snapshot, 1).iter().find(|(n, _)| n == "Shark").unwrap().1, [2, 3]);
    // Nothing was computed by the engine: no replay, no cache entry.
    assert_eq!(cache.counts().replays + cache.counts().hits, 0);
}

#[cfg(feature = "strategic-analysis")]
#[test]
fn strategic_side_panel_carries_quality_tags_and_reads() {
    let conn = three_tables();
    let mut cache = EngineCache::new();
    let snapshot = side_panel_snapshot(&conn, &mut cache, &tables()).unwrap();
    let soft = snapshot.tables[0].quality.expect("soft table scored");
    let tough = snapshot.tables[1].quality.expect("tough table scored");
    assert!(soft.score > tough.score, "{soft:?} vs {tough:?}");
    assert_eq!(soft.label, QualityLabel::Soft);
    // The snapshot's score is the engine's own.
    assert_eq!(Some(soft), engine_quality(&conn, "Soft I"));
    let v = json(&snapshot);
    assert_eq!(keys(&v["tables"][0]["quality"]), ["basisHands", "label", "score"]);
    assert_eq!(v["tables"][0]["quality"]["label"], "soft");

    // Reads: the fish limp far above the prior.
    let fish3 = snapshot.tables[0].villains.iter().find(|p| p.name == "Fish3").unwrap();
    let read = fish3.top_read.as_ref().expect("a top read for a 40-hand limper");
    assert!(!read.observation.is_empty() && !read.advice.is_empty());
    assert!(fish3.tag.is_some());
    let fish3_json = v["tables"][0]["villains"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["name"] == "Fish3")
        .unwrap()
        .clone();
    assert_eq!(keys(&fish3_json["topRead"]), ["advice", "confidencePct", "observation", "ruleId"]);
    // The top read and tag are the overlay payload's own.
    let payloads =
        velora_poker_lib::commands::active_table_payloads(&conn, &mut cache, Some("Soft I"), None).unwrap();
    let engine = payloads.iter().find(|p| p.name == "Fish3").unwrap().engine.as_ref().unwrap();
    assert_eq!(read.rule_id, engine.top_reads[0].rule_id);
    assert_eq!(fish3.tag.as_deref(), engine.tag.as_ref().map(|t| t.text.as_str()));
}
