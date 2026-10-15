use rusqlite::{params, Connection, OptionalExtension};

use crate::db;
use crate::parser::{HandHistoryParser, ParsedHand, PokerStarsParser};

pub mod validate;

#[derive(Debug, Default, Clone, Copy)]
pub struct ImportSummary {
    pub hands_imported: i64,
    pub hands_skipped_duplicate: i64,
    pub hands_failed: i64,
    /// Hands the integrity gate refused to store (`validate::Severity::Reject`).
    pub hands_rejected_invalid: i64,
    /// Hands stored despite a recorded warning.
    pub hands_with_warnings: i64,
    /// Seat lines seen but not dealt into their hand — sitting out, or moved in
    /// from another table. Counted, not an error.
    pub seats_not_dealt_in: i64,
    /// Files that could not be read at all (locked, deleted or moved
    /// mid-scan). Skipped and logged, never fatal: the watcher re-imports a
    /// file whole on its next change, so its hands arrive then.
    pub files_failed: i64,
}

impl ImportSummary {
    fn merge(&mut self, other: ImportSummary) {
        self.hands_imported += other.hands_imported;
        self.hands_skipped_duplicate += other.hands_skipped_duplicate;
        self.hands_failed += other.hands_failed;
        self.hands_rejected_invalid += other.hands_rejected_invalid;
        self.hands_with_warnings += other.hands_with_warnings;
        self.seats_not_dealt_in += other.seats_not_dealt_in;
        self.files_failed += other.files_failed;
    }
}

/// Parses `text` as PokerStars hand history content and imports every hand
/// into the database. Hands already present (matched by their PokerStars
/// hand id) are skipped, making this safe to call repeatedly on the same or
/// overlapping file contents.
pub fn import_text(conn: &mut Connection, text: &str) -> Result<ImportSummary, String> {
    let parser = PokerStarsParser;
    let results = parser.parse(text);
    let mut summary = ImportSummary::default();
    // Only the hands this call adds are scanned for auto-notes below.
    let notes_cursor = if cfg!(feature = "strategic-analysis") {
        crate::engine::max_hand_row_id(conn).ok()
    } else {
        None
    };

    for result in results {
        match result {
            Ok(hand) => {
                summary.seats_not_dealt_in += hand.skipped_seats.len() as i64;

                // Integrity gate. A hand that fails
                // never reaches the database, and never fails silently: it is
                // logged here and counted into the persisted totals below, which
                // the Settings → Diagnostics report reads back.
                let problems = validate::check(&hand);
                if !validate::is_storable(&problems) {
                    summary.hands_rejected_invalid += 1;
                    for problem in problems.iter().filter(|p| p.severity == validate::Severity::Reject) {
                        eprintln!(
                            "REJECTED hand {} [{}]: {}",
                            hand.hand_id, problem.code, problem.detail
                        );
                    }
                    let _ = db::record_import_problems(conn, &hand.hand_id, &problems);
                    continue;
                }

                // Storing a hand clears the findings of any earlier, rejected
                // copy of it (see `import_hand`), so its warnings are recorded
                // after the insert, not before.
                let stored = import_hand(conn, &hand);
                if !problems.is_empty() {
                    summary.hands_with_warnings += 1;
                    for problem in &problems {
                        eprintln!(
                            "WARNING hand {} [{}]: {}",
                            hand.hand_id, problem.code, problem.detail
                        );
                    }
                    let _ = db::record_import_problems(conn, &hand.hand_id, &problems);
                }

                match stored {
                    Ok(true) => summary.hands_imported += 1,
                    Ok(false) => summary.hands_skipped_duplicate += 1,
                    Err(err) => {
                        summary.hands_failed += 1;
                        eprintln!("failed to import hand {}: {}", hand.hand_id, err);
                    }
                }
            }
            Err(_) => {
                summary.hands_failed += 1;
            }
        }
    }

    // Auto-notes (spec section 11), strategic-analysis build only. The hands
    // are already committed; a detector failure is logged and never undoes
    // or fails the import.
    if let Some(cursor) = notes_cursor.filter(|_| summary.hands_imported > 0) {
        if let Err(err) = crate::engine::generate_auto_notes_after(conn, cursor) {
            eprintln!("auto-note generation failed after import: {err}");
        }
    }

    Ok(summary)
}

/// Bytes that are not UTF-8 (a stray byte, a write cut inside a multi-byte
/// character) are decoded as U+FFFD instead of failing the whole file: only a
/// hand whose names they land in is lost, and the integrity gate counts it
/// (`undecodable_text`).
pub fn import_file(conn: &mut Connection, path: &std::path::Path) -> Result<ImportSummary, String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    import_text(conn, &String::from_utf8_lossy(&bytes))
}

/// Imports every `.txt` file found anywhere under `dir`, including nested
/// per-screen-name subdirectories PokerStars creates (e.g.
/// `HandHistory\<ScreenName>\*.txt`). Never looks outside `dir`.
pub fn import_directory(
    conn: &mut Connection,
    dir: &std::path::Path,
) -> Result<ImportSummary, String> {
    Ok(import_files(conn, &collect_txt_files(dir)))
}

