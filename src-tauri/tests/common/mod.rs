//! A PokerStars hand-history writer for engine tests that need more hands
//! than a hand-written fixture can carry (recency, tilt, pool, head-to-head).
//!
//! It renders the exact text PokerStars writes — header, seats, blinds,
//! actions with `raises X to Y` / `calls X and is all-in`, uncalled bets,
//! showdown and summary — and keeps every player's stack from one hand to
//! the next at a table, so consecutive starting stacks tell the truth.
//! Tests import the text through `import::import_text` like a real file.

#![allow(dead_code)]

use std::collections::HashMap;

use chrono::{Duration, NaiveDate};

pub const HERO: &str = "Hero";

const FLOP: &str = "Kh 7d 2c";
const TURN: &str = "4s";
const RIVER: &str = "9h";
/// Showdown cards: the winner holds the first, losers the next ones.
const SHOWN: [(&str, &str); 4] = [
    ("Ks Kd", "three of a kind, Kings"),
    ("Qc Qh", "a pair of Queens"),
    ("Jc Jh", "a pair of Jacks"),
    ("Tc Th", "a pair of Tens"),
];

/// One action; amounts are in big blinds.
#[derive(Debug, Clone, Copy)]
pub enum Act {
    Fold,
    Check,
    Call,
    Bet(f64),
    /// Raise to this street total.
    Raise(f64),
    /// Everything behind: a raise (or bet) all-in.
    AllIn,
}

pub type Line<'a> = (&'a str, Act);

#[derive(Debug, Clone)]
pub enum Kind {
    Cash,
    /// `level` is the Roman numeral of the header.
    Tournament { id: u64, buy_in: String, level: String },
    Spin { id: u64, buy_in: String, level: String },
}

/// One table: its seats, current stacks (chips) and the next hand number.
pub struct Table {
    pub kind: Kind,
    pub name: String,
    pub max: u32,
    pub sb: f64,
    pub bb: f64,
    pub seats: Vec<(u32, String)>,
    pub stacks: HashMap<String, f64>,
    pub bounties: HashMap<String, f64>,
    pub button: u32,
    pub next_id: u64,
}

impl Table {
    /// A $0.25/$0.50 6-max cash table, every stack 100bb.
    pub fn cash(name: &str, first_id: u64, players: &[&str]) -> Table {
        Table::new(Kind::Cash, name, first_id, 0.25, 0.5, players, 100.0)
    }

    /// A 100/200 6-max tournament table at level III, every stack `stack_bb`.
    pub fn mtt(name: &str, first_id: u64, players: &[&str], stack_bb: f64) -> Table {
        let kind = Kind::Tournament { id: 4_200_000_001, buy_in: "$10+$1".into(), level: "III".into() };
        Table::new(kind, name, first_id, 100.0, 200.0, players, stack_bb)
    }

    pub fn new(kind: Kind, name: &str, first_id: u64, sb: f64, bb: f64, players: &[&str], stack_bb: f64) -> Table {
        Table {
            kind,
            name: name.into(),
            max: 6,
            sb,
            bb,
            seats: players.iter().enumerate().map(|(i, p)| (i as u32 + 1, p.to_string())).collect(),
            stacks: players.iter().map(|p| (p.to_string(), stack_bb * bb)).collect(),
            bounties: HashMap::new(),
            button: 1,
            next_id: first_id,
        }
    }

    pub fn cash_game(&self) -> bool {
        matches!(self.kind, Kind::Cash)
    }

    /// A stack in big blinds.
    pub fn stack_bb(&self, name: &str) -> f64 {
        self.stacks[name] / self.bb
    }

    fn money(&self, chips: f64) -> String {
        if self.cash_game() {
            let text = format!("{chips:.2}");
            format!("${}", text.strip_suffix(".00").unwrap_or(&text))
        } else {
            format!("{}", chips.round() as i64)
        }
    }

    fn timestamp(minute: i64) -> String {
        let base = NaiveDate::from_ymd_opt(2026, 9, 10).unwrap().and_hms_opt(12, 0, 0).unwrap();
        (base + Duration::minutes(minute)).format("%Y/%m/%d %H:%M:%S").to_string()
    }

    fn header(&self, id: u64, minute: i64) -> String {
        let at = Table::timestamp(minute);
        match &self.kind {
            Kind::Cash => format!(
                "PokerStars Hand #{id}: Hold'em No Limit ({}/{} USD) - {at} ET",
                self.money(self.sb),
                self.money(self.bb)
            ),
            Kind::Tournament { id: t, buy_in, level } | Kind::Spin { id: t, buy_in, level } => format!(
                "PokerStars Hand #{id}: Tournament #{t}, {buy_in} USD Hold'em No Limit - Level {level} ({}/{}) - {at} ET",
                self.money(self.sb),
                self.money(self.bb)
            ),
        }
    }

