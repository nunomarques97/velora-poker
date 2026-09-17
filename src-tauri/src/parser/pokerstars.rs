use std::collections::HashMap;

use regex::Regex;
use std::sync::OnceLock;

use super::model::{
    ActionType, HandFormat, ParseError, ParsedAction, ParsedHand, ParsedPlayerResult, ParsedSeat,
    SkippedSeat, Street,
};
use super::position;

/// Parses PokerStars hand history text into structured [`ParsedHand`] values.
///
/// Supports cash-game and tournament (including Zoom tournament) No Limit /
/// Limit / Pot Limit Hold'em style hand histories with a standard
/// `PokerStars Hand #...:` header. Other poker sites are out of scope for
/// this parser; additional site/format parsers can be added later behind the
/// [`HandHistoryParser`] trait.
pub trait HandHistoryParser {
    fn site(&self) -> &'static str;
    fn parse(&self, text: &str) -> Vec<Result<ParsedHand, ParseError>>;
}

pub struct PokerStarsParser;

impl HandHistoryParser for PokerStarsParser {
    fn site(&self) -> &'static str {
        "pokerstars"
    }

    fn parse(&self, text: &str) -> Vec<Result<ParsedHand, ParseError>> {
        split_hands(text)
            .into_iter()
            .map(|block| parse_hand_block(&block))
            .collect()
    }
}

/// Splits a raw hand history file into individual hand text blocks.
///
/// Strips a leading UTF-8 BOM (PokerStars writes one on some installs/OS
/// locales) so the anchored header regex still matches the first hand, and
/// drops any leading/trailing text that isn't itself a hand block (e.g. the
/// `*** # N ***` separators some exported multi-hand transcripts prefix each
/// hand with) instead of surfacing it as a bogus parse failure.
pub fn split_hands(text: &str) -> Vec<String> {
    let text = text.strip_prefix('\u{FEFF}').unwrap_or(text);
    let mut hands = Vec::new();
    let mut current = String::new();

    for line in text.lines() {
        if line.starts_with("PokerStars Hand #") && !current.trim().is_empty() {
            push_hand_block(&mut hands, &current);
            current.clear();
        }
        current.push_str(line);
        current.push('\n');
    }

    push_hand_block(&mut hands, &current);

    hands
}

fn push_hand_block(hands: &mut Vec<String>, block: &str) {
    let trimmed = block.trim();
    if trimmed.starts_with("PokerStars Hand #") {
        hands.push(trimmed.to_string());
    }
}

fn header_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"^PokerStars Hand #(\d+):\s+(.+?)\s+\(([\$€£]?)([\d.]+)/([\$€£]?)([\d.]+)(?:\s+([A-Z]{3}))?\)\s+-\s+(\d{4}/\d{2}/\d{2})\s+(\d{1,2}:\d{2}:\d{2})",
        )
        .expect("valid header regex")
    })
}

fn tournament_header_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"^PokerStars Hand #(\d+): (?:Zoom )?Tournament #(\d+), (.+?) - Level (\S+)\s*\(([\d,]+)/([\d,]+)\) - (\d{4}/\d{2}/\d{2}) (\d{1,2}:\d{2}:\d{2})",
        )
        .expect("valid tournament header regex")
    })
}

fn table_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"^Table '(.+?)' (\d+)-max Seat #(\d+) is the button").expect("valid table regex")
    })
}

