use std::path::Path;

use rusqlite::{params, Connection, OptionalExtension};

pub fn now_iso() -> String {
    chrono::Utc::now().to_rfc3339()
}

pub fn open(db_path: &Path) -> rusqlite::Result<Connection> {
    if let Some(parent) = db_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let conn = Connection::open(db_path)?;
    conn.execute_batch("PRAGMA foreign_keys = ON; PRAGMA journal_mode = WAL;")?;
    init_schema(&conn)?;
    migrate_schema(&conn)?;
    seed_defaults(&conn)?;
    Ok(conn)
}

fn init_schema(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS players (
            id INTEGER PRIMARY KEY,
            site TEXT NOT NULL DEFAULT 'pokerstars',
            name TEXT NOT NULL,
            created_at TEXT NOT NULL,
            UNIQUE(site, name)
        );

        CREATE TABLE IF NOT EXISTS hands (
            id INTEGER PRIMARY KEY,
            site TEXT NOT NULL DEFAULT 'pokerstars',
            hand_id TEXT NOT NULL UNIQUE,
            format TEXT NOT NULL DEFAULT 'cash',
            table_name TEXT,
            game_type TEXT,
            tournament_id TEXT,
            buy_in TEXT,
            level TEXT,
            small_blind REAL,
            big_blind REAL,
            currency TEXT,
            max_seats INTEGER,
            button_seat INTEGER,
            played_at TEXT,
            raw_text TEXT,
            imported_at TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS player_hands (
            id INTEGER PRIMARY KEY,
            hand_id INTEGER NOT NULL REFERENCES hands(id) ON DELETE CASCADE,
            player_id INTEGER NOT NULL REFERENCES players(id) ON DELETE CASCADE,
            seat INTEGER,
            starting_stack REAL,
            position TEXT,
            hole_cards TEXT,
            is_hero INTEGER NOT NULL DEFAULT 0,
            went_to_showdown INTEGER NOT NULL DEFAULT 0,
            won_at_showdown INTEGER NOT NULL DEFAULT 0,
            net_result REAL,
            UNIQUE(hand_id, player_id)
        );

        CREATE TABLE IF NOT EXISTS actions (
            id INTEGER PRIMARY KEY,
            hand_id INTEGER NOT NULL REFERENCES hands(id) ON DELETE CASCADE,
            player_id INTEGER NOT NULL REFERENCES players(id) ON DELETE CASCADE,
            street TEXT NOT NULL,
            action_index INTEGER NOT NULL,
            action_type TEXT NOT NULL,
            amount REAL,
            is_all_in INTEGER NOT NULL DEFAULT 0
        );

        CREATE TABLE IF NOT EXISTS settings (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS classification_rules (
            id TEXT PRIMARY KEY,
            label TEXT NOT NULL,
            color TEXT NOT NULL,
            priority INTEGER NOT NULL,
            min_hands INTEGER NOT NULL DEFAULT 0,
            vpip_min REAL,
            vpip_max REAL,
            pfr_min REAL,
            pfr_max REAL,
            three_bet_min REAL,
            three_bet_max REAL,
            is_builtin INTEGER NOT NULL DEFAULT 0
        );

        CREATE TABLE IF NOT EXISTS player_color_overrides (
            player_id INTEGER PRIMARY KEY REFERENCES players(id) ON DELETE CASCADE,
            color TEXT NOT NULL,
            label TEXT,
            updated_at TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS hud_profiles (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            visual_model TEXT NOT NULL,
            stat_pages TEXT NOT NULL,
            min_hands INTEGER NOT NULL DEFAULT 25,
            is_builtin INTEGER NOT NULL DEFAULT 0,
            created_at TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS hud_positions (
            player_id INTEGER PRIMARY KEY REFERENCES players(id) ON DELETE CASCADE,
            x REAL NOT NULL,
            y REAL NOT NULL,
            updated_at TEXT NOT NULL
        );

        CREATE INDEX IF NOT EXISTS idx_player_hands_player ON player_hands(player_id);
        CREATE INDEX IF NOT EXISTS idx_player_hands_hand ON player_hands(hand_id);
        CREATE INDEX IF NOT EXISTS idx_actions_hand ON actions(hand_id);
        CREATE INDEX IF NOT EXISTS idx_actions_player ON actions(player_id);
        CREATE INDEX IF NOT EXISTS idx_hands_played_at ON hands(played_at DESC, id DESC);
        "#,
    )
}

/// Adds columns introduced after a database was first created. `CREATE
/// TABLE IF NOT EXISTS` above only applies to brand-new databases, so an
/// existing `hands` table (from before tournament support was added) needs
/// these columns added explicitly. Safe to run on every startup.
fn migrate_schema(conn: &Connection) -> rusqlite::Result<()> {
    add_column_if_missing(conn, "hands", "format", "TEXT NOT NULL DEFAULT 'cash'")?;
    add_column_if_missing(conn, "hands", "tournament_id", "TEXT")?;
    add_column_if_missing(conn, "hands", "buy_in", "TEXT")?;
    add_column_if_missing(conn, "hands", "level", "TEXT")?;
    Ok(())
}

/// Seeds built-in classification rules and HUD profiles the first time a
/// database is opened. Uses fixed ids and `INSERT OR IGNORE` so it never
/// overwrites a user's own edits to these rows, and is safe to call on every
/// startup.
pub fn seed_defaults(conn: &Connection) -> rusqlite::Result<()> {
    crate::classification::seed_builtin_rules(conn)?;
    crate::hud::seed_builtin_profiles(conn)?;
    Ok(())
}

fn add_column_if_missing(
    conn: &Connection,
    table: &str,
    column: &str,
    declaration: &str,
) -> rusqlite::Result<()> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let existing: Vec<String> = stmt
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<Result<_, _>>()?;

    if !existing.iter().any(|c| c == column) {
        conn.execute(
            &format!("ALTER TABLE {table} ADD COLUMN {column} {declaration}"),
            [],
        )?;
    }
    Ok(())
}

pub fn get_or_create_player(conn: &Connection, site: &str, name: &str) -> rusqlite::Result<i64> {
    conn.execute(
        "INSERT INTO players (site, name, created_at) VALUES (?1, ?2, ?3)
         ON CONFLICT(site, name) DO NOTHING",
        params![site, name, now_iso()],
    )?;
    conn.query_row(
        "SELECT id FROM players WHERE site = ?1 AND name = ?2",
        params![site, name],
        |row| row.get(0),
    )
}

pub fn get_setting(conn: &Connection, key: &str) -> rusqlite::Result<Option<String>> {
    conn.query_row(
        "SELECT value FROM settings WHERE key = ?1",
        params![key],
        |row| row.get(0),
    )
    .optional()
}

pub fn set_setting(conn: &Connection, key: &str, value: &str) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO settings (key, value, updated_at) VALUES (?1, ?2, ?3)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
        params![key, value, now_iso()],
    )?;
    Ok(())
}

