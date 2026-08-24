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
}

#[derive(Debug, Clone, Default)]
pub struct ParsedPlayerResult {
    pub went_to_showdown: bool,
    pub won_at_showdown: bool,
}

#[derive(Debug, Clone)]
pub struct ParsedHand {
    pub hand_id: String,
    pub site: String,
    pub table_name: String,
    pub max_seats: i64,
    pub button_seat: i64,
    pub game_type: String,
    pub small_blind: f64,
    pub big_blind: f64,
    pub currency: String,
    pub played_at: String,
    pub hero_name: Option<String>,
    pub seats: Vec<ParsedSeat>,
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
