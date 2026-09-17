use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Street {
    Preflop,
    Flop,
    Turn,
    River,
}

impl Street {
    pub fn as_str(&self) -> &'static str {
        match self {
            Street::Preflop => "preflop",
            Street::Flop => "flop",
            Street::Turn => "turn",
            Street::River => "river",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionType {
    PostSmallBlind,
    PostBigBlind,
    PostAnte,
    Fold,
    Check,
    Call,
    Bet,
    Raise,
}

impl ActionType {
    pub fn as_str(&self) -> &'static str {
        match self {
            ActionType::PostSmallBlind => "post_small_blind",
            ActionType::PostBigBlind => "post_big_blind",
            ActionType::PostAnte => "post_ante",
            ActionType::Fold => "fold",
            ActionType::Check => "check",
            ActionType::Call => "call",
            ActionType::Bet => "bet",
            ActionType::Raise => "raise",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ParsedAction {
    pub street: Street,
    pub order: i64,
    pub player_name: String,
    pub action_type: ActionType,
    pub amount: Option<f64>,
    pub is_all_in: bool,
}

#[derive(Debug, Clone)]
pub struct ParsedSeat {
    pub seat_number: i64,
    pub player_name: String,
    pub starting_stack: f64,
    /// Table position (`BTN`, `SB`, `BB`, `UTG`, …) derived from the button
    /// seat and the ring of players actually dealt in — see
    /// [`crate::parser::position`]. `None` only when the ring could not be
    /// oriented (fewer than two players dealt in, or a button seat that is not
    /// one of them), never a guess.
    pub position: Option<String>,
}

/// Why a parsed `Seat` line did **not** become a dealt-in player.
///
/// PokerStars writes a seat line for everyone *sitting at* the table, which is
/// not the same as everyone *dealt into the hand*. Measured across the
/// user's 22 real hand-history files (1,599 seat lines): 28 seats were never
/// dealt in, and the seat-line marker alone does **not** identify them — 174 of
/// the 197 seats marked `is sitting out` were dealt in and played the hand
/// normally, because PokerStars writes that marker when the player's sit-out
/// flag is set at the time the hand is written, not when the hand is dealt.
///
/// The discriminator is therefore behavioural, not textual: a player was dealt
/// in if they took at least one action (including posting a blind or ante) or
/// carry a real description in the `*** SUMMARY ***` section. Those two signals
/// were checked independently against all 1,599 seat lines and agreed on every
/// one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkippedSeat {
    pub seat_number: i64,
    pub player_name: String,
    /// The trailing text after `in chips`, e.g. `") is sitting out"` or
    /// `") out of hand (moved from another table into small blind)"`. Recorded
    /// for diagnostics only — never used to decide whether the seat was dealt
    /// in.
    pub seat_line_marker: String,
}

#[derive(Debug, Clone, Default)]
pub struct ParsedPlayerResult {
    pub went_to_showdown: bool,
    pub won_at_showdown: bool,
    /// Net money result for this player in this hand (amount collected from
    /// the pot plus any uncalled bet returned, minus everything they put in).
    /// Only ever populated for cash-game hands — tournament hand-history text
    /// has no real-money figures to compute this from (chips aren't money),
    /// so this stays `None` for every tournament hand rather than reporting a
    /// fabricated or chip-denominated value.
    pub net_result: Option<f64>,
}

/// Cash-game stakes and tournament levels have different semantics (real/play
/// money per-hand stakes vs. a shared, escalating chip-count level shared by
/// the whole field) — callers must not assume `small_blind`/`big_blind` mean
/// "money" for a tournament hand.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandFormat {
    Cash,
    Tournament,
}

impl HandFormat {
    pub fn as_str(&self) -> &'static str {
        match self {
            HandFormat::Cash => "cash",
            HandFormat::Tournament => "tournament",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ParsedHand {
    pub hand_id: String,
    pub site: String,
    pub format: HandFormat,
    pub table_name: String,
    pub max_seats: i64,
    pub button_seat: i64,
    pub game_type: String,
    /// For tournaments this is the current level's blinds (chips, not
    /// money), not a cash-game stake.
    pub small_blind: f64,
    pub big_blind: f64,
    pub currency: String,
    /// Tournament-only metadata; `None` for cash hands.
    pub tournament_id: Option<String>,
    pub buy_in: Option<String>,
    pub level: Option<String>,
    pub played_at: String,
    pub hero_name: Option<String>,
    /// Only the players actually **dealt into** this hand. Seats present at the
    /// table but not in the hand are in [`ParsedHand::skipped_seats`].
    pub seats: Vec<ParsedSeat>,
    /// Seats parsed from the hand text that were not dealt in. Kept so the
    /// import path can report them rather than discard them silently.
    pub skipped_seats: Vec<SkippedSeat>,
    pub actions: Vec<ParsedAction>,
    pub results: HashMap<String, ParsedPlayerResult>,
    pub raw_text: String,
}

#[derive(Debug, PartialEq, Eq)]
pub enum ParseError {
    EmptyBlock,
    MissingHeader,
    UnsupportedFormat(String),
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ParseError::EmptyBlock => write!(f, "empty hand history block"),
            ParseError::MissingHeader => write!(f, "hand history block is missing a recognizable header"),
            ParseError::UnsupportedFormat(reason) => {
                write!(f, "unsupported hand history format: {reason}")
            }
        }
    }
}

impl std::error::Error for ParseError {}
