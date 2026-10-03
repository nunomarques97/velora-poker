//! Opponent engine, task T9: auto-notes (catalogue row N01, section 11 of
//! `docs/specs/opponent-engine.md`).
//!
//! Fixture: `engine_auto_notes.txt`. Cash table 'Notes I' ($0.25/$0.50,
//! 100bb) seats Hero (button), Sam (SB), Bob (BB), Vic (UTG), Otto (HJ) and
//! Tess (CO); tournament table '4100000009 1' (100/200) seats Hero, Sam,
//! Bob (BB), Ugo (UTG, 12bb), Hal and Cole (40bb). One positive and at
//! least one negative hand per detector:
//! - 901: Vic open-limps, Tess raises, Vic re-raises and shows AA (note);
//!   902: Vic limp-calls with AA; 903: Vic limp-reraises 7c 6c.
//! - 904: Otto overbets the river ($8 into $6.25) with a missed draw (note);
//!   905: the same overbet with two pair; 906: a small river bet bluff.
//! - 907: Vic 4-bets K-J suited and shows it (note); 908: Vic 4-bets KK.
//! - 909: Bob only calls flop and turn with trip sevens (note); 910: Bob
//!   check-raises the flop with them; 911: Bob calls down with K-Q.
//! - 912: Ugo open-shoves 12bb with 8-3 offsuit (note); 913: the same shove
//!   with A-4; 914: Cole shoves 40bb with 8-3 offsuit.

use std::path::Path;

use rusqlite::Connection;
use velora_poker_lib::engine::{
    detect_hand, load_hand, parse_board, AutoNoteKind, AutoNotePayload, DetectedNote, NoteSource,
};
use velora_poker_lib::{db, import};

const NOTES: &str = include_str!("fixtures/engine_auto_notes.txt");
const SHOWDOWNS: &str = include_str!("fixtures/engine_showdown_cash.txt");

fn import_db(text: &str) -> Connection {
    let mut conn = db::open(Path::new(":memory:")).expect("open db");
    import::import_text(&mut conn, text).expect("import fixture");
    conn
}

fn player_id(conn: &Connection, name: &str) -> i64 {
    conn.query_row("SELECT id FROM players WHERE name = ?1", [name], |row| row.get(0))
        .unwrap_or_else(|_| panic!("player {name} not found"))
}

fn name_of(conn: &Connection, id: i64) -> String {
    conn.query_row("SELECT name FROM players WHERE id = ?1", [id], |row| row.get(0)).unwrap()
}

/// The detectors' notes for one stored hand, read back like the generator
/// reads it: the stored facts plus the `Board [..]` line.
fn detected(conn: &Connection, hand_ref: &str) -> Vec<DetectedNote> {
    let (row_id, raw_text): (i64, String) = conn
        .query_row("SELECT id, raw_text FROM hands WHERE hand_id = ?1", [hand_ref], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })
        .unwrap_or_else(|_| panic!("hand {hand_ref} not stored"));
    let hand = load_hand(conn, row_id).unwrap().expect("hand loads");
    let board = parse_board(&raw_text);
    detect_hand(&hand, board.as_deref())
}

/// `(player, kind, text)` of every note a hand yields.
fn notes_of(conn: &Connection, hand_ref: &str) -> Vec<(String, AutoNoteKind, String)> {
    detected(conn, hand_ref)
        .into_iter()
        .map(|n| (name_of(conn, n.player_id), n.kind, n.text))
        .collect()
}

/// `(player, hand number, kind, text)` of every stored auto-note, by hand.
fn stored(conn: &Connection) -> Vec<(String, String, String, String)> {
    let mut stmt = conn
        .prepare(
            "SELECT p.name, h.hand_id, n.kind, n.text
             FROM player_auto_notes n
             JOIN players p ON p.id = n.player_id
             JOIN hands h ON h.id = n.hand_id
             ORDER BY h.hand_id, n.kind",
        )
        .unwrap();
    let rows = stmt
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)))
        .unwrap();
    rows.collect::<Result<_, _>>().unwrap()
}

fn auto_note_count(conn: &Connection) -> i64 {
    conn.query_row("SELECT COUNT(*) FROM player_auto_notes", [], |row| row.get(0)).unwrap()
}

// ------------------------------------------------------------ detectors

#[test]
fn auto_note_limp_reraise_premium_detected() {
    let conn = import_db(NOTES);
    assert_eq!(
        notes_of(&conn, "300000000901"),
        [(
            "Vic".to_string(),
            AutoNoteKind::LimpReraisePremium,
            "Hand #300000000901: limp-reraised preflop and showed Ah As.".to_string()
        )]
    );
    assert_eq!(AutoNoteKind::LimpReraisePremium.rule_id(), "an.limp_reraise_premium");
}

