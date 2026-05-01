// Integration tests for the WhatsApp-specific extension of the
// `channel_send` tool (Phase 07 PLAN-02 / SR-B):
//
// - JID format validation (regex permissive across DM, group, lid, multi-device)
// - `as_voice_note` routing into `send_channel_media(media_type="voice", ...)`
// - `reply_to_msg_id` propagation through every adapter call site
// - Silent-ignore semantics on non-WhatsApp channels (telegram, slack, ...)
// - Tool definition schema includes the two new optional properties

use async_trait::async_trait;
use librefang_kernel_handle::{AgentInfo, KernelHandle};
use librefang_runtime::tool_runner::{builtin_tool_definitions, execute_tool_raw, ToolExecContext};
use serde_json::json;
use std::sync::{Arc, Mutex};

// ---------------------------------------------------------------------------
// Capturing kernel: records every channel-send call so tests can assert on
// the exact arguments forwarded down the trait. Every other KernelHandle
// method panics on use (we only exercise the channel-send surface).
// ---------------------------------------------------------------------------

// Mock-capture structs: every field is recorded so a future assertion can
// inspect it. Tests today only assert on a subset; the unused ones are
// intentional and not dead code.
#[allow(dead_code)]
#[derive(Debug, Clone, Default)]
struct CapturedTextSend {
    channel: String,
    recipient: String,
    message: String,
    thread_id: Option<String>,
    account_id: Option<String>,
    reply_to_msg_id: Option<String>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Default)]
struct CapturedMediaSend {
    channel: String,
    recipient: String,
    media_type: String,
    media_url: String,
    caption: Option<String>,
    filename: Option<String>,
    thread_id: Option<String>,
    account_id: Option<String>,
    reply_to_msg_id: Option<String>,
}

#[derive(Debug, Default)]
struct ChannelSendLog {
    text: Mutex<Vec<CapturedTextSend>>,
    media: Mutex<Vec<CapturedMediaSend>>,
}

struct CapturingKernel {
    log: Arc<ChannelSendLog>,
    /// Optional canned error returned by the next channel-send call. Used to
    /// exercise the gateway-error translation path (test 5).
    next_error: Mutex<Option<String>>,
}

impl CapturingKernel {
    fn new() -> (Arc<Self>, Arc<ChannelSendLog>) {
        let log = Arc::new(ChannelSendLog::default());
        let kernel = Arc::new(Self {
            log: Arc::clone(&log),
            next_error: Mutex::new(None),
        });
        (kernel, log)
    }

    fn arm_error(&self, msg: &str) {
        *self.next_error.lock().unwrap() = Some(msg.to_string());
    }
}

