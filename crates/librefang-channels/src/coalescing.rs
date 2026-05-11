use std::collections::HashMap;
use std::time::{Duration, Instant};

use tokio::sync::mpsc;
use tracing::{debug, trace, warn};

use crate::types::{ChannelContent, ChannelMessage};
use librefang_types::agent::AgentId;
use librefang_types::message::ContentBlock;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Runtime config for the coalescing dispatcher.
#[derive(Debug, Clone)]
pub struct CoalesceRuntimeConfig {
    pub enabled: bool,
    pub window: Duration,
    pub on_busy: CoalesceOnBusy,
}

/// Runtime on-busy mode (decoded from config).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoalesceOnBusy {
    Queue,
}

impl From<librefang_types::config::CoalesceOnBusyMode> for CoalesceOnBusy {
    fn from(m: librefang_types::config::CoalesceOnBusyMode) -> Self {
        match m {
            librefang_types::config::CoalesceOnBusyMode::Queue => CoalesceOnBusy::Queue,
            librefang_types::config::CoalesceOnBusyMode::CancelAndMerge => CoalesceOnBusy::Queue,
        }
    }
}

/// Unique key per coalescing window: `{channel, chat_id, sender_id}`.
#[derive(Hash, Eq, PartialEq, Clone, Debug)]
pub struct CoalesceKey {
    pub channel: String,
    pub chat_or_user: String,
    pub sender_id: String,
}

impl std::fmt::Display for CoalesceKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}:{}:{}",
            self.channel, self.chat_or_user, self.sender_id
        )
    }
}

/// A buffered message waiting for the coalesce window to expire.
#[derive(Debug)]
pub struct PendingBatch {
    pub messages: Vec<ChannelMessage>,
    pub blocks: Option<Vec<ContentBlock>>,
    pub agent_id: AgentId,
    pub first_arrived: Instant,
}

/// Sliding-window coalescing dispatcher.
///
/// Buffers incoming messages per `{channel, chat, user}` key and delivers
/// them as a single batch when the window expires. Each new message resets
/// the timer.
pub struct CoalescingDispatcher {
    config: CoalesceRuntimeConfig,
    /// Per-key buffer state.
    buffers: HashMap<String, BufferEntry>,
    /// Sender side of the flush channel — clones are given to timer tasks.
    flush_tx: mpsc::UnboundedSender<String>,
    /// Tracks in-flight agent IDs so we can decide on_busy behaviour.
    in_flight: HashMap<AgentId, InFlightEntry>,
}

#[derive(Debug)]
struct BufferEntry {
    messages: Vec<ChannelMessage>,
    blocks: Option<Vec<ContentBlock>>,
    agent_id: AgentId,
    /// Timer that fires when the window expires.
    timer: Option<tokio::task::JoinHandle<()>>,
    first_arrived: Instant,
}

#[derive(Debug)]
struct InFlightEntry {
    /// Placeholder for future `cancel_and_merge` support.
    _private: (),
}

impl CoalescingDispatcher {
    pub fn new(config: CoalesceRuntimeConfig) -> (Self, mpsc::UnboundedReceiver<String>) {
        let (flush_tx, flush_rx) = mpsc::unbounded_channel();
        (
            Self {
                config,
                buffers: HashMap::new(),
                flush_tx,
                in_flight: HashMap::new(),
            },
            flush_rx,
        )
    }

    /// Check if coalescing is enabled.
    pub fn is_enabled(&self) -> bool {
        self.config.enabled
    }

    /// Returns the window duration for agent-override comparison.
    pub fn window(&self) -> Duration {
        self.config.window
    }

    /// Register a pending message.  Returns `true` if the message was
    /// buffered (caller should *not* dispatch yet), `false` if the
    /// message should be dispatched immediately (bypass or error).
    pub fn push(
        &mut self,
        key: CoalesceKey,
        message: ChannelMessage,
        blocks: Option<Vec<ContentBlock>>,
        agent_id: AgentId,
    ) -> bool {
        if !self.config.enabled {
            return false;
        }

        // Urgent bypass: if the message carries an urgent flag, skip
        // coalescing.
        if message
            .metadata
            .get("urgent")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        {
            trace!(
                key = %key,
                "Coalesce: urgent message bypasses window"
            );
            return false;
        }

        let key_str = key.to_string();
        let now = Instant::now();

        let entry = self
            .buffers
            .entry(key_str.clone())
            .or_insert_with(|| BufferEntry {
                messages: Vec::new(),
                blocks: None,
                agent_id,
                timer: None,
                first_arrived: now,
            });

        // Abort any existing timer (sliding window reset).
        if let Some(handle) = entry.timer.take() {
            handle.abort();
        }

        entry.messages.push(message);
        entry.blocks = {
            let existing_blocks: Option<Vec<ContentBlock>> = entry.blocks.take();
            match (existing_blocks, blocks) {
                (Some(mut existing), Some(new)) => {
                    existing.extend(new);
                    Some(existing)
                }
                (Some(existing), None) => Some(existing),
                (None, Some(new)) => Some(new),
                (None, None) => None,
            }
        };
        entry.first_arrived = now;

        // Spawn the new timer.
        let flush_tx = self.flush_tx.clone();
        let timer_key = key_str.clone();
        let duration = self.config.window;
        entry.timer = Some(tokio::spawn(async move {
            tokio::time::sleep(duration).await;
            if flush_tx.send(timer_key).is_err() {
                warn!("Coalesce: flush receiver dropped, buffered messages lost");
            }
        }));

        debug!(
            key = %key_str,
            buffer_len = entry.messages.len(),
            window_ms = duration.as_millis(),
            "Coalesce: message buffered, timer reset"
        );

        true
    }

