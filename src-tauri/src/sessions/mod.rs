use std::collections::HashSet;

use chrono::{NaiveDate, NaiveDateTime};
use rusqlite::Connection;
use serde::Serialize;

/// No gap between two consecutive imported hands longer than this closes the
/// current session and opens a new one. Not user-facing in v1.
const SESSION_GAP_SECONDS: i64 = 30 * 60;

const PLAYED_AT_FORMAT: &str = "%Y-%m-%dT%H:%M:%S";

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SessionSummary {
    pub start_at: String,
    pub end_at: String,
    pub duration_secs: i64,
    pub hand_count: i64,
    pub table_count: i64,
    pub has_cash: bool,
    pub has_tournament: bool,
    /// Net cash result in `currency`, summed from the hero's cash hands only.
    /// `None` whenever the session has no cash hand with a known hero result
    /// — in particular, always `None` for a tournament-only session. Never a
    /// fabricated or estimated tournament result (PokerStars hand history has
    /// no buy-in/finish/payout to compute one from).
    pub net_result_cash: Option<f64>,
    pub currency: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SessionsTodaySummary {
    pub session_count: i64,
    pub total_duration_secs: i64,
    pub total_hands: i64,
    pub net_result_cash: Option<f64>,
    pub currency: Option<String>,
}

struct HandRow {
    format: String,
    table_name: Option<String>,
    played_at: NaiveDateTime,
    currency: Option<String>,
    hero_net_result: Option<f64>,
}

fn round_cents(value: f64) -> f64 {
    (value * 100.0).round() / 100.0
}

fn fetch_hands(conn: &Connection) -> rusqlite::Result<Vec<HandRow>> {
    // INNER JOIN, not LEFT: a hand with no is_hero=1 row was never the
    // user's own play (e.g. bulk-imported third-party hand histories with
    // no "Dealt to" line for anyone) and must not feed session grouping at
    // all — not even to extend an existing session's hand/table counts.
    let mut stmt = conn.prepare(
        "SELECT h.format, h.table_name, h.played_at, h.currency, ph.net_result
         FROM hands h
         JOIN player_hands ph ON ph.hand_id = h.id AND ph.is_hero = 1
         WHERE h.played_at IS NOT NULL
         ORDER BY h.played_at ASC, h.id ASC",
    )?;

    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, Option<String>>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, Option<String>>(3)?,
            row.get::<_, Option<f64>>(4)?,
        ))
    })?;

    let mut hands = Vec::new();
    for row in rows {
        let (format, table_name, played_at_raw, currency, hero_net_result) = row?;
        // Malformed timestamps can't be placed on the session timeline; skip
        // rather than guess, matching every other invariant in this file.
        if let Ok(played_at) = NaiveDateTime::parse_from_str(&played_at_raw, PLAYED_AT_FORMAT) {
            hands.push(HandRow {
                format,
                table_name,
                played_at,
                currency,
                hero_net_result,
            });
        }
    }
    Ok(hands)
}

struct SessionBuilder {
    start_at: NaiveDateTime,
    last_played_at: NaiveDateTime,
    hand_count: i64,
    tables: HashSet<String>,
    has_cash: bool,
    has_tournament: bool,
    net_result_cash: f64,
    has_cash_net_result: bool,
    currency: Option<String>,
}

impl SessionBuilder {
    fn start(hand: &HandRow) -> Self {
        let mut builder = Self {
            start_at: hand.played_at,
            last_played_at: hand.played_at,
            hand_count: 0,
            tables: HashSet::new(),
            has_cash: false,
            has_tournament: false,
            net_result_cash: 0.0,
            has_cash_net_result: false,
            currency: None,
        };
        builder.add(hand);
        builder
    }

    fn gap_from(&self, hand: &HandRow) -> i64 {
        hand.played_at
            .signed_duration_since(self.last_played_at)
            .num_seconds()
    }