#[async_trait]
impl KernelHandle for CapturingKernel {
    async fn spawn_agent(&self, _: &str, _: Option<&str>) -> Result<(String, String), String> {
        Err("not implemented".into())
    }
    async fn send_to_agent(&self, _: &str, _: &str) -> Result<String, String> {
        Err("not implemented".into())
    }
    fn list_agents(&self) -> Vec<AgentInfo> {
        vec![]
    }
    fn kill_agent(&self, _: &str) -> Result<(), String> {
        Err("not implemented".into())
    }
    fn memory_store(
        &self,
        _key: &str,
        _value: serde_json::Value,
        _peer_id: Option<&str>,
    ) -> Result<(), String> {
        Ok(())
    }
    fn memory_recall(
        &self,
        _key: &str,
        _peer_id: Option<&str>,
    ) -> Result<Option<serde_json::Value>, String> {
        Ok(None)
    }
    fn memory_list(&self, _peer_id: Option<&str>) -> Result<Vec<String>, String> {
        Ok(vec![])
    }
    fn find_agents(&self, _: &str) -> Vec<AgentInfo> {
        vec![]
    }
    async fn task_post(
        &self,
        _: &str,
        _: &str,
        _: Option<&str>,
        _: Option<&str>,
    ) -> Result<String, String> {
        Err("not implemented".into())
    }
    async fn task_claim(&self, _: &str) -> Result<Option<serde_json::Value>, String> {
        Err("not implemented".into())
    }
    async fn task_complete(&self, _: &str, _: &str, _: &str) -> Result<(), String> {
        Err("not implemented".into())
    }
    async fn task_list(&self, _: Option<&str>) -> Result<Vec<serde_json::Value>, String> {
        Err("not implemented".into())
    }
    async fn task_delete(&self, _: &str) -> Result<bool, String> {
        Err("not implemented".into())
    }
    async fn task_retry(&self, _: &str) -> Result<bool, String> {
        Err("not implemented".into())
    }
    async fn task_get(&self, _: &str) -> Result<Option<serde_json::Value>, String> {
        Err("not implemented".into())
    }
    async fn task_update_status(&self, _: &str, _: &str) -> Result<bool, String> {
        Err("not implemented".into())
    }
    async fn publish_event(&self, _: &str, _: serde_json::Value) -> Result<(), String> {
        Err("not implemented".into())
    }
    async fn knowledge_add_entity(
        &self,
        _: librefang_types::memory::Entity,
    ) -> Result<String, String> {
        Err("not implemented".into())
    }
    async fn knowledge_add_relation(
        &self,
        _: librefang_types::memory::Relation,
    ) -> Result<String, String> {
        Err("not implemented".into())
    }
    async fn knowledge_query(
        &self,
        _: librefang_types::memory::GraphPattern,
    ) -> Result<Vec<librefang_types::memory::GraphMatch>, String> {
        Err("not implemented".into())
    }

    // ── channel send overrides under test ────────────────────────────────

    async fn send_channel_message(
        &self,
        channel: &str,
        recipient: &str,
        message: &str,
        thread_id: Option<&str>,
        account_id: Option<&str>,
        reply_to_msg_id: Option<&str>,
    ) -> Result<String, String> {
        if let Some(err) = self.next_error.lock().unwrap().take() {
            return Err(err);
        }
        self.log.text.lock().unwrap().push(CapturedTextSend {
            channel: channel.to_string(),
            recipient: recipient.to_string(),
            message: message.to_string(),
            thread_id: thread_id.map(str::to_string),
            account_id: account_id.map(str::to_string),
            reply_to_msg_id: reply_to_msg_id.map(str::to_string),
        });
        Ok(format!("Message sent to {recipient} via {channel}"))
    }

    async fn send_channel_media(
        &self,
        channel: &str,
        recipient: &str,
        media_type: &str,
        media_url: &str,
        caption: Option<&str>,
        filename: Option<&str>,
        thread_id: Option<&str>,
        account_id: Option<&str>,
        reply_to_msg_id: Option<&str>,
    ) -> Result<String, String> {
        if let Some(err) = self.next_error.lock().unwrap().take() {
            return Err(err);
        }
        self.log.media.lock().unwrap().push(CapturedMediaSend {
            channel: channel.to_string(),
            recipient: recipient.to_string(),
            media_type: media_type.to_string(),
            media_url: media_url.to_string(),
            caption: caption.map(str::to_string),
            filename: filename.map(str::to_string),
            thread_id: thread_id.map(str::to_string),
            account_id: account_id.map(str::to_string),
            reply_to_msg_id: reply_to_msg_id.map(str::to_string),
        });
        Ok(format!("{media_type} sent to {recipient} via {channel}"))
    }
}

fn make_ctx<'a>(
    kernel: &'a Arc<dyn KernelHandle>,
    sender_id: Option<&'a str>,
) -> ToolExecContext<'a> {
    ToolExecContext {
        kernel: Some(kernel),
        allowed_tools: None,
        available_tools: None,
        caller_agent_id: Some("test-agent"),
        skill_registry: None,
        allowed_skills: None,
        mcp_connections: None,
        web_ctx: None,
        browser_ctx: None,
        allowed_env_vars: None,
        workspace_root: None,
        media_engine: None,
        media_drivers: None,
        exec_policy: None,
        tts_engine: None,
        docker_config: None,
        process_manager: None,
        process_registry: None,
        sender_id,
        channel: None,
        checkpoint_manager: None,
        interrupt: None,
        dangerous_command_checker: None,
    }
}

