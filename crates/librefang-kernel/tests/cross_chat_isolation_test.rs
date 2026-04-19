//! Phase 05 §B cross-chat isolation — zero-leak regression guard.
//!
//! These tests are the load-bearing acceptance gate for CI-01: they prove
//! that a DM turn's content cannot leak into a subsequent group turn's
//! canonical context for the same agent, and symmetrically in the reverse
//! direction.
//!
//! They exercise the `SessionStore::canonical_context` read path — the
//! same path the kernel calls to assemble an agent's prompt before every
//! LLM call. If any future refactor re-introduces cross-chat bleed at the
//! canonical-context layer, these tests fail.
//!
//! The fixture uses the new strict `whatsapp-dm:` / `whatsapp-group:`
//! channel_type prefixes (Phase 05 §B.1), but the invariant holds for any
//! two distinct `channel_type` strings because `SessionId::for_channel`
//! treats the string as opaque when deriving the session id.

use std::sync::{Arc, Mutex};

use librefang_memory::migration::run_migrations;
use librefang_memory::session::SessionStore;
use librefang_types::agent::{AgentId, SessionId};
use librefang_types::message::{Message, MessageContent, Role};
use rusqlite::Connection;

const DM_PAYLOAD: &str = "PRIVATE_BUDAPEST_PAYLOAD_should_never_leak_cross_chat";
const GROUP_PAYLOAD: &str = "GROUP_NOTE_CODE_should_never_leak_cross_chat";

fn setup() -> (SessionStore, AgentId) {
    let conn = Connection::open_in_memory().expect("in-memory sqlite");
    run_migrations(&conn).expect("run migrations");
    let store = SessionStore::new(Arc::new(Mutex::new(conn)));
    let agent = AgentId::new();
    (store, agent)
}

fn user_msg(text: &str) -> Message {
    Message {
        role: Role::User,
        content: MessageContent::Text(text.to_string()),
        pinned: false,
    }
}