/// Matches a seat line and captures everything trailing `in chips`.
///
/// The closing parenthesis is deliberately **not** required immediately after
/// `in chips`. PokerStars appends content there in several real formats, and
/// requiring the paren silently dropped every seat of every bounty-tournament
/// hand: the whole seat block failed to match, so the hand was imported with no
/// players at all (26 of 275 stored hands, 9.5% — see
/// the notes §G.2).
///
/// Every trailing variant found in the user's own 22 hand-history files
/// (1,599 seat lines), all covered by `tests/parser_seat_line_tests.rs`:
///
/// ```text
/// Seat 3: NAME (1500 in chips)
/// Seat 3: NAME (€2 in chips)
/// Seat 3: NAME (1500 in chips) is sitting out
/// Seat 3: NAME (11262 in chips, €13.50 bounty)
/// Seat 3: NAME (11262 in chips, €13.50 bounty) is sitting out
/// Seat 3: NAME (50000 in chips) out of hand (moved from another table into small blind)
/// ```
///
/// Capture 4 is that trailing text, kept for diagnostics only — it is never
/// used to decide whether the seat was dealt in (see [`ParsedHand::seats`]).
fn seat_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"^Seat (\d+): (.+) \([\$€£]?([\d.]+) in chips(.*)$").expect("valid seat regex")
    })
}

fn dealt_to_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^Dealt to (.+) \[.+\]").expect("valid dealt-to regex"))
}

fn summary_seat_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^Seat (\d+): (.+)$").expect("valid summary seat regex"))
}

fn currency_from_symbol(symbol: &str, code: Option<&str>) -> String {
    if let Some(code) = code {
        return code.to_string();
    }
    match symbol {
        "$" => "USD",
        "€" => "EUR",
        "£" => "GBP",
        _ => "PLAY",
    }
    .to_string()
}

/// PokerStars doesn't always zero-pad a single-digit hour (e.g. `0:06:23`
/// rather than `00:06:23`), but a stored `played_at` should stay
/// lexicographically sortable like a real ISO-8601 timestamp.
fn pad_time(time: &str) -> String {
    match time.split_once(':') {
        Some((hour, rest)) if hour.len() == 1 => format!("0{hour}:{rest}"),
        _ => time.to_string(),
    }
}

