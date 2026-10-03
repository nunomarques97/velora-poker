//! Dev-only database seeder for the simulator's e2e driver
//! (`scripts/sim/e2e.mjs`): creates a fresh Velora database through the
//! app's own migrations (`db::open`) and marks onboarding complete for
//! PokerStars with the simulator's hand-history folder, so the real app
//! starts straight into a simulated session. Never part of the app or its
//! bundle.
//!
//! Usage: `cargo run --example sim_seed -- <new velora.db> <hand-history folder>`
//!
//! The driver only ever passes paths inside its own temporary root; this
//! program additionally refuses an existing database, so it can never alter
//! one that is in use.

use std::path::Path;
use std::process::ExitCode;

use velora_poker_lib::{db, settings};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [db_path, hand_history_dir] = args.as_slice() else {
        eprintln!("usage: sim_seed <new velora.db> <hand-history folder>");
        return ExitCode::from(2);
    };
    let db_path = Path::new(db_path);
    if db_path.exists() {
        eprintln!("sim_seed: refusing to touch an existing database: {}", db_path.display());
        return ExitCode::from(1);
    }
    if !Path::new(hand_history_dir).is_dir() {
        eprintln!("sim_seed: hand-history folder not found: {hand_history_dir}");
        return ExitCode::from(1);
    }
    let seeded = db::open(db_path).and_then(|conn| {
        db::set_setting(&conn, settings::SETTING_POKER_ROOM, "pokerstars")?;
        db::set_setting(&conn, settings::SETTING_HAND_HISTORY_DIR, hand_history_dir)?;
        db::set_setting(&conn, settings::SETTING_ONBOARDING_COMPLETE, "true")
    });
    match seeded {
        Ok(()) => {
            println!("sim_seed: seeded {}", db_path.display());
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("sim_seed: {err}");
            ExitCode::from(1)
        }
    }
}