fn recent_texts(recent: &[Message]) -> Vec<String> {
    recent
        .iter()
        .filter_map(|m| match &m.content {
            MessageContent::Text(s) => Some(s.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn test_dm_context_does_not_leak_to_group() {
    let (store, agent) = setup();

    // Strict Phase 05 §B.1 channel_type prefixes — DM and group produce
    // different SessionIds because `for_channel` hashes the full string.
    let dm_channel = "whatsapp-dm:391234@s.whatsapp.net";
    let group_channel = "whatsapp-group:120363abc@g.us";
    let session_dm = SessionId::for_channel(agent, dm_channel);
    let session_group = SessionId::for_channel(agent, group_channel);

    assert_ne!(
        session_dm, session_group,
        "dm and group must derive different SessionIds"
    );

    // Turn 1 — DM with a private payload.
    store
        .append_canonical(agent, &[user_msg(DM_PAYLOAD)], None, Some(session_dm))
        .expect("append DM turn");

    // Turn 2 — group, unrelated prompt.
    store
        .append_canonical(agent, &[user_msg("ciao gruppo")], None, Some(session_group))
        .expect("append group turn");

    // Canonical context for the group session — the DM payload must not
    // appear anywhere.
    let (_summary, group_recent) = store
        .canonical_context(agent, Some(session_group), None)
        .expect("canonical_context group");
    let texts = recent_texts(&group_recent);

    assert!(
        !texts.iter().any(|t| t.contains(DM_PAYLOAD)),
        "DM payload leaked into group canonical context: {texts:?}"
    );
    assert!(
        texts.iter().any(|t| t.contains("ciao gruppo")),
        "group context should contain its own turn: {texts:?}"
    );
}

#[test]
fn test_group_context_does_not_leak_to_dm() {
    // Symmetric reverse: a group payload must never appear in a DM's
    // canonical context.
    let (store, agent) = setup();
    let dm_channel = "whatsapp-dm:391234@s.whatsapp.net";
    let group_channel = "whatsapp-group:120363abc@g.us";
    let session_dm = SessionId::for_channel(agent, dm_channel);
    let session_group = SessionId::for_channel(agent, group_channel);

    // Turn 1 — group with a note payload.
    store
        .append_canonical(agent, &[user_msg(GROUP_PAYLOAD)], None, Some(session_group))
        .expect("append group turn");

    // Turn 2 — DM, unrelated prompt.
    store
        .append_canonical(
            agent,
            &[user_msg("hey what do you remember")],
            None,
            Some(session_dm),
        )
        .expect("append DM turn");

    let (_summary, dm_recent) = store
        .canonical_context(agent, Some(session_dm), None)
        .expect("canonical_context DM");
    let texts = recent_texts(&dm_recent);

    assert!(
        !texts.iter().any(|t| t.contains(GROUP_PAYLOAD)),
        "group payload leaked into DM canonical context: {texts:?}"
    );
    assert!(
        texts.iter().any(|t| t.contains("hey what do you remember")),
        "DM context should contain its own turn: {texts:?}"
    );
}

#[test]
fn test_same_chat_context_persists() {
    // Anti-false-positive sanity check. Two consecutive turns in the SAME
    // chat session must see each other's content — otherwise the test
    // fixture is trivially "passing" by over-filtering everything.
    let (store, agent) = setup();
    let dm_channel = "whatsapp-dm:391234@s.whatsapp.net";
    let session_dm = SessionId::for_channel(agent, dm_channel);

    store
        .append_canonical(
            agent,
            &[user_msg("turn-one-marker")],
            None,
            Some(session_dm),
        )
        .expect("append turn 1");
    store
        .append_canonical(
            agent,
            &[user_msg("turn-two-marker")],
            None,
            Some(session_dm),
        )
        .expect("append turn 2");

    let (_summary, recent) = store
        .canonical_context(agent, Some(session_dm), None)
        .expect("canonical_context");
    let texts = recent_texts(&recent);

    assert!(
        texts.iter().any(|t| t.contains("turn-one-marker")),
        "same-chat turn 1 missing from context: {texts:?}"
    );
    assert!(
        texts.iter().any(|t| t.contains("turn-two-marker")),
        "same-chat turn 2 missing from context: {texts:?}"
    );
}

#[test]
fn test_cross_chat_mixed_turns_zero_leak_at_scale() {
    // Belt-and-braces: alternate 5 DM turns with 5 group turns and assert
    // that each side's canonical_context contains ONLY its own payloads.
    let (store, agent) = setup();
    let dm_channel = "whatsapp-dm:391234@s.whatsapp.net";
    let group_channel = "whatsapp-group:120363abc@g.us";
    let session_dm = SessionId::for_channel(agent, dm_channel);
    let session_group = SessionId::for_channel(agent, group_channel);

    for i in 0..5 {
        store
            .append_canonical(
                agent,
                &[user_msg(&format!("dm-marker-{i}"))],
                None,
                Some(session_dm),
            )
            .unwrap();
        store
            .append_canonical(
                agent,
                &[user_msg(&format!("group-marker-{i}"))],
                None,
                Some(session_group),
            )
            .unwrap();
    }

    let (_s, dm_ctx) = store
        .canonical_context(agent, Some(session_dm), None)
        .unwrap();
    let dm_texts = recent_texts(&dm_ctx);
    for t in &dm_texts {
        assert!(
            !t.contains("group-marker"),
            "group leaked into DM context: {t:?}"
        );
    }
    assert_eq!(
        dm_texts.iter().filter(|t| t.contains("dm-marker")).count(),
        5,
        "all 5 DM markers visible in DM context"
    );

    let (_s, g_ctx) = store
        .canonical_context(agent, Some(session_group), None)
        .unwrap();
    let g_texts = recent_texts(&g_ctx);
    for t in &g_texts {
        assert!(
            !t.contains("dm-marker"),
            "DM leaked into group context: {t:?}"
        );
    }
    assert_eq!(
        g_texts
            .iter()
            .filter(|t| t.contains("group-marker"))
            .count(),
        5,
        "all 5 group markers visible in group context"
    );
}
