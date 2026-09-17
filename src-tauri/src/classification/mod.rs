use rusqlite::{params, Connection};
use serde::Serialize;

use crate::db;
use crate::stats::PlayerStats;

/// The player-archetype buckets the built-in rules classify into. A custom
/// rule set (future work) can still only ever resolve to one of these plus
/// `Unknown` — the enum is what the HUD colors/labels key off of.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum PlayerClassification {
    Unknown,
    LoosePassive,
    LooseAggressive,
    TightAggressive,
    Maniac,
    NittyRock,
    Recreational,
}

#[derive(Debug, Clone)]
pub struct ClassificationRule {
    pub id: String,
    pub label: String,
    pub color: String,
    pub priority: i64,
    pub min_hands: i64,
    pub vpip_min: Option<f64>,
    pub vpip_max: Option<f64>,
    pub pfr_min: Option<f64>,
    pub pfr_max: Option<f64>,
    pub three_bet_min: Option<f64>,
    pub three_bet_max: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClassificationResult {
    pub classification: PlayerClassification,
    pub label: String,
    pub color: String,
    /// True when this result comes from a manual per-player override rather
    /// than the automatic rule engine.
    pub is_override: bool,
    /// False only when the automatic rule engine is not compiled into this
    /// build (`auto-classification` off) and there is no manual override —
    /// distinguishes "this build can't classify" from a genuine Unknown
    /// (below `min_hands`, or no rule matched). The PROFILE section must
    /// render differently for the two: "Classification unavailable in this
    /// build" here, vs. a normal Unknown badge when `available` is true.
    /// TENDENCIES/EXPLOITS/CONFIDENCE never read this field — they come from
    /// `description_rules`, gated by the separate `strategic-analysis` flag,
    /// and must render unaffected either way (Phase 1 flag-independence).
    pub available: bool,
}

const UNKNOWN_COLOR: &str = "#6b7480";
const UNKNOWN_LABEL: &str = "Unknown";
#[cfg(not(feature = "auto-classification"))]
const UNAVAILABLE_LABEL: &str = "Classification unavailable in this build";

fn unknown_result() -> ClassificationResult {
    ClassificationResult {
        classification: PlayerClassification::Unknown,
        label: UNKNOWN_LABEL.to_string(),
        color: UNKNOWN_COLOR.to_string(),
        is_override: false,
        available: true,
    }
}

#[cfg(not(feature = "auto-classification"))]
fn unavailable_result() -> ClassificationResult {
    ClassificationResult {
        classification: PlayerClassification::Unknown,
        label: UNAVAILABLE_LABEL.to_string(),
        color: UNKNOWN_COLOR.to_string(),
        is_override: false,
        available: false,
    }
}

/// Built-in rule thresholds. Evaluated in ascending `priority` order; the
/// first matching rule wins. These are sensible defaults, not a claim of
/// statistical rigor — tightening/loosening them later only means editing
/// this table's rows, not the engine.
pub fn builtin_rules() -> Vec<ClassificationRule> {
    vec![
        ClassificationRule {
            id: "builtin-maniac".to_string(),
            label: "Maniac".to_string(),
            color: "#e0524f".to_string(),
            priority: 0,
            min_hands: 25,
            vpip_min: Some(40.0),
            vpip_max: None,
            pfr_min: Some(28.0),
            pfr_max: None,
            three_bet_min: Some(12.0),
            three_bet_max: None,
        },
        ClassificationRule {
            id: "builtin-loose-aggressive".to_string(),
            label: "Loose Aggressive".to_string(),
            color: "#e0954f".to_string(),
            priority: 1,
            min_hands: 25,
            vpip_min: Some(30.0),
            vpip_max: None,
            pfr_min: Some(18.0),
            pfr_max: None,
            three_bet_min: None,
            three_bet_max: None,
        },
        ClassificationRule {
            id: "builtin-loose-passive".to_string(),
            label: "Loose Passive".to_string(),
            color: "#d9b44a".to_string(),
            priority: 2,
            min_hands: 25,
            vpip_min: Some(30.0),
            vpip_max: None,
            pfr_min: None,
            pfr_max: Some(12.0),
            three_bet_min: None,
            three_bet_max: None,
        },
        ClassificationRule {
            id: "builtin-tight-aggressive".to_string(),
            label: "Tight Aggressive".to_string(),
            color: "#6f8fff".to_string(),
            priority: 3,
            min_hands: 25,
            vpip_min: None,
            vpip_max: Some(30.0),
            pfr_min: Some(13.0),
            pfr_max: None,
            three_bet_min: None,
            three_bet_max: None,
        },
        ClassificationRule {
            id: "builtin-nitty-rock".to_string(),
            label: "Nitty / Rock".to_string(),
            color: "#5b7a99".to_string(),
            priority: 4,
            min_hands: 25,
            vpip_min: None,
            vpip_max: Some(15.0),
            pfr_min: None,
            pfr_max: Some(10.0),
            three_bet_min: None,
            three_bet_max: None,
        },
        ClassificationRule {
            id: "builtin-recreational".to_string(),
            label: "Recreational".to_string(),
            color: "#57b88b".to_string(),
            priority: 5,
            min_hands: 25,
            vpip_min: None,
            vpip_max: None,
            pfr_min: None,
            pfr_max: None,
            three_bet_min: None,
            three_bet_max: None,
        },
    ]
}

fn label_to_classification(id: &str) -> PlayerClassification {
    match id {
        "builtin-maniac" => PlayerClassification::Maniac,
        "builtin-loose-aggressive" => PlayerClassification::LooseAggressive,
        "builtin-loose-passive" => PlayerClassification::LoosePassive,
        "builtin-tight-aggressive" => PlayerClassification::TightAggressive,
        "builtin-nitty-rock" => PlayerClassification::NittyRock,
        _ => PlayerClassification::Recreational,
    }
}

pub fn seed_builtin_rules(conn: &Connection) -> rusqlite::Result<()> {
    for rule in builtin_rules() {
        conn.execute(
            "INSERT OR IGNORE INTO classification_rules
             (id, label, color, priority, min_hands, vpip_min, vpip_max, pfr_min, pfr_max, three_bet_min, three_bet_max, is_builtin)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, 1)",
            params![
                rule.id,
                rule.label,
                rule.color,
                rule.priority,
                rule.min_hands,
                rule.vpip_min,
                rule.vpip_max,
                rule.pfr_min,
                rule.pfr_max,
                rule.three_bet_min,
                rule.three_bet_max,
            ],
        )?;
    }
    Ok(())
}

