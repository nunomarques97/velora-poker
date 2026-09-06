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

        CREATE TABLE IF NOT EXISTS player_notes (
            player_id INTEGER PRIMARY KEY REFERENCES players(id) ON DELETE CASCADE,
            note TEXT NOT NULL,
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

        CREATE TABLE IF NOT EXISTS seat_templates (
            max_players INTEGER NOT NULL,
            seat INTEGER NOT NULL,
            x REAL NOT NULL,
            y REAL NOT NULL,
            updated_at TEXT NOT NULL,
            PRIMARY KEY (max_players, seat)
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
    migrate_hud_positions_to_relative(conn)?;
    Ok(())
}

/// Phase E (table auto-detection): `hud_positions.x`/`y` changed
/// meaning from absolute screen pixels to fractions (0..1) of the overlay
/// window, which now tracks the PokerStars table window instead of sitting
/// at a fixed screen location. Any row saved before this migration holds a
/// pixel-scale value (typically in the hundreds) that is meaningless
/// reinterpreted as a fraction, so this one-time, idempotent step clears the
/// table exactly once — the user just re-drags cards under the new
/// tracking behavior, which is a one-time cost, not a recurring one. Guarded
/// by a settings flag so it never re-fires (and never touches positions
/// saved after the migration, which are already relative).
const HUD_POSITIONS_RELATIVE_MIGRATION_FLAG: &str = "hud_positions_migrated_to_relative";

fn migrate_hud_positions_to_relative(conn: &Connection) -> rusqlite::Result<()> {
    if get_setting(conn, HUD_POSITIONS_RELATIVE_MIGRATION_FLAG)?.is_some() {
        return Ok(());
    }
    conn.execute("DELETE FROM hud_positions", [])?;
    set_setting(conn, HUD_POSITIONS_RELATIVE_MIGRATION_FLAG, "true")?;
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

/// This player's seat number in whichever hand is currently the "active
/// table" (the same latest-hand subquery `list_active_table_players` uses).
pub struct ActiveTablePlayerRow {
    pub id: i64,
    pub name: String,
    pub hands: i64,
    pub seat: Option<i64>,
}

/// Same player set as `list_active_table_players`, plus each player's seat
/// in the current hand — needed to look up their seat-mapping template
/// (Phase E). A separate query rather than widening `PlayerRow`
/// everywhere: "seat" is only meaningful in the active-table context, not
/// the all-time roster `list_players` serves.
///
/// `table_name`:  multi-table fix. `None` keeps the pre- behavior
/// (latest hand *anywhere*) — used when no table window is currently
/// tracked (cold start, non-Windows, or the title didn't parse), so the
/// existing single-table experience is unchanged in that case. `Some(name)`
/// scopes "the active table" to the latest hand played specifically *at
/// that table*, so a hand completing on a different simultaneously-open
/// table (multi-tabling) can no longer flip which players' cards the
/// overlay renders out from under whichever table it's actually
/// positioned over — see the notes  for the confirmed root
/// cause. `(?1 IS NULL OR hands.table_name = ?1)` lets one query serve both
/// cases without duplicating the SQL.
pub fn list_active_table_players_with_seats(
    conn: &Connection,
    table_name: Option<&str>,
) -> rusqlite::Result<Vec<ActiveTablePlayerRow>> {
    let mut stmt = conn.prepare(
        "SELECT p.id, p.name, COUNT(ph.id) as hands, latest.seat
         FROM players p
         JOIN player_hands ph ON ph.player_id = p.id
         JOIN player_hands latest ON latest.player_id = p.id
             AND latest.hand_id = (
                 SELECT id FROM hands
                 WHERE (?1 IS NULL OR table_name = ?1)
                 ORDER BY played_at DESC, id DESC LIMIT 1
             )
         WHERE p.id IN (
             SELECT player_id FROM player_hands
             WHERE hand_id = (
                 SELECT id FROM hands
                 WHERE (?1 IS NULL OR table_name = ?1)
                 ORDER BY played_at DESC, id DESC LIMIT 1
             )
         )
         GROUP BY p.id
         ORDER BY hands DESC, p.name ASC",
    )?;
    let rows = stmt
        .query_map(params![table_name], |row| {
            Ok(ActiveTablePlayerRow {
                id: row.get(0)?,
                name: row.get(1)?,
                hands: row.get(2)?,
                seat: row.get(3)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// `max_seats` of the current active table (same latest-hand definition and
/// `table_name` scoping as `list_active_table_players_with_seats` —).
pub fn active_table_max_players(
    conn: &Connection,
    table_name: Option<&str>,
) -> rusqlite::Result<Option<i64>> {
    conn.query_row(
        "SELECT max_seats FROM hands
         WHERE (?1 IS NULL OR table_name = ?1)
         ORDER BY played_at DESC, id DESC LIMIT 1",
        params![table_name],
        |row| row.get(0),
    )
    .optional()
}

/// One row of the  diagnostics report's "recently imported hands" list —
/// each hand's own identity plus which table it belongs to, so a user
/// pasting the report shows whether recent hands are landing on the table
/// the overlay is scoped to or a different one (multi-tabling).
pub struct RecentHandRow {
    pub hand_id: String,
    pub table_name: Option<String>,
    pub tournament_id: Option<String>,
    pub format: String,
    pub played_at: Option<String>,
}

pub fn recent_hands(conn: &Connection, limit: i64) -> rusqlite::Result<Vec<RecentHandRow>> {
    let mut stmt = conn.prepare(
        "SELECT hand_id, table_name, tournament_id, format, played_at
         FROM hands
         ORDER BY played_at DESC, id DESC
         LIMIT ?1",
    )?;
    let rows = stmt
        .query_map(params![limit], |row| {
            Ok(RecentHandRow {
                hand_id: row.get(0)?,
                table_name: row.get(1)?,
                tournament_id: row.get(2)?,
                format: row.get(3)?,
                played_at: row.get(4)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// The active-table hand's own identity ( diagnostics) — same scoping
/// rule as `list_active_table_players_with_seats`, but returning the hand
/// itself rather than its players, so the report can show exactly which
/// hand/table/tournament Velora currently considers "active" and let the
/// user compare that against what he's actually looking at.
pub struct ActiveHandInfo {
    pub hand_id: String,
    pub table_name: Option<String>,
    pub tournament_id: Option<String>,
    pub played_at: Option<String>,
}

pub fn active_hand_info(
    conn: &Connection,
    table_name: Option<&str>,
) -> rusqlite::Result<Option<ActiveHandInfo>> {
    conn.query_row(
        "SELECT hand_id, table_name, tournament_id, played_at
         FROM hands
         WHERE (?1 IS NULL OR table_name = ?1)
         ORDER BY played_at DESC, id DESC LIMIT 1",
        params![table_name],
        |row| {
            Ok(ActiveHandInfo {
                hand_id: row.get(0)?,
                table_name: row.get(1)?,
                tournament_id: row.get(2)?,
                played_at: row.get(3)?,
            })
        },
    )
    .optional()
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

/// One free-text note per player (Phase E). Stored in its own table
/// rather than as a `players` column so the note is optional data hanging off
/// a player, exactly like `player_color_overrides` — a player with no note has
/// no row at all, and `ON DELETE CASCADE` cleans up with the player.
pub fn get_player_note(conn: &Connection, player_id: i64) -> rusqlite::Result<Option<String>> {
    conn.query_row(
        "SELECT note FROM player_notes WHERE player_id = ?1",
        params![player_id],
        |row| row.get::<_, String>(0),
    )
    .optional()
}

/// Upserts a player's note, overwriting any previous one (v1 is one note per
/// player, no history — see the spec's out-of-scope list). A blank note
/// deletes the row instead of storing an empty string, so "cleared" and
/// "never written" are the same state everywhere downstream.
pub fn set_player_note(conn: &Connection, player_id: i64, note: &str) -> rusqlite::Result<()> {
    let trimmed = note.trim();
    if trimmed.is_empty() {
        conn.execute(
            "DELETE FROM player_notes WHERE player_id = ?1",
            params![player_id],
        )?;
        return Ok(());
    }
    conn.execute(
        "INSERT INTO player_notes (player_id, note, updated_at) VALUES (?1, ?2, ?3)
         ON CONFLICT(player_id) DO UPDATE SET note = excluded.note, updated_at = excluded.updated_at",
        params![player_id, trimmed, now_iso()],
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

pub struct SeatTemplateRow {
    pub seat: i64,
    pub x: f64,
    pub y: f64,
}

/// One calibrated card position for a given seat, keyed by table size
/// (Phase E seat mapping) — set once per max-players count and reused
/// automatically on every future table of that size.
pub fn get_seat_template(
    conn: &Connection,
    max_players: i64,
    seat: i64,
) -> rusqlite::Result<Option<(f64, f64)>> {
    conn.query_row(
        "SELECT x, y FROM seat_templates WHERE max_players = ?1 AND seat = ?2",
        params![max_players, seat],
        |row| Ok((row.get::<_, f64>(0)?, row.get::<_, f64>(1)?)),
    )
    .optional()
}

pub fn list_seat_templates(
    conn: &Connection,
    max_players: i64,
) -> rusqlite::Result<Vec<SeatTemplateRow>> {
    let mut stmt = conn.prepare(
        "SELECT seat, x, y FROM seat_templates WHERE max_players = ?1 ORDER BY seat ASC",
    )?;
    let rows = stmt
        .query_map(params![max_players], |row| {
            Ok(SeatTemplateRow {
                seat: row.get(0)?,
                x: row.get(1)?,
                y: row.get(2)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

pub fn set_seat_template(
    conn: &Connection,
    max_players: i64,
    seat: i64,
    x: f64,
    y: f64,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO seat_templates (max_players, seat, x, y, updated_at) VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(max_players, seat) DO UPDATE SET x = excluded.x, y = excluded.y, updated_at = excluded.updated_at",
        params![max_players, seat, x, y, now_iso()],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_conn() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        init_schema(&conn).unwrap();
        conn
    }

    #[test]
    fn seat_template_lookup_is_keyed_by_max_players_and_seat() {
        let conn = test_conn();

        assert_eq!(get_seat_template(&conn, 6, 1).unwrap(), None);

        set_seat_template(&conn, 6, 1, 0.5, 0.9).unwrap();
        set_seat_template(&conn, 9, 1, 0.5, 0.95).unwrap();

        assert_eq!(get_seat_template(&conn, 6, 1).unwrap(), Some((0.5, 0.9)));
        assert_eq!(get_seat_template(&conn, 9, 1).unwrap(), Some((0.5, 0.95)));
        // A different seat at the same table size has no template yet.
        assert_eq!(get_seat_template(&conn, 6, 2).unwrap(), None);
        // A max-players count with no calibration at all returns nothing.
        assert_eq!(get_seat_template(&conn, 2, 1).unwrap(), None);
    }

    #[test]
    fn seat_template_upsert_overwrites_in_place() {
        let conn = test_conn();
        set_seat_template(&conn, 6, 3, 0.1, 0.2).unwrap();
        set_seat_template(&conn, 6, 3, 0.4, 0.6).unwrap();
        assert_eq!(get_seat_template(&conn, 6, 3).unwrap(), Some((0.4, 0.6)));
    }

    #[test]
    fn list_seat_templates_scopes_to_one_max_players_size() {
        let conn = test_conn();
        set_seat_template(&conn, 6, 1, 0.1, 0.1).unwrap();
        set_seat_template(&conn, 6, 2, 0.2, 0.2).unwrap();
        set_seat_template(&conn, 9, 1, 0.9, 0.9).unwrap();

        let six_max = list_seat_templates(&conn, 6).unwrap();
        assert_eq!(six_max.len(), 2);
        assert_eq!(six_max[0].seat, 1);
        assert_eq!(six_max[1].seat, 2);

        let nine_max = list_seat_templates(&conn, 9).unwrap();
        assert_eq!(nine_max.len(), 1);
        assert_eq!(nine_max[0].seat, 1);
    }

    fn insert_player(conn: &Connection, name: &str) -> i64 {
        get_or_create_player(conn, "pokerstars", name).unwrap()
    }

    #[test]
    fn player_note_round_trips_and_overwrites_in_place() {
        let conn = test_conn();
        let id = insert_player(&conn, "Villain");

        assert_eq!(get_player_note(&conn, id).unwrap(), None);

        set_player_note(&conn, id, "always overbets river").unwrap();
        assert_eq!(
            get_player_note(&conn, id).unwrap().as_deref(),
            Some("always overbets river")
        );

        // v1 keeps exactly one note per player — a second write replaces it
        // rather than accumulating history.
        set_player_note(&conn, id, "folds turn to any raise").unwrap();
        assert_eq!(
            get_player_note(&conn, id).unwrap().as_deref(),
            Some("folds turn to any raise")
        );
        let rows: i64 = conn
            .query_row("SELECT COUNT(*) FROM player_notes", [], |row| row.get(0))
            .unwrap();
        assert_eq!(rows, 1);
    }

    #[test]
    fn blank_player_note_clears_the_row_instead_of_storing_empty_text() {
        let conn = test_conn();
        let id = insert_player(&conn, "Villain");
        set_player_note(&conn, id, "  spews on the button  ").unwrap();
        // Surrounding whitespace is trimmed before storage.
        assert_eq!(
            get_player_note(&conn, id).unwrap().as_deref(),
            Some("spews on the button")
        );

        set_player_note(&conn, id, "   ").unwrap();
        assert_eq!(get_player_note(&conn, id).unwrap(), None);
        let rows: i64 = conn
            .query_row("SELECT COUNT(*) FROM player_notes", [], |row| row.get(0))
            .unwrap();
        assert_eq!(rows, 0);
    }

    #[test]
    fn player_notes_are_scoped_per_player() {
        let conn = test_conn();
        let a = insert_player(&conn, "Villain");
        let b = insert_player(&conn, "Robot");
        set_player_note(&conn, a, "note A").unwrap();

        assert_eq!(get_player_note(&conn, a).unwrap().as_deref(), Some("note A"));
        assert_eq!(get_player_note(&conn, b).unwrap(), None);
    }

    #[test]
    fn hud_positions_relative_migration_clears_stale_absolute_values_once() {
        let conn = test_conn();
        conn.execute(
            "INSERT INTO players (site, name, created_at) VALUES ('pokerstars', 'p1', ?1)",
            params![now_iso()],
        )
        .unwrap();
        // Simulate a pre-migration row holding an absolute pixel value.
        conn.execute(
            "INSERT INTO hud_positions (player_id, x, y, updated_at) VALUES (1, 420.0, 260.0, ?1)",
            params![now_iso()],
        )
        .unwrap();

        migrate_hud_positions_to_relative(&conn).unwrap();
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM hud_positions", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 0);

        // Re-running (e.g. next app startup) must not touch positions saved
        // afterward under the new relative scheme.
        set_hud_position(&conn, 1, 0.33, 0.5).unwrap();
        migrate_hud_positions_to_relative(&conn).unwrap();
        let (x, y) = get_hud_position(&conn, 1).unwrap().unwrap();
        assert_eq!((x, y), (0.33, 0.5));
    }
}