fn parse_money(s: &str) -> Option<f64> {
    let cleaned: String = s
        .chars()
        .filter(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    if cleaned.is_empty() {
        None
    } else {
        cleaned.parse::<f64>().ok()
    }
}

/// Splits a tournament header's descriptive segment (e.g. `"44000+6000
/// Hold'em No Limit"` or `"$10+$1 USD Hold'em No Limit"`) into a best-effort
/// buy-in prefix and the remaining game-type text. Falls back to treating
/// the whole segment as the game type when no leading buy-in-shaped prefix
/// is found (e.g. `"Freeroll Hold'em No Limit"`), rather than guessing.
fn split_buyin_and_game_type(raw: &str) -> (Option<String>, String) {
    let buyin_end = raw.find(|c: char| c.is_alphabetic()).unwrap_or(0);
    if buyin_end > 0 {
        let buy_in = raw[..buyin_end].trim().to_string();
        let game_type = raw[buyin_end..].trim().to_string();
        if !buy_in.is_empty() && !game_type.is_empty() {
            return (Some(buy_in), game_type);
        }
    }
    (None, raw.trim().to_string())
}

struct HeaderInfo {
    hand_id: String,
    format: HandFormat,
    game_type: String,
    small_blind: f64,
    big_blind: f64,
    currency: String,
    tournament_id: Option<String>,
    buy_in: Option<String>,
    level: Option<String>,
    played_at: String,
}

fn parse_header(line: &str) -> Result<HeaderInfo, ParseError> {
    if line.contains("Tournament #") {
        parse_tournament_header(line)
    } else {
        parse_cash_header(line)
    }
}

fn parse_cash_header(line: &str) -> Result<HeaderInfo, ParseError> {
    let header = header_regex().captures(line).ok_or(ParseError::MissingHeader)?;

    let hand_id = header[1].to_string();
    let game_type = header[2].to_string();
    let sb_symbol = &header[3];
    let small_blind = parse_money(&header[4]).unwrap_or(0.0);
    let bb_symbol = &header[5];
    let big_blind = parse_money(&header[6]).unwrap_or(0.0);
    let currency_code = header.get(7).map(|m| m.as_str());
    let currency = currency_from_symbol(
        if !sb_symbol.is_empty() { sb_symbol } else { bb_symbol },
        currency_code,
    );
    let date = &header[8];
    let time = &header[9];

    Ok(HeaderInfo {
        hand_id,
        format: HandFormat::Cash,
        game_type,
        small_blind,
        big_blind,
        currency,
        tournament_id: None,
        buy_in: None,
        level: None,
        played_at: format!("{}T{}", date.replace('/', "-"), pad_time(time)),
    })
}

fn parse_tournament_header(line: &str) -> Result<HeaderInfo, ParseError> {
    let header = tournament_header_regex()
        .captures(line)
        .ok_or_else(|| ParseError::UnsupportedFormat("unrecognized tournament header format".to_string()))?;

    let hand_id = header[1].to_string();
    let tournament_id = header[2].to_string();
    let (buy_in, game_type) = split_buyin_and_game_type(&header[3]);
    let level = header[4].to_string();
    let small_blind = parse_money(&header[5]).unwrap_or(0.0);
    let big_blind = parse_money(&header[6]).unwrap_or(0.0);
    let date = &header[7];
    let time = &header[8];

    Ok(HeaderInfo {
        hand_id,
        format: HandFormat::Tournament,
        game_type,
        small_blind,
        big_blind,
        currency: "CHIPS".to_string(),
        tournament_id: Some(tournament_id),
        buy_in,
        level: Some(level),
        played_at: format!("{}T{}", date.replace('/', "-"), pad_time(time)),
    })
}

fn parse_action_desc(desc: &str) -> Option<(ActionType, Option<f64>, bool)> {
    let is_all_in = desc.trim_end().ends_with("and is all-in");
    let desc = if is_all_in {
        desc.trim_end()
            .trim_end_matches("and is all-in")
            .trim_end()
    } else {
        desc.trim_end()
    };

    if desc == "folds" {
        return Some((ActionType::Fold, None, is_all_in));
    }
    if desc == "checks" {
        return Some((ActionType::Check, None, is_all_in));
    }
    if let Some(rest) = desc.strip_prefix("calls ") {
        return Some((ActionType::Call, parse_money(rest), is_all_in));
    }
    if let Some(rest) = desc.strip_prefix("bets ") {
        return Some((ActionType::Bet, parse_money(rest), is_all_in));
    }
    if let Some(rest) = desc.strip_prefix("raises ") {
        if let Some(idx) = rest.find(" to ") {
            return Some((ActionType::Raise, parse_money(&rest[idx + 4..]), is_all_in));
        }
        return Some((ActionType::Raise, parse_money(rest), is_all_in));
    }
    if let Some(rest) = desc.strip_prefix("posts small blind ") {
        return Some((ActionType::PostSmallBlind, parse_money(rest), is_all_in));
    }
    if let Some(rest) = desc.strip_prefix("posts big blind ") {
        return Some((ActionType::PostBigBlind, parse_money(rest), is_all_in));
    }
    if let Some(rest) = desc.strip_prefix("posts the ante ") {
        return Some((ActionType::PostAnte, parse_money(rest), is_all_in));
    }

    None
}

/// Finds the longest known player name that prefixes `rest`, returning the
/// matched name and the trimmed remainder of the string. Used for parsing the
/// `*** SUMMARY ***` section, where player names may themselves contain
/// characters (spaces, parentheses) that make a purely regex-based split
/// ambiguous.
fn strip_known_name<'a>(rest: &'a str, known_names: &[String]) -> Option<(String, &'a str)> {
    let mut best: Option<&String> = None;
    for name in known_names {
        if rest.starts_with(name.as_str()) && best.map_or(true, |b| name.len() > b.len()) {
            best = Some(name);
        }
    }
    best.map(|name| (name.clone(), rest[name.len()..].trim_start()))
}

/// Strips **every** leading position tag, not just the first.
///
/// A heads-up summary line carries two, because the button also posts the small
/// blind: `Seat 1: NAME (button) (small blind) collected (€0.04)`. Stripping
/// one left `(small blind) …` as the description, which made "is this
/// description empty" — the dealt-in test — read a tag as content.
fn strip_position_tags(desc: &str) -> &str {
    let mut rest = desc.trim_start();
    'outer: loop {
        for tag in ["(button)", "(small blind)", "(big blind)"] {
            if let Some(stripped) = rest.strip_prefix(tag) {
                rest = stripped.trim_start();
                continue 'outer;
            }
        }
        return rest;
    }
}

