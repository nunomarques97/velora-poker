use std::path::Path;

use rusqlite::{params, Connection, OptionalExtension};

pub fn now_iso() -> String {
    chrono::Utc::now().to_rfc3339()
}

/// A timestamp on the exact same clock and format as a parsed hand's
/// `played_at` — the local machine's wall clock, `%Y-%m-%dT%H:%M:%S`
/// (`parser::pokerstars::pad_time` builds `played_at` from PokerStars' own
/// header date/time with no timezone conversion; `sessions::PLAYED_AT_FORMAT`
/// and `sessions_today`'s `chrono::Local::now()` already rely on this same
/// assumption to compare "today" against it). Deliberately *not* `now_iso()`:
/// that is UTC with an offset suffix, a different clock and format, and a
/// lexicographic `played_at >= ?` comparison against it would be meaningless
/// — in any timezone behind UTC it would wrongly treat a hand played seconds
/// ago as older than "now". Used to stamp `TrackedTable::first_seen_at`,
/// the floor an "active hand for this table" query is bound by.
pub fn now_played_at() -> String {
    chrono::Local::now().format("%Y-%m-%dT%H:%M:%S").to_string()
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

        -- `seat_offset` is the seat *relative to the hero's own seat* in the
        -- hand, not PokerStars' absolute seat number. PokerStars'
        -- "Auto-Center me" rotates the table display so the hero is always at
        -- the same screen anchor, which means absolute seat N is not a fixed
        -- screen slot — only the offset from the hero is. Rows written before
        --  held absolute seats and are cleared once by
        -- `migrate_seat_templates_to_hero_relative`.
        CREATE TABLE IF NOT EXISTS seat_templates (
            max_players INTEGER NOT NULL,
            seat_offset INTEGER NOT NULL,
            x REAL NOT NULL,
            y REAL NOT NULL,
            updated_at TEXT NOT NULL,
            PRIMARY KEY (max_players, seat_offset)
        );

        -- Import-time integrity findings (work unit 2, requirement 5). One row
        -- per (hand, check) so a re-import of the same file replaces rather than
        -- accumulates. A rejected hand has no `hands` row, so this table is the
        -- only record that it was seen at all.
        CREATE TABLE IF NOT EXISTS import_problems (
            id INTEGER PRIMARY KEY,
            hand_id TEXT NOT NULL,
            severity TEXT NOT NULL,
            code TEXT NOT NULL,
            detail TEXT NOT NULL,
            detected_at TEXT NOT NULL,
            UNIQUE(hand_id, code)
        );

        CREATE INDEX IF NOT EXISTS idx_import_problems_code ON import_problems(code);
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
    migrate_seat_templates_to_hero_relative(conn)?;
    clamp_stored_positions(conn)?;
    repair_ingestion_integrity(conn)?;
    Ok(())
}

/// Work unit 2: reparse every stored hand and repair the rows the old parser
/// wrote wrongly.
///
/// Three defects, all measured in the notes, were
/// write-time corruption rather than read-time misinterpretation, so fixing the
/// parser alone would have left the existing database wrong forever:
///
/// * bounty-tournament seat lines never matched, so 26 of 275 hands (9.5%)
///   stored **no players at all**, stranding 226 action rows and creating 26
///   players that had never been seen to play a hand (§G.2);
/// * 28 seats that were never dealt in were stored as dealt-in players (§G.3);
/// * `player_hands.position` was NULL in all 1,498 rows because nothing ever
///   wrote it (§B.2).
///
/// All 275 hands retain their complete original text in `hands.raw_text`, so the
/// repair is a reparse rather than a re-read of files that may be gone.
///
/// # Idempotence
///
/// Guarded by a settings flag like the two migrations above, but the body is
/// also naturally re-runnable: it derives the correct row set for each hand from
/// `raw_text` and converges the stored rows onto it — deleting what should not
/// be there, inserting what is missing, updating the rest. Running it twice
/// changes nothing the second time, and it never touches the `hands` table, so
/// it cannot duplicate a hand.
const INGESTION_INTEGRITY_REPAIR_FLAG: &str = "ingestion_integrity_repaired_v1";

#[derive(Debug, Default, Clone, Copy)]
pub struct RepairReport {
    pub hands_examined: i64,
    pub hands_reparse_failed: i64,
    pub player_rows_inserted: i64,
    pub player_rows_deleted: i64,
    /// Rows whose position column was written this pass. Counts rows touched,
    /// not rows whose value changed — a re-run rewrites the same labels.
    pub player_rows_position_written: i64,
    pub phantom_players_deleted: i64,
}

fn repair_ingestion_integrity(conn: &Connection) -> rusqlite::Result<()> {
    if get_setting(conn, INGESTION_INTEGRITY_REPAIR_FLAG)?.is_some() {
        return Ok(());
    }
    run_ingestion_repair(conn)?;
    set_setting(conn, INGESTION_INTEGRITY_REPAIR_FLAG, "true")?;
    Ok(())
}

