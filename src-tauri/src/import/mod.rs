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
}

impl ImportSummary {
    fn merge(&mut self, other: ImportSummary) {
        self.hands_imported += other.hands_imported;
        self.hands_skipped_duplicate += other.hands_skipped_duplicate;
        self.hands_failed += other.hands_failed;
        self.hands_rejected_invalid += other.hands_rejected_invalid;
        self.hands_with_warnings += other.hands_with_warnings;
        self.seats_not_dealt_in += other.seats_not_dealt_in;
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

    for result in results {
        match result {
            Ok(hand) => {
                summary.seats_not_dealt_in += hand.skipped_seats.len() as i64;

                // Integrity gate (work unit 2, requirement 5). A hand that fails
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

                match import_hand(conn, &hand) {
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

    Ok(summary)
}

pub fn import_file(conn: &mut Connection, path: &std::path::Path) -> Result<ImportSummary, String> {
    let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    import_text(conn, &text)
}

/// Imports every `.txt` file found anywhere under `dir`, including nested
/// per-screen-name subdirectories PokerStars creates (e.g.
/// `HandHistory\<ScreenName>\*.txt`). Never looks outside `dir`.
pub fn import_directory(
    conn: &mut Connection,
    dir: &std::path::Path,
) -> Result<ImportSummary, String> {
    let mut summary = ImportSummary::default();
    for path in collect_txt_files(dir) {
        let file_summary = import_file(conn, &path)?;
        summary.merge(file_summary);
    }
    Ok(summary)
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
        "INSERT INTO hands (site, hand_id, format, table_name, game_type, tournament_id, buy_in, level, small_blind, big_blind, currency, max_seats, button_seat, played_at, raw_text, imported_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
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

    tx.commit()?;
    Ok(true)
}