/// Matches a `"<Name> collected <amount> from pot"` (or "main pot" / "side
/// pot") line — the authoritative record of money a player won, whether by
/// showdown or by taking down an uncontested pot. These lines appear in the
/// hand body (not the `*** SUMMARY ***` section) and carry no leading colon,
/// so they never collide with the `NAME: action` action-line parsing.
fn parse_collected_line(line: &str, known_names: &[String]) -> Option<(String, f64)> {
    let (name, rest) = strip_known_name(line, known_names)?;
    let rest = rest.strip_prefix("collected ")?.trim();
    let amount_str = rest.split(" from ").next()?;
    let amount = parse_money(amount_str)?;
    Some((name, amount))
}

/// Matches `"Uncalled bet (<amount>) returned to <Name>"` — money a player
/// wagered that nobody could call, handed straight back to them without ever
/// entering the pot.
fn parse_uncalled_bet_line(line: &str) -> Option<(String, f64)> {
    let rest = line.strip_prefix("Uncalled bet (")?;
    let (amount_str, rest) = rest.split_once(')')?;
    let amount = parse_money(amount_str)?;
    let name = rest.trim().strip_prefix("returned to ")?.trim().to_string();
    Some((name, amount))
}

fn round_cents(value: f64) -> f64 {
    (value * 100.0).round() / 100.0
}

/// Reconstructs how much money each player put into the pot across the whole
/// hand from their actions alone. Blinds/antes/bets/calls carry the literal
/// increment PokerStars logs; a raise's logged amount is the *absolute*
/// street total after raising ("raises X to Y" — Y, not the increment), so
/// raises are resolved against a running per-street commitment that resets at
/// every street change instead of being summed directly.
fn compute_contributed(actions: &[ParsedAction]) -> HashMap<String, f64> {
    let mut contributed: HashMap<String, f64> = HashMap::new();
    let mut street_commitment: HashMap<String, f64> = HashMap::new();
    let mut current_street: Option<Street> = None;

    for action in actions {
        if current_street != Some(action.street) {
            street_commitment.clear();
            current_street = Some(action.street);
        }

        match action.action_type {
            ActionType::PostSmallBlind | ActionType::PostBigBlind => {
                if let Some(amount) = action.amount {
                    *contributed.entry(action.player_name.clone()).or_insert(0.0) += amount;
                    street_commitment.insert(action.player_name.clone(), amount);
                }
            }
            ActionType::PostAnte => {
                if let Some(amount) = action.amount {
                    *contributed.entry(action.player_name.clone()).or_insert(0.0) += amount;
                }
            }
            ActionType::Bet | ActionType::Call => {
                if let Some(amount) = action.amount {
                    *contributed.entry(action.player_name.clone()).or_insert(0.0) += amount;
                    *street_commitment
                        .entry(action.player_name.clone())
                        .or_insert(0.0) += amount;
                }
            }
            ActionType::Raise => {
                if let Some(to_amount) = action.amount {
                    let prior = street_commitment
                        .get(&action.player_name)
                        .copied()
                        .unwrap_or(0.0);
                    let increment = (to_amount - prior).max(0.0);
                    *contributed.entry(action.player_name.clone()).or_insert(0.0) += increment;
                    street_commitment.insert(action.player_name.clone(), to_amount);
                }
            }
            ActionType::Fold | ActionType::Check => {}
        }
    }

    contributed
}

