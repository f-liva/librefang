//! Integration regression test for Phase 05 §B.2 — chat-aware memory recall.
//!
//! The semantic store must filter recalled fragments by
//! `metadata["chat_jid"]` when `MemoryFilter::active_chat_jid` is set.
//! Untagged (legacy) fragments pass through unconditionally. Setting
//! `LIBREFANG_MEMORY_CHAT_AWARE=off` disables the filter as an in-field
//! revert path.
//!
//! These tests exercise `SemanticStore` directly — the same code path the
//! substrate uses in production.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

/// Serialises tests that toggle the `LIBREFANG_MEMORY_CHAT_AWARE` env var.
/// The rest of the tests in this file don't touch the env, but they share
/// the process-wide default, so a concurrently-running `test_4_*` can
/// spuriously flip the filter off/on mid-query. Taking the lock at the
/// top of every test makes this deterministic under `cargo test`'s
/// multi-threaded default.
fn env_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

use librefang_memory::semantic::SemanticStore;
use librefang_types::agent::AgentId;
use librefang_types::memory::{MemoryFilter, MemorySource};
use rusqlite::Connection;
use serde_json::json;

fn setup() -> (SemanticStore, AgentId) {
    let conn = Connection::open_in_memory().expect("in-memory sqlite");
    librefang_memory::migration::run_migrations(&conn).expect("migrations");
    let store = SemanticStore::new(Arc::new(Mutex::new(conn)));
    let agent = AgentId::new();
    (store, agent)
}

fn meta_with_chat(jid: &str) -> HashMap<String, serde_json::Value> {
    let mut m = HashMap::new();
    m.insert("chat_jid".to_string(), json!(jid));
    m
}

fn write(store: &SemanticStore, agent: AgentId, content: &str, chat_jid: Option<&str>) {
    let metadata = match chat_jid {
        Some(jid) => meta_with_chat(jid),
        None => HashMap::new(),
    };
    store
        .remember(agent, content, MemorySource::Conversation, "chat", metadata)
        .expect("store fragment");
}

/// Reset the env flag between tests — `cargo test` runs integration tests in
/// the same process but sequentially per `#[test]` within a single binary
/// when `--test-threads=1` is used.  We use `set_var`/`remove_var` from
/// a single thread inside the test body; while unsafe in principle, it is
/// sequential here.
fn clear_flag() {
    // SAFETY: integration tests in this file are sequential.
    unsafe { std::env::remove_var("LIBREFANG_MEMORY_CHAT_AWARE") };
}

#[test]
fn test_1_chat_x_recall_returns_only_x_tagged() {
    let _guard = env_lock();
    clear_flag();
    let (store, agent) = setup();

    write(
        &store,
        agent,
        "DM X private payload",
        Some("dm-X@s.whatsapp.net"),
    );
    write(&store, agent, "group Y discussion", Some("group-Y@g.us"));

    let filter = MemoryFilter::agent(agent).with_active_chat_jid("dm-X@s.whatsapp.net");
    let results = store.recall("", 20, Some(filter)).unwrap();
    assert_eq!(
        results.len(),
        1,
        "expected exactly the DM-X fragment, got {:?}",
        results.iter().map(|f| &f.content).collect::<Vec<_>>()
    );
    assert_eq!(results[0].content, "DM X private payload");
}

#[test]
fn test_2_no_active_chat_jid_returns_all() {
    let _guard = env_lock();
    clear_flag();
    let (store, agent) = setup();
    write(&store, agent, "item-a", Some("dm-X@s.whatsapp.net"));
    write(&store, agent, "item-b", Some("group-Y@g.us"));

    // No active_chat_jid → caller opts out, all fragments returned.
    let filter = MemoryFilter::agent(agent);
    let results = store.recall("", 20, Some(filter)).unwrap();
    assert_eq!(results.len(), 2, "caller without active_chat_jid sees all");
}

#[test]
fn test_3_untagged_fragments_always_included() {
    let _guard = env_lock();
    clear_flag();
    let (store, agent) = setup();
    write(&store, agent, "legacy untagged", None);
    write(&store, agent, "tagged dm-X", Some("dm-X@s.whatsapp.net"));
    write(&store, agent, "tagged group-Y", Some("group-Y@g.us"));

    let filter = MemoryFilter::agent(agent).with_active_chat_jid("dm-X@s.whatsapp.net");
    let results = store.recall("", 20, Some(filter)).unwrap();
    let contents: Vec<&str> = results.iter().map(|f| f.content.as_str()).collect();
    assert!(
        contents.contains(&"legacy untagged"),
        "legacy untagged fragment must survive chat filter"
    );
    assert!(
        contents.contains(&"tagged dm-X"),
        "active-chat-tagged fragment must survive"
    );
    assert!(
        !contents.contains(&"tagged group-Y"),
        "other-chat fragment must be filtered out"
    );
    assert_eq!(results.len(), 2);
}

