use std::collections::HashMap;

use rusqlite::{params, Connection};
use serde::Serialize;

/// Calculated statistics for a single player, aggregated from every hand
/// currently stored locally. All percentages are rounded to one decimal
/// place; a stat whose denominator is zero (no qualifying opportunity ever
/// occurred) reports `None` rather than a fabricated 0.0.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayerStats {
    pub vpip: Option<f64>,
    pub pfr: Option<f64>,
    pub three_bet: Option<f64>,
    pub fold_to_three_bet: Option<f64>,
    pub four_bet: Option<f64>,
    pub fold_to_four_bet: Option<f64>,
    pub rfi: Option<f64>,
    pub limp: Option<f64>,
    pub cold_call: Option<f64>,
    pub squeeze: Option<f64>,
    pub fold_to_squeeze: Option<f64>,
    pub c_bet: Option<f64>,
    pub fold_to_c_bet: Option<f64>,
    pub aggression_factor: Option<f64>,
    pub wtsd: Option<f64>,
    pub wsd: Option<f64>,
    pub steal_attempt: Option<f64>,
    pub fold_to_steal: Option<f64>,
}

/// Raw opportunity counts behind `PlayerStats`'s percentages — the
/// denominators `Accumulator` already tracks internally but `PlayerStats`
/// never exposed. The description-rule engine needs these directly: its
/// confidence formula is `opportunities / 200`, using the specific
/// denominator named per rule, never `hands` as a fallback. Added as a
/// companion struct rather than folded into `PlayerStats` so every existing
/// consumer of that struct's shape is unaffected.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayerStatsOpportunities {
    pub hands: i64,
    pub three_bet_opportunities: i64,
    pub faced_3bet_opportunities: i64,
    pub four_bet_opportunities: i64,
    pub faced_4bet_opportunities: i64,
    pub rfi_limp_opportunities: i64,
    pub cold_call_opportunities: i64,
    pub squeeze_opportunities: i64,
    pub faced_squeeze_opportunities: i64,
    pub cbet_opportunities: i64,
    pub faced_cbet_opportunities: i64,
    pub saw_flop_hands: i64,
    pub went_to_showdown_hands: i64,
    pub postflop_calls: i64,
    pub postflop_bets_raises: i64,
    pub steal_attempt_opportunities: i64,
    pub fold_to_steal_opportunities: i64,
}

struct ActionRow {
    player_id: i64,
    street: String,
    action_type: String,
    is_all_in: bool,
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
    four_bet_opportunities: i64,
    four_bets: i64,
    faced_4bet_opportunities: i64,
    folded_to_4bet: i64,
    rfi_limp_opportunities: i64,
    raises_first_in: i64,
    limps: i64,
    cold_call_opportunities: i64,
    cold_calls: i64,
    squeeze_opportunities: i64,
    squeezes: i64,
    faced_squeeze_opportunities: i64,
    folded_to_squeeze: i64,
    cbet_opportunities: i64,
    cbets: i64,
    faced_cbet_opportunities: i64,
    folded_to_cbet: i64,
    postflop_bets_raises: i64,
    postflop_calls: i64,
    saw_flop_hands: i64,
    went_to_showdown_hands: i64,
    won_at_showdown_hands: i64,
    steal_attempt_opportunities: i64,
    steal_attempts: i64,
    fold_to_steal_opportunities: i64,
    folded_to_steal: i64,
}

/// A player is in a steal position if
/// their `player_hands.position` label is `CO`, `BTN`, or `SB` — the seats
/// from which an unanswered open is conventionally read as attacking the
/// blinds rather than a standard open. `SB` counts even though it's also a
/// blind; what matters is the seat a steal is stolen *from*, not blind status.
fn is_steal_position(position: &str) -> bool {
    matches!(position, "CO" | "BTN" | "SB")
}

pub fn compute_player_stats(conn: &Connection, player_id: i64) -> rusqlite::Result<PlayerStats> {
    Ok(finalize(&compute_accumulator(conn, player_id)?))
}