    /// The occupied seats clockwise starting after `seat`.
    fn after(&self, seat: u32) -> Vec<(u32, String)> {
        let mut seats = self.seats.clone();
        seats.sort();
        let split = seats.iter().position(|(s, _)| *s > seat).unwrap_or(seats.len());
        seats.rotate_left(split);
        seats
    }

    /// Plays one hand and returns its text. `streets` holds the voluntary
    /// actions of preflop, flop, turn and river (blinds are posted
    /// automatically by the seats after the button). When more than one
    /// player is left after the last street, `winner` takes the pot at
    /// showdown.
    pub fn play(&mut self, minute: i64, streets: &[&[Line]], winner: Option<&str>) -> String {
        let id = self.next_id;
        self.next_id += 1;
        let order = self.after(self.button);
        let sb_name = order[0].1.clone();
        let bb_name = order[1].1.clone();
        let role = |name: &str| -> &'static str {
            if order.last().is_some_and(|(_, n)| n == name) {
                " (button)"
            } else if name == sb_name {
                " (small blind)"
            } else if name == bb_name {
                " (big blind)"
            } else {
                ""
            }
        };

        let mut out = vec![self.header(id, minute)];
        out.push(format!("Table '{}' {}-max Seat #{} is the button", self.name, self.max, self.button));
        let mut seats = self.seats.clone();
        seats.sort();
        for (seat, name) in &seats {
            let bounty = match self.bounties.get(name) {
                Some(b) => format!(", ${b:.2} bounty"),
                None => String::new(),
            };
            out.push(format!("Seat {seat}: {name} ({} in chips{bounty})", self.money(self.stacks[name])));
        }

        let mut invested: HashMap<String, f64> = HashMap::new();
        let mut street: HashMap<String, f64> = HashMap::new();
        let mut folded: Vec<String> = Vec::new();
        let mut folded_at: HashMap<String, &str> = HashMap::new();
        let behind = |name: &str, invested: &HashMap<String, f64>, stacks: &HashMap<String, f64>| {
            stacks[name] - invested.get(name).copied().unwrap_or(0.0)
        };
        for (name, amount, label) in [(&sb_name, self.sb, "small"), (&bb_name, self.bb, "big")] {
            out.push(format!("{name}: posts {label} blind {}", self.money(amount)));
            *invested.entry(name.clone()).or_default() += amount;
            *street.entry(name.clone()).or_default() += amount;
        }

        let active_at_end: Vec<String> =
            seats.iter().map(|(_, n)| n.clone()).collect();
        let showdown_names = |folded: &Vec<String>| -> Vec<String> {
            active_at_end.iter().filter(|n| !folded.contains(n)).cloned().collect()
        };
        // Cards: decided up front so the hero's `Dealt to` matches his show.
        let mut cards: HashMap<String, (&str, &str)> = HashMap::new();
        {
            let mut all_folds: Vec<String> = Vec::new();
            for actions in streets {
                for (name, act) in actions.iter() {
                    if matches!(act, Act::Fold) {
                        all_folds.push(name.to_string());
                    }
                }
            }
            let mut contenders = showdown_names(&all_folds);
            if contenders.len() > 1 {
                let w = winner.expect("a showdown needs a winner").to_string();
                contenders.retain(|n| *n != w);
                cards.insert(w, SHOWN[0]);
                for (i, n) in contenders.into_iter().enumerate() {
                    cards.insert(n, SHOWN[i + 1]);
                }
            }
        }
        out.push("*** HOLE CARDS ***".into());
        let hero_cards = cards.get(HERO).map(|c| c.0).unwrap_or("7c 2d");
        if self.seats.iter().any(|(_, n)| n == HERO) {
            out.push(format!("Dealt to {HERO} [{hero_cards}]"));
        }

        let boards = [
            String::new(),
            format!("*** FLOP *** [{FLOP}]"),
            format!("*** TURN *** [{FLOP}] [{TURN}]"),
            format!("*** RIVER *** [{FLOP} {TURN}] [{RIVER}]"),
        ];
        let street_names = ["before Flop", "on the Flop", "on the Turn", "on the River"];
        let mut last_street = 0;
        for (s, actions) in streets.iter().enumerate() {
            if s > 0 {
                if actions.is_empty() {
                    continue;
                }
                street.clear();
                out.push(boards[s].clone());
                last_street = s;
            }
            for (name, act) in actions.iter() {
                let name = name.to_string();
                let max = street.values().copied().fold(0.0, f64::max);
                let mine = street.get(&name).copied().unwrap_or(0.0);
                let left = behind(&name, &invested, &self.stacks);
                let (text, put) = match *act {
                    Act::Fold => {
                        folded.push(name.clone());
                        folded_at.insert(name.clone(), street_names[s]);
                        ("folds".to_string(), 0.0)
                    }
                    Act::Check => ("checks".to_string(), 0.0),
                    Act::Call => {
                        let put = (max - mine).min(left);
                        let all_in = if put >= left { " and is all-in" } else { "" };
                        (format!("calls {}{all_in}", self.money(put)), put)
                    }
                    Act::Bet(bb) => {
                        let put = bb * self.bb;
                        (format!("bets {}", self.money(put)), put)
                    }
                    Act::Raise(to_bb) => {
                        let to = to_bb * self.bb;
                        (format!("raises {} to {}", self.money(to - max), self.money(to)), to - mine)
                    }
                    Act::AllIn => {
                        let to = mine + left;
                        if max == 0.0 {
                            (format!("bets {} and is all-in", self.money(left)), left)
                        } else {
                            (
                                format!("raises {} to {} and is all-in", self.money(to - max), self.money(to)),
                                left,
                            )
                        }
                    }
                };
                *invested.entry(name.clone()).or_default() += put;
                *street.entry(name.clone()).or_default() += put;
                out.push(format!("{name}: {text}"));
            }
        }