#[test]
fn auto_note_river_overbet_bluff_detected() {
    let conn = import_db(NOTES);
    // $8 into a $6.25 pot.
    assert_eq!(
        notes_of(&conn, "300000000904"),
        [(
            "Otto".to_string(),
            AutoNoteKind::RiverOverbetBluff,
            "Hand #300000000904: overbet the river (1.3× pot) and showed 6h 5h.".to_string()
        )]
    );
    assert_eq!(AutoNoteKind::RiverOverbetBluff.rule_id(), "an.river_overbet_bluff");
}

#[test]
fn auto_note_light_4bet_shown_detected() {
    let conn = import_db(NOTES);
    assert_eq!(
        notes_of(&conn, "300000000907"),
        [(
            "Vic".to_string(),
            AutoNoteKind::Light4betShown,
            "Hand #300000000907: re-raised a re-raise preflop and showed Kc Jc.".to_string()
        )]
    );
}

#[test]
fn auto_note_slowplay_monster_detected() {
    let conn = import_db(NOTES);
    assert_eq!(
        notes_of(&conn, "300000000909"),
        [(
            "Bob".to_string(),
            AutoNoteKind::SlowplayMonster,
            "Hand #300000000909: only called flop and turn with trips.".to_string()
        )]
    );
}

#[test]
fn auto_note_weak_shove_shown_detected() {
    let conn = import_db(NOTES);
    assert_eq!(
        notes_of(&conn, "300000000912"),
        [(
            "Ugo".to_string(),
            AutoNoteKind::WeakShoveShown,
            "Hand #300000000912: shoved 12bb and showed 8c 3d.".to_string()
        )]
    );
}

#[test]
fn auto_note_detectors_ignore_non_matching_hands() {
    let conn = import_db(NOTES);
    for (hand_ref, why) in [
        ("300000000902", "limp-call with AA is not a limp-reraise"),
        ("300000000903", "limp-reraise shown with 7c 6c is not premium"),
        ("300000000905", "river overbet shown with two pair is value"),
        ("300000000906", "a small river bet is not an overbet"),
        ("300000000908", "a 4-bet with KK is not light"),
        ("300000000910", "a flop check-raise is not a slowplay"),
        ("300000000911", "two pair with the board pair is not trips with a hole card"),
        ("300000000913", "a shove with an ace is not weak"),
        ("300000000914", "a 40bb shove is not a short-stack shove"),
    ] {
        assert_eq!(notes_of(&conn, hand_ref), [], "{hand_ref}: {why}");
    }
    // Only shown villain cards count: the hero's dealt cards never yield a
    // note, and hands without shown villain cards yield nothing.
    let hero = player_id(&conn, "Hero");
    let hand_refs: Vec<String> = conn
        .prepare("SELECT hand_id FROM hands ORDER BY id")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(hand_refs.len(), 14);
    for hand_ref in &hand_refs {
        assert!(detected(&conn, hand_ref).iter().all(|n| n.player_id != hero), "{hand_ref}");
    }
}

#[test]
fn auto_note_detectors_need_shown_cards() {
    // Showdown fixture 509: Vic shows after winning uncontested on the flop;
    // 510: Vic checks down and mucks unseen. Neither has a qualifying line.
    let conn = import_db(SHOWDOWNS);
    for hand_ref in ["300000000509", "300000000510"] {
        assert_eq!(notes_of(&conn, hand_ref), [], "{hand_ref}");
    }
    // 504–506: Otto's three river overbets with missed hands are all found.
    for hand_ref in ["300000000504", "300000000505", "300000000506"] {
        let kinds: Vec<AutoNoteKind> = detected(&conn, hand_ref).iter().map(|n| n.kind).collect();
        assert_eq!(kinds, [AutoNoteKind::RiverOverbetBluff], "{hand_ref}");
    }
}

// ------------------------------------------------------------ storage

#[test]
fn auto_note_kinds_and_rule_ids_follow_the_spec() {
    let kinds: Vec<(&str, &str)> = AutoNoteKind::ALL.iter().map(|k| (k.as_str(), k.rule_id())).collect();
    assert_eq!(
        kinds,
        [
            ("limp_reraise_premium", "an.limp_reraise_premium"),
            ("river_overbet_bluff", "an.river_overbet_bluff"),
            ("light_4bet_shown", "an.light_4bet_shown"),
            ("slowplay_monster", "an.slowplay_monster"),
            ("weak_shove_shown", "an.weak_shove_shown"),
        ]
    );
}

