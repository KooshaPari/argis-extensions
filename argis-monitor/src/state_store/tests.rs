//! Unit tests for the state store.

use std::sync::atomic::{AtomicU64, Ordering};

use super::connection::parse_state;
use super::types::{StateStoreError, TrackerSnapshot};
use super::StateStore;
use crate::alerts::{AlertState, AlertStateTracker};

static SEQ: AtomicU64 = AtomicU64::new(0);

fn tmpfile(label: &str) -> std::path::PathBuf {
    let n = SEQ.fetch_add(1, Ordering::SeqCst);
    let tid = std::thread::current().id();
    let dir = std::env::temp_dir().join(format!(
        "argis-monitor-test-pid{}-{:?}-{}-{}",
        std::process::id(), tid, label, n,
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("state.sqlite")
}

#[test]
fn round_trip_ok_state() {
    let path = tmpfile("ok");
    let _ = std::fs::remove_file(&path);
    let mut store = StateStore::open(&path).unwrap();
    let snap = TrackerSnapshot::ok();
    store.save("gw::rule_a", &snap).unwrap();
    let all = store.load_all().unwrap();
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].0, "gw::rule_a");
    assert_eq!(all[0].1, snap);
}

#[test]
fn round_trip_pending_state() {
    let path = tmpfile("pending");
    let _ = std::fs::remove_file(&path);
    let mut store = StateStore::open(&path).unwrap();
    let snap = TrackerSnapshot {
        state: AlertState::Pending { since: 1234567890 },
        sustained_secs: 12,
    };
    store.save("gw::rule_b", &snap).unwrap();
    let all = store.load_all().unwrap();
    assert_eq!(all[0].1.state, AlertState::Pending { since: 1234567890 });
    assert_eq!(all[0].1.sustained_secs, 12);
}

#[test]
fn upsert_overwrites_previous_state() {
    let path = tmpfile("upsert");
    let _ = std::fs::remove_file(&path);
    let mut store = StateStore::open(&path).unwrap();
    store.save("gw::rule_c", &TrackerSnapshot::ok()).unwrap();
    store.save("gw::rule_c", &TrackerSnapshot {
        state: AlertState::Firing { since: 100, last_fired_at: 100 },
        sustained_secs: 0,
    }).unwrap();
    let all = store.load_all().unwrap();
    assert_eq!(all.len(), 1);
    assert!(matches!(all[0].1.state, AlertState::Firing { .. }));
}

#[test]
fn restart_rehydration_matches_in_memory() {
    let path = tmpfile("restart");
    let snaps = vec![
        ("gw::r1", TrackerSnapshot { state: AlertState::Pending { since: 1000 }, sustained_secs: 5 }),
        ("gw::r2", TrackerSnapshot::ok()),
        ("openai::r1", TrackerSnapshot { state: AlertState::Firing { since: 2000, last_fired_at: 2000 }, sustained_secs: 30 }),
    ];
    {
        let mut store = StateStore::open(&path).unwrap();
        for (k, s) in &snaps {
            store.save(k, s).unwrap();
        }
    }
    let store = StateStore::open(&path).unwrap();
    let restored = store.load_all().unwrap();
    assert_eq!(restored.len(), snaps.len());
    for (k, s) in &snaps {
        let found = restored.iter().find(|(rk, _)| rk == k).expect("missing key");
        assert_eq!(&found.1, s);
    }
}

#[test]
fn delete_removes_row() {
    let path = tmpfile("delete");
    let mut store = StateStore::open(&path).unwrap();
    store.save("gw::r1", &TrackerSnapshot::ok()).unwrap();
    store.delete("gw::r1").unwrap();
    assert_eq!(store.load_all().unwrap().len(), 0);
}

#[test]
fn parse_state_err_handles_unknown_string() {
    let ok = parse_state("ok", 0, 0).unwrap();
    assert_eq!(ok, AlertState::Ok);
    let pending = parse_state("pending", 100, 0).unwrap();
    assert_eq!(pending, AlertState::Pending { since: 100 });
    let firing = parse_state("firing", 200, 250).unwrap();
    assert_eq!(firing, AlertState::Firing { since: 200, last_fired_at: 250 });
    let err = parse_state("wat", 0, 0).unwrap_err();
    assert!(matches!(err, StateStoreError::InvalidState(s) if s == "wat"));
}