// ---------------------------------------------------------------------------
// Test 1 — text_ok: WhatsApp DM JID accepted, reply_to_msg_id=None.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn channel_send_whatsapp_text_ok() {
    let (capturing, log) = CapturingKernel::new();
    let kernel: Arc<dyn KernelHandle> = capturing.clone();
    let ctx = make_ctx(&kernel, None);

    let input = json!({
        "channel": "whatsapp",
        "recipient": "393401234567@s.whatsapp.net",
        "message": "ciao",
    });
    let result = execute_tool_raw("t1", "channel_send", &input, &ctx).await;
    assert!(!result.is_error, "expected ok, got: {}", result.content);

    let calls = log.text.lock().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].channel, "whatsapp");
    assert_eq!(calls[0].recipient, "393401234567@s.whatsapp.net");
    assert_eq!(calls[0].message, "ciao");
    assert_eq!(calls[0].reply_to_msg_id, None);
}

// ---------------------------------------------------------------------------
// Test 2 — invalid_jid: malformed recipient on whatsapp channel rejects
// before any adapter call. JID regex must match all 4 shapes (plain dom,
// group, lid, multi-device suffix) and reject everything else.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn channel_send_whatsapp_invalid_jid_rejected() {
    let (capturing, log) = CapturingKernel::new();
    let kernel: Arc<dyn KernelHandle> = capturing.clone();
    let ctx = make_ctx(&kernel, None);

    let input = json!({
        "channel": "whatsapp",
        "recipient": "not_a_jid",
        "message": "x",
    });
    let result = execute_tool_raw("t2", "channel_send", &input, &ctx).await;
    assert!(result.is_error, "expected error for malformed JID");
    assert!(
        result.content.contains("Invalid WhatsApp JID format"),
        "expected JID-format hint in error, got: {}",
        result.content
    );

    // Adapter must NOT be called.
    assert!(log.text.lock().unwrap().is_empty());
    assert!(log.media.lock().unwrap().is_empty());
}

#[tokio::test]
async fn channel_send_whatsapp_jid_variants_accepted() {
    let valid_jids = [
        "393401234567@s.whatsapp.net",   // DM, plain
        "393401234567:1@s.whatsapp.net", // multi-device suffix
        "120363012345678901@g.us",       // group
        "12345@lid",                     // linked-device
    ];
    for jid in valid_jids {
        let (capturing, log) = CapturingKernel::new();
        let kernel: Arc<dyn KernelHandle> = capturing.clone();
        let ctx = make_ctx(&kernel, None);

        let input = json!({
            "channel": "whatsapp",
            "recipient": jid,
            "message": "ciao",
        });
        let result = execute_tool_raw("t-jid", "channel_send", &input, &ctx).await;
        assert!(
            !result.is_error,
            "JID '{jid}' should be accepted, got: {}",
            result.content
        );
        assert_eq!(
            log.text.lock().unwrap().len(),
            1,
            "JID '{jid}' must reach adapter"
        );
    }
}

// ---------------------------------------------------------------------------
// Test 3 — voice_note_routing: as_voice_note=true + file_url + audio mime
// must route through send_channel_media with media_type="voice".
// ---------------------------------------------------------------------------