pub fn parse_hand_block(block: &str) -> Result<ParsedHand, ParseError> {
    let block = block.trim();
    if block.is_empty() {
        return Err(ParseError::EmptyBlock);
    }

    let mut lines = block.lines();
    let header_line = lines.next().ok_or(ParseError::MissingHeader)?;

    let header = parse_header(header_line)?;

    let mut table_name = String::new();
    let mut max_seats = 0i64;
    let mut button_seat = 0i64;
    let mut seats: Vec<ParsedSeat> = Vec::new();
    let mut actions: Vec<ParsedAction> = Vec::new();
    let mut hero_name: Option<String> = None;
    // Blind/ante posts appear before the "*** HOLE CARDS ***" marker but are
    // always preflop actions, so preflop is the default street.
    let mut current_street: Option<Street> = Some(Street::Preflop);
    let mut has_showdown = false;
    let mut in_summary = false;
    let mut order: i64 = 0;
    let mut results: HashMap<String, ParsedPlayerResult> = HashMap::new();
    let mut collected: HashMap<String, f64> = HashMap::new();
    let mut uncalled_returned: HashMap<String, f64> = HashMap::new();
    // Trailing text after `in chips` per seat, and the set of players carrying a
    // real `*** SUMMARY ***` description. Both feed the dealt-in decision below.
    let mut seat_markers: HashMap<String, String> = HashMap::new();
    let mut summary_described: std::collections::HashSet<String> = std::collections::HashSet::new();

    for line in lines {
        let line = line.trim_end();
        if line.is_empty() {
            continue;
        }

        if let Some(caps) = table_regex().captures(line) {
            table_name = caps[1].to_string();
            max_seats = caps[2].parse().unwrap_or(0);
            button_seat = caps[3].parse().unwrap_or(0);
            continue;
        }

        if !in_summary {
            if let Some(caps) = seat_regex().captures(line) {
                let name = caps[2].to_string();
                seat_markers.insert(name.clone(), caps[4].to_string());
                seats.push(ParsedSeat {
                    seat_number: caps[1].parse().unwrap_or(0),
                    player_name: name,
                    starting_stack: parse_money(&caps[3]).unwrap_or(0.0),
                    position: None,
                });
                continue;
            }
        }

        if line == "*** HOLE CARDS ***" {
            current_street = Some(Street::Preflop);
            continue;
        }
        if line.starts_with("*** FLOP ***") {
            current_street = Some(Street::Flop);
            continue;
        }
        if line.starts_with("*** TURN ***") {
            current_street = Some(Street::Turn);
            continue;
        }
        if line.starts_with("*** RIVER ***") {
            current_street = Some(Street::River);
            continue;
        }
        if line.starts_with("*** SHOW DOWN ***") {
            has_showdown = true;
            continue;
        }
        if line.starts_with("*** SUMMARY ***") {
            in_summary = true;
            current_street = None;
            continue;
        }

        if in_summary {
            let known_names: Vec<String> = seats.iter().map(|s| s.player_name.clone()).collect();
            if let Some(caps) = summary_seat_regex().captures(line) {
                let rest = caps[2].trim();
                if let Some((name, desc)) = strip_known_name(rest, &known_names) {
                    let desc = strip_position_tags(desc);
                    if !desc.is_empty() {
                        summary_described.insert(name.clone());
                    }
                    let folded = desc.starts_with("folded");
                    let won = desc.contains("collected") || desc.contains("won (");
                    let went_to_showdown = has_showdown && !folded;
                    let won_at_showdown = went_to_showdown && won;
                    results.insert(
                        name.to_string(),
                        ParsedPlayerResult {
                            went_to_showdown,
                            won_at_showdown,
                            net_result: None,
                        },
                    );
                }
            }
            continue;
        }

        if let Some(name) = dealt_to_regex()
            .captures(line)
            .and_then(|c| c.get(1))
            .map(|m| m.as_str().to_string())
        {
            hero_name = Some(name);
            continue;
        }

        {
            let known_names: Vec<String> = seats.iter().map(|s| s.player_name.clone()).collect();
            if let Some((name, amount)) = parse_uncalled_bet_line(line) {
                *uncalled_returned.entry(name).or_insert(0.0) += amount;
                continue;
            }
            if let Some((name, amount)) = parse_collected_line(line, &known_names) {
                *collected.entry(name).or_insert(0.0) += amount;
                continue;
            }
        }

        if let Some(street) = current_street {
            if let Some(idx) = line.find(": ") {
                let name = &line[..idx];
                let desc = &line[idx + 2..];
                if let Some((action_type, amount, is_all_in)) = parse_action_desc(desc) {
                    actions.push(ParsedAction {
                        street,
                        order,
                        player_name: name.to_string(),
                        action_type,
                        amount,
                        is_all_in,
                    });
                    order += 1;
                }
            }
        }
    }

    // Split the parsed seat lines into players actually dealt into this hand and
    // players merely sitting at the table.
    //
    // The test is behavioural, never textual. A seat counts as dealt in when the
    // player took at least one action (a posted blind or ante counts — every
    // dealt-in player produces at least one) or carries a real description in
    // the summary section. Checked independently against all 1,599 seat lines in
    // the user's 22 hand-history files: the two signals agreed on every one,
    // identifying the same 1,571 dealt in and 28 not.
    //
    // Deliberately *not* the `is sitting out` marker: 174 of the 197 seats
    // carrying it were dealt in and played the hand (see `SkippedSeat`), so
    // excluding on the marker would discard real players — a worse defect than
    // the one being fixed. The union of the two signals is used rather than
    // either alone so that a hand where one signal is unexpectedly absent still
    // keeps the player.
    let acted: std::collections::HashSet<&str> =
        actions.iter().map(|a| a.player_name.as_str()).collect();
    let mut skipped_seats: Vec<SkippedSeat> = Vec::new();
    seats.retain(|seat| {
        let dealt_in = acted.contains(seat.player_name.as_str())
            || summary_described.contains(&seat.player_name);
        if !dealt_in {
            skipped_seats.push(SkippedSeat {
                seat_number: seat.seat_number,
                player_name: seat.player_name.clone(),
                seat_line_marker: seat_markers
                    .get(&seat.player_name)
                    .cloned()
                    .unwrap_or_default(),
            });
        }
        dealt_in
    });

    // Position is derived from the dealt-in ring only — see `position.rs` for
    // the measurement that made that a hard requirement rather than a detail.
    let seat_numbers: Vec<i64> = seats.iter().map(|s| s.seat_number).collect();
    for (seat, label) in seats
        .iter_mut()
        .zip(position::derive(&seat_numbers, button_seat))
    {
        seat.position = label;
    }

    // Net money result is only meaningful for cash games: tournament chips
    // aren't money, and PokerStars hand-history text carries no buy-in/payout
    // to convert them with, so tournament hands never get a `net_result`
    // (see `ParsedPlayerResult::net_result` doc).
    if header.format == HandFormat::Cash {
        let contributed = compute_contributed(&actions);
        let mut names: std::collections::HashSet<String> = std::collections::HashSet::new();
        names.extend(contributed.keys().cloned());
        names.extend(collected.keys().cloned());
        names.extend(uncalled_returned.keys().cloned());

        for name in names {
            let won = collected.get(&name).copied().unwrap_or(0.0);
            let returned = uncalled_returned.get(&name).copied().unwrap_or(0.0);
            let spent = contributed.get(&name).copied().unwrap_or(0.0);
            let net = round_cents(won + returned - spent);
            results
                .entry(name)
                .or_insert_with(ParsedPlayerResult::default)
                .net_result = Some(net);
        }
    }

    Ok(ParsedHand {
        hand_id: header.hand_id,
        site: "pokerstars".to_string(),
        format: header.format,
        table_name,
        max_seats,
        button_seat,
        game_type: header.game_type,
        small_blind: header.small_blind,
        big_blind: header.big_blind,
        currency: header.currency,
        tournament_id: header.tournament_id,
        buy_in: header.buy_in,
        level: header.level,
        played_at: header.played_at,
        hero_name,
        seats,
        skipped_seats,
        actions,
        results,
        raw_text: block.to_string(),
    })
}
