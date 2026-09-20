use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::db;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StatPage {
    pub id: String,
    pub label: String,
    /// Keys into the existing `PlayerStats` payload (`vpip`, `pfr`,
    /// `threeBet`, `foldToThreeBet`, `cBet`, `foldToCBet`,
    /// `aggressionFactor`, `wtsd`, `wsd`). No invented stats.
    pub stat_keys: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HudProfile {
    pub id: String,
    pub name: String,
    pub visual_model: String,
    pub stat_pages: Vec<StatPage>,
    pub min_hands: i64,
    pub is_builtin: bool,
}

fn canonical_stat_pages() -> Vec<StatPage> {
    vec![
        StatPage {
            id: "core".to_string(),
            label: "Core".to_string(),
            stat_keys: vec!["vpip".into(), "pfr".into(), "threeBet".into()],
        },
        StatPage {
            id: "preflop".to_string(),
            label: "Preflop".to_string(),
            stat_keys: vec!["threeBet".into(), "foldToThreeBet".into()],
        },
        StatPage {
            id: "postflop".to_string(),
            label: "Postflop".to_string(),
            stat_keys: vec![
                "cBet".into(),
                "foldToCBet".into(),
                "aggressionFactor".into(),
            ],
        },
        StatPage {
            id: "showdown".to_string(),
            label: "Showdown".to_string(),
            stat_keys: vec!["wtsd".into(), "wsd".into()],
        },
    ]
}

pub fn builtin_profiles() -> Vec<HudProfile> {
    let pages = canonical_stat_pages();
    vec![
        HudProfile {
            id: "velora-hud".to_string(),
            name: "Velora HUD".to_string(),
            visual_model: "velora_hud".to_string(),
            stat_pages: pages.clone(),
            min_hands: 25,
            is_builtin: true,
        },
        HudProfile {
            id: "velora-classic".to_string(),
            name: "Velora Classic".to_string(),
            visual_model: "velora_classic".to_string(),
            stat_pages: pages.clone(),
            min_hands: 25,
            is_builtin: true,
        },
        HudProfile {
            id: "minimal".to_string(),
            name: "Minimal".to_string(),
            visual_model: "minimal".to_string(),
            stat_pages: pages.clone(),
            min_hands: 25,
            is_builtin: true,
        },
        // Badge model: no stats on the table at all, just a small clickable
        // pill with the player's initials and hand count. The stats live one
        // click away, in the detail drawer. Built for multi-tabling, where
        // full cards on every table cover the tables themselves. It carries
        // the same stat pages as the others so switching models never loses
        // a page configuration; the frontend simply doesn't render them.
        HudProfile {
            id: "badge".to_string(),
            name: "Badge".to_string(),
            visual_model: "badge".to_string(),
            stat_pages: pages.clone(),
            min_hands: 25,
            is_builtin: true,
        },
        // The Jivaro-style model. Backend-wise it is deliberately nothing new: the
        // same canonical stat pages, the same 25-hand gate and the same
        // `classification::resolve_for_player` colour source as the other three
        // — the whole difference lives in the frontend's rendering of it
        // (`src/hud/JivaroHudCard.tsx`).
        HudProfile {
            id: "jivaro".to_string(),
            name: "Jivaro".to_string(),
            visual_model: "jivaro".to_string(),
            stat_pages: pages,
            min_hands: 25,
            is_builtin: true,
        },
    ]
}

pub const ACTIVE_PROFILE_SETTING: &str = "active_hud_profile_id";
/// The badge model: a new install lands on the smallest HUD there is, one
/// clickable pill per player, and the full cards are one click away in HUD
/// Profiles. Chosen as the default because the full cards cover the tables
/// themselves once more than one is open.
const DEFAULT_PROFILE_ID: &str = "badge";

/// Renames the old "jivaro-inspired" builtin profile (from before the
/// product's own default HUD visual was implemented) to "velora-hud" in
/// place, preserving the row's `min_hands` and whichever setting pointed at
/// it, rather than leaving an orphaned Jivaro-named row behind.
fn migrate_legacy_jivaro_profile(conn: &Connection) -> rusqlite::Result<()> {
    let exists: Option<String> = conn
        .query_row(
            "SELECT id FROM hud_profiles WHERE id = 'jivaro-inspired'",
            [],
            |row| row.get(0),
        )
        .optional()?;
    if exists.is_none() {
        return Ok(());
    }

    let pages_json =
        serde_json::to_string(&canonical_stat_pages()).unwrap_or_else(|_| "[]".to_string());
    conn.execute(
        "UPDATE hud_profiles SET id = 'velora-hud', name = 'Velora HUD', visual_model = 'velora_hud', stat_pages = ?1
         WHERE id = 'jivaro-inspired'",
        params![pages_json],
    )?;

    if db::get_setting(conn, ACTIVE_PROFILE_SETTING)? == Some("jivaro-inspired".to_string()) {
        db::set_setting(conn, ACTIVE_PROFILE_SETTING, "velora-hud")?;
    }

    Ok(())
}

pub fn seed_builtin_profiles(conn: &Connection) -> rusqlite::Result<()> {
    migrate_legacy_jivaro_profile(conn)?;

    for profile in builtin_profiles() {
        let pages_json =
            serde_json::to_string(&profile.stat_pages).unwrap_or_else(|_| "[]".to_string());
        conn.execute(
            "INSERT OR IGNORE INTO hud_profiles (id, name, visual_model, stat_pages, min_hands, is_builtin, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, 1, ?6)",
            params![
                profile.id,
                profile.name,
                profile.visual_model,
                pages_json,
                profile.min_hands,
                db::now_iso(),
            ],
        )?;
    }
    if db::get_setting(conn, ACTIVE_PROFILE_SETTING)?.is_none() {
        db::set_setting(conn, ACTIVE_PROFILE_SETTING, DEFAULT_PROFILE_ID)?;
    }
    Ok(())
}

fn row_to_profile(
    id: String,
    name: String,
    visual_model: String,
    stat_pages_json: String,
    min_hands: i64,
    is_builtin: i64,
) -> HudProfile {
    let stat_pages: Vec<StatPage> = serde_json::from_str(&stat_pages_json).unwrap_or_default();
    HudProfile {
        id,
        name,
        visual_model,
        stat_pages,
        min_hands,
        is_builtin: is_builtin != 0,
    }
}

pub fn list_profiles(conn: &Connection) -> rusqlite::Result<Vec<HudProfile>> {
    let mut stmt = conn.prepare(
        "SELECT id, name, visual_model, stat_pages, min_hands, is_builtin
         FROM hud_profiles ORDER BY is_builtin DESC, created_at ASC",
    )?;
    let rows = stmt
        .query_map([], |row| {
            Ok(row_to_profile(
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

pub fn get_profile(conn: &Connection, id: &str) -> rusqlite::Result<Option<HudProfile>> {
    conn.query_row(
        "SELECT id, name, visual_model, stat_pages, min_hands, is_builtin FROM hud_profiles WHERE id = ?1",
        params![id],
        |row| {
            Ok(row_to_profile(
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
            ))
        },
    )
    .optional()
}

pub fn get_active_profile(conn: &Connection) -> rusqlite::Result<HudProfile> {
    let active_id =
        db::get_setting(conn, ACTIVE_PROFILE_SETTING)?.unwrap_or_else(|| DEFAULT_PROFILE_ID.to_string());
    if let Some(profile) = get_profile(conn, &active_id)? {
        return Ok(profile);
    }
    // Settings pointed at a profile that no longer exists; fall back to the
    // built-in default rather than erroring the whole HUD out.
    get_profile(conn, DEFAULT_PROFILE_ID)?.ok_or(rusqlite::Error::QueryReturnedNoRows)
}

pub fn set_active_profile(conn: &Connection, id: &str) -> rusqlite::Result<()> {
    db::set_setting(conn, ACTIVE_PROFILE_SETTING, id)
}

pub fn set_profile_min_hands(conn: &Connection, id: &str, min_hands: i64) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE hud_profiles SET min_hands = ?1 WHERE id = ?2",
        params![min_hands, id],
    )?;
    Ok(())
}