#[test]
fn auto_note_serialises_with_source_auto() {
    let note = AutoNotePayload {
        id: 7,
        hand_id: "300000000904".into(),
        kind: "river_overbet_bluff".into(),
        text: "Hand #300000000904: overbet the river (1.3× pot) and showed 6h 5h.".into(),
        created_at: "2026-10-03T12:00:00Z".into(),
        source: NoteSource::Auto,
    };
    let v = serde_json::to_value(&note).unwrap();
    let mut keys: Vec<&str> = v.as_object().unwrap().keys().map(String::as_str).collect();
    keys.sort_unstable();
    assert_eq!(keys, ["createdAt", "handId", "id", "kind", "source", "text"]);
    assert_eq!(v["source"], "auto");
    assert_eq!(v["handId"], "300000000904");
}

#[test]
fn auto_notes_table_is_created_idempotently_with_its_unique_key() {
    let conn = import_db(NOTES);
    let vic = player_id(&conn, "Vic");
    let hand: i64 = conn
        .query_row("SELECT id FROM hands WHERE hand_id = '300000000902'", [], |row| row.get(0))
        .unwrap();
    let before = auto_note_count(&conn);
    assert!(db::insert_auto_note(&conn, vic, hand, "light_4bet_shown", "first").unwrap());
    // Same player, hand and kind: ignored, the first text kept.
    assert!(!db::insert_auto_note(&conn, vic, hand, "light_4bet_shown", "second").unwrap());
    // Another kind for the same hand is a separate note.
    assert!(db::insert_auto_note(&conn, vic, hand, "weak_shove_shown", "third").unwrap());
    assert_eq!(auto_note_count(&conn), before + 2);
    let texts: Vec<String> = db::list_auto_notes(&conn, vic)
        .unwrap()
        .into_iter()
        .filter(|n| n.hand_ref == "300000000902")
        .map(|n| n.text)
        .collect();
    assert_eq!(texts, ["first", "third"]);
}

#[test]
fn manual_notes_untouched_by_auto_notes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("velora.db");
    {
        let mut conn = db::open(&path).unwrap();
        import::import_text(&mut conn, NOTES).unwrap();
        db::set_player_note(&conn, player_id(&conn, "Vic"), "3-bets light from the blinds").unwrap();
        db::set_player_note(&conn, player_id(&conn, "Otto"), "tilts after losing big pots").unwrap();
        // Re-import and a full detector pass over every hand.
        import::import_text(&mut conn, NOTES).unwrap();
        velora_poker_lib::engine::generate_auto_notes_after(&conn, 0).unwrap();
    }
    // Reopen: every migration and the flag-guarded backfill run again.
    let conn = db::open(&path).unwrap();
    let manual: Vec<(String, String)> = conn
        .prepare("SELECT p.name, n.note FROM player_notes n JOIN players p ON p.id = n.player_id ORDER BY p.name")
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(
        manual,
        [
            ("Otto".to_string(), "tilts after losing big pots".to_string()),
            ("Vic".to_string(), "3-bets light from the blinds".to_string()),
        ]
    );
    assert_eq!(
        db::get_player_note(&conn, player_id(&conn, "Vic")).unwrap().as_deref(),
        Some("3-bets light from the blinds")
    );
}

// ------------------------------------------------------- generation gate

const EXPECTED: [(&str, &str, &str); 5] = [
    ("Vic", "300000000901", "limp_reraise_premium"),
    ("Otto", "300000000904", "river_overbet_bluff"),
    ("Vic", "300000000907", "light_4bet_shown"),
    ("Bob", "300000000909", "slowplay_monster"),
    ("Ugo", "300000000912", "weak_shove_shown"),
];

fn stored_keys(conn: &Connection) -> Vec<(String, String, String)> {
    stored(conn).into_iter().map(|(p, h, k, _)| (p, h, k)).collect()
}

fn expected_keys() -> Vec<(String, String, String)> {
    EXPECTED.iter().map(|(p, h, k)| (p.to_string(), h.to_string(), k.to_string())).collect()
}

#[cfg(feature = "strategic-analysis")]
#[test]
fn import_generates_auto_notes_for_new_hands() {
    let conn = import_db(NOTES);
    assert_eq!(stored_keys(&conn), expected_keys());
    let texts: Vec<String> = stored(&conn).into_iter().map(|(_, _, _, t)| t).collect();
    assert_eq!(texts[0], "Hand #300000000901: limp-reraised preflop and showed Ah As.");
}