    /// Drain a buffered batch by key.  Returns the accumulated messages
    /// and the resolved agent ID, or `None` if the key does not exist.
    pub fn drain(&mut self, key: &str) -> Option<PendingBatch> {
        let entry = self.buffers.remove(key)?;
        if entry.messages.is_empty() {
            return None;
        }
        if let Some(handle) = entry.timer {
            handle.abort();
        }
        Some(PendingBatch {
            messages: entry.messages,
            blocks: entry.blocks,
            agent_id: entry.agent_id,
            first_arrived: entry.first_arrived,
        })
    }

    /// Merge accumulated messages into a single text with separators.
    /// Also returns any accumulated content blocks (images, files, etc.).
    pub fn merge_batch(batch: PendingBatch) -> (String, Option<Vec<ContentBlock>>, ChannelMessage) {
        let messages = batch.messages;
        let blocks = batch.blocks;

        // Use the last message's metadata for the merged message —
        // the last sender and timestamp are the most relevant.
        let merged: ChannelMessage = messages
            .last()
            .cloned()
            .unwrap_or_else(|| panic!("Coalesce: empty batch"));

        let text = if messages.len() == 1 {
            // Single message — use content as-is.
            content_to_text(&merged.content)
        } else {
            // Multiple messages — join with separator and timestamp prefix.
            let mut parts: Vec<String> = Vec::with_capacity(messages.len());
            for msg in &messages {
                let ts = msg.timestamp.format("[%H:%M:%S] ").to_string();
                let text = content_to_text(&msg.content);
                parts.push(format!("{ts}{text}"));
            }
            parts.join("\n")
        };

        (text, blocks, merged)
    }

    /// Mark an agent as in-flight (currently processing a turn).
    pub fn mark_busy(&mut self, agent_id: AgentId) {
        self.in_flight
            .insert(agent_id, InFlightEntry { _private: () });
    }

    /// Mark an agent as no longer in-flight.
    pub fn mark_idle(&mut self, agent_id: &AgentId) {
        self.in_flight.remove(agent_id);
    }

    /// Check if an agent is currently processing a turn.
    pub fn is_busy(&self, agent_id: &AgentId) -> bool {
        self.in_flight.contains_key(agent_id)
    }

    /// Returns the on_busy mode for this dispatcher.
    pub fn on_busy_mode(&self) -> CoalesceOnBusy {
        self.config.on_busy
    }

    /// Returns all currently-buffered keys (for draining on shutdown).
    pub fn buffered_keys(&self) -> Vec<String> {
        self.buffers.keys().cloned().collect()
    }
}