/// Exercises BOTH flag-on and flag-off in a single test body under the
/// shared mutex. Multi-threaded `cargo test` makes `std::env::set_var`
/// visible across every thread in the same binary, so splitting the two
/// states into separate `#[test]` functions produces flaky races (test_N
/// sees the env var set by test_M on a sibling thread — the
/// `env_lock()` mutex can't protect queries that run between lock
/// release and another test's acquire). Serialising both states in the
/// same body is the cleanest fix without adding the `serial_test` crate.
#[test]
fn test_4_flag_matrix_single_threaded() {
    let _guard = env_lock();
    clear_flag();

    // Phase A: flag on (default) — cross-chat filter applied.
    {
        let (store, agent) = setup();
        write(&store, agent, "dm-X payload", Some("dm-X@s.whatsapp.net"));
        write(&store, agent, "group-Y payload", Some("group-Y@g.us"));
        let filter = MemoryFilter::agent(agent).with_active_chat_jid("dm-X@s.whatsapp.net");
        let results = store.recall("", 20, Some(filter)).unwrap();
        assert_eq!(
            results.len(),
            1,
            "flag on (default): only dm-X fragment is visible"
        );
        assert_eq!(results[0].content, "dm-X payload");
    }

    // Phase B: flag off — filter disabled, both fragments visible.
    // SAFETY: env_lock serializes this file's env-var mutations.
    unsafe { std::env::set_var("LIBREFANG_MEMORY_CHAT_AWARE", "off") };
    {
        let (store, agent) = setup();
        write(&store, agent, "dm-X payload", Some("dm-X@s.whatsapp.net"));
        write(&store, agent, "group-Y payload", Some("group-Y@g.us"));
        let filter = MemoryFilter::agent(agent).with_active_chat_jid("dm-X@s.whatsapp.net");
        let results = store.recall("", 20, Some(filter)).unwrap();
        assert_eq!(
            results.len(),
            2,
            "flag off: both fragments visible (filter disabled)"
        );
    }

    // Phase C: clearing the flag mid-process — filter back in effect.
    clear_flag();
    {
        let (store, agent) = setup();
        write(&store, agent, "dm-X payload", Some("dm-X@s.whatsapp.net"));
        write(&store, agent, "group-Y payload", Some("group-Y@g.us"));
        let filter = MemoryFilter::agent(agent).with_active_chat_jid("dm-X@s.whatsapp.net");
        let results = store.recall("", 20, Some(filter)).unwrap();
        assert_eq!(results.len(), 1, "flag restored to on: filter re-engaged");
    }

    clear_flag();
}

#[test]
fn test_5_cross_chat_zero_leak_at_scale() {
    let _guard = env_lock();
    clear_flag();
    let (store, agent) = setup();
    // Ten DM-X fragments + ten group-Y fragments.
    for i in 0..10 {
        write(
            &store,
            agent,
            &format!("dm-x-{i}"),
            Some("dm-X@s.whatsapp.net"),
        );
        write(&store, agent, &format!("group-y-{i}"), Some("group-Y@g.us"));
    }

    // Recall for the group — zero DM-X fragments must survive.
    let filter = MemoryFilter::agent(agent).with_active_chat_jid("group-Y@g.us");
    let results = store.recall("", 100, Some(filter)).unwrap();
    for frag in &results {
        assert!(
            !frag.content.starts_with("dm-x-"),
            "DM fragment {:?} leaked into group recall",
            frag.content
        );
    }
    assert_eq!(results.len(), 10, "all 10 group-Y fragments visible");
}

#[test]
fn test_6_write_with_chat_jid_round_trips() {
    // A fragment written with metadata["chat_jid"] = X must round-trip
    // through the store and be recallable when the filter targets exactly
    // that chat. This is the agent-loop write-side contract.
    let _guard = env_lock();
    clear_flag();
    let (store, agent) = setup();
    let jid = "dm-alpha@s.whatsapp.net";
    write(&store, agent, "signore-specific note", Some(jid));

    let filter = MemoryFilter::agent(agent).with_active_chat_jid(jid);
    let results = store.recall("signore", 10, Some(filter)).unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(
        results[0].metadata.get("chat_jid").and_then(|v| v.as_str()),
        Some(jid),
        "chat_jid metadata must round-trip through the store"
    );
}

#[test]
fn test_7_with_active_chat_jid_builder() {
    // Smoke-test the builder method — `.with_active_chat_jid` must return
    // a filter whose field is populated.
    let filter = MemoryFilter::agent(AgentId::new()).with_active_chat_jid("abc@g.us");
    assert_eq!(filter.active_chat_jid.as_deref(), Some("abc@g.us"));
}

#[test]
fn test_8_serde_default_on_legacy_json() {
    // A MemoryFilter deserialized from a JSON blob that predates the new
    // field must default `active_chat_jid` to None — guarantees backward
    // compatibility for on-disk / wire-format consumers.
    let legacy_json = r#"{
        "agent_id": null,
        "source": null,
        "scope": null,
        "min_confidence": null,
        "after": null,
        "before": null,
        "metadata": {},
        "peer_id": null
    }"#;
    let f: MemoryFilter = serde_json::from_str(legacy_json).expect("legacy json parses");
    assert!(f.active_chat_jid.is_none());
}
