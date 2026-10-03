//! The window contract of `CLAUDE.md`, checked against the files that
//! define it rather than against a running app:
//!
//! - every window label the app can have is covered by the default
//!   capability, or that window's event `listen` is silently denied (D63);
//! - the side panel is a static window that is only shown or hidden;
//! - `WebviewWindowBuilder` is used only on `overlay::manager`'s own thread,
//!   and the one `destroy` is the overlay manager's documented one.

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;

fn manifest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read_json(relative: &str) -> Value {
    let path = manifest_dir().join(relative);
    let text = fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("parse {}: {e}", path.display()))
}

/// Every window declared in `tauri.conf.json`. A window without a `label`
/// is Tauri's default, `main`.
fn declared_windows() -> Vec<Value> {
    read_json("tauri.conf.json")["app"]["windows"]
        .as_array()
        .expect("app.windows is an array")
        .clone()
}

fn label_of(window: &Value) -> String {
    window["label"].as_str().unwrap_or("main").to_string()
}

fn capability_windows() -> Vec<String> {
    read_json("capabilities/default.json")["windows"]
        .as_array()
        .expect("capability windows is an array")
        .iter()
        .map(|w| w.as_str().expect("capability window is a string").to_string())
        .collect()
}

/// Tauri's capability window patterns: a literal label, or a glob where `*`
/// matches any run of characters.
fn glob_matches(pattern: &str, label: &str) -> bool {
    let parts: Vec<&str> = pattern.split('*').collect();
    if parts.len() == 1 {
        return pattern == label;
    }
    let (first, last) = (parts[0], parts[parts.len() - 1]);
    if !label.starts_with(first) || label.len() < first.len() + last.len() || !label.ends_with(last) {
        return false;
    }
    let mut rest = &label[first.len()..label.len() - last.len()];
    for middle in &parts[1..parts.len() - 1] {
        match rest.find(middle) {
            Some(at) => rest = &rest[at + middle.len()..],
            None => return false,
        }
    }
    true
}

fn covered(label: &str, patterns: &[String]) -> bool {
    patterns.iter().any(|p| glob_matches(p, label))
}

#[test]
fn glob_matching_follows_literal_and_star_patterns() {
    assert!(glob_matches("main", "main"));
    assert!(!glob_matches("main", "main2"));
    assert!(glob_matches("overlay*", "overlay"));
    assert!(glob_matches("overlay*", "overlay12"));
    assert!(!glob_matches("overlay*", "panel"));
    assert!(!glob_matches("panel", "panel2"));
    assert!(glob_matches("a*b*c", "a-x-b-y-c"));
    assert!(!glob_matches("a*b*c", "a-x-c"));
}

#[test]
fn every_declared_window_label_is_covered_by_the_default_capability() {
    let patterns = capability_windows();
    let labels: Vec<String> = declared_windows().iter().map(label_of).collect();
    for expected in ["main", "overlay", "panel"] {
        assert!(labels.iter().any(|l| l == expected), "tauri.conf.json declares '{expected}': {labels:?}");
    }
    for label in &labels {
        assert!(covered(label, &patterns), "window '{label}' is not covered by capabilities/default.json {patterns:?}");
    }
}

#[test]
fn runtime_overlay_labels_are_covered_by_the_default_capability() {
    // `overlay::overlay_label` names pooled windows `overlay<slot>`; with
    // 6-12 tables there are at least as many slots.
    let patterns = capability_windows();
    for slot in 1..=24 {
        let label = format!("overlay{slot}");
        assert!(covered(&label, &patterns), "runtime label '{label}' is not covered by {patterns:?}");
    }
}

#[test]
fn the_default_capability_lists_exactly_main_overlays_and_the_panel() {
    assert_eq!(capability_windows(), vec!["main", "overlay*", "panel"]);
}

#[test]
fn the_side_panel_is_a_hidden_decorated_opaque_normal_window() {
    let windows = declared_windows();
    let panel = windows.iter().find(|w| label_of(w) == "panel").expect("a 'panel' window is declared");
    assert_eq!(panel["url"], "panel.html");
    assert_eq!(panel["visible"], false, "hidden at start; shown only by show_side_panel");
    assert_eq!(panel["decorations"], true);
    assert_ne!(panel["transparent"], true, "opaque");
    assert_ne!(panel["alwaysOnTop"], true, "a normal window, not an overlay");
    assert_ne!(panel["resizable"], false, "resizable");
    assert!(panel["minWidth"].as_f64().is_some_and(|w| w > 0.0), "has a minimum width");
    assert!(panel["minHeight"].as_f64().is_some_and(|h| h > 0.0), "has a minimum height");
    assert_ne!(panel["skipTaskbar"], true, "reachable from the taskbar on the second monitor");
}

#[test]
fn the_overlay_prototype_stays_declared_and_never_shown() {
    let windows = declared_windows();
    let overlay = windows.iter().find(|w| label_of(w) == "overlay").expect("the 'overlay' prototype is declared");
    assert_eq!(overlay["visible"], false);
    assert_eq!(overlay["url"], "overlay.html");
}

fn rust_sources(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).unwrap_or_else(|e| panic!("read {}: {e}", dir.display())) {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            rust_sources(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
}

/// The `src/` files containing `needle`, as `/`-separated paths relative to `src/`.
fn sources_containing(needle: &str) -> Vec<String> {
    let src = manifest_dir().join("src");
    let mut files = Vec::new();
    rust_sources(&src, &mut files);
    assert!(files.len() > 10, "found the Rust sources under {}", src.display());
    let mut hits: Vec<String> = files
        .iter()
        .filter(|f| fs::read_to_string(f).expect("read source").contains(needle))
        .map(|f| f.strip_prefix(&src).unwrap().to_string_lossy().replace('\\', "/"))
        .collect();
    hits.sort();
    hits
}

#[test]
fn webview_window_builder_appears_only_in_the_overlay_manager() {
    assert_eq!(sources_containing("WebviewWindowBuilder"), vec!["overlay/manager.rs"]);
}

#[test]
fn the_only_window_destroy_is_the_overlay_managers() {
    assert_eq!(sources_containing(".destroy("), vec!["overlay/manager.rs"]);
}

#[test]
fn the_side_panel_close_request_is_turned_into_a_hide() {
    let lib = fs::read_to_string(manifest_dir().join("src/lib.rs")).expect("read lib.rs");
    let at = lib.find("SIDE_PANEL_LABEL").expect("lib.rs wires the side panel window");
    let wiring = &lib[at..];
    let close = wiring.find("CloseRequested").expect("intercepts the panel's close request");
    let prevent = wiring.find("prevent_close()").expect("prevents the close");
    let hide = wiring.find(".hide()").expect("hides the panel instead");
    assert!(close < prevent && prevent < hide, "close request -> prevent_close -> hide");
}