pub struct PlayerRow {
    pub id: i64,
    pub name: String,
    pub hands: i64,
}

pub fn list_players(conn: &Connection) -> rusqlite::Result<Vec<PlayerRow>> {
    let mut stmt = conn.prepare(
        "SELECT p.id, p.name, COUNT(ph.id) as hands
         FROM players p
         JOIN player_hands ph ON ph.player_id = p.id
         GROUP BY p.id
         ORDER BY hands DESC, p.name ASC",
    )?;
    let rows = stmt
        .query_map([], |row| {
            Ok(PlayerRow {
                id: row.get(0)?,
                name: row.get(1)?,
                hands: row.get(2)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Players seated in the most recently *played* hand — i.e. the active
/// table, not the whole all-time roster. Ordered by `played_at` (falling
/// back to `id` only to break exact ties), not insertion order: a cold-start
/// backlog import walks the filesystem in OS directory order, not
/// chronological order, so the highest-`id` hand right after that import is
/// not reliably the most recently played one (see — this is what
/// let stale/mismatched rosters through briefly on cold start). The subquery
/// resolves the latest hand via `idx_hands_played_at` and only then joins
/// full hand counts for that handful of players, so this stays cheap even
/// with a large all-time `players`/`player_hands` table (import history
/// unrelated to the current table never enters the scan).
pub fn list_active_table_players(conn: &Connection) -> rusqlite::Result<Vec<PlayerRow>> {
    let mut stmt = conn.prepare(
        "SELECT p.id, p.name, COUNT(ph.id) as hands
         FROM players p
         JOIN player_hands ph ON ph.player_id = p.id
         WHERE p.id IN (
             SELECT player_id FROM player_hands
             WHERE hand_id = (SELECT id FROM hands ORDER BY played_at DESC, id DESC LIMIT 1)
         )
         GROUP BY p.id
         ORDER BY hands DESC, p.name ASC",
    )?;
    let rows = stmt
        .query_map([], |row| {
            Ok(PlayerRow {
                id: row.get(0)?,
                name: row.get(1)?,
                hands: row.get(2)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

pub fn count_hands(conn: &Connection) -> rusqlite::Result<i64> {
    conn.query_row("SELECT COUNT(*) FROM hands", [], |row| row.get(0))
}

/// A player's most recently played hand, used to display a "live-ish" chip
/// stack and BB depth. Not a live table read (no overlay/table connection
/// exists) — it's the stack/blind context from the last hand we imported for
/// this player.
pub struct PlayerSnapshot {
    pub starting_stack: f64,
    pub big_blind: f64,
    pub currency: String,
    pub format: String,
}

pub fn latest_player_snapshot(
    conn: &Connection,
    player_id: i64,
) -> rusqlite::Result<Option<PlayerSnapshot>> {
    conn.query_row(
        "SELECT ph.starting_stack, h.big_blind, h.currency, h.format
         FROM player_hands ph
         JOIN hands h ON h.id = ph.hand_id
         WHERE ph.player_id = ?1
         ORDER BY h.played_at DESC, h.id DESC
         LIMIT 1",
        params![player_id],
        |row| {
            Ok(PlayerSnapshot {
                starting_stack: row.get(0)?,
                big_blind: row.get(1)?,
                currency: row.get(2)?,
                format: row.get(3)?,
            })
        },
    )
    .optional()
}

pub fn get_player_color_override(
    conn: &Connection,
    player_id: i64,
) -> rusqlite::Result<Option<(String, Option<String>)>> {
    conn.query_row(
        "SELECT color, label FROM player_color_overrides WHERE player_id = ?1",
        params![player_id],
        |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?)),
    )
    .optional()
}

pub fn set_player_color_override(
    conn: &Connection,
    player_id: i64,
    color: &str,
    label: Option<&str>,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO player_color_overrides (player_id, color, label, updated_at)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(player_id) DO UPDATE SET color = excluded.color, label = excluded.label, updated_at = excluded.updated_at",
        params![player_id, color, label, now_iso()],
    )?;
    Ok(())
}

pub fn clear_player_color_override(conn: &Connection, player_id: i64) -> rusqlite::Result<()> {
    conn.execute(
        "DELETE FROM player_color_overrides WHERE player_id = ?1",
        params![player_id],
    )?;
    Ok(())
}

pub fn get_hud_position(conn: &Connection, player_id: i64) -> rusqlite::Result<Option<(f64, f64)>> {
    conn.query_row(
        "SELECT x, y FROM hud_positions WHERE player_id = ?1",
        params![player_id],
        |row| Ok((row.get::<_, f64>(0)?, row.get::<_, f64>(1)?)),
    )
    .optional()
}

pub fn set_hud_position(conn: &Connection, player_id: i64, x: f64, y: f64) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO hud_positions (player_id, x, y, updated_at) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(player_id) DO UPDATE SET x = excluded.x, y = excluded.y, updated_at = excluded.updated_at",
        params![player_id, x, y, now_iso()],
    )?;
    Ok(())
}
