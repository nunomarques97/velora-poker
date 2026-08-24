use std::collections::HashMap;

use rusqlite::{params, Connection};
use serde::Serialize;

/// Calculated statistics for a single player, aggregated from every hand
/// currently stored locally. All percentages are rounded to one decimal
/// place; percentages with a zero-opportunity denominator report as 0.0
/// rather than NaN.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayerStats {
    pub vpip: f64,
    pub pfr: f64,
    pub three_bet: f64,
    pub fold_to_three_bet: f64,
    pub c_bet: f64,
    pub fold_to_c_bet: f64,
    pub aggression_factor: f64,
    pub wtsd: f64,
    pub wsd: f64,
}

struct ActionRow {
    player_id: i64,
    street: String,
    action_type: String,
}

#[derive(Default)]
struct Accumulator {
    hands: i64,
    vpip_hands: i64,
    pfr_hands: i64,
    three_bet_opportunities: i64,
    three_bets: i64,
    faced_3bet_opportunities: i64,
    folded_to_3bet: i64,
    cbet_opportunities: i64,
    cbets: i64,
    faced_cbet_opportunities: i64,
    folded_to_cbet: i64,
    postflop_bets_raises: i64,
    postflop_calls: i64,
    saw_flop_hands: i64,
    went_to_showdown_hands: i64,
    won_at_showdown_hands: i64,
}

pub fn compute_player_stats(conn: &Connection, player_id: i64) -> rusqlite::Result<PlayerStats> {
    let mut acc = Accumulator::default();

    {
        let mut stmt = conn.prepare(
            "SELECT went_to_showdown, won_at_showdown FROM player_hands WHERE player_id = ?1",
        )?;
        let rows = stmt.query_map(params![player_id], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?))
        })?;
        for row in rows {
            let (wtsd, wsd) = row?;
            acc.hands += 1;
            if wtsd != 0 {
                acc.went_to_showdown_hands += 1;
            }
            if wsd != 0 {
                acc.won_at_showdown_hands += 1;
            }
        }
    }

    if acc.hands == 0 {
        return Ok(finalize(&acc));
    }

    let mut stmt = conn.prepare(
        "SELECT a.hand_id, a.player_id, a.street, a.action_type
         FROM actions a
         WHERE a.hand_id IN (SELECT hand_id FROM player_hands WHERE player_id = ?1)
         ORDER BY a.hand_id,
           CASE a.street WHEN 'preflop' THEN 0 WHEN 'flop' THEN 1 WHEN 'turn' THEN 2 WHEN 'river' THEN 3 ELSE 4 END,
           a.action_index",
    )?;

    let mut hands: HashMap<i64, Vec<ActionRow>> = HashMap::new();
    let rows = stmt.query_map(params![player_id], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            ActionRow {
                player_id: row.get(1)?,
                street: row.get(2)?,
                action_type: row.get(3)?,
            },
        ))
    })?;
    for row in rows {
        let (hand_id, action) = row?;
        hands.entry(hand_id).or_default().push(action);
    }

    for actions in hands.values() {
        evaluate_hand(actions, player_id, &mut acc);
    }

    Ok(finalize(&acc))
}

fn evaluate_hand(actions: &[ActionRow], player_id: i64, acc: &mut Accumulator) {
    let mut raise_count = 0i64;
    let mut player_vpip = false;
    let mut player_pfr = false;
    let mut was_opener = false;
    let mut facing_3bet = false;
    let mut last_preflop_raiser: Option<i64> = None;

    for action in actions.iter().filter(|a| a.street == "preflop") {
        let is_us = action.player_id == player_id;

        if is_us && raise_count == 1 {
            acc.three_bet_opportunities += 1;
        }
        if is_us && facing_3bet {
            acc.faced_3bet_opportunities += 1;
            if action.action_type == "fold" {
                acc.folded_to_3bet += 1;
            }
            facing_3bet = false;
        }

        match action.action_type.as_str() {
            "call" | "bet" => {
                if is_us {
                    player_vpip = true;
                }
            }
            "raise" => {
                if is_us {
                    player_vpip = true;
                    player_pfr = true;
                    if raise_count == 1 {
                        acc.three_bets += 1;
                    }
                    if raise_count == 0 {
                        was_opener = true;
                    }
                } else if was_opener && raise_count == 1 {
                    facing_3bet = true;
                }
                raise_count += 1;
                last_preflop_raiser = Some(action.player_id);
            }
            _ => {}
        }
    }

    if player_vpip {
        acc.vpip_hands += 1;
    }
    if player_pfr {
        acc.pfr_hands += 1;
    }

    let flop_actions: Vec<&ActionRow> = actions.iter().filter(|a| a.street == "flop").collect();
    let saw_flop = flop_actions.iter().any(|a| a.player_id == player_id);
    if saw_flop {
        acc.saw_flop_hands += 1;

        if last_preflop_raiser == Some(player_id) {
            acc.cbet_opportunities += 1;
            if let Some(first_own) = flop_actions.iter().find(|a| a.player_id == player_id) {
                if first_own.action_type == "bet" {
                    acc.cbets += 1;
                }
            }
        }

        if let Some(bet_pos) = flop_actions.iter().position(|a| a.action_type == "bet") {
            let first_bet = flop_actions[bet_pos];
            if Some(first_bet.player_id) == last_preflop_raiser && first_bet.player_id != player_id
            {
                if let Some(our_response) = flop_actions[bet_pos + 1..]
                    .iter()
                    .find(|a| a.player_id == player_id)
                {
                    acc.faced_cbet_opportunities += 1;
                    if our_response.action_type == "fold" {
                        acc.folded_to_cbet += 1;
                    }
                }
            }
        }
    }

    for action in actions
        .iter()
        .filter(|a| a.player_id == player_id && a.street != "preflop")
    {
        match action.action_type.as_str() {
            "bet" | "raise" => acc.postflop_bets_raises += 1,
            "call" => acc.postflop_calls += 1,
            _ => {}
        }
    }
}

fn pct(numerator: i64, denominator: i64) -> f64 {
    if denominator == 0 {
        0.0
    } else {
        round1(numerator as f64 / denominator as f64 * 100.0)
    }
}

fn round1(value: f64) -> f64 {
    (value * 10.0).round() / 10.0
}

fn finalize(acc: &Accumulator) -> PlayerStats {
    let aggression_factor = if acc.postflop_calls > 0 {
        round1(acc.postflop_bets_raises as f64 / acc.postflop_calls as f64)
    } else {
        round1(acc.postflop_bets_raises as f64)
    };

    PlayerStats {
        vpip: pct(acc.vpip_hands, acc.hands),
        pfr: pct(acc.pfr_hands, acc.hands),
        three_bet: pct(acc.three_bets, acc.three_bet_opportunities),
        fold_to_three_bet: pct(acc.folded_to_3bet, acc.faced_3bet_opportunities),
        c_bet: pct(acc.cbets, acc.cbet_opportunities),
        fold_to_c_bet: pct(acc.folded_to_cbet, acc.faced_cbet_opportunities),
        aggression_factor,
        wtsd: pct(acc.went_to_showdown_hands, acc.saw_flop_hands),
        wsd: pct(acc.won_at_showdown_hands, acc.went_to_showdown_hands),
    }
}