#[tokio::test]
async fn channel_send_whatsapp_voice_note_routes_via_media() {
    let (capturing, log) = CapturingKernel::new();
    let kernel: Arc<dyn KernelHandle> = capturing.clone();
    let ctx = make_ctx(&kernel, None);

    let input = json!({
        "channel": "whatsapp",
        "recipient": "393401234567@s.whatsapp.net",
        "file_url": "https://example.com/sample.ogg",
        "as_voice_note": true,
        "message": "vn caption",
    });
    let result = execute_tool_raw("t3", "channel_send", &input, &ctx).await;
    assert!(!result.is_error, "expected ok, got: {}", result.content);

    let media = log.media.lock().unwrap();
    assert_eq!(media.len(), 1);
    assert_eq!(media[0].media_type, "voice");
    assert_eq!(media[0].media_url, "https://example.com/sample.ogg");
    assert_eq!(media[0].caption.as_deref(), Some("vn caption"));
    assert_eq!(media[0].channel, "whatsapp");
    // Text branch must NOT fire (single media call only).
    assert!(log.text.lock().unwrap().is_empty());
}

// ---------------------------------------------------------------------------
// Test 4 — reply_to_threading: WhatsApp reply_to_msg_id propagates through
// to the captured kernel call.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn channel_send_whatsapp_reply_to_msg_id_propagates() {
    let (capturing, log) = CapturingKernel::new();
    let kernel: Arc<dyn KernelHandle> = capturing.clone();
    let ctx = make_ctx(&kernel, None);

    let input = json!({
        "channel": "whatsapp",
        "recipient": "393401234567@s.whatsapp.net",
        "message": "re: ciao",
        "reply_to_msg_id": "WAID:abc123",
    });
    let result = execute_tool_raw("t4", "channel_send", &input, &ctx).await;
    assert!(!result.is_error, "expected ok, got: {}", result.content);

    let calls = log.text.lock().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(
        calls[0].reply_to_msg_id.as_deref(),
        Some("WAID:abc123"),
        "reply_to_msg_id must reach the kernel",
    );
}

// ---------------------------------------------------------------------------
// Test 5 — gateway_error_translated: when the adapter returns a structured
// error, the tool must surface it without leaking raw bodies / status codes.
// (Since the kernel-mock can return whatever string it wants, we simply
// verify the tool propagates the kernel's error verbatim and that a
// realistic structured phrase comes back. The status-code-stripping policy
// itself lives in `whatsapp.rs::extract_gateway_error_reason` and is
// covered by adapter-level tests, not the tool wrapper.)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn channel_send_whatsapp_gateway_error_propagates() {
    let (capturing, _log) = CapturingKernel::new();
    capturing.arm_error("WhatsApp send failed: baileys disconnected");
    let kernel: Arc<dyn KernelHandle> = capturing.clone();
    let ctx = make_ctx(&kernel, None);

    let input = json!({
        "channel": "whatsapp",
        "recipient": "393401234567@s.whatsapp.net",
        "message": "test",
    });
    let result = execute_tool_raw("t5", "channel_send", &input, &ctx).await;
    assert!(result.is_error, "expected error result");
    assert!(
        result.content.contains("baileys disconnected"),
        "expected reason phrase in error content, got: {}",
        result.content
    );
    // Tool wrapper must not synthesize a numeric status code on its own.
    assert!(
        !result.content.contains("503") && !result.content.contains("HTTP "),
        "tool result must not leak status codes, got: {}",
        result.content
    );
}

// ---------------------------------------------------------------------------
// Test 6 — as_voice_note_without_audio: rejected before adapter call.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn channel_send_whatsapp_voice_note_without_audio_rejected() {
    let (capturing, log) = CapturingKernel::new();
    let kernel: Arc<dyn KernelHandle> = capturing.clone();
    let ctx = make_ctx(&kernel, None);

    let input = json!({
        "channel": "whatsapp",
        "recipient": "393401234567@s.whatsapp.net",
        "as_voice_note": true,
        "message": "no audio here",
    });
    let result = execute_tool_raw("t6", "channel_send", &input, &ctx).await;
    assert!(
        result.is_error,
        "as_voice_note=true without audio must error"
    );
    assert!(
        result.content.contains("file_url"),
        "expected hint about file_url, got: {}",
        result.content
    );

    assert!(log.text.lock().unwrap().is_empty());
    assert!(log.media.lock().unwrap().is_empty());
}