pub fn list_rules(conn: &Connection) -> rusqlite::Result<Vec<ClassificationRule>> {
    let mut stmt = conn.prepare(
        "SELECT id, label, color, priority, min_hands, vpip_min, vpip_max, pfr_min, pfr_max, three_bet_min, three_bet_max
         FROM classification_rules ORDER BY priority ASC",
    )?;
    let rows = stmt
        .query_map([], |row| {
            Ok(ClassificationRule {
                id: row.get(0)?,
                label: row.get(1)?,
                color: row.get(2)?,
                priority: row.get(3)?,
                min_hands: row.get(4)?,
                vpip_min: row.get(5)?,
                vpip_max: row.get(6)?,
                pfr_min: row.get(7)?,
                pfr_max: row.get(8)?,
                three_bet_min: row.get(9)?,
                three_bet_max: row.get(10)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

fn in_range(value: f64, min: Option<f64>, max: Option<f64>) -> bool {
    min.map_or(true, |m| value >= m) && max.map_or(true, |m| value <= m)
}

fn rule_matches(rule: &ClassificationRule, hands: i64, stats: &PlayerStats) -> bool {
    // `unwrap_or(0.0)` here only feeds rule bucketing, never a displayed
    // value: vpip/pfr are `None` only when `hands == 0`, already excluded by
    // the `min_hands` gate below, and a `None` three_bet (no opportunity yet)
    // sits at the low end of every threshold rule same as a real 0% would.
    hands >= rule.min_hands
        && in_range(stats.vpip.unwrap_or(0.0), rule.vpip_min, rule.vpip_max)
        && in_range(stats.pfr.unwrap_or(0.0), rule.pfr_min, rule.pfr_max)
        && in_range(stats.three_bet.unwrap_or(0.0), rule.three_bet_min, rule.three_bet_max)
}

/// Runs the automatic rule engine only (ignores manual overrides).
pub fn classify(rules: &[ClassificationRule], hands: i64, stats: &PlayerStats) -> ClassificationResult {
    for rule in rules {
        if rule_matches(rule, hands, stats) {
            return ClassificationResult {
                classification: label_to_classification(&rule.id),
                label: rule.label.clone(),
                color: rule.color.clone(),
                is_override: false,
                available: true,
            };
        }
    }
    unknown_result()
}

/// Resolves a player's HUD classification: a manual color override always
/// wins (allowed under PokerStars' TOS regardless of the `auto-classification`
/// feature); otherwise falls back to the automatic rule engine, whose result
/// is only surfaced when that feature is compiled in — see the flag's doc
/// comment in Cargo.toml for why.
pub fn resolve_for_player(
    conn: &Connection,
    rules: &[ClassificationRule],
    player_id: i64,
    hands: i64,
    stats: &PlayerStats,
) -> rusqlite::Result<ClassificationResult> {
    if let Some((color, label)) = db::get_player_color_override(conn, player_id)? {
        return Ok(ClassificationResult {
            classification: PlayerClassification::Unknown,
            label: label.unwrap_or_else(|| "Custom".to_string()),
            color,
            is_override: true,
            available: true,
        });
    }

    #[cfg(feature = "auto-classification")]
    {
        Ok(classify(rules, hands, stats))
    }
    #[cfg(not(feature = "auto-classification"))]
    {
        let _ = (rules, hands, stats);
        Ok(unavailable_result())
    }
}
