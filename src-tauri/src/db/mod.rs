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
            table_name TEXT,
            game_type TEXT,
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

        CREATE INDEX IF NOT EXISTS idx_player_hands_player ON player_hands(player_id);
        CREATE INDEX IF NOT EXISTS idx_player_hands_hand ON player_hands(hand_id);
        CREATE INDEX IF NOT EXISTS idx_actions_hand ON actions(hand_id);
        CREATE INDEX IF NOT EXISTS idx_actions_player ON actions(player_id);
        "#,
    )
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

pub fn count_hands(conn: &Connection) -> rusqlite::Result<i64> {
    conn.query_row("SELECT COUNT(*) FROM hands", [], |row| row.get(0))
}