/// Same computation as `compute_player_stats`, also returning the raw
/// opportunity counts behind each percentage. Kept as a separate function
/// (rather than changing `compute_player_stats`'s return type) so its
/// existing callers/tests are unaffected.
pub fn compute_player_stats_with_opportunities(
    conn: &Connection,
    player_id: i64,
) -> rusqlite::Result<(PlayerStats, PlayerStatsOpportunities)> {
    let acc = compute_accumulator(conn, player_id)?;
    Ok((finalize(&acc), opportunities(&acc)))
}

fn compute_accumulator(conn: &Connection, player_id: i64) -> rusqlite::Result<Accumulator> {
    let mut acc = Accumulator::default();
    let mut showdown_by_hand: HashMap<i64, bool> = HashMap::new();
    // The scored player's own position per hand — a wider SELECT on the
    // query this function already runs, not a new query.
    let mut position_by_hand: HashMap<i64, String> = HashMap::new();

    {
        let mut stmt = conn.prepare(
            "SELECT hand_id, went_to_showdown, won_at_showdown, position FROM player_hands WHERE player_id = ?1",
        )?;
        let rows = stmt.query_map(params![player_id], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, Option<String>>(3)?,
            ))
        })?;
        for row in rows {
            let (hand_id, wtsd, wsd, position) = row?;
            acc.hands += 1;
            let went_to_showdown = wtsd != 0;
            if went_to_showdown {
                acc.went_to_showdown_hands += 1;
            }
            if wsd != 0 {
                acc.won_at_showdown_hands += 1;
            }
            showdown_by_hand.insert(hand_id, went_to_showdown);
            if let Some(pos) = position {
                position_by_hand.insert(hand_id, pos);
            }
        }
    }

    if acc.hands == 0 {
        return Ok(acc);
    }

    let mut stmt = conn.prepare(
        "SELECT a.hand_id, a.player_id, a.street, a.action_type, a.is_all_in
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
                is_all_in: row.get::<_, i64>(4)? != 0,
            },
        ))
    })?;
    for row in rows {
        let (hand_id, action) = row?;
        hands.entry(hand_id).or_default().push(action);
    }

    // Every player's position for each of these hands — the one genuinely
    // new piece of plumbing this feature needs, beyond widening a query
    // already run above: `fold_to_steal%`'s opportunity depends on the
    // *raiser's* position, and the raiser is frequently not the player being
    // scored.
    let mut hand_positions: HashMap<i64, HashMap<i64, String>> = HashMap::new();
    {
        let mut pos_stmt = conn.prepare(
            "SELECT hand_id, player_id, position FROM player_hands
             WHERE hand_id IN (SELECT hand_id FROM player_hands WHERE player_id = ?1)",
        )?;
        let pos_rows = pos_stmt.query_map(params![player_id], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, Option<String>>(2)?,
            ))
        })?;
        for row in pos_rows {
            let (hand_id, pid, position) = row?;
            if let Some(pos) = position {
                hand_positions.entry(hand_id).or_default().insert(pid, pos);
            }
        }
    }

    let empty_positions: HashMap<i64, String> = HashMap::new();
    for (hand_id, actions) in hands.iter() {
        let went_to_showdown = showdown_by_hand.get(hand_id).copied().unwrap_or(false);
        let own_position = position_by_hand.get(hand_id).map(|s| s.as_str());
        let positions = hand_positions.get(hand_id).unwrap_or(&empty_positions);
        evaluate_hand(
            actions,
            player_id,
            went_to_showdown,
            own_position,
            positions,
            &mut acc,
        );
    }

    Ok(acc)
}