/// Imports each file in turn. A file that cannot be read is logged, counted
/// in `files_failed` and skipped: one locked or vanished file must never stop
/// the rest of the backlog (or, at startup, the watcher) from starting.
pub fn import_files(conn: &mut Connection, paths: &[std::path::PathBuf]) -> ImportSummary {
    let mut summary = ImportSummary::default();
    for path in paths {
        match import_file(conn, path) {
            Ok(file_summary) => summary.merge(file_summary),
            Err(err) => {
                summary.files_failed += 1;
                eprintln!("skipped unreadable hand history file {path:?}: {err}");
            }
        }
    }
    summary
}

/// The startup backlog import. A lock poisoned by a panic elsewhere is
/// recovered rather than treated as "nothing to do": a panicking hand import
/// has already rolled its transaction back, so the connection is consistent.
pub fn run_backlog_import(
    conn: &std::sync::Mutex<Connection>,
    dir: &std::path::Path,
) -> ImportSummary {
    let mut conn = conn.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    import_files(&mut conn, &collect_txt_files(dir))
}

/// Parser status once the backlog import ran and the watcher was (or was
/// not) started. `"watching"` only when a watcher really exists.
pub fn status_after_startup(summary: &ImportSummary, watcher_error: Option<&str>) -> String {
    match watcher_error {
        Some(err) => format!("error: hand history watcher did not start: {err}"),
        None if summary.files_failed > 0 => format!(
            "watching ({} file{} could not be read; retried when PokerStars writes {})",
            summary.files_failed,
            if summary.files_failed == 1 { "" } else { "s" },
            if summary.files_failed == 1 { "it" } else { "them" },
        ),
        None => "watching".to_string(),
    }
}

/// Parser status after the running watcher reported an error event.
pub fn status_for_watcher_error(err: &str) -> String {
    format!("error: watcher: {err}")
}

/// After a watched file imported fine, the watcher evidently works again:
/// clears a watcher error. Any other status is left unchanged (`None`).
pub fn status_after_watched_import(current: &str) -> Option<String> {
    current
        .starts_with("error: watcher:")
        .then(|| "watching".to_string())
}

/// Recursively collects `.txt` file paths under `dir`, descending into any
/// number of nested subdirectories (e.g. one per PokerStars screen name).
///
/// `pub(crate)` (rather than private) so `commands::get_onboarding_readiness`
/// can pick its language/hand-count sample file from exactly the same
/// recursive scan the real import pipeline uses — never a different, possibly
/// shallower traversal that could sample a different file.
pub(crate) fn collect_txt_files(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut files = Vec::new();
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(_) => return files,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            files.extend(collect_txt_files(&path));
        } else if path.extension().map_or(false, |ext| ext == "txt") {
            files.push(path);
        }
    }

    files
}

fn import_hand(conn: &mut Connection, hand: &ParsedHand) -> Result<bool, rusqlite::Error> {
    let tx = conn.transaction()?;

    let already_exists: Option<i64> = tx
        .query_row(
            "SELECT id FROM hands WHERE hand_id = ?1",
            params![hand.hand_id],
            |row| row.get(0),
        )
        .optional()?;

    if already_exists.is_some() {
        tx.rollback()?;
        return Ok(false);
    }

    tx.execute(
        "INSERT INTO hands (site, hand_id, format, table_name, game_type, tournament_id, buy_in, level, small_blind, big_blind, currency, max_seats, button_seat, played_at, raw_text, imported_at, variant)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)",
        params![
            hand.site,
            hand.hand_id,
            hand.format.as_str(),
            hand.table_name,
            hand.game_type,
            hand.tournament_id,
            hand.buy_in,
            hand.level,
            hand.small_blind,
            hand.big_blind,
            hand.currency,
            hand.max_seats,
            hand.button_seat,
            hand.played_at,
            hand.raw_text,
            db::now_iso(),
            hand.variant.as_str(),
        ],
    )?;
    let hand_row_id = tx.last_insert_rowid();

    for seat in &hand.seats {
        let player_id = db::get_or_create_player(&tx, &hand.site, &seat.player_name)?;
        let is_hero = hand.hero_name.as_deref() == Some(seat.player_name.as_str());
        let result = hand
            .results
            .get(&seat.player_name)
            .cloned()
            .unwrap_or_default();

        tx.execute(
            "INSERT INTO player_hands (hand_id, player_id, seat, starting_stack, position, is_hero, went_to_showdown, won_at_showdown, net_result, hole_cards, bounty)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
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
                seat.hole_cards,
                seat.bounty,
            ],
        )?;
    }

    for action in &hand.actions {
        let player_id = db::get_or_create_player(&tx, &hand.site, &action.player_name)?;
        tx.execute(
            "INSERT INTO actions (hand_id, player_id, street, action_index, action_type, amount, is_all_in)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                hand_row_id,
                player_id,
                action.street.as_str(),
                action.order,
                action.action_type.as_str(),
                action.amount,
                action.is_all_in as i64,
            ],
        )?;
    }

    // Findings recorded for an earlier copy of this hand id described text
    // that was refused (typically the same hand read before PokerStars
    // finished writing it). The hand is stored now, so they no longer apply.
    tx.execute("DELETE FROM import_problems WHERE hand_id = ?1", params![hand.hand_id])?;

    tx.commit()?;
    Ok(true)
}
