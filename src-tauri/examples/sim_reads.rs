//! Dev-only reader for the simulator's e2e driver (`scripts/sim/e2e.mjs`):
//! opens a database the real app filled during a simulated session,
//! **read-only**, and prints, as one JSON line per simulated villain and
//! format pool he played, the chip tag and the ranked reads the HUD shows for
//! him at his latest hand of that format (the same `EngineCache::payload` the
//! overlay uses), with the profile's expected reads from `scripts/sim/profiles.json` and whether they match.
//! Never part of the app or its bundle.
//!
//! Usage: `cargo run --example sim_reads --features strategic-analysis -- <velora.db> <profiles.json>`

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [db_path, profiles_path] = args.as_slice() else {
        eprintln!("usage: sim_reads <velora.db> <profiles.json>");
        return ExitCode::from(2);
    };
    match run(db_path, profiles_path) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("sim_reads: {err}");
            ExitCode::from(1)
        }
    }
}

#[cfg(not(feature = "strategic-analysis"))]
fn run(_db_path: &str, _profiles_path: &str) -> Result<(), String> {
    Err("built without the strategic-analysis feature: there are no reads".into())
}

#[cfg(feature = "strategic-analysis")]
fn run(db_path: &str, profiles_path: &str) -> Result<(), String> {
    use rusqlite::{Connection, OpenFlags, OptionalExtension};
    use serde_json::{json, Value};
    use velora_poker_lib::engine::{load_hand, villain_context, EngineCache, HandFacts};

    let conn = Connection::open_with_flags(db_path, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX)
        .map_err(|e| format!("open {db_path}: {e}"))?;
    let profiles: Value = serde_json::from_str(&std::fs::read_to_string(profiles_path).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    let mut cache = EngineCache::new();
    for profile in profiles["profiles"].as_array().ok_or("profiles.json: no profiles")? {
        for name in profile["players"].as_array().ok_or("profile without players")? {
            let name = name.as_str().ok_or("player name")?;
            let Some(pid) = conn
                .query_row("SELECT id FROM players WHERE name = ?1", [name], |row| row.get::<_, i64>(0))
                .optional()
                .map_err(|e| e.to_string())?
            else {
                println!("{}", json!({ "player": name, "profile": profile["id"], "hands": 0 }));
                continue;
            };
            let hands: i64 = conn
                .query_row("SELECT COUNT(*) FROM player_hands WHERE player_id = ?1", [pid], |row| row.get(0))
                .map_err(|e| e.to_string())?;
            // His latest hand at every table he sat at; per format pool, the
            // latest of those is the context the overlay reads him in.
            let mut latest: Vec<(String, HandFacts)> = Vec::new();
            let mut stmt = conn
                .prepare(
                    "SELECT MAX(h.id) FROM hands h JOIN player_hands p ON p.hand_id = h.id
                      WHERE p.player_id = ?1 GROUP BY h.table_name",
                )
                .map_err(|e| e.to_string())?;
            let ids: Vec<i64> = stmt
                .query_map([pid], |row| row.get(0))
                .and_then(|rows| rows.collect())
                .map_err(|e| e.to_string())?;
            for id in ids {
                let Some(hand) = load_hand(&conn, id).map_err(|e| e.to_string())? else { continue };
                let Some(context) = villain_context(&hand, pid) else { continue };
                let format = format!("{:?}", context.format);
                let newer = |held: &HandFacts| (&hand.played_at, hand.id) > (&held.played_at, held.id);
                match latest.iter_mut().find(|(f, _)| *f == format) {
                    Some(slot) if newer(&slot.1) => slot.1 = hand,
                    Some(_) => {}
                    None => latest.push((format, hand)),
                }
            }
            latest.sort_by(|a, b| a.0.cmp(&b.0));
            for (format, hand) in latest {
                let context = villain_context(&hand, pid);
                let payload = cache.payload(&conn, pid, context).map_err(|e| e.to_string())?;
                let reads: Vec<Value> = payload
                    .reads
                    .iter()
                    .take(4)
                    .map(|r| json!({ "ruleId": r.rule_id, "tag": r.tag, "score": (r.score * 100.0).round() / 100.0 }))
                    .collect();
                // Facts of the latest hand (stack depth, his bounty) are not reads
                // of the player and carry no sample (opponent-engine.md, section 7):
                // the comparison takes the first tendency read below them.
                let top = payload
                    .reads
                    .iter()
                    .find(|r| !r.rule_id.starts_with("ctx.") && r.rule_id != "ko.big_bounty");
                let tournament = matches!(format.as_str(), "Mtt" | "Spin");
                let expected = if tournament && profile["tournament"]["expected"].is_object() {
                    &profile["tournament"]["expected"]
                } else {
                    &profile["expected"]
                };
                let matches = top.is_some_and(|top| {
                    expected["reads"]
                        .as_array()
                        .is_some_and(|reads| reads.iter().any(|r| r["ruleId"].as_str() == Some(top.rule_id.as_str())))
                });
                println!(
                    "{}",
                    json!({
                        "player": name,
                        "profile": profile["id"],
                        "hands": hands,
                        "table": hand.table_name,
                        "format": format,
                        "chip": payload.tag.as_ref().map(|t| &t.text),
                        "top": top.map(|r| json!({ "ruleId": r.rule_id, "tag": r.tag, "sample": r.sample })),
                        "reads": reads,
                        "expected": expected["reads"].as_array().map(|reads| reads.iter().map(|r| &r["ruleId"]).collect::<Vec<_>>()),
                        "during": expected["during"],
                        "matches": matches,
                    })
                );
            }
        }
    }
    Ok(())
}