fn evaluate_hand(
    actions: &[ActionRow],
    player_id: i64,
    went_to_showdown: bool,
    own_position: Option<&str>,
    positions: &HashMap<i64, String>,
    acc: &mut Accumulator,
) {
    let mut raise_count = 0i64;
    let mut player_vpip = false;
    let mut player_pfr = false;
    let mut was_opener = false;
    let mut was_3better = false;
    let mut facing_3bet = false;
    let mut facing_4bet = false;
    let mut player_all_in_preflop = false;
    let mut last_preflop_raiser: Option<i64> = None;
    // True once anyone has voluntarily called/bet/raised preflop — posting a
    // blind/ante doesn't count, and folds don't either, so this stays false
    // through any number of leading folds. RFI%/Limp%'s opportunity is "my
    // turn arrives and this is still false"; distinct from `raise_count`
    // because a limp (a call, not a raise) also closes the window without
    // moving `raise_count`.
    let mut entered_pot = false;
    // Per-player counterpart to `entered_pot`: true once *this* player has
    // voluntarily called/bet/raised preflop, regardless of what anyone else
    // has done. Cold-call%'s opportunity is "my first voluntary action is a
    // response to a raise that's already happened" — `entered_pot` alone
    // can't express that, since it flips true the moment *any* player (not
    // necessarily us) raises, and by definition a cold-call spot only exists
    // once someone else has raised.
    let mut we_acted_voluntarily = false;
    // True from the moment of a raise until the next raise, once at least one
    // player has called in between — i.e. "has this raising level seen a flat
    // caller yet". Reset to false the instant a new raise happens (that raise
    // hasn't been called by anyone yet), and set true by a call that isn't
    // itself the very first voluntary entry (`entered_pot` already true).
    // This is what turns a plain re-raise into a squeeze: a direct 3-bet with
    // no caller in between never sees this flip true.
    let mut called_since_last_raise = false;
    // True for exactly one player at a time: the most recent raiser, once
    // their raise has been called by someone and then re-raised over. Mirrors
    // `facing_3bet`/`facing_4bet`'s one-shot consume-on-next-action shape, but
    // is independent of them — a squeeze can and does co-occur with a direct
    // 3bet/4bet-facing flag on the very same re-raise action, since they read
    // the same event two different ways.
    let mut facing_squeeze = false;
    // True exactly while the most recent raise (the one that brought
    // `raise_count` to its current value) was itself a genuine steal attempt
    // — a first-in raise (`!entered_pot` at the moment it was made) from a
    // steal position. Re-derived at every raise, not just the first: an
    // isolation raise over a limp from a steal position must NOT set this
    // (its raiser already had `entered_pot == true` against them, the same
    // reason it's excluded from `steal_attempt_opportunities`), and any
    // second raise — whether or not it's itself a steal — invalidates a
    // pending steal from an earlier raise, since `fold_to_steal%` requires
    // *exactly* one live raise (mirrors `three_bet_opportunities`'s own
    // `raise_count == 1` framing, just tracked as its own flag because
    // "was this raise unanswered when it happened" isn't reconstructable
    // from `raise_count`/`last_preflop_raiser` alone).
    let mut current_raise_is_steal = false;

    for action in actions.iter().filter(|a| a.street == "preflop") {
        let is_us = action.player_id == player_id;

        if is_us && action.is_all_in && action.action_type != "fold" {
            player_all_in_preflop = true;
        }

        // A player's own `post_ante`/`post_small_blind`/`post_big_blind` row
        // is always their first row in the hand, at a point where nobody has
        // voluntarily acted yet (`entered_pot` is still false) — without
        // `is_post` here, that forced post itself would wrongly register as
        // an RFI/limp opportunity every single hand, for every player who
        // posts, since posts don't set `entered_pot` and so never protect
        // themselves from the check below.
        let is_post = matches!(
            action.action_type.as_str(),
            "post_ante" | "post_small_blind" | "post_big_blind"
        );
        if is_us && !entered_pot && !is_post {
            acc.rfi_limp_opportunities += 1;
            match action.action_type.as_str() {
                "raise" => acc.raises_first_in += 1,
                "call" => acc.limps += 1,
                _ => {}
            }
            // steal_attempt%'s opportunity is this exact RFI/Limp gate,
            // narrowed by the player's own position — a strict subset, not a
            // parallel mechanism.
            if own_position.is_some_and(is_steal_position) {
                acc.steal_attempt_opportunities += 1;
                if action.action_type == "raise" {
                    acc.steal_attempts += 1;
                }
            }
        }
        if is_us && raise_count == 1 {
            acc.three_bet_opportunities += 1;
        }
        // fold_to_steal%'s opportunity: a player in SB/BB, at the moment
        // they face exactly one raise (freshly re-checked here, the same
        // `raise_count == 1` spot `three_bet_opportunities` already reads,
        // rather than a sticky flag — so a later re-raise before this
        // player's turn naturally stops matching) that was itself a genuine
        // steal (`current_raise_is_steal`, not just "raised from a steal
        // position" — an isolation raise over a limp from CO/BTN/SB must not
        // count, mirroring steal_attempt%'s own `!entered_pot` exclusion),
        // made by someone other than themselves.
        if is_us
            && raise_count == 1
            && current_raise_is_steal
            && matches!(own_position, Some("SB") | Some("BB"))
        {
            if let Some(raiser_id) = last_preflop_raiser {
                if raiser_id != player_id {
                    acc.fold_to_steal_opportunities += 1;
                    if action.action_type == "fold" {
                        acc.folded_to_steal += 1;
                    }
                }
            }
        }
        if is_us && facing_3bet {
            acc.faced_3bet_opportunities += 1;
            if action.action_type == "fold" {
                acc.folded_to_3bet += 1;
            }
            facing_3bet = false;
        }
        if is_us && raise_count == 2 {
            acc.four_bet_opportunities += 1;
        }
        if is_us && facing_4bet {
            acc.faced_4bet_opportunities += 1;
            if action.action_type == "fold" {
                acc.folded_to_4bet += 1;
            }
            facing_4bet = false;
        }
        if is_us && facing_squeeze {
            acc.faced_squeeze_opportunities += 1;
            if action.action_type == "fold" {
                acc.folded_to_squeeze += 1;
            }
            facing_squeeze = false;
        }
        if is_us && !we_acted_voluntarily && raise_count >= 1 {
            acc.cold_call_opportunities += 1;
            if action.action_type == "call" {
                acc.cold_calls += 1;
            }
            if called_since_last_raise {
                acc.squeeze_opportunities += 1;
                if action.action_type == "raise" {
                    acc.squeezes += 1;
                }
            }
        }

        match action.action_type.as_str() {
            "call" => {
                if entered_pot {
                    called_since_last_raise = true;
                }
                if is_us {
                    player_vpip = true;
                    we_acted_voluntarily = true;
                }
                entered_pot = true;
            }
            "bet" => {
                if is_us {
                    player_vpip = true;
                    we_acted_voluntarily = true;
                }
                entered_pot = true;
            }
            "raise" => {
                if !is_us && last_preflop_raiser == Some(player_id) && called_since_last_raise {
                    facing_squeeze = true;
                }
                // Capture "was this raise unanswered" (a genuine steal
                // candidate) before `entered_pot` flips true below — mirrors
                // the same `!entered_pot` read `steal_attempt_opportunities`
                // already does for this player's own turn, here read for
                // whoever is raising.
                let is_first_in_raise = !entered_pot;
                current_raise_is_steal = is_first_in_raise
                    && positions
                        .get(&action.player_id)
                        .is_some_and(|p| is_steal_position(p));
                entered_pot = true;
                if is_us {
                    player_vpip = true;
                    player_pfr = true;
                    we_acted_voluntarily = true;
                    if raise_count == 1 {
                        acc.three_bets += 1;
                    }
                    if raise_count == 2 {
                        acc.four_bets += 1;
                    }
                    if raise_count == 0 {
                        was_opener = true;
                    }
                    if raise_count == 1 {
                        was_3better = true;
                    }
                } else if was_opener && raise_count == 1 {
                    facing_3bet = true;
                } else if was_3better && raise_count == 2 {
                    facing_4bet = true;
                }
                called_since_last_raise = false;
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
    // A player who shoved all-in preflop and was called is still in the hand
    // when the flop is dealt but is never required to act again, so they have
    // no flop-street action row of their own — without this they'd wrongly be
    // excluded from the WTSD denominator despite reaching showdown. But an
    // all-in preflop that gets folded to never sees a flop dealt at all, and
    // looks identical here (zero flop-street rows for anyone) — `went_to_showdown`
    // is what discriminates the two: no flop, no showdown, ever.
    let saw_flop = flop_actions.iter().any(|a| a.player_id == player_id)
        || (player_all_in_preflop && went_to_showdown);
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

fn pct(numerator: i64, denominator: i64) -> Option<f64> {
    if denominator == 0 {
        None
    } else {
        Some(round1(numerator as f64 / denominator as f64 * 100.0))
    }
}

fn round1(value: f64) -> f64 {
    (value * 10.0).round() / 10.0
}

fn aggression_factor(bets_raises: i64, calls: i64) -> Option<f64> {
    if calls == 0 {
        None
    } else {
        Some(round1(bets_raises as f64 / calls as f64))
    }
}

fn opportunities(acc: &Accumulator) -> PlayerStatsOpportunities {
    PlayerStatsOpportunities {
        hands: acc.hands,
        three_bet_opportunities: acc.three_bet_opportunities,
        faced_3bet_opportunities: acc.faced_3bet_opportunities,
        four_bet_opportunities: acc.four_bet_opportunities,
        faced_4bet_opportunities: acc.faced_4bet_opportunities,
        rfi_limp_opportunities: acc.rfi_limp_opportunities,
        cold_call_opportunities: acc.cold_call_opportunities,
        squeeze_opportunities: acc.squeeze_opportunities,
        faced_squeeze_opportunities: acc.faced_squeeze_opportunities,
        cbet_opportunities: acc.cbet_opportunities,
        faced_cbet_opportunities: acc.faced_cbet_opportunities,
        saw_flop_hands: acc.saw_flop_hands,
        went_to_showdown_hands: acc.went_to_showdown_hands,
        postflop_calls: acc.postflop_calls,
        postflop_bets_raises: acc.postflop_bets_raises,
        steal_attempt_opportunities: acc.steal_attempt_opportunities,
        fold_to_steal_opportunities: acc.fold_to_steal_opportunities,
    }
}

fn finalize(acc: &Accumulator) -> PlayerStats {
    let aggression_factor = aggression_factor(acc.postflop_bets_raises, acc.postflop_calls);

    PlayerStats {
        vpip: pct(acc.vpip_hands, acc.hands),
        pfr: pct(acc.pfr_hands, acc.hands),
        three_bet: pct(acc.three_bets, acc.three_bet_opportunities),
        fold_to_three_bet: pct(acc.folded_to_3bet, acc.faced_3bet_opportunities),
        four_bet: pct(acc.four_bets, acc.four_bet_opportunities),
        fold_to_four_bet: pct(acc.folded_to_4bet, acc.faced_4bet_opportunities),
        rfi: pct(acc.raises_first_in, acc.rfi_limp_opportunities),
        limp: pct(acc.limps, acc.rfi_limp_opportunities),
        cold_call: pct(acc.cold_calls, acc.cold_call_opportunities),
        squeeze: pct(acc.squeezes, acc.squeeze_opportunities),
        fold_to_squeeze: pct(acc.folded_to_squeeze, acc.faced_squeeze_opportunities),
        c_bet: pct(acc.cbets, acc.cbet_opportunities),
        fold_to_c_bet: pct(acc.folded_to_cbet, acc.faced_cbet_opportunities),
        aggression_factor,
        wtsd: pct(acc.went_to_showdown_hands, acc.saw_flop_hands),
        wsd: pct(acc.won_at_showdown_hands, acc.went_to_showdown_hands),
        steal_attempt: pct(acc.steal_attempts, acc.steal_attempt_opportunities),
        fold_to_steal: pct(acc.folded_to_steal, acc.fold_to_steal_opportunities),
    }
}
