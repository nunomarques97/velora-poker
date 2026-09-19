//! The reconcile *decisions* of `overlay::manager`, with none of its effects.
//!
//! `manager` owns everything that touches a native window: building a webview,
//! `navigate`, show/hide, hit-testing. None of that can run under `cargo test`
//! — there is no event loop, no WebView2, and the module's own header records
//! why calling those APIs from the wrong thread deadlocks Windows. So the only
//! verification the pooling logic ever had was a human opening and closing real
//! PokerStars tables, which is slow, and which found the create/destroy
//! deadlock only after it had already shipped.
//!
//! This module is the half of that logic which is just arithmetic on a list:
//! which tables should have a HUD, which pool slot each one gets, whether that
//! means building a window or re-pointing an idle one, and which windows are
//! released. `plan_reconcile` reads a snapshot and returns what to do; `manager`
//! does it. Nothing here calls Win32, WebView2 or Tauri, so all of it is
//! exercised by plain unit tests against fake table lists.
//!
//! Deliberately still out of scope, and still covered by the manual method:
//! whether a built window actually paints, positions itself over its table,
//! passes clicks through, or survives churn without wedging the event loop.

use super::overlay_label;

/// One pooled overlay window as the planner sees it: its stable window label
/// and the table it currently serves, if any.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlotState {
    pub label: String,
    pub table_id: Option<u32>,
}

impl SlotState {
    /// Test-only: a pooled window that is alive but serving nobody.
    #[cfg(test)]
    pub fn idle(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            table_id: None,
        }
    }

    pub fn serving(label: impl Into<String>, table_id: u32) -> Self {
        Self {
            label: label.into(),
            table_id: Some(table_id),
        }
    }
}

/// What one wanted table gets out of a reconciliation pass.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Assignment {
    /// Already has this window; only its bounds need re-syncing.
    Sync { label: String, table_id: u32 },
    /// Takes an idle pooled window: re-point it at this table's URL.
    Repoint { label: String, table_id: u32 },
    /// Gets the pass's single new window.
    Create { label: String, table_id: u32 },
    /// No idle window and the pass's one build is already spent — next pass.
    Defer { table_id: u32 },
}

impl Assignment {
    pub fn table_id(&self) -> u32 {
        match self {
            Assignment::Sync { table_id, .. }
            | Assignment::Repoint { table_id, .. }
            | Assignment::Create { table_id, .. }
            | Assignment::Defer { table_id } => *table_id,
        }
    }
}

/// A whole reconciliation pass, decided up front.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ReconcilePlan {
    /// Windows whose table is gone: hide them and mark their slot idle. Applied
    /// before any assignment, so a table that just closed frees its window for
    /// one that just opened instead of forcing a build.
    pub released: Vec<String>,
    /// One entry per wanted table, in the order the tables are tracked.
    pub assignments: Vec<Assignment>,
    /// What the slot counter must hold afterwards. Monotonic on purpose: see
    /// `next_slot_never_reuses_a_dropped_slots_number`.
    pub next_slot: u32,
}

impl ReconcilePlan {
    /// The label of the one window this pass builds, if it builds one.
    /// Test-only: `manager` reads the label off the `Create` assignment.
    #[cfg(test)]
    pub fn created_label(&self) -> Option<&str> {
        self.assignments.iter().find_map(|a| match a {
            Assignment::Create { label, .. } => Some(label.as_str()),
            _ => None,
        })
    }
}

/// The tables that should have a *window* right now: every tracked table,
/// unless the global kill switch is off (then none).
///
/// A by-hand per-table dismissal does not appear here. It used to
/// (`dismissed: &[u32]`, filtered out like the kill switch), which meant
/// "Hide" released the actual window — and a released window can't render a
/// "Show" button of its own, which is exactly why bringing the HUD back
/// required leaving the overlay for the main window or a hotkey. The window
/// now always stays up for a wanted table; `overlay::OverlayApp` (frontend)
/// decides on its own whether to paint the full HUD or a small "Show" pill
/// in the same spot, from the dismissed flag `overlay::manager` still tracks
/// and broadcasts — see `manager::is_dismissed`.
///
/// Minimized tables stay wanted — they keep their window, which is hidden while
/// they are minimized and comes back with them, rather than being released and
/// rebuilt on every minimize.
pub fn wanted_table_ids(tracked: &[u32], enabled: bool) -> Vec<u32> {
    if !enabled {
        return Vec::new();
    }
    tracked.to_vec()
}