/// The repair itself, callable directly so tests and the maintenance command can
/// run it without clearing the flag.
pub fn run_ingestion_repair(conn: &Connection) -> rusqlite::Result<RepairReport> {
    let mut report = RepairReport::default();

    let rows: Vec<(i64, String, String)> = {
        let mut stmt =
            conn.prepare("SELECT id, site, COALESCE(raw_text, '') FROM hands ORDER BY id")?;
        let mapped = stmt.query_map([], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?))
        })?;
        mapped.collect::<Result<Vec<_>, _>>()?
    };

    for (hand_row_id, site, raw_text) in rows {
        report.hands_examined += 1;
        if raw_text.trim().is_empty() {
            report.hands_reparse_failed += 1;
            continue;
        }
        let hand = match crate::parser::parse_hand_block(&raw_text) {
            Ok(hand) => hand,
            Err(_) => {
                report.hands_reparse_failed += 1;
                continue;
            }
        };

        // The dealt-in player set this hand should have, keyed by player id.
        let mut wanted: std::collections::HashMap<i64, &crate::parser::ParsedSeat> =
            std::collections::HashMap::new();
        for seat in &hand.seats {
            let player_id = get_or_create_player(conn, &site, &seat.player_name)?;
            wanted.insert(player_id, seat);
        }

        let existing: Vec<i64> = {
            let mut stmt =
                conn.prepare("SELECT player_id FROM player_hands WHERE hand_id = ?1")?;
            let mapped = stmt.query_map(params![hand_row_id], |row| row.get::<_, i64>(0))?;
            mapped.collect::<Result<Vec<_>, _>>()?
        };

        // Remove rows for players who were never dealt into this hand.
        for player_id in &existing {
            if !wanted.contains_key(player_id) {
                report.player_rows_deleted += conn.execute(
                    "DELETE FROM player_hands WHERE hand_id = ?1 AND player_id = ?2",
                    params![hand_row_id, player_id],
                )? as i64;
            }
        }

        for (player_id, seat) in &wanted {
            let is_hero = hand.hero_name.as_deref() == Some(seat.player_name.as_str());
            let result = hand.results.get(&seat.player_name).cloned().unwrap_or_default();

            if existing.contains(player_id) {
                let changed = conn.execute(
                    "UPDATE player_hands
                        SET seat = ?3, starting_stack = ?4, position = ?5, is_hero = ?6,
                            went_to_showdown = ?7, won_at_showdown = ?8, net_result = ?9
                      WHERE hand_id = ?1 AND player_id = ?2",
                    params![
                        hand_row_id,
                        player_id,
                        seat.seat_number,
                        seat.starting_stack,
                        seat.position,
                        is_hero as i64,
                        result.went_to_showdown as i64,
                        result.won_at_showdown as i64,
                        result.net_result,
                    ],
                )?;
                if changed > 0 && seat.position.is_some() {
                    report.player_rows_position_written += 1;
                }
            } else {
                conn.execute(
                    "INSERT INTO player_hands (hand_id, player_id, seat, starting_stack, position, is_hero, went_to_showdown, won_at_showdown, net_result)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                    params![
                        hand_row_id,
                        player_id,
                        seat.seat_number,
                        seat.starting_stack,
                        seat.position,
                        is_hero as i64,
                        result.went_to_showdown as i64,
                        result.won_at_showdown as i64,
                        result.net_result,
                    ],
                )?;
                report.player_rows_inserted += 1;
                if seat.position.is_some() {
                    report.player_rows_position_written += 1;
                }
            }
        }
    }

    // A player left with no hand rows and no action rows was only ever created
    // as a side effect of a hand that stored nobody. Deleting one cannot cascade
    // anything away, because the `actions.player_id` foreign key has nothing to
    // cascade to — that is exactly what "and no action rows" establishes.
    report.phantom_players_deleted = conn.execute(
        "DELETE FROM players
          WHERE NOT EXISTS (SELECT 1 FROM player_hands ph WHERE ph.player_id = players.id)
            AND NOT EXISTS (SELECT 1 FROM actions a WHERE a.player_id = players.id)",
        [],
    )? as i64;

    Ok(report)
}

/// Pulls any already-stored card position back inside the 0..1 fraction range
/// (, a known issue).
///
/// One real `hud_positions` row held `x = 1.104`, dated before the clamp in
/// `set_hud_position` existed — enough to park that player's card off the
/// right edge of the overlay, where it cannot even be dragged back. Unlike the
/// two migrations above this is not flag-guarded and does not clear anything:
/// it runs on every startup, is a no-op when every row is already in range,
/// and repairs rather than discards, because the position it fixes is still
/// approximately where the user put it.
fn clamp_stored_positions(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE hud_positions SET x = max(0.0, min(1.0, x)), y = max(0.0, min(1.0, y))
         WHERE x < 0.0 OR x > 1.0 OR y < 0.0 OR y > 1.0",
        [],
    )?;
    conn.execute(
        "UPDATE seat_templates SET x = max(0.0, min(1.0, x)), y = max(0.0, min(1.0, y))
         WHERE x < 0.0 OR x > 1.0 OR y < 0.0 OR y > 1.0",
        [],
    )?;
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