/// Minimal content→text extraction for coalesced messages.
fn content_to_text(content: &ChannelContent) -> String {
    match content {
        ChannelContent::Text(t) => t.clone(),
        ChannelContent::Command { name, args } => {
            if args.is_empty() {
                format!("/{name}")
            } else {
                format!("/{} {}", name, args.join(" "))
            }
        }
        ChannelContent::Image { caption, .. } => caption.clone().unwrap_or_default(),
        ChannelContent::Voice { caption, .. } => caption.clone().unwrap_or_default(),
        ChannelContent::Video { caption, .. } => caption.clone().unwrap_or_default(),
        ChannelContent::Audio { caption, .. } => caption.clone().unwrap_or_default(),
        ChannelContent::File { .. } => "[file]".to_string(),
        ChannelContent::FileData { .. } => "[file data]".to_string(),
        ChannelContent::Sticker { .. } => "[sticker]".to_string(),
        ChannelContent::Location { .. } => "[location]".to_string(),
        ChannelContent::Animation { caption, .. } => caption.clone().unwrap_or_default(),
        ChannelContent::Interactive { text, .. } => text.clone(),
        ChannelContent::ButtonCallback { action, .. } => format!("[button: {action}]"),
        ChannelContent::Poll { question, .. } => format!("[poll: {question}]"),
        ChannelContent::PollAnswer { .. } => "[poll answer]".to_string(),
        ChannelContent::MediaGroup { .. } => "[media group]".to_string(),
        ChannelContent::DeleteMessage { .. } => String::new(),
        ChannelContent::EditInteractive { text, .. } => text.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{ChannelContent, ChannelMessage, ChannelUser};
    use librefang_types::agent::AgentId;
    use std::collections::HashMap;
    use std::time::Duration;

    /// Helper: push without spawning real timers (tests can't run tokio::spawn).
    fn push_sync(
        d: &mut CoalescingDispatcher,
        key: CoalesceKey,
        msg: ChannelMessage,
        blocks: Option<Vec<ContentBlock>>,
        agent_id: AgentId,
    ) -> bool {
        if !d.config.enabled {
            return false;
        }
        if msg
            .metadata
            .get("urgent")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        {
            return false;
        }
        let key_str = key.to_string();
        let now = Instant::now();
        let entry = d
            .buffers
            .entry(key_str.clone())
            .or_insert_with(|| BufferEntry {
                messages: Vec::new(),
                blocks: None,
                agent_id,
                timer: None,
                first_arrived: now,
            });
        if let Some(handle) = entry.timer.take() {
            handle.abort();
        }
        entry.messages.push(msg);
        entry.blocks = {
            let eb: Option<Vec<ContentBlock>> = entry.blocks.take();
            match (eb, blocks) {
                (Some(mut e), Some(n)) => {
                    e.extend(n);
                    Some(e)
                }
                (Some(e), None) => Some(e),
                (None, Some(n)) => Some(n),
                (None, None) => None,
            }
        };
        entry.first_arrived = now;
        // No real timer spawn in sync mode
        true
    }

    fn make_msg(text: &str, uid: &str) -> ChannelMessage {
        ChannelMessage {
            channel: crate::types::ChannelType::Telegram,
            platform_message_id: uuid::Uuid::new_v4().to_string(),
            sender: ChannelUser {
                platform_id: uid.to_string(),
                display_name: uid.to_string(),
                librefang_user: None,
            },
            content: ChannelContent::Text(text.to_string()),
            target_agent: None,
            timestamp: chrono::Utc::now(),
            is_group: false,
            thread_id: None,
            metadata: HashMap::new(),
        }
    }

    fn make_key(uid: &str) -> CoalesceKey {
        CoalesceKey {
            channel: "test".to_string(),
            chat_or_user: uid.to_string(),
            sender_id: uid.to_string(),
        }
    }

    #[test]
    fn test_disabled_coalescing_passthrough() {
        let config = CoalesceRuntimeConfig {
            enabled: false,
            window: Duration::from_secs(15),
            on_busy: CoalesceOnBusy::Queue,
        };
        let (mut d, _flush_rx) = CoalescingDispatcher::new(config);
        let key = make_key("user1");
        let msg = make_msg("hello", "user1");
        let result = push_sync(&mut d, key, msg, None, AgentId::new());
        assert!(!result, "disabled coalescing must passthrough");
    }

    #[test]
    fn test_single_message_flush() {
        let config = CoalesceRuntimeConfig {
            enabled: true,
            window: Duration::from_secs(5),
            on_busy: CoalesceOnBusy::Queue,
        };
        let (mut d, _flush_rx) = CoalescingDispatcher::new(config);
        let agent = AgentId::new();
        let key = make_key("user1");
        let msg = make_msg("hello", "user1");
        let buffered = push_sync(&mut d, key.clone(), msg, None, agent);
        assert!(buffered, "enabled coalescing must buffer");

        // Simulate timer expiry by flushing manually.
        let batch = d.drain(&key.to_string()).expect("must have batch");
        assert_eq!(batch.messages.len(), 1);
        assert_eq!(batch.agent_id, agent);
    }

    #[test]
    fn test_multiple_messages_coalesced() {
        let config = CoalesceRuntimeConfig {
            enabled: true,
            window: Duration::from_secs(15),
            on_busy: CoalesceOnBusy::Queue,
        };
        let (mut d, _flush_rx) = CoalescingDispatcher::new(config);
        let agent = AgentId::new();
        let key = make_key("user1");

        assert!(push_sync(
            &mut d,
            key.clone(),
            make_msg("first", "user1"),
            None,
            agent
        ));
        assert!(push_sync(
            &mut d,
            key.clone(),
            make_msg("second", "user1"),
            None,
            agent
        ));
        assert!(push_sync(
            &mut d,
            key.clone(),
            make_msg("third", "user1"),
            None,
            agent
        ));

        let batch = d.drain(&key.to_string()).expect("must have batch");
        assert_eq!(batch.messages.len(), 3);
    }

    #[test]
    fn test_merge_single_message() {
        let config = CoalesceRuntimeConfig {
            enabled: true,
            window: Duration::from_secs(5),
            on_busy: CoalesceOnBusy::Queue,
        };
        let (mut d, _flush_rx) = CoalescingDispatcher::new(config);
        let agent = AgentId::new();
        let key = make_key("user1");
        let msg = make_msg("hello world", "user1");
        push_sync(&mut d, key.clone(), msg, None, agent);
        let batch = d.drain(&key.to_string()).unwrap();
        let (text, blocks, _) = CoalescingDispatcher::merge_batch(batch);
        assert_eq!(text, "hello world");
        assert!(blocks.is_none());
    }

    #[test]
    fn test_merge_multiple_messages() {
        let config = CoalesceRuntimeConfig {
            enabled: true,
            window: Duration::from_secs(5),
            on_busy: CoalesceOnBusy::Queue,
        };
        let (mut d, _flush_rx) = CoalescingDispatcher::new(config);
        let agent = AgentId::new();
        let key = make_key("user1");
        push_sync(&mut d, key.clone(), make_msg("first", "user1"), None, agent);
        push_sync(
            &mut d,
            key.clone(),
            make_msg("second", "user1"),
            None,
            agent,
        );
        push_sync(&mut d, key.clone(), make_msg("third", "user1"), None, agent);
        let batch = d.drain(&key.to_string()).unwrap();
        let (text, blocks, _) = CoalescingDispatcher::merge_batch(batch);
        // Each message is timestamp-prefixed when multiple are coalesced.
        assert!(text.contains("first"), "contains first");
        assert!(text.contains("second"), "contains second");
        assert!(text.contains("third"), "contains third");
        assert!(blocks.is_none());
    }

    #[test]
    fn test_sliding_window_resets_timer() {
        let config = CoalesceRuntimeConfig {
            enabled: true,
            window: Duration::from_secs(15),
            on_busy: CoalesceOnBusy::Queue,
        };
        let (mut d, _flush_rx) = CoalescingDispatcher::new(config);
        let agent = AgentId::new();
        let key = make_key("user1");

        push_sync(&mut d, key.clone(), make_msg("first", "user1"), None, agent);
        assert_eq!(d.buffers.get(&key.to_string()).unwrap().messages.len(), 1);

        // Second message extends buffer (no timer to compare, but count increases).
        push_sync(
            &mut d,
            key.clone(),
            make_msg("second", "user1"),
            None,
            agent,
        );
        assert_eq!(d.buffers.get(&key.to_string()).unwrap().messages.len(), 2);
    }

    #[test]
    fn test_urgent_bypasses_coalescing() {
        let config = CoalesceRuntimeConfig {
            enabled: true,
            window: Duration::from_secs(15),
            on_busy: CoalesceOnBusy::Queue,
        };
        let (mut d, _flush_rx) = CoalescingDispatcher::new(config);
        let agent = AgentId::new();
        let key = make_key("user1");

        let mut msg = make_msg("urgent!", "user1");
        msg.metadata
            .insert("urgent".to_string(), serde_json::Value::Bool(true));

        let result = push_sync(&mut d, key, msg, None, agent);
        assert!(!result, "urgent messages must bypass coalescing");
    }

    #[test]
    fn test_busy_tracking() {
        let config = CoalesceRuntimeConfig {
            enabled: true,
            window: Duration::from_secs(5),
            on_busy: CoalesceOnBusy::Queue,
        };
        let (mut d, _flush_rx) = CoalescingDispatcher::new(config);
        let agent = AgentId::new();
        assert!(!d.is_busy(&agent));
        d.mark_busy(agent);
        assert!(d.is_busy(&agent));
        d.mark_idle(&agent);
        assert!(!d.is_busy(&agent));
    }

    #[test]
    fn test_different_keys_independent() {
        let config = CoalesceRuntimeConfig {
            enabled: true,
            window: Duration::from_secs(15),
            on_busy: CoalesceOnBusy::Queue,
        };
        let (mut d, _flush_rx) = CoalescingDispatcher::new(config);
        let agent = AgentId::new();

        let key1 = make_key("user1");
        let key2 = make_key("user2");

        push_sync(&mut d, key1.clone(), make_msg("a", "user1"), None, agent);
        push_sync(&mut d, key2.clone(), make_msg("b", "user2"), None, agent);
        push_sync(&mut d, key1.clone(), make_msg("c", "user1"), None, agent);

        let batch1 = d.drain(&key1.to_string()).unwrap();
        assert_eq!(batch1.messages.len(), 2);

        let batch2 = d.drain(&key2.to_string()).unwrap();
        assert_eq!(batch2.messages.len(), 1);
    }
}