#[cfg(feature = "strategic-analysis")]
#[test]
fn reimport_creates_no_duplicate_auto_notes() {
    let mut conn = import_db(NOTES);
    let ids: Vec<i64> = conn
        .prepare("SELECT id FROM player_auto_notes ORDER BY id")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(ids.len(), 5);
    let summary = import::import_text(&mut conn, NOTES).unwrap();
    assert_eq!(summary.hands_imported, 0);
    // A full pass over every stored hand adds nothing either.
    assert_eq!(velora_poker_lib::engine::generate_auto_notes_after(&conn, 0).unwrap(), 0);
    let after: Vec<i64> = conn
        .prepare("SELECT id FROM player_auto_notes ORDER BY id")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(after, ids);
    assert_eq!(stored_keys(&conn), expected_keys());
}

#[cfg(feature = "strategic-analysis")]
#[test]
fn import_scans_only_the_hands_it_adds() {
    let mut conn = import_db(NOTES);
    // Pretend the fixture's notes predate this feature: an import of other
    // hands must not reach back and regenerate them.
    conn.execute("DELETE FROM player_auto_notes", []).unwrap();
    let summary = import::import_text(&mut conn, SHOWDOWNS).unwrap();
    assert_eq!(summary.hands_imported, 10);
    let keys = stored_keys(&conn);
    assert_eq!(
        keys,
        ["300000000504", "300000000505", "300000000506"]
            .iter()
            .map(|h| ("Otto".to_string(), h.to_string(), "river_overbet_bluff".to_string()))
            .collect::<Vec<_>>()
    );
}

#[cfg(feature = "strategic-analysis")]
#[test]
fn auto_note_backfill_runs_once_behind_its_flag() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("velora.db");
    {
        let mut conn = db::open(&path).unwrap();
        assert_eq!(db::get_setting(&conn, db::AUTO_NOTES_BACKFILL_FLAG).unwrap().as_deref(), Some("true"));
        import::import_text(&mut conn, NOTES).unwrap();
        // A database whose hands were stored before auto-notes existed: no
        // notes and no flag.
        conn.execute("DELETE FROM player_auto_notes", []).unwrap();
        conn.execute("DELETE FROM settings WHERE key = ?1", rusqlite::params![db::AUTO_NOTES_BACKFILL_FLAG]).unwrap();
    }
    {
        let conn = db::open(&path).unwrap();
        assert_eq!(stored_keys(&conn), expected_keys(), "backfill on first open");
        assert_eq!(db::get_setting(&conn, db::AUTO_NOTES_BACKFILL_FLAG).unwrap().as_deref(), Some("true"));
        conn.execute("DELETE FROM player_auto_notes", []).unwrap();
    }
    // Flag set: the backfill does not run again.
    let conn = db::open(&path).unwrap();
    assert_eq!(auto_note_count(&conn), 0);
}

#[cfg(feature = "strategic-analysis")]
#[test]
fn engine_payload_carries_auto_notes_newest_first() {
    let conn = import_db(NOTES);
    let mut cache = velora_poker_lib::engine::EngineCache::new();
    let vic = player_id(&conn, "Vic");
    let payload = cache.payload(&conn, vic, None).unwrap();
    let notes: Vec<(&str, &str)> =
        payload.auto_notes.iter().map(|n| (n.hand_id.as_str(), n.kind.as_str())).collect();
    assert_eq!(notes, [("300000000907", "light_4bet_shown"), ("300000000901", "limp_reraise_premium")]);
    assert!(payload.auto_notes.iter().all(|n| n.source == NoteSource::Auto));
    let v = serde_json::to_value(&payload).unwrap();
    assert_eq!(v["autoNotes"][0]["source"], "auto");
    // A player without notes carries an empty array.
    let tess = player_id(&conn, "Tess");
    assert!(cache.payload(&conn, tess, None).unwrap().auto_notes.is_empty());
}

#[cfg(not(feature = "strategic-analysis"))]
#[test]
fn default_build_generates_no_auto_notes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("velora.db");
    let mut conn = db::open(&path).unwrap();
    import::import_text(&mut conn, NOTES).unwrap();
    assert_eq!(stored_keys(&conn), []);
    // The backfill neither ran nor marked itself done, so a later strategic
    // build still backfills.
    drop(conn);
    let conn = db::open(&path).unwrap();
    assert_eq!(auto_note_count(&conn), 0);
    assert_eq!(db::get_setting(&conn, db::AUTO_NOTES_BACKFILL_FLAG).unwrap(), None);
    // The detectors themselves still compile and run (pure functions).
    assert_eq!(notes_of(&conn, "300000000901").len(), 1);
    assert_eq!(expected_keys().len(), EXPECTED.len());
}