/// `seat_templates` changed its key from PokerStars'
/// absolute seat number to the seat's offset from the hero's own seat. The
/// old key was simply wrong: "Auto-Center me" rotates the table display so
/// the hero is always the fixed screen anchor, so absolute seat N is not a
/// stable screen slot — confirmed live, with Auto-Center correctly
/// configured the user's own card still landed in the wrong slot.
///
/// Rows written under the old key hold a number that is meaningless as an
/// offset, and unlike a stale row that simply never matches, one of these
/// *does* match — it would place cards at confidently wrong positions the
/// moment the overlay opens, which is the opposite of 's requirement that
/// opening the overlay just works. So this clears the table exactly once,
/// the same one-time, settings-flag-guarded shape
/// `migrate_hud_positions_to_relative` uses for the same reason (a column's
/// meaning changed, not its type). The user re-drags one layout per table
/// size, a one-time cost.
const SEAT_TEMPLATES_HERO_RELATIVE_MIGRATION_FLAG: &str = "seat_templates_migrated_to_hero_relative";

fn migrate_seat_templates_to_hero_relative(conn: &Connection) -> rusqlite::Result<()> {
    rename_column_if_present(conn, "seat_templates", "seat", "seat_offset")?;
    if get_setting(conn, SEAT_TEMPLATES_HERO_RELATIVE_MIGRATION_FLAG)?.is_some() {
        return Ok(());
    }
    conn.execute("DELETE FROM seat_templates", [])?;
    set_setting(conn, SEAT_TEMPLATES_HERO_RELATIVE_MIGRATION_FLAG, "true")?;
    Ok(())
}

/// The 6-max card layout a brand-new database starts with, as
/// `(max_players, seat_offset, x, y)` — hero-relative seat offsets and
/// fractions of the overlay window, exactly like any row the user drags into
/// place (see `seat_templates` in `init_schema` and).
///
/// Measured, not derived: these are the user's own calibrated rows, read
/// out of his local database after he dragged one 6-max layout by hand.
/// Before this existed, `seat_templates` started empty on every install and
/// the first table showed six cards stacked in `OverlayApp`'s fallback grid
/// in the top-left corner until the user dragged each one out — a first-run
/// experience nobody but the user could have fixed for themselves.
///
/// Only 6-max ships. The other table sizes have no measured layout behind
/// them, and an evenly-spaced ellipse fitted to these six points misses the
/// real side seats by 4-7% of the window width (~40px on the ~800px table
/// this was calibrated at), so those sizes deliberately keep starting empty
/// rather than start confidently wrong.
///
/// Two known limits, both fixed the same way — by dragging, which still
/// overrides any default:
/// - The values only apply when PokerStars' "Auto-Center me" is on, since
///   that is what makes a hero-relative offset a fixed screen slot;
///   `OverlayApp` already gates seat templates on the `auto_center_enabled`
///   setting, so nothing here changes when it is off.
/// - The positions are fractions of the window but the cards themselves are
///   a fixed pixel width, so a table window much wider or narrower than the
///   one this was calibrated on drifts progressively off-centre.
const BUILTIN_SEAT_TEMPLATES: &[(i64, i64, f64, f64)] = &[
    (6, 0, 0.38025864, 0.68320590),
    (6, 1, 0.01080431, 0.49984758),
    (6, 2, 0.02123839, 0.20103448),
    (6, 3, 0.36757288, 0.13304348),
    (6, 4, 0.71559598, 0.21716392),
    (6, 5, 0.71186081, 0.51867566),
];

/// Seeds the built-in seat layout. `INSERT OR IGNORE` on the
/// `(max_players, seat_offset)` primary key, so a user who has dragged that
/// seat keeps their own position forever and a user who has not gets the
/// default back if they somehow clear it — the same never-overwrite shape
/// `seed_builtin_rules`/`seed_builtin_profiles` use for their own rows.
///
/// Depends on running *after* `migrate_schema`, which `open` guarantees:
/// `migrate_seat_templates_to_hero_relative` clears `seat_templates` exactly
/// once on a database that has never been migrated — including a brand-new
/// one — and seeding before that would hand every fresh install an empty
/// table again.
fn seed_builtin_seat_templates(conn: &Connection) -> rusqlite::Result<()> {
    for (max_players, seat_offset, x, y) in BUILTIN_SEAT_TEMPLATES {
        conn.execute(
            "INSERT OR IGNORE INTO seat_templates (max_players, seat_offset, x, y, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![max_players, seat_offset, x, y, now_iso()],
        )?;
    }
    Ok(())
}