    fn add(&mut self, hand: &HandRow) {
        self.last_played_at = hand.played_at;
        self.hand_count += 1;
        if let Some(table) = &hand.table_name {
            if !table.is_empty() {
                self.tables.insert(table.clone());
            }
        }

        match hand.format.as_str() {
            "cash" => {
                self.has_cash = true;
                if let Some(net) = hand.hero_net_result {
                    self.net_result_cash += net;
                    self.has_cash_net_result = true;
                    if self.currency.is_none() {
                        self.currency = hand.currency.clone();
                    }
                }
            }
            "tournament" => self.has_tournament = true,
            _ => {}
        }
    }

    fn finish(self) -> SessionSummary {
        let duration_secs = self
            .last_played_at
            .signed_duration_since(self.start_at)
            .num_seconds()
            .max(0);

        SessionSummary {
            start_at: self.start_at.format(PLAYED_AT_FORMAT).to_string(),
            end_at: self.last_played_at.format(PLAYED_AT_FORMAT).to_string(),
            duration_secs,
            hand_count: self.hand_count,
            table_count: self.tables.len() as i64,
            has_cash: self.has_cash,
            has_tournament: self.has_tournament,
            net_result_cash: if self.has_cash_net_result {
                Some(round_cents(self.net_result_cash))
            } else {
                None
            },
            currency: if self.has_cash_net_result {
                self.currency
            } else {
                None
            },
        }
    }
}

/// Groups every imported hand into contiguous play sessions (see module doc
/// for the gap rule) and returns them most-recent-first.
pub fn list_sessions(conn: &Connection) -> rusqlite::Result<Vec<SessionSummary>> {
    let hands = fetch_hands(conn)?;
    let mut sessions = Vec::new();
    let mut current: Option<SessionBuilder> = None;

    for hand in &hands {
        let starts_new_session = match &current {
            None => true,
            Some(builder) => builder.gap_from(hand) > SESSION_GAP_SECONDS,
        };

        if starts_new_session {
            if let Some(builder) = current.take() {
                sessions.push(builder.finish());
            }
            current = Some(SessionBuilder::start(hand));
        } else if let Some(builder) = current.as_mut() {
            builder.add(hand);
        }
    }
    if let Some(builder) = current.take() {
        sessions.push(builder.finish());
    }

    sessions.reverse();
    Ok(sessions)
}

/// Aggregates every session that *started* on `today` (a session that starts
/// the day before and runs past midnight is attributed to the day it
/// started). Returns `None` when there are no sessions today at all, so the
/// Dashboard can hide the card entirely instead of showing zeros.
pub fn sessions_today(
    conn: &Connection,
    today: NaiveDate,
) -> rusqlite::Result<Option<SessionsTodaySummary>> {
    let todays_sessions: Vec<SessionSummary> = list_sessions(conn)?
        .into_iter()
        .filter(|s| {
            NaiveDateTime::parse_from_str(&s.start_at, PLAYED_AT_FORMAT)
                .map(|dt| dt.date() == today)
                .unwrap_or(false)
        })
        .collect();

    if todays_sessions.is_empty() {
        return Ok(None);
    }

    let session_count = todays_sessions.len() as i64;
    let total_duration_secs = todays_sessions.iter().map(|s| s.duration_secs).sum();
    let total_hands = todays_sessions.iter().map(|s| s.hand_count).sum();

    let mut net_result_cash: Option<f64> = None;
    let mut currency: Option<String> = None;
    for session in &todays_sessions {
        if let Some(net) = session.net_result_cash {
            net_result_cash = Some(net_result_cash.unwrap_or(0.0) + net);
            if currency.is_none() {
                currency = session.currency.clone();
            }
        }
    }

    Ok(Some(SessionsTodaySummary {
        session_count,
        total_duration_secs,
        total_hands,
        net_result_cash: net_result_cash.map(round_cents),
        currency,
    }))
}