        let remaining = showdown_names(&folded);
        let mut pot: f64 = invested.values().sum();
        let mut collected: HashMap<String, f64> = HashMap::new();
        if remaining.len() == 1 {
            let lone = &remaining[0];
            let mine = street.get(lone).copied().unwrap_or(0.0);
            let others = street.iter().filter(|(n, _)| *n != lone).map(|(_, v)| *v).fold(0.0, f64::max);
            if mine > others {
                let back = mine - others;
                out.push(format!("Uncalled bet ({}) returned to {lone}", self.money(back)));
                *invested.get_mut(lone).unwrap() -= back;
                pot -= back;
            }
            out.push(format!("{lone} collected {} from pot", self.money(pot)));
            collected.insert(lone.clone(), pot);
        } else {
            out.push("*** SHOW DOWN ***".into());
            for name in &remaining {
                let (c, desc) = cards[name];
                out.push(format!("{name}: shows [{c}] ({desc})"));
            }
            let w = winner.unwrap().to_string();
            out.push(format!("{w} collected {} from pot", self.money(pot)));
            collected.insert(w, pot);
        }

        out.push("*** SUMMARY ***".into());
        out.push(format!("Total pot {} | Rake {}", self.money(pot), self.money(0.0)));
        let board = match last_street {
            0 => None,
            1 => Some(FLOP.to_string()),
            2 => Some(format!("{FLOP} {TURN}")),
            _ => Some(format!("{FLOP} {TURN} {RIVER}")),
        };
        if let Some(board) = board {
            out.push(format!("Board [{board}]"));
        }
        for (seat, name) in &seats {
            let tag = role(name);
            let line = if let Some(at) = folded_at.get(name) {
                format!("folded {at}")
            } else if remaining.len() > 1 {
                let (c, desc) = cards[name];
                match collected.get(name) {
                    Some(won) => format!("showed [{c}] and won ({}) with {desc}", self.money(*won)),
                    None => format!("showed [{c}] and lost with {desc}"),
                }
            } else if let Some(won) = collected.get(name) {
                format!("collected ({})", self.money(*won))
            } else {
                "folded before Flop (didn't bet)".to_string()
            };
            out.push(format!("Seat {seat}: {name}{tag} {line}"));
        }

        for (name, put) in &invested {
            *self.stacks.get_mut(name).unwrap() -= put;
        }
        for (name, won) in &collected {
            *self.stacks.get_mut(name).unwrap() += won;
        }
        out.push(String::new());
        out.push(String::new());
        out.join("\n")
    }

    /// Everyone folds to the big blind.
    pub fn walk(&mut self, minute: i64) -> String {
        let order = self.after(self.button);
        let folders: Vec<String> = order[2..].iter().chain(order[..1].iter()).map(|(_, n)| n.clone()).collect();
        let lines: Vec<Line> = folders.iter().map(|n| (n.as_str(), Act::Fold)).collect();
        self.play(minute, &[&lines], None)
    }

    /// `opener` raises to 2.5bb when folded to; everyone else folds.
    pub fn open_and_take(&mut self, minute: i64, opener: &str) -> String {
        let order = self.after(self.button);
        let preflop: Vec<String> = order[2..].iter().chain(order[..2].iter()).map(|(_, n)| n.clone()).collect();
        let at = preflop.iter().position(|n| n == opener).expect("opener seated");
        let mut lines: Vec<Line> = Vec::new();
        for (i, n) in preflop.iter().enumerate() {
            lines.push((n.as_str(), if i == at { Act::Raise(2.5) } else { Act::Fold }));
        }
        self.play(minute, &[&lines], None)
    }
}

/// Imports generated text into a fresh in-memory database.
pub fn import(text: &str) -> rusqlite::Connection {
    let mut conn = velora_poker_lib::db::open(std::path::Path::new(":memory:")).expect("open db");
    velora_poker_lib::import::import_text(&mut conn, text).expect("import generated hands");
    conn
}

pub fn player_id(conn: &rusqlite::Connection, name: &str) -> i64 {
    conn.query_row("SELECT id FROM players WHERE name = ?1", [name], |row| row.get(0))
        .unwrap_or_else(|_| panic!("player {name} not found"))
}