// ---------------------------------------------------------------------------
// Test 7 — non_whatsapp_ignores_extras: the two new fields are silently
// ignored on telegram. The kernel still receives them via the trait
// (typed-through, not unset) so adapter implementations can opt in later;
// what matters is that the tool does NOT error out.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn channel_send_non_whatsapp_ignores_extras() {
    let (capturing, log) = CapturingKernel::new();
    let kernel: Arc<dyn KernelHandle> = capturing.clone();
    let ctx = make_ctx(&kernel, None);

    let input = json!({
        "channel": "telegram",
        "recipient": "@user",
        "message": "hi",
        "reply_to_msg_id": "ignored-on-telegram",
        "as_voice_note": true,
    });
    let result = execute_tool_raw("t7", "channel_send", &input, &ctx).await;
    assert!(
        !result.is_error,
        "non-WhatsApp channel must ignore extras silently, got: {}",
        result.content
    );

    let calls = log.text.lock().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].channel, "telegram");
    // Tool always forwards the field; it's the adapter's job to decide.
    assert_eq!(
        calls[0].reply_to_msg_id.as_deref(),
        Some("ignored-on-telegram")
    );
}

// ---------------------------------------------------------------------------
// Test 8 — tool definition schema includes the two new properties and the
// description mentions WhatsApp + voice-note semantics. Determinism of
// builtin_tool_definitions is asserted by checking the same Vec is
// produced across repeated invocations (serde_json::json! preserves
// insertion order, but pin it explicitly so a future refactor that swaps
// to a HashMap-backed builder breaks here, not at prompt-cache time).
// ---------------------------------------------------------------------------

#[test]
fn channel_send_schema_exposes_reply_to_and_voice_note() {
    let defs = builtin_tool_definitions();
    let cs = defs
        .iter()
        .find(|d| d.name == "channel_send")
        .expect("channel_send must be registered");

    let props = cs
        .input_schema
        .get("properties")
        .and_then(|v| v.as_object())
        .expect("channel_send schema must have a 'properties' object");

    let rid = props
        .get("reply_to_msg_id")
        .expect("reply_to_msg_id property missing from channel_send schema");
    assert_eq!(rid.get("type").and_then(|v| v.as_str()), Some("string"));
    let rid_desc = rid
        .get("description")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    assert!(
        rid_desc.contains("WhatsApp"),
        "reply_to_msg_id description must mention WhatsApp scope, got: {rid_desc}"
    );

    let vn = props
        .get("as_voice_note")
        .expect("as_voice_note property missing from channel_send schema");
    assert_eq!(vn.get("type").and_then(|v| v.as_str()), Some("boolean"));
    let vn_desc = vn
        .get("description")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    assert!(
        vn_desc.contains("WhatsApp") && vn_desc.contains("voice"),
        "as_voice_note description must mention WhatsApp and voice semantics, got: {vn_desc}"
    );

    // Determinism: same call twice yields the same JSON byte-for-byte.
    let pass1 = serde_json::to_string(&cs.input_schema).unwrap();
    let pass2 = serde_json::to_string(
        &builtin_tool_definitions()
            .into_iter()
            .find(|d| d.name == "channel_send")
            .unwrap()
            .input_schema,
    )
    .unwrap();
    assert_eq!(
        pass1, pass2,
        "channel_send schema must serialize deterministically across calls"
    );
}

// Confirm the symbol `whatsapp_send` was NOT added to the registry.
#[test]
fn no_whatsapp_send_tool_registered() {
    let defs = builtin_tool_definitions();
    assert!(
        defs.iter().all(|d| d.name != "whatsapp_send"),
        "phase 07 PLAN-02 must NOT register a parallel whatsapp_send tool"
    );
}