/// Seeds built-in classification rules, HUD profiles and seat templates the
/// first time a database is opened. Uses fixed ids and `INSERT OR IGNORE` so
/// it never overwrites a user's own edits to these rows, and is safe to call
/// on every startup.
pub fn seed_defaults(conn: &Connection) -> rusqlite::Result<()> {
    crate::classification::seed_builtin_rules(conn)?;
    crate::hud::seed_builtin_profiles(conn)?;
    seed_builtin_seat_templates(conn)?;
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

/// Renames a column only if the old name is still there, so this is safe to
/// run on every startup: a database created after the rename already has the
/// new name and is left alone. SQLite rewrites the stored schema text, so a
/// column named in a PRIMARY KEY clause comes along correctly.
fn rename_column_if_present(
    conn: &Connection,
    table: &str,
    from: &str,
    to: &str,
) -> rusqlite::Result<()> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let existing: Vec<String> = stmt
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<Result<_, _>>()?;

    if existing.iter().any(|c| c == from) && !existing.iter().any(|c| c == to) {
        conn.execute(
            &format!("ALTER TABLE {table} RENAME COLUMN {from} TO {to}"),
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

/// Records what the import-time integrity gate found for one hand.
///
/// Keyed `UNIQUE(hand_id, code)` and written with `INSERT OR REPLACE`, so
/// re-importing the same file — which the watcher does routinely, since
/// PokerStars rewrites a file as the session goes on — refreshes each finding
/// instead of stacking duplicates.
pub fn record_import_problems(
    conn: &Connection,
    hand_id: &str,
    problems: &[crate::import::validate::Problem],
) -> rusqlite::Result<()> {
    let now = now_iso();
    for problem in problems {
        let severity = match problem.severity {
            crate::import::validate::Severity::Reject => "reject",
            crate::import::validate::Severity::Warn => "warn",
        };
        conn.execute(
            "INSERT OR REPLACE INTO import_problems (hand_id, severity, code, detail, detected_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![hand_id, severity, problem.code, problem.detail, now],
        )?;
    }
    Ok(())
}

/// `(severity, code, count, most recent detail)` for every distinct finding,
/// worst first. Drives the Settings → Diagnostics integrity section.
pub fn import_problem_summary(
    conn: &Connection,
) -> rusqlite::Result<Vec<(String, String, i64, String)>> {
    let mut stmt = conn.prepare(
        "SELECT severity, code, COUNT(*) AS n, MAX(detail)
           FROM import_problems
          GROUP BY severity, code
          ORDER BY CASE severity WHEN 'reject' THEN 0 ELSE 1 END, n DESC",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
    })?;
    rows.collect::<Result<Vec<_>, _>>()
}

/// Live integrity counters, recomputed from the data rather than remembered, so
/// the diagnostics report can never claim a clean database that isn't.
/// Returns `(hands_with_no_players, orphan_actions, phantom_players,
/// player_rows_without_position)`.
pub fn integrity_counters(conn: &Connection) -> rusqlite::Result<(i64, i64, i64, i64)> {
    let hands_with_no_players: i64 = conn.query_row(
        "SELECT COUNT(*) FROM hands h WHERE NOT EXISTS (SELECT 1 FROM player_hands ph WHERE ph.hand_id = h.id)",
        [],
        |r| r.get(0),
    )?;
    let orphan_actions: i64 = conn.query_row(
        "SELECT COUNT(*) FROM actions a WHERE NOT EXISTS
           (SELECT 1 FROM player_hands ph WHERE ph.hand_id = a.hand_id AND ph.player_id = a.player_id)",
        [],
        |r| r.get(0),
    )?;
    let phantom_players: i64 = conn.query_row(
        "SELECT COUNT(*) FROM players p WHERE NOT EXISTS (SELECT 1 FROM player_hands ph WHERE ph.player_id = p.id)",
        [],
        |r| r.get(0),
    )?;
    let rows_without_position: i64 = conn.query_row(
        "SELECT COUNT(*) FROM player_hands WHERE position IS NULL",
        [],
        |r| r.get(0),
    )?;
    Ok((hands_with_no_players, orphan_actions, phantom_players, rows_without_position))
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

/// Total tracked-player count, optionally restricted to names containing
/// `name_filter` (case-insensitive substring, SQLite `LIKE` semantics). Pairs
/// with [`list_players_page`] so a caller can render "showing N of TOTAL"
/// without pulling every row.
pub fn count_players(conn: &Connection, name_filter: Option<&str>) -> rusqlite::Result<i64> {
    match name_filter {
        Some(filter) => conn.query_row(
            "SELECT COUNT(*) FROM players WHERE name LIKE '%' || ?1 || '%' ESCAPE '\\'",
            params![escape_like(filter)],
            |row| row.get(0),
        ),
        None => conn.query_row("SELECT COUNT(*) FROM players", [], |row| row.get(0)),
    }
}

/// A bounded, ordered slice of [`list_players`] — same ordering (most-played
/// first), but only `limit` rows starting at `offset`, and only players whose
/// name contains `name_filter` when given. Exists so a caller with a huge
/// `players` table (a bulk import can put this in the thousands) can render a
/// page without paying for `build_player_payload`'s per-player stats queries
/// on every row in the table.
pub fn list_players_page(
    conn: &Connection,
    offset: i64,
    limit: i64,
    name_filter: Option<&str>,
) -> rusqlite::Result<Vec<PlayerRow>> {
    let sql = "SELECT p.id, p.name, COUNT(ph.id) as hands
         FROM players p
         JOIN player_hands ph ON ph.player_id = p.id
         WHERE (?1 IS NULL OR p.name LIKE '%' || ?1 || '%' ESCAPE '\\')
         GROUP BY p.id
         ORDER BY hands DESC, p.name ASC
         LIMIT ?2 OFFSET ?3";
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt
        .query_map(
            params![name_filter.map(escape_like), limit, offset],
            |row| {
                Ok(PlayerRow {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    hands: row.get(2)?,
                })
            },
        )?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Escapes `%`, `_` and `\` so a user-typed search term is matched literally
/// inside a `LIKE '%...%' ESCAPE '\\'` pattern instead of being interpreted
/// as SQL wildcards.
fn escape_like(term: &str) -> String {
    term.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_")
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

/// This player's seat in whichever hand is currently the "active table" (the
/// same latest-hand subquery `list_active_table_players` uses).
pub struct ActiveTablePlayerRow {
    pub id: i64,
    pub name: String,
    pub hands: i64,
    /// PokerStars' absolute seat number in that hand.
    pub seat: Option<i64>,
    /// Whether this row is the hero (the user's own player), from
    /// `player_hands.is_hero` for that same hand.
    pub is_hero: bool,
    /// The seat rotated so the hero is the fixed anchor — the key HUD card
    /// positions are actually stored under. `None` when the hand's
    /// hero or `max_seats` is unknown, in which case no seat template can be
    /// resolved and the card falls back to its manual position.
    pub seat_offset: Option<i64>,
}

/// Rotates an absolute PokerStars seat number into the hero-relative offset
/// HUD card positions are keyed by.
///
/// PokerStars' "Auto-Center me" table option rotates the *display* so the
/// hero always sits at the same screen anchor. It does not mean absolute seat
/// N is always screen slot N — everyone else's screen slot depends on how far
/// they sit from the hero, so that distance is the only stable key. The hero
/// is always offset 0 by construction.
///
/// Returns `None` when the rotation is undefined: no hero in the hand, or a
/// hand with no recorded `max_seats` (nothing to take the modulus by).
pub fn relative_seat(seat: i64, hero_seat: i64, max_seats: i64) -> Option<i64> {
    if max_seats <= 0 {
        return None;
    }
    Some((seat - hero_seat).rem_euclid(max_seats))
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
///
/// `since`:  fix. PokerStars reuses table *names* from a pool, so
/// `table_name = ?1` alone can resolve to a hand from an earlier sitting of
/// the same name — hours or days old, indistinguishable from a fresh one.
/// `Some(table.first_seen_at)` (on the same clock as `played_at`, see
/// `db::now_played_at`) excludes any hand older than the table itself; a
/// match that only exists before that floor resolves the same way "no hand
/// imported yet" already does — an empty roster, never another sitting's.
/// `None` for a caller with no specific table to bound by.
pub fn list_active_table_players_with_seats(
    conn: &Connection,
    table_name: Option<&str>,
    since: Option<&str>,
) -> rusqlite::Result<Vec<ActiveTablePlayerRow>> {
    let mut stmt = conn.prepare(
        "SELECT p.id, p.name, COUNT(ph.id) as hands, latest.seat, latest.is_hero
         FROM players p
         JOIN player_hands ph ON ph.player_id = p.id
         JOIN player_hands latest ON latest.player_id = p.id
             AND latest.hand_id = (
                 SELECT id FROM hands
                 WHERE (?1 IS NULL OR table_name = ?1)
                   AND (?2 IS NULL OR played_at >= ?2)
                 ORDER BY played_at DESC, id DESC LIMIT 1
             )
         WHERE p.id IN (
             SELECT player_id FROM player_hands
             WHERE hand_id = (
                 SELECT id FROM hands
                 WHERE (?1 IS NULL OR table_name = ?1)
                   AND (?2 IS NULL OR played_at >= ?2)
                 ORDER BY played_at DESC, id DESC LIMIT 1
             )
         )
         GROUP BY p.id
         ORDER BY hands DESC, p.name ASC",
    )?;
    let mut rows = stmt
        .query_map(params![table_name, since], |row| {
            Ok(ActiveTablePlayerRow {
                id: row.get(0)?,
                name: row.get(1)?,
                hands: row.get(2)?,
                seat: row.get(3)?,
                is_hero: row.get::<_, i64>(4)? != 0,
                seat_offset: None,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

    // resolved here rather than by the caller so there is exactly one
    // place that knows how a stored HUD position is keyed. Both inputs come
    // from the same hand the seats above came from.
    let max_seats = active_table_max_players(conn, table_name)?;
    let hero_seat = rows.iter().find(|r| r.is_hero).and_then(|r| r.seat);
    if let (Some(hero_seat), Some(max_seats)) = (hero_seat, max_seats) {
        for row in &mut rows {
            row.seat_offset = row
                .seat
                .and_then(|seat| relative_seat(seat, hero_seat, max_seats));
        }
    }

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

/// `since`: same  recency floor as `list_active_table_players_with_seats`
/// — `Some(table.first_seen_at)` excludes a hand from an earlier sitting of a
/// reused table name; `None` for a caller with no specific table to bound by.
pub fn active_hand_info(
    conn: &Connection,
    table_name: Option<&str>,
    since: Option<&str>,
) -> rusqlite::Result<Option<ActiveHandInfo>> {
    conn.query_row(
        "SELECT hand_id, table_name, tournament_id, played_at
         FROM hands
         WHERE (?1 IS NULL OR table_name = ?1)
           AND (?2 IS NULL OR played_at >= ?2)
         ORDER BY played_at DESC, id DESC LIMIT 1",
        params![table_name, since],
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

/// Forces a stored card position into the 0..1 fraction range every reader
/// assumes (, a known issue).
///
/// A real `hud_positions` row had drifted to `x = 1.104`, which would place
/// that player's card past the right edge of every overlay it ever appeared
/// on — invisible, and impossible to drag back because there is nothing left
/// on screen to grab. A drag can legitimately end past the window edge (the
/// pointer is captured, so it keeps reporting), so the fix belongs on write
/// rather than in the drag handler: whatever the gesture ends up saying, what
/// gets stored is a position that is still on the overlay.
///
/// `NaN` clamps to 0.0 rather than propagating: `f64::clamp` panics on a NaN
/// bound and `min`/`max` would carry it into the database.
fn clamp_fraction(value: f64) -> f64 {
    if value.is_nan() {
        0.0
    } else {
        value.clamp(0.0, 1.0)
    }
}

pub fn set_hud_position(conn: &Connection, player_id: i64, x: f64, y: f64) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO hud_positions (player_id, x, y, updated_at) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(player_id) DO UPDATE SET x = excluded.x, y = excluded.y, updated_at = excluded.updated_at",
        params![player_id, clamp_fraction(x), clamp_fraction(y), now_iso()],
    )?;
    Ok(())
}

pub struct SeatTemplateRow {
    pub seat_offset: i64,
    pub x: f64,
    pub y: f64,
}

/// One calibrated card position for a given *hero-relative* seat, keyed by
/// table size (Phase E seat mapping, ; re-keyed in) — set once per
/// max-players count and reused automatically on every future table of that
/// size. See `relative_seat` for why the key is an offset and not
/// PokerStars' absolute seat number.
pub fn get_seat_template(
    conn: &Connection,
    max_players: i64,
    seat_offset: i64,
) -> rusqlite::Result<Option<(f64, f64)>> {
    conn.query_row(
        "SELECT x, y FROM seat_templates WHERE max_players = ?1 AND seat_offset = ?2",
        params![max_players, seat_offset],
        |row| Ok((row.get::<_, f64>(0)?, row.get::<_, f64>(1)?)),
    )
    .optional()
}

pub fn list_seat_templates(
    conn: &Connection,
    max_players: i64,
) -> rusqlite::Result<Vec<SeatTemplateRow>> {
    let mut stmt = conn.prepare(
        "SELECT seat_offset, x, y FROM seat_templates WHERE max_players = ?1 ORDER BY seat_offset ASC",
    )?;
    let rows = stmt
        .query_map(params![max_players], |row| {
            Ok(SeatTemplateRow {
                seat_offset: row.get(0)?,
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
    seat_offset: i64,
    x: f64,
    y: f64,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO seat_templates (max_players, seat_offset, x, y, updated_at) VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(max_players, seat_offset) DO UPDATE SET x = excluded.x, y = excluded.y, updated_at = excluded.updated_at",
        params![
            max_players,
            seat_offset,
            clamp_fraction(x),
            clamp_fraction(y),
            now_iso()
        ],
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
        assert_eq!(six_max[0].seat_offset, 1);
        assert_eq!(six_max[1].seat_offset, 2);

        let nine_max = list_seat_templates(&conn, 9).unwrap();
        assert_eq!(nine_max.len(), 1);
        assert_eq!(nine_max[0].seat_offset, 1);
    }

    #[test]
    fn a_fresh_database_starts_with_the_builtin_six_max_layout() {
        let conn = test_conn();
        seed_builtin_seat_templates(&conn).unwrap();

        let six_max = list_seat_templates(&conn, 6).unwrap();
        assert_eq!(six_max.len(), 6);
        // Every hero-relative offset a 6-max table can produce is covered;
        // one missing offset would leave that seat's card in the top-left
        // fallback grid while the other five sit correctly, which is the
        // failure this feature exists to prevent.
        let offsets: Vec<i64> = six_max.iter().map(|row| row.seat_offset).collect();
        assert_eq!(offsets, vec![0, 1, 2, 3, 4, 5]);
        assert_eq!(get_seat_template(&conn, 6, 0).unwrap(), Some((0.38025864, 0.68320590)));
        assert_eq!(get_seat_template(&conn, 6, 3).unwrap(), Some((0.36757288, 0.13304348)));
    }

    #[test]
    fn seeding_never_overwrites_a_position_the_user_dragged() {
        let conn = test_conn();
        seed_builtin_seat_templates(&conn).unwrap();

        set_seat_template(&conn, 6, 2, 0.44, 0.55).unwrap();
        // Seeding runs on *every* startup, not just the first, so the
        // user's own calibration has to survive it unchanged.
        seed_builtin_seat_templates(&conn).unwrap();

        assert_eq!(get_seat_template(&conn, 6, 2).unwrap(), Some((0.44, 0.55)));
        assert_eq!(get_seat_template(&conn, 6, 3).unwrap(), Some((0.36757288, 0.13304348)));
    }

    #[test]
    fn no_table_size_other_than_six_max_is_seeded() {
        let conn = test_conn();
        seed_builtin_seat_templates(&conn).unwrap();

        // Only 6-max was measured live. Every other size must still start
        // empty rather than inherit a guessed layout.
        for max_players in [2, 3, 4, 5, 7, 8, 9, 10] {
            assert!(
                list_seat_templates(&conn, max_players).unwrap().is_empty(),
                "{max_players}-max must not be seeded"
            );
        }
    }

    #[test]
    fn opening_a_brand_new_database_leaves_the_builtin_layout_in_place() {
        // Ordering regression: `migrate_seat_templates_to_hero_relative`
        // clears `seat_templates` once on any database that has never been
        // migrated, a brand-new one included. If seeding ever moved ahead of
        // the migrations, every fresh install would silently get the empty
        // table this feature replaced — and the unit tests above, which seed
        // a bare schema directly, would all still pass.
        let dir = tempfile::tempdir().expect("temp dir");
        let conn = open(&dir.path().join("velora.db")).unwrap();

        assert_eq!(list_seat_templates(&conn, 6).unwrap().len(), 6);
        assert_eq!(get_seat_template(&conn, 6, 0).unwrap(), Some((0.38025864, 0.68320590)));
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

    /// one real row had drifted to `x = 1.104`, which
    /// parks that player's card past the right edge of every overlay it ever
    /// appears on — invisible, and impossible to drag back because there is
    /// nothing left on screen to grab.
    #[test]
    fn a_saved_card_position_can_never_leave_the_overlay() {
        let conn = test_conn();
        let player = insert_player(&conn, "Villain");

        set_hud_position(&conn, player, 1.104, -0.3).unwrap();
        assert_eq!(get_hud_position(&conn, player).unwrap(), Some((1.0, 0.0)));

        set_hud_position(&conn, player, f64::NAN, 0.5).unwrap();
        assert_eq!(
            get_hud_position(&conn, player).unwrap(),
            Some((0.0, 0.5)),
            "a NaN must land somewhere on the overlay, not be stored as NaN"
        );

        // An in-range position is stored exactly as given.
        set_hud_position(&conn, player, 0.25, 0.75).unwrap();
        assert_eq!(get_hud_position(&conn, player).unwrap(), Some((0.25, 0.75)));
    }

    #[test]
    fn a_saved_seat_template_can_never_leave_the_overlay() {
        let conn = test_conn();
        set_seat_template(&conn, 6, 2, 1.4, -2.0).unwrap();
        assert_eq!(get_seat_template(&conn, 6, 2).unwrap(), Some((1.0, 0.0)));

        set_seat_template(&conn, 6, 3, 0.4, 0.9).unwrap();
        assert_eq!(get_seat_template(&conn, 6, 3).unwrap(), Some((0.4, 0.9)));
    }

    /// The clamp above only protects new writes. This is the repair pass for
    /// the row that is already out of range in a real database — it runs on
    /// every startup, and unlike the two one-shot migrations it fixes the
    /// value rather than discarding the row, because the position is still
    /// roughly where the user put it.
    #[test]
    fn startup_pulls_already_stored_positions_back_onto_the_overlay() {
        let conn = test_conn();
        insert_player(&conn, "Villain");
        conn.execute(
            "INSERT INTO hud_positions (player_id, x, y, updated_at) VALUES (1, 1.104, 0.42, ?1)",
            params![now_iso()],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO seat_templates (max_players, seat_offset, x, y, updated_at)
             VALUES (6, 1, -0.2, 1.9, ?1)",
            params![now_iso()],
        )
        .unwrap();

        clamp_stored_positions(&conn).unwrap();

        assert_eq!(get_hud_position(&conn, 1).unwrap(), Some((1.0, 0.42)));
        assert_eq!(get_seat_template(&conn, 6, 1).unwrap(), Some((0.0, 1.0)));
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

    // -----------------------------------------------------------------
    // seat templates are keyed by the hero-relative seat
    // -----------------------------------------------------------------

    #[test]
    fn relative_seat_puts_the_hero_at_zero_and_wraps_the_rest_of_the_table() {
        // Hero in seat 4 of a 6-max table: everyone is measured clockwise
        // from him, wrapping past the highest seat back to seat 1.
        assert_eq!(relative_seat(4, 4, 6), Some(0));
        assert_eq!(relative_seat(5, 4, 6), Some(1));
        assert_eq!(relative_seat(6, 4, 6), Some(2));
        assert_eq!(relative_seat(1, 4, 6), Some(3));
        assert_eq!(relative_seat(3, 4, 6), Some(5));
    }

    #[test]
    fn relative_seat_is_undefined_without_a_table_size() {
        assert_eq!(relative_seat(3, 1, 0), None);
        assert_eq!(relative_seat(3, 1, -6), None);
    }

    /// Inserts one synthetic hand: `seats` is (player name, absolute seat),
    /// and `hero` names which of them is the user.
    fn insert_hand(
        conn: &Connection,
        hand_id: &str,
        table_name: &str,
        played_at: &str,
        max_seats: i64,
        hero: &str,
        seats: &[(&str, i64)],
    ) {
        conn.execute(
            "INSERT INTO hands (site, hand_id, format, table_name, max_seats, played_at, imported_at)
             VALUES ('pokerstars', ?1, 'cash', ?2, ?3, ?4, ?5)",
            params![hand_id, table_name, max_seats, played_at, now_iso()],
        )
        .unwrap();
        let db_hand_id = conn.last_insert_rowid();
        for (name, seat) in seats {
            let player_id = insert_player(conn, name);
            conn.execute(
                "INSERT INTO player_hands (hand_id, player_id, seat, is_hero) VALUES (?1, ?2, ?3, ?4)",
                params![db_hand_id, player_id, seat, (*name == hero) as i64],
            )
            .unwrap();
        }
    }

    /// The  regression: the same physical table arrangement must resolve
    /// to the same layout regardless of which absolute seat the hero happens
    /// to be dealt into. Two hands, same 6-max size, hero two seats apart,
    /// everyone else keeping the same distance from him.
    #[test]
    fn seat_offsets_are_identical_when_the_hero_moves_absolute_seats() {
        let conn = test_conn();

        insert_hand(
            &conn,
            "HAND-A",
            "Aegle IV",
            "2026-09-06T10:00:00Z",
            6,
            "Hero",
            &[("Hero", 1), ("Villain", 2), ("Robot", 3)],
        );
        insert_hand(
            &conn,
            "HAND-B",
            "Orion II",
            "2026-09-06T11:00:00Z",
            6,
            "Hero",
            &[("Hero", 4), ("Villain", 5), ("Robot", 6)],
        );

        let layout = |table: &str| {
            let mut resolved: Vec<(String, Option<i64>)> =
                list_active_table_players_with_seats(&conn, Some(table), None)
                    .unwrap()
                    .into_iter()
                    .map(|r| (r.name, r.seat_offset))
                    .collect();
            resolved.sort();
            resolved
        };

        let from_seat_one = layout("Aegle IV");
        let from_seat_four = layout("Orion II");

        assert_eq!(
            from_seat_one,
            vec![
                ("Hero".to_string(), Some(0)),
                ("Robot".to_string(), Some(2)),
                ("Villain".to_string(), Some(1)),
            ]
        );
        assert_eq!(
            from_seat_one, from_seat_four,
            "the same arrangement must resolve to the same layout no matter which absolute seat the hero holds"
        );
    }

    /// Wrapping case: the hero sits at the highest seat, so his neighbours'
    /// absolute seat numbers run backwards past seat 1.
    #[test]
    fn seat_offsets_wrap_around_the_table_when_the_hero_sits_last() {
        let conn = test_conn();
        insert_hand(
            &conn,
            "HAND-C",
            "Wrap",
            "2026-09-06T12:00:00Z",
            6,
            "Hero",
            &[("Hero", 6), ("Villain", 1), ("Robot", 2)],
        );

        let mut resolved: Vec<(String, Option<i64>)> =
            list_active_table_players_with_seats(&conn, Some("Wrap"), None)
                .unwrap()
                .into_iter()
                .map(|r| (r.name, r.seat_offset))
                .collect();
        resolved.sort();
        assert_eq!(
            resolved,
            vec![
                ("Hero".to_string(), Some(0)),
                ("Robot".to_string(), Some(2)),
                ("Villain".to_string(), Some(1)),
            ]
        );
    }

    /// A hand Velora never saw the hero in (an observed table) has no anchor
    /// to rotate around, so no seat template may be resolved — the cards fall
    /// back to their manual positions rather than being placed confidently
    /// wrong.
    #[test]
    fn seat_offsets_are_absent_when_the_hand_has_no_hero() {
        let conn = test_conn();
        insert_hand(
            &conn,
            "HAND-D",
            "Observed",
            "2026-09-06T13:00:00Z",
            6,
            "nobody",
            &[("Villain", 2), ("Robot", 3)],
        );

        let rows = list_active_table_players_with_seats(&conn, Some("Observed"), None).unwrap();
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().all(|r| r.seat_offset.is_none()));
        assert!(rows.iter().all(|r| !r.is_hero));
    }

    #[test]
    fn seat_templates_hero_relative_migration_renames_the_column_and_clears_stale_rows() {
        let conn = Connection::open_in_memory().unwrap();
        // A pre- database: the old column name, holding absolute seats.
        conn.execute_batch(
            "CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL, updated_at TEXT NOT NULL);
             CREATE TABLE seat_templates (
                max_players INTEGER NOT NULL,
                seat INTEGER NOT NULL,
                x REAL NOT NULL,
                y REAL NOT NULL,
                updated_at TEXT NOT NULL,
                PRIMARY KEY (max_players, seat)
             );",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO seat_templates (max_players, seat, x, y, updated_at) VALUES (6, 3, 0.4, 0.7, ?1)",
            params![now_iso()],
        )
        .unwrap();

        migrate_seat_templates_to_hero_relative(&conn).unwrap();

        // The column is renamed in place, and the row calibrated against the
        // wrong key is gone rather than left to place a card confidently
        // wrong.
        assert_eq!(list_seat_templates(&conn, 6).unwrap().len(), 0);

        // Re-running on the next startup must not clear layouts saved since.
        set_seat_template(&conn, 6, 2, 0.25, 0.8).unwrap();
        migrate_seat_templates_to_hero_relative(&conn).unwrap();
        assert_eq!(get_seat_template(&conn, 6, 2).unwrap(), Some((0.25, 0.8)));
    }
}