/// Decides a whole reconciliation pass from a snapshot of the pool and the
/// list of tables that should have a HUD.
///
/// Two rules carry the weight here, and both come from measured failures:
///
/// * **At most one window is built per pass.** Rapid webview creation is what
///   wedged the event loop — creating a webview runs a nested message
///   pump, so a burst of builds re-enters window management inside window
///   management. Extra tables are deferred to later passes, a poll tick apart.
/// * **An idle pooled window is always preferred to a new one**, which is what
///   keeps a session of opening and closing tables from building windows
///   without bound.
pub fn plan_reconcile(slots: &[SlotState], wanted: &[u32], next_slot: u32) -> ReconcilePlan {
    // Working copy of the pool: released slots become idle here first, so the
    // assign pass below can hand them straight to a table.
    let mut pool: Vec<SlotState> = slots.to_vec();

    let mut released = Vec::new();
    for slot in pool.iter_mut() {
        if let Some(id) = slot.table_id {
            if !wanted.contains(&id) {
                slot.table_id = None;
                released.push(slot.label.clone());
            }
        }
    }

    let mut next_slot = next_slot;
    let mut built_one = false;
    let mut assignments = Vec::with_capacity(wanted.len());

    for &table_id in wanted {
        if let Some(slot) = pool.iter().find(|s| s.table_id == Some(table_id)) {
            assignments.push(Assignment::Sync {
                label: slot.label.clone(),
                table_id,
            });
            continue;
        }

        match pool.iter_mut().find(|s| s.table_id.is_none()) {
            Some(slot) => {
                slot.table_id = Some(table_id);
                assignments.push(Assignment::Repoint {
                    label: slot.label.clone(),
                    table_id,
                });
            }
            None if !built_one => {
                built_one = true;
                let label = overlay_label(next_slot);
                next_slot += 1;
                pool.push(SlotState::serving(label.clone(), table_id));
                assignments.push(Assignment::Create { label, table_id });
            }
            None => assignments.push(Assignment::Defer { table_id }),
        }
    }

    ReconcilePlan {
        released,
        assignments,
        next_slot,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labels(plan: &ReconcilePlan) -> Vec<&str> {
        plan.assignments
            .iter()
            .map(|a| match a {
                Assignment::Sync { label, .. }
                | Assignment::Repoint { label, .. }
                | Assignment::Create { label, .. } => label.as_str(),
                Assignment::Defer { .. } => "-",
            })
            .collect()
    }

    fn creates(plan: &ReconcilePlan) -> usize {
        plan.assignments
            .iter()
            .filter(|a| matches!(a, Assignment::Create { .. }))
            .count()
    }

    // -----------------------------------------------------------------
    // Which tables want a HUD
    // -----------------------------------------------------------------

    #[test]
    fn every_tracked_table_wants_a_hud_by_default() {
        assert_eq!(wanted_table_ids(&[1, 2, 3], true), vec![1, 2, 3]);
    }

    /// The kill switch is global: it takes every HUD down, not the focused
    /// table's. With the switch off the wanted set is empty, which is what
    /// makes the release pass hide every window.
    #[test]
    fn the_kill_switch_wants_no_hud_on_any_table() {
        assert!(wanted_table_ids(&[1, 2, 3], false).is_empty());
    }

    // -----------------------------------------------------------------
    // First windows
    // -----------------------------------------------------------------

    #[test]
    fn the_first_table_gets_the_first_slot_built() {
        let plan = plan_reconcile(&[], &[10], 1);
        assert_eq!(
            plan.assignments,
            vec![Assignment::Create {
                label: "overlay1".to_string(),
                table_id: 10
            }]
        );
        assert_eq!(plan.next_slot, 2);
        assert!(plan.released.is_empty());
    }

    /// The create/destroy deadlock in one assertion: four tables opening at once must not
    /// stack four webview builds into one pass. Three of them wait a tick.
    #[test]
    fn a_burst_of_tables_builds_exactly_one_window_and_defers_the_rest() {
        let plan = plan_reconcile(&[], &[1, 2, 3, 4], 1);
        assert_eq!(creates(&plan), 1);
        assert_eq!(
            plan.assignments,
            vec![
                Assignment::Create {
                    label: "overlay1".to_string(),
                    table_id: 1
                },
                Assignment::Defer { table_id: 2 },
                Assignment::Defer { table_id: 3 },
                Assignment::Defer { table_id: 4 },
            ]
        );
    }

    /// The deferred tables are picked up one per pass, so four tables are all
    /// served after four passes and never more than one build happens at once.
    #[test]
    fn deferred_tables_are_served_one_pass_at_a_time() {
        let wanted = vec![1, 2, 3, 4];
        let mut pool: Vec<SlotState> = Vec::new();
        let mut next_slot = 1;

        for pass in 1..=4 {
            let plan = plan_reconcile(&pool, &wanted, next_slot);
            assert_eq!(creates(&plan), 1, "pass {pass} built more than one window");
            next_slot = plan.next_slot;
            pool = apply(&pool, &plan);
        }

        assert_eq!(pool.len(), 4);
        let plan = plan_reconcile(&pool, &wanted, next_slot);
        assert_eq!(creates(&plan), 0, "a fifth pass built a window it did not need");
        assert!(plan
            .assignments
            .iter()
            .all(|a| matches!(a, Assignment::Sync { .. })));
    }

    /// Applies a plan to a pool snapshot the way `manager` does, so a test can
    /// run several passes in a row.
    fn apply(pool: &[SlotState], plan: &ReconcilePlan) -> Vec<SlotState> {
        let mut pool = pool.to_vec();
        for slot in pool.iter_mut() {
            if plan.released.contains(&slot.label) {
                slot.table_id = None;
            }
        }
        for assignment in &plan.assignments {
            match assignment {
                Assignment::Repoint { label, table_id } => {
                    if let Some(slot) = pool.iter_mut().find(|s| &s.label == label) {
                        slot.table_id = Some(*table_id);
                    }
                }
                Assignment::Create { label, table_id } => {
                    pool.push(SlotState::serving(label.clone(), *table_id));
                }
                Assignment::Sync { .. } | Assignment::Defer { .. } => {}
            }
        }
        pool
    }

    // -----------------------------------------------------------------
    // Reuse — the reason the pool exists
    // -----------------------------------------------------------------

    /// A settled pool builds nothing and re-points nothing; every pass is just
    /// a bounds sync. If this ever regressed into repeated builds it would be
    /// the event-loop wedge again, one poll tick at a time.
    #[test]
    fn a_settled_pool_only_syncs_bounds() {
        let pool = vec![SlotState::serving("overlay1", 1), SlotState::serving("overlay2", 2)];
        let plan = plan_reconcile(&pool, &[1, 2], 3);
        assert_eq!(
            plan.assignments,
            vec![
                Assignment::Sync {
                    label: "overlay1".to_string(),
                    table_id: 1
                },
                Assignment::Sync {
                    label: "overlay2".to_string(),
                    table_id: 2
                },
            ]
        );
        assert_eq!(plan.next_slot, 3);
    }

    /// A table closing releases its window instead of destroying it.
    #[test]
    fn a_closed_table_releases_its_window_to_the_pool() {
        let pool = vec![SlotState::serving("overlay1", 1), SlotState::serving("overlay2", 2)];
        let plan = plan_reconcile(&pool, &[1], 3);
        assert_eq!(plan.released, vec!["overlay2".to_string()]);
        assert_eq!(creates(&plan), 0);
    }

    /// The swap that the whole pool design exists for: one table closes and
    /// another opens in the same pass, and the freed window is handed straight
    /// over. Under the old destroy/create model this pass was a destroy
    /// plus a build — the churn that wedged the event loop on the 24th window.
    #[test]
    fn a_window_freed_this_pass_is_reused_in_the_same_pass() {
        let pool = vec![SlotState::serving("overlay1", 1)];
        let plan = plan_reconcile(&pool, &[2], 1);
        assert_eq!(plan.released, vec!["overlay1".to_string()]);
        assert_eq!(
            plan.assignments,
            vec![Assignment::Repoint {
                label: "overlay1".to_string(),
                table_id: 2
            }]
        );
        assert_eq!(creates(&plan), 0);
        assert_eq!(plan.next_slot, 1, "a reuse must not consume a slot number");
    }

    /// Opening and closing tables all session long must not grow the pool: the
    /// window count settles at the high-water mark of simultaneously open
    /// tables, never at the number of tables ever opened.
    #[test]
    fn table_churn_never_grows_the_pool_past_the_high_water_mark() {
        let mut pool: Vec<SlotState> = Vec::new();
        let mut next_slot = 1;

        // Twenty tables, opened and closed one after another, two at a time at
        // the most.
        for table_id in 1..=20u32 {
            for wanted in [vec![table_id], vec![table_id, table_id + 100]] {
                // Two passes each, since only one window can be built per pass.
                for _ in 0..2 {
                    let plan = plan_reconcile(&pool, &wanted, next_slot);
                    next_slot = plan.next_slot;
                    pool = apply(&pool, &plan);
                }
            }
        }

        assert_eq!(pool.len(), 2, "pool grew with churn instead of being reused");
    }

    /// The kill switch releases every window and builds none; flipping it back
    /// on re-points the same windows rather than building new ones.
    #[test]
    fn the_kill_switch_releases_every_window_and_gives_them_back() {
        let pool = vec![SlotState::serving("overlay1", 1), SlotState::serving("overlay2", 2)];

        let off = plan_reconcile(&pool, &wanted_table_ids(&[1, 2], false), 3);
        assert_eq!(off.released.len(), 2);
        assert!(off.assignments.is_empty());

        let pool = apply(&pool, &off);
        let on = plan_reconcile(&pool, &wanted_table_ids(&[1, 2], true), 3);
        assert_eq!(creates(&on), 0);
        assert_eq!(labels(&on), vec!["overlay1", "overlay2"]);
    }

    // -----------------------------------------------------------------
    // Slot numbering
    // -----------------------------------------------------------------

    /// Slot numbers are handed out monotonically, never derived from the pool's
    /// length. A slot dropped because its window vanished leaves a hole, and
    /// numbering from `pool.len() + 1` would try to build "overlay2" a second
    /// time — a label Tauri refuses, so that slot could never be rebuilt.
    #[test]
    fn next_slot_never_reuses_a_dropped_slots_number() {
        // overlay2's slot was dropped; overlay1 and overlay3 remain, both busy.
        let pool = vec![SlotState::serving("overlay1", 1), SlotState::serving("overlay3", 3)];
        let plan = plan_reconcile(&pool, &[1, 3, 4], 4);
        assert_eq!(plan.created_label(), Some("overlay4"));
        assert_eq!(plan.next_slot, 5);
    }

    /// A build that never made it into the pool — the window came back with no
    /// HWND, so it was destroyed unshown — must not have its
    /// label handed out again. `reconcile` stores the plan's counter before it
    /// executes the plan, so the label is spent whether or not the build stuck;
    /// this pins that the retry asks for a fresh one, which is what makes
    /// destroying the abandoned window safe (Tauri refuses a duplicate label,
    /// so a reused one could fail against a window still being torn down).
    #[test]
    fn a_build_that_never_joined_the_pool_does_not_get_its_label_reissued() {
        // Pass one: nothing pooled, one table wants a window.
        let first = plan_reconcile(&[], &[7], 1);
        assert_eq!(first.created_label(), Some("overlay1"));
        assert_eq!(first.next_slot, 2);

        // The build produced an unusable window, so the pool stayed empty —
        // but the counter has already moved on.
        let pool: Vec<SlotState> = Vec::new();
        let second = plan_reconcile(&pool, &[7], first.next_slot);
        assert_eq!(second.created_label(), Some("overlay2"));
        assert_ne!(second.created_label(), first.created_label());
    }

    /// A pass that only re-points must not burn a slot number, or the labels
    /// would run away from the pool over a long session.
    #[test]
    fn passes_that_build_nothing_leave_the_slot_counter_alone() {
        let pool = vec![SlotState::idle("overlay1")];
        let plan = plan_reconcile(&pool, &[9], 2);
        assert_eq!(plan.next_slot, 2);
        assert_eq!(creates(&plan), 0);
    }

    // -----------------------------------------------------------------
    // Identity
    // -----------------------------------------------------------------

    /// A pooled window serves one table at a time. Two tables sharing a label
    /// would mean one of them showing the other's players over its felt.
    #[test]
    fn no_two_tables_are_ever_assigned_the_same_window() {
        let pool = vec![SlotState::idle("overlay1"), SlotState::idle("overlay2")];
        let plan = plan_reconcile(&pool, &[5, 6], 3);
        let mut used = labels(&plan);
        used.sort_unstable();
        used.dedup();
        assert_eq!(used.len(), plan.assignments.len());
    }

    /// Every wanted table is accounted for in the plan, in order — a table
    /// silently missing from the pass is a real past bug (a second table with
    /// no HUD and nothing saying so).
    #[test]
    fn every_wanted_table_appears_in_the_plan() {
        let pool = vec![SlotState::serving("overlay1", 1), SlotState::idle("overlay2")];
        let wanted = vec![1, 2, 3, 4];
        let plan = plan_reconcile(&pool, &wanted, 3);
        assert_eq!(
            plan.assignments.iter().map(|a| a.table_id()).collect::<Vec<_>>(),
            wanted
        );
    }
}
