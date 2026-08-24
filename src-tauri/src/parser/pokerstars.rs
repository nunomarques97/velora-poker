use std::collections::HashMap;

use regex::Regex;
use std::sync::OnceLock;

use super::model::{
    ActionType, ParseError, ParsedAction, ParsedHand, ParsedPlayerResult, ParsedSeat, Street,
};

/// Parses PokerStars hand history text into structured [`ParsedHand`] values.
///
/// Only cash-game No Limit / Limit / Pot Limit Hold'em style hand histories with a
/// standard `PokerStars Hand #...:` header are currently supported. Tournament
/// summaries and other poker sites are out of scope for this parser; additional
/// site/format parsers can be added later behind the [`HandHistoryParser`] trait.
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
pub fn split_hands(text: &str) -> Vec<String> {
    let mut hands = Vec::new();
    let mut current = String::new();

    for line in text.lines() {
        if line.starts_with("PokerStars Hand #") && !current.trim().is_empty() {
            hands.push(current.trim().to_string());
            current.clear();
        }
        current.push_str(line);
        current.push('\n');
    }

    if !current.trim().is_empty() {
        hands.push(current.trim().to_string());
    }

    hands
}

fn header_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"^PokerStars Hand #(\d+):\s+(.+?)\s+\(([\$€£]?)([\d.]+)/([\$€£]?)([\d.]+)(?:\s+([A-Z]{3}))?\)\s+-\s+(\d{4}/\d{2}/\d{2})\s+(\d{2}:\d{2}:\d{2})",
        )
        .expect("valid header regex")
    })
}

fn table_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"^Table '(.+?)' (\d+)-max Seat #(\d+) is the button").expect("valid table regex")
    })
}

fn seat_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"^Seat (\d+): (.+) \([\$€£]?([\d.]+) in chips\)").expect("valid seat regex")
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

fn strip_position_tag(desc: &str) -> &str {
    for tag in ["(button)", "(small blind)", "(big blind)"] {
        if let Some(rest) = desc.strip_prefix(tag) {
            return rest.trim_start();
        }
    }
    desc
}

pub fn parse_hand_block(block: &str) -> Result<ParsedHand, ParseError> {
    let block = block.trim();
    if block.is_empty() {
        return Err(ParseError::EmptyBlock);
    }

    let mut lines = block.lines();
    let header_line = lines.next().ok_or(ParseError::MissingHeader)?;

    let header = header_regex()
        .captures(header_line)
        .ok_or(ParseError::MissingHeader)?;

    let hand_id = header[1].to_string();
    let game_type = header[2].to_string();

    if game_type.contains("Tournament") {
        return Err(ParseError::UnsupportedFormat(
            "tournament hand histories are not yet supported".to_string(),
        ));
    }

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
    let played_at = format!("{}T{}", date.replace('/', "-"), time);

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
                seats.push(ParsedSeat {
                    seat_number: caps[1].parse().unwrap_or(0),
                    player_name: caps[2].to_string(),
                    starting_stack: parse_money(&caps[3]).unwrap_or(0.0),
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
                    let desc = strip_position_tag(desc);
                    let folded = desc.starts_with("folded");
                    let won = desc.contains("collected") || desc.contains("won (");
                    let went_to_showdown = has_showdown && !folded;
                    let won_at_showdown = went_to_showdown && won;
                    results.insert(
                        name.to_string(),
                        ParsedPlayerResult {
                            went_to_showdown,
                            won_at_showdown,
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

    Ok(ParsedHand {
        hand_id,
        site: "pokerstars".to_string(),
        table_name,
        max_seats,
        button_seat,
        game_type,
        small_blind,
        big_blind,
        currency,
        played_at,
        hero_name,
        seats,
        actions,
        results,
        raw_text: block.to_string(),
    })
}