#[test]
fn alert_state_tracker_conversion() {
    let path = tmpfile("tracker");
    let mut store = StateStore::open(&path).unwrap();
    let mut tracker = AlertStateTracker::default();
    tracker.state = AlertState::Pending { since: 42 };
    tracker.sustained_for = std::time::Duration::from_secs(7);
    let snap = TrackerSnapshot { state: tracker.state.clone(), sustained_secs: tracker.sustained_for.as_secs() };
    store.save("gw::r1", &snap).unwrap();
    let restored = store.load_all().unwrap();
    assert_eq!(restored[0].1.state, AlertState::Pending { since: 42 });
    assert_eq!(restored[0].1.sustained_secs, 7);
}

// ============================================================
// alert_history (slice 7)
// ============================================================

#[test]
fn record_event_appends_history_row() {
    let path = tmpfile("event");
    let mut store = StateStore::open(&path).unwrap();
    store.record_event(
        "gw::r1", "fired", "critical", 5.0, 2.0,
        r#"{"rule":"r1","target":"gw","slo":"s","burn_rate":5.0,"threshold":2.0,"severity":"Critical","fired_at_unix":1000,"message":"m"}"#,
        1000,
    ).unwrap();
    store.record_event(
        "gw::r1", "resolved", "ok", 0.5, 2.0,
        r#"{"rule":"r1","target":"gw","slo":"s","burn_rate":0.5,"threshold":2.0,"severity":"Ok","fired_at_unix":2000,"message":"m"}"#,
        2000,
    ).unwrap();
    let all = store.list_history(None, 100).unwrap();
    assert_eq!(all.len(), 2);
    assert_eq!(all[0].event, "resolved");
    assert_eq!(all[0].severity, "ok");
    assert_eq!(all[1].event, "fired");
}

#[test]
fn list_history_filters_by_key_prefix() {
    let path = tmpfile("prefix");
    let mut store = StateStore::open(&path).unwrap();
    for k in ["gw::r1", "gw::r2", "openai::r1"] {
        store.record_event(
            k, "fired", "warning", 3.0, 2.0,
            r#"{"rule":"r","target":"t","slo":"s","burn_rate":3.0,"threshold":2.0,"severity":"Warning","fired_at_unix":1,"message":"m"}"#,
            1,
        ).unwrap();
    }
    let gw_only = store.list_history(Some("gw::"), 100).unwrap();
    assert_eq!(gw_only.len(), 2);
    for r in &gw_only { assert!(r.key.starts_with("gw::")); }
}

#[test]
fn list_history_respects_limit() {
    let path = tmpfile("limit");
    let mut store = StateStore::open(&path).unwrap();
    for i in 0..50u64 {
        store.record_event(
            "k", "fired", "warning", 1.0, 1.0,
            r#"{"rule":"r","target":"t","slo":"s","burn_rate":1.0,"threshold":1.0,"severity":"Warning","fired_at_unix":0,"message":"m"}"#,
            i,
        ).unwrap();
    }
    let ten = store.list_history(None, 10).unwrap();
    assert_eq!(ten.len(), 10);
}

#[test]
fn history_persists_across_reopen() {
    let path = tmpfile("persist");
    {
        let mut store = StateStore::open(&path).unwrap();
        store.record_event(
            "gw::r1", "fired", "critical", 4.0, 2.0,
            r#"{"rule":"r1","target":"gw","slo":"s","burn_rate":4.0,"threshold":2.0,"severity":"Critical","fired_at_unix":42,"message":"m"}"#,
            42,
        ).unwrap();
    }
    let store = StateStore::open(&path).unwrap();
    let history = store.list_history(None, 100).unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].burn_rate, 4.0);
    assert_eq!(history[0].fired_at_unix, 42);
}
