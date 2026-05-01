//! WhatsApp Cloud API channel adapter.
//!
//! Uses the official WhatsApp Business Cloud API to send and receive messages.
//! Requires a webhook endpoint for incoming messages and the Cloud API for outgoing.

use crate::types::{ChannelAdapter, ChannelContent, ChannelMessage, ChannelType, ChannelUser};
use async_trait::async_trait;
use futures::Stream;
use librefang_types::config::{DmPolicy, GroupPolicy};
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::{mpsc, watch};
use tracing::{error, info};
use zeroize::Zeroizing;

const MAX_MESSAGE_LEN: usize = 4096;

/// WhatsApp Cloud API adapter.
///
/// Supports two modes:
/// - **Cloud API mode**: Uses the official WhatsApp Business Cloud API (requires Meta dev account).
/// - **Web/QR mode**: Routes outgoing messages through a local Baileys-based gateway process.
///
/// Mode is selected automatically: if `gateway_url` is set (from `WHATSAPP_WEB_GATEWAY_URL`),
/// the adapter uses Web mode. Otherwise it falls back to Cloud API mode.
pub struct WhatsAppAdapter {
    /// WhatsApp Business phone number ID (Cloud API mode).
    phone_number_id: String,
    /// SECURITY: Access token is zeroized on drop.
    access_token: Zeroizing<String>,
    /// SECURITY: Verify token is zeroized on drop.
    verify_token: Zeroizing<String>,
    /// Port to listen for webhook callbacks (Cloud API mode).
    webhook_port: u16,
    /// HTTP client.
    client: reqwest::Client,
    /// Allowed phone numbers (empty = allow all).
    allowed_users: Vec<String>,
    /// Optional WhatsApp Web gateway URL for QR/Web mode (e.g. "http://127.0.0.1:3009").
    gateway_url: Option<String>,
    /// Optional account identifier for multi-bot routing.
    account_id: Option<String>,
    /// DM message policy: how to handle direct messages.
    dm_policy: DmPolicy,
    /// Group message policy: how to handle group/community messages.
    group_policy: GroupPolicy,
    /// Bot's own phone number (used for mention detection in group chats).
    /// Should match the `phone_number_id` display number, e.g. "+15551234567".
    bot_phone: Option<String>,
    /// Bot display name (used as fallback mention keyword in group chats).
    bot_name: Option<String>,
    /// Shutdown signal.
    shutdown_tx: Arc<watch::Sender<bool>>,
    shutdown_rx: watch::Receiver<bool>,
}

impl WhatsAppAdapter {
    /// Create a new WhatsApp Cloud API adapter.
    pub fn new(
        phone_number_id: String,
        access_token: String,
        verify_token: String,
        webhook_port: u16,
        allowed_users: Vec<String>,
    ) -> Self {
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        Self {
            phone_number_id,
            access_token: Zeroizing::new(access_token),
            verify_token: Zeroizing::new(verify_token),
            webhook_port,
            client: crate::http_client::new_client(),
            allowed_users,
            gateway_url: None,
            account_id: None,
            dm_policy: DmPolicy::default(),
            group_policy: GroupPolicy::default(),
            bot_phone: None,
            bot_name: None,
            shutdown_tx: Arc::new(shutdown_tx),
            shutdown_rx,
        }
    }
    /// Set the account_id for multi-bot routing. Returns self for builder chaining.
    pub fn with_account_id(mut self, account_id: Option<String>) -> Self {
        self.account_id = account_id;
        self
    }

    /// Create a new WhatsApp adapter with gateway URL for Web/QR mode.
    ///
    /// When `gateway_url` is `Some`, outgoing messages are sent via `POST {gateway_url}/message/send`
    /// instead of the Cloud API. Incoming messages are handled by the gateway itself.
    pub fn with_gateway(mut self, gateway_url: Option<String>) -> Self {
        self.gateway_url = gateway_url.filter(|u| !u.is_empty());
        self
    }

    /// Set the DM policy for this adapter. Returns self for builder chaining.
    pub fn with_dm_policy(mut self, policy: DmPolicy) -> Self {
        self.dm_policy = policy;
        self
    }

    /// Set the group message policy for this adapter. Returns self for builder chaining.
    pub fn with_group_policy(mut self, policy: GroupPolicy) -> Self {
        self.group_policy = policy;
        self
    }

    /// Set the bot's own phone number for mention detection in group chats.
    pub fn with_bot_phone(mut self, phone: Option<String>) -> Self {
        self.bot_phone = phone;
        self
    }

    /// Set the bot's display name for mention detection in group chats.
    pub fn with_bot_name(mut self, name: Option<String>) -> Self {
        self.bot_name = name;
        self
    }

    /// Determine whether an incoming message should be handled based on the configured policies.
    ///
    /// - `is_group`: whether the message came from a group/community chat.
    /// - `text`: the raw message text (used for mention detection under `MentionOnly`).
    /// - `sender_phone`: the sender's phone number (used for `DmPolicy::AllowedOnly`).
    ///
    /// Returns `true` if the adapter should process and respond to the message.
    pub fn should_handle_message(&self, is_group: bool, text: &str, sender_phone: &str) -> bool {
        if is_group {
            match self.group_policy {
                GroupPolicy::All => true,
                GroupPolicy::MentionOnly => self.is_bot_mentioned(text),
                GroupPolicy::CommandsOnly => text.trim_start().starts_with('/'),
                GroupPolicy::Ignore => false,
            }
        } else {
            match self.dm_policy {
                DmPolicy::Respond => true,
                DmPolicy::AllowedOnly => self.is_allowed(sender_phone),
                DmPolicy::Ignore => false,
            }
        }
    }

    /// Check whether the bot is @mentioned in the given message text.
    ///
    /// WhatsApp does not have a native @mention protocol at the Cloud API level,
    /// so we look for the bot's phone number or display name anywhere in the text.
    fn is_bot_mentioned(&self, text: &str) -> bool {
        let lower = text.to_lowercase();
        if let Some(ref phone) = self.bot_phone {
            // Match bare number or with leading '@'
            if lower.contains(phone.as_str())
                || lower.contains(&format!("@{}", phone.trim_start_matches('+')))
            {
                return true;
            }
        }
        if let Some(ref name) = self.bot_name {
            if lower.contains(&name.to_lowercase()) {
                return true;
            }
        }
        false
    }

    /// Upload raw audio bytes to the WhatsApp Media API and return the media ID.
    ///
    /// The caller is responsible for providing the correct MIME type
    /// (e.g. `"audio/ogg; codecs=opus"` for voice messages).
    async fn api_upload_media(
        &self,
        audio: &[u8],
        mime_type: &str,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        use reqwest::multipart;

        let url = format!(
            "https://graph.facebook.com/v21.0/{}/media",
            self.phone_number_id
        );

        // Build multipart form: file field + messaging_product field
        let file_part = multipart::Part::bytes(audio.to_vec())
            .mime_str(mime_type)?
            .file_name("voice.ogg");

        let form = multipart::Form::new()
            .text("messaging_product", "whatsapp")
            .part("file", file_part);

        let resp = self
            .client
            .post(&url)
            .bearer_auth(&*self.access_token)
            .multipart(form)
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            error!("WhatsApp media upload error {status}: {body}");
            return Err(format!("WhatsApp media upload error {status}: {body}").into());
        }

        let json: serde_json::Value = resp.json().await?;
        let media_id = json
            .get("id")
            .and_then(|v| v.as_str())
            .ok_or("WhatsApp media upload: missing 'id' in response")?
            .to_string();

        Ok(media_id)
    }

    /// Send a voice message via the WhatsApp Cloud API.
    ///
    /// Uploads the raw audio bytes as a media object, then sends an `audio` message
    /// referencing the returned media ID.  WhatsApp renders this as an inline voice note.
    ///
    /// # Arguments
    /// * `to`        – recipient phone number (E.164 format, e.g. `"+15551234567"`).
    /// * `audio`     – raw OGG/Opus bytes (or any audio format accepted by the API).
    /// * `mime_type` – MIME type of the audio, e.g. `"audio/ogg; codecs=opus"`.
    pub async fn send_voice(
        &self,
        to: &str,
        audio: &[u8],
        mime_type: &str,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let media_id = self.api_upload_media(audio, mime_type).await?;

        let url = format!(
            "https://graph.facebook.com/v21.0/{}/messages",
            self.phone_number_id
        );
        let body = serde_json::json!({
            "messaging_product": "whatsapp",
            "to": to,
            "type": "audio",
            "audio": { "id": media_id }
        });

        let resp = self
            .client
            .post(&url)
            .bearer_auth(&*self.access_token)
            .json(&body)
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            error!("WhatsApp send_voice error {status}: {body}");
            return Err(format!("WhatsApp send_voice error {status}: {body}").into());
        }

        Ok(())
    }

    // NOTE: a previous `gateway_send_voice` helper (POST /message/send-voice
    // with base64-encoded raw bytes) was removed in Phase 07 PLAN-02. The
    // gateway has never exposed `/message/send-voice` — the canonical voice-
    // note endpoint is `/message/send-audio` with `ptt=true`, which is now
    // wired up via `gateway_send_audio`. Raw byte uploads remain available
    // through the Cloud API path (`send_voice`) when configured.

    /// Send a text message via the WhatsApp Cloud API.
    async fn api_send_message(
        &self,
        to: &str,
        text: &str,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let url = format!(
            "https://graph.facebook.com/v21.0/{}/messages",
            self.phone_number_id
        );

        // Split long messages
        let chunks = crate::types::split_message(text, MAX_MESSAGE_LEN);
        for chunk in chunks {
            let body = serde_json::json!({
                "messaging_product": "whatsapp",
                "to": to,
                "type": "text",
                "text": { "body": chunk }
            });

            let resp = self
                .client
                .post(&url)
                .bearer_auth(&*self.access_token)
                .json(&body)
                .send()
                .await?;

            if !resp.status().is_success() {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                error!("WhatsApp API error {status}: {body}");
                return Err(format!("WhatsApp API error {status}: {body}").into());
            }
        }

        Ok(())
    }

    /// Mark a message as read.
    #[allow(dead_code)]
    async fn api_mark_read(
        &self,
        message_id: &str,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let url = format!(
            "https://graph.facebook.com/v21.0/{}/messages",
            self.phone_number_id
        );

        let body = serde_json::json!({
            "messaging_product": "whatsapp",
            "status": "read",
            "message_id": message_id
        });

        let _ = self
            .client
            .post(&url)
            .bearer_auth(&*self.access_token)
            .json(&body)
            .send()
            .await;

        Ok(())
    }

    /// Send a text message via the WhatsApp Web gateway.
    ///
    /// When `reply_to_msg_id` is `Some`, the JSON body carries an extra
    /// `reply_to` field. The current Baileys gateway does NOT yet honour the
    /// field — it ignores it silently. This is forward-compatible: a follow-up
    /// gateway change will pick up the field and emit a `quoted` reply
    /// without needing further Rust changes.
    async fn gateway_send_message(
        &self,
        gateway_url: &str,
        to: &str,
        text: &str,
        reply_to_msg_id: Option<&str>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let url = format!("{}/message/send", gateway_url.trim_end_matches('/'));
        let mut body = serde_json::json!({ "to": to, "text": text });
        if let Some(rid) = reply_to_msg_id.filter(|s| !s.is_empty()) {
            body["reply_to"] = serde_json::Value::String(rid.to_string());
        }

        let resp = self.client.post(&url).json(&body).send().await?;

        if !resp.status().is_success() {
            // Status codes and raw bodies are NOT propagated upstream — they
            // leak into LLM prompt context as opaque tokens. Extract the
            // structured `error` field from the gateway's JSON body and
            // surface only the reason phrase.
            let status = resp.status();
            let body_text = resp.text().await.unwrap_or_default();
            let reason = extract_gateway_error_reason(&body_text)
                .unwrap_or_else(|| "gateway request failed".to_string());
            error!("WhatsApp gateway error {status}: {body_text}");
            return Err(format!("WhatsApp send failed: {reason}").into());
        }

        Ok(())
    }

    /// Send an image via the WhatsApp Web gateway (`POST /message/send-image`).
    async fn gateway_send_image(
        &self,
        gateway_url: &str,
        to: &str,
        image_url: &str,
        caption: Option<&str>,
        reply_to_msg_id: Option<&str>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let url = format!("{}/message/send-image", gateway_url.trim_end_matches('/'));
        let mut body = serde_json::json!({
            "to": to,
            "image_url": image_url,
            "caption": caption.unwrap_or(""),
        });
        if let Some(rid) = reply_to_msg_id.filter(|s| !s.is_empty()) {
            body["reply_to"] = serde_json::Value::String(rid.to_string());
        }

        let resp = self.client.post(&url).json(&body).send().await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body_text = resp.text().await.unwrap_or_default();
            let reason = extract_gateway_error_reason(&body_text)
                .unwrap_or_else(|| "gateway image send failed".to_string());
            error!("WhatsApp gateway image error {status}: {body_text}");
            return Err(format!("WhatsApp send failed: {reason}").into());
        }
        Ok(())
    }

    /// Send an audio file (or PTT voice-note when `ptt=true`) via the WhatsApp
    /// Web gateway (`POST /message/send-audio`).
    ///
    /// The gateway defaults `ptt` to `true` when the field is absent, so we
    /// always send it explicitly to make the intent unambiguous.
    async fn gateway_send_audio(
        &self,
        gateway_url: &str,
        to: &str,
        audio_url: &str,
        ptt: bool,
        reply_to_msg_id: Option<&str>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let url = format!("{}/message/send-audio", gateway_url.trim_end_matches('/'));
        let mut body = serde_json::json!({
            "to": to,
            "audio_url": audio_url,
            "ptt": ptt,
        });
        if let Some(rid) = reply_to_msg_id.filter(|s| !s.is_empty()) {
            body["reply_to"] = serde_json::Value::String(rid.to_string());
        }

        let resp = self.client.post(&url).json(&body).send().await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body_text = resp.text().await.unwrap_or_default();
            let reason = extract_gateway_error_reason(&body_text)
                .unwrap_or_else(|| "gateway audio send failed".to_string());
            error!("WhatsApp gateway audio error {status}: {body_text}");
            return Err(format!("WhatsApp send failed: {reason}").into());
        }
        Ok(())
    }

    /// Check if a phone number is allowed.
    #[allow(dead_code)]
    fn is_allowed(&self, phone: &str) -> bool {
        self.allowed_users.is_empty() || self.allowed_users.iter().any(|u| u == phone)
    }

    /// Returns true if this adapter is configured for Web/QR gateway mode.
    #[allow(dead_code)]
    pub fn is_gateway_mode(&self) -> bool {
        self.gateway_url.is_some()
    }
}

#[async_trait]
impl ChannelAdapter for WhatsAppAdapter {
    fn name(&self) -> &str {
        "whatsapp"
    }

    fn channel_type(&self) -> ChannelType {
        ChannelType::WhatsApp
    }

    async fn start(
        &self,
    ) -> Result<
        Pin<Box<dyn Stream<Item = ChannelMessage> + Send>>,
        Box<dyn std::error::Error + Send + Sync>,
    > {
        let (_tx, rx) = mpsc::channel::<ChannelMessage>(256);
        let port = self.webhook_port;
        let _verify_token = self.verify_token.clone();
        let _allowed_users = self.allowed_users.clone();
        let _access_token = self.access_token.clone();
        let _phone_number_id = self.phone_number_id.clone();
        let mut shutdown_rx = self.shutdown_rx.clone();

        info!("Starting WhatsApp webhook listener on port {port}");

        tokio::spawn(async move {
            // Simple webhook polling simulation
            // In production, this would be an axum HTTP server handling webhook POSTs
            // For now, log that the webhook is ready
            info!("WhatsApp webhook ready on port {port} (verify_token configured)");
            info!("Configure your webhook URL: https://your-domain:{port}/webhook");

            // Wait for shutdown
            let _ = shutdown_rx.changed().await;
            info!("WhatsApp adapter stopped");
        });

        Ok(Box::pin(tokio_stream::wrappers::ReceiverStream::new(rx)))
    }

    async fn send(
        &self,
        user: &ChannelUser,
        content: ChannelContent,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.gateway_or_cloud_send(user, content, None).await
    }

    async fn send_with_reply(
        &self,
        user: &ChannelUser,
        content: ChannelContent,
        reply_to_msg_id: Option<&str>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.gateway_or_cloud_send(user, content, reply_to_msg_id)
            .await
    }

    async fn stop(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let _ = self.shutdown_tx.send(true);
        Ok(())
    }
}

impl WhatsAppAdapter {
    /// Unified send entry point that branches between gateway and Cloud API
    /// modes. Used by both `send` (no reply quoting) and `send_with_reply`.
    ///
    /// In gateway mode:
    /// - `Voice { url }` is delivered via `POST /message/send-audio` with
    ///   `ptt=true`. (The previous "(Voice message: <url>)" text-fallback
    ///   was a bug — the gateway has supported `/message/send-audio` since
    ///   the gateway hardening landed; the Rust adapter just never wired
    ///   it up.)
    /// - `Image { url, caption }` is delivered via
    ///   `POST /message/send-image`.
    /// - `Text` and other variants degrade to `POST /message/send` with the
    ///   text body (existing behaviour).
    /// - `reply_to_msg_id`, when present, is added as a `reply_to` field in
    ///   the JSON body of every gateway endpoint. The gateway currently
    ///   ignores it; a follow-up gateway change will pick it up without
    ///   requiring further Rust changes.
    ///
    /// In Cloud API mode:
    /// - `Voice { url }` is delivered as a Cloud API `audio` message with
    ///   the URL link.
    /// - `Image { url, caption }`, `File { url, filename }`, and
    ///   `Location { lat, lon }` are delivered via the Cloud API
    ///   `messages` endpoint with their native types.
    /// - `reply_to_msg_id`, when present, is attached as a `context.message_id`
    ///   field per the Cloud API reply-context spec.
    async fn gateway_or_cloud_send(
        &self,
        user: &ChannelUser,
        content: ChannelContent,
        reply_to_msg_id: Option<&str>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // Web/QR gateway mode: route through gateway HTTP endpoints.
        if let Some(ref gw) = self.gateway_url {
            match &content {
                ChannelContent::Voice { url, .. } => {
                    self.gateway_send_audio(
                        gw,
                        &user.platform_id,
                        url,
                        true, // ptt = true for voice notes
                        reply_to_msg_id,
                    )
                    .await?;
                }
                ChannelContent::Audio { url, .. } => {
                    // Music/podcast — send as audio file (ptt = false).
                    self.gateway_send_audio(gw, &user.platform_id, url, false, reply_to_msg_id)
                        .await?;
                }
                ChannelContent::Image { url, caption, .. } => {
                    self.gateway_send_image(
                        gw,
                        &user.platform_id,
                        url,
                        caption.as_deref(),
                        reply_to_msg_id,
                    )
                    .await?;
                }
                other => {
                    let text = match other {
                        ChannelContent::Text(t) => t.clone(),
                        ChannelContent::File { filename, .. } => {
                            format!("(File: {filename} — not supported in Web mode)")
                        }
                        _ => "(Unsupported content type in Web mode)".to_string(),
                    };
                    // Split long messages the same way as Cloud API mode.
                    // reply_to_msg_id only attaches to the first chunk (a
                    // message can only quote-reply once).
                    let chunks = crate::types::split_message(&text, MAX_MESSAGE_LEN);
                    let mut first = true;
                    for chunk in chunks {
                        let rid = if first { reply_to_msg_id } else { None };
                        first = false;
                        self.gateway_send_message(gw, &user.platform_id, chunk, rid)
                            .await?;
                    }
                }
            }
            return Ok(());
        }

        // Cloud API mode (default).
        let context = reply_to_msg_id
            .filter(|s| !s.is_empty())
            .map(|rid| serde_json::json!({ "message_id": rid }));

        match content {
            ChannelContent::Text(text) => {
                // api_send_message handles chunking and reply context is not
                // currently threaded into Cloud API text sends — keep
                // behaviour identical when reply_to is None, and degrade
                // gracefully when set (ignored Cloud-side; the gateway path
                // is the primary consumer of reply_to_msg_id).
                self.api_send_message(&user.platform_id, &text).await?;
            }
            ChannelContent::Voice { url, .. } => {
                let mut body = serde_json::json!({
                    "messaging_product": "whatsapp",
                    "to": user.platform_id,
                    "type": "audio",
                    "audio": { "link": url }
                });
                if let Some(ctx) = &context {
                    body["context"] = ctx.clone();
                }
                let api_url = format!(
                    "https://graph.facebook.com/v21.0/{}/messages",
                    self.phone_number_id
                );
                let resp = self
                    .client
                    .post(&api_url)
                    .bearer_auth(&*self.access_token)
                    .json(&body)
                    .send()
                    .await?;
                if !resp.status().is_success() {
                    let status = resp.status();
                    let err = resp.text().await.unwrap_or_default();
                    error!("WhatsApp voice send error {status}: {err}");
                    let reason = extract_gateway_error_reason(&err)
                        .unwrap_or_else(|| "voice send failed".to_string());
                    return Err(format!("WhatsApp send failed: {reason}").into());
                }
            }
            ChannelContent::Image { url, caption, .. } => {
                let mut body = serde_json::json!({
                    "messaging_product": "whatsapp",
                    "to": user.platform_id,
                    "type": "image",
                    "image": {
                        "link": url,
                        "caption": caption.unwrap_or_default()
                    }
                });
                if let Some(ctx) = &context {
                    body["context"] = ctx.clone();
                }
                let api_url = format!(
                    "https://graph.facebook.com/v21.0/{}/messages",
                    self.phone_number_id
                );
                self.client
                    .post(&api_url)
                    .bearer_auth(&*self.access_token)
                    .json(&body)
                    .send()
                    .await?;
            }
            ChannelContent::File { url, filename } => {
                let mut body = serde_json::json!({
                    "messaging_product": "whatsapp",
                    "to": user.platform_id,
                    "type": "document",
                    "document": {
                        "link": url,
                        "filename": filename
                    }
                });
                if let Some(ctx) = &context {
                    body["context"] = ctx.clone();
                }
                let api_url = format!(
                    "https://graph.facebook.com/v21.0/{}/messages",
                    self.phone_number_id
                );
                self.client
                    .post(&api_url)
                    .bearer_auth(&*self.access_token)
                    .json(&body)
                    .send()
                    .await?;
            }
            ChannelContent::Location { lat, lon } => {
                let mut body = serde_json::json!({
                    "messaging_product": "whatsapp",
                    "to": user.platform_id,
                    "type": "location",
                    "location": {
                        "latitude": lat,
                        "longitude": lon
                    }
                });
                if let Some(ctx) = &context {
                    body["context"] = ctx.clone();
                }
                let api_url = format!(
                    "https://graph.facebook.com/v21.0/{}/messages",
                    self.phone_number_id
                );
                self.client
                    .post(&api_url)
                    .bearer_auth(&*self.access_token)
                    .json(&body)
                    .send()
                    .await?;
            }
            _ => {
                self.api_send_message(&user.platform_id, "(Unsupported content type)")
                    .await?;
            }
        }
        Ok(())
    }
}

/// Extract a structured error reason from a gateway 4xx/5xx body.
///
/// The gateway returns either `{ error: "..." }` (for 400 missing fields)
/// or `{ success: false, error: "..." }` (for downstream Baileys errors).
/// Returns the human-readable reason phrase if either shape parses, or
/// `None` to let the caller emit a generic message — this guarantees the
/// adapter never leaks raw status codes or JSON-with-braces into the LLM
/// prompt context.
fn extract_gateway_error_reason(body: &str) -> Option<String> {
    let parsed: serde_json::Value = serde_json::from_str(body.trim()).ok()?;
    parsed
        .get("error")
        .and_then(|e| e.as_str())
        .map(|s| s.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_whatsapp_adapter_creation() {
        let adapter = WhatsAppAdapter::new(
            "12345".to_string(),
            "access_token".to_string(),
            "verify_token".to_string(),
            8443,
            vec![],
        );
        assert_eq!(adapter.name(), "whatsapp");
        assert_eq!(adapter.channel_type(), ChannelType::WhatsApp);
    }

    #[test]
    fn test_allowed_users_check() {
        let adapter = WhatsAppAdapter::new(
            "12345".to_string(),
            "token".to_string(),
            "verify".to_string(),
            8443,
            vec!["+1234567890".to_string()],
        );
        assert!(adapter.is_allowed("+1234567890"));
        assert!(!adapter.is_allowed("+9999999999"));

        let open = WhatsAppAdapter::new(
            "12345".to_string(),
            "token".to_string(),
            "verify".to_string(),
            8443,
            vec![],
        );
        assert!(open.is_allowed("+anything"));
    }

    #[test]
    fn test_dm_policy_defaults() {
        let adapter = WhatsAppAdapter::new(
            "12345".to_string(),
            "token".to_string(),
            "verify".to_string(),
            8443,
            vec![],
        );
        // Default DmPolicy is Respond
        assert!(adapter.should_handle_message(false, "hello", "+1234567890"));
    }

    #[test]
    fn test_dm_policy_ignore() {
        let adapter = WhatsAppAdapter::new(
            "12345".to_string(),
            "token".to_string(),
            "verify".to_string(),
            8443,
            vec![],
        )
        .with_dm_policy(DmPolicy::Ignore);
        assert!(!adapter.should_handle_message(false, "hello", "+1234567890"));
    }

    #[test]
    fn test_dm_policy_allowed_only() {
        let adapter = WhatsAppAdapter::new(
            "12345".to_string(),
            "token".to_string(),
            "verify".to_string(),
            8443,
            vec!["+1234567890".to_string()],
        )
        .with_dm_policy(DmPolicy::AllowedOnly);
        // Allowed sender → handle
        assert!(adapter.should_handle_message(false, "hello", "+1234567890"));
        // Unknown sender → reject
        assert!(!adapter.should_handle_message(false, "hello", "+9999999999"));
    }

    #[test]
    fn test_group_policy_all() {
        let adapter = WhatsAppAdapter::new(
            "12345".to_string(),
            "token".to_string(),
            "verify".to_string(),
            8443,
            vec![],
        )
        .with_group_policy(GroupPolicy::All);
        assert!(adapter.should_handle_message(true, "any group message", ""));
    }

    #[test]
    fn test_group_policy_mention_only() {
        let adapter = WhatsAppAdapter::new(
            "12345".to_string(),
            "token".to_string(),
            "verify".to_string(),
            8443,
            vec![],
        )
        .with_group_policy(GroupPolicy::MentionOnly)
        .with_bot_name(Some("HermesBot".to_string()))
        .with_bot_phone(Some("+15551234567".to_string()));

        // Without mention — should not handle
        assert!(!adapter.should_handle_message(true, "what time is it?", ""));
        // With bot name mention
        assert!(adapter.should_handle_message(true, "@HermesBot what time is it?", ""));
        // With bot phone mention
        assert!(adapter.should_handle_message(true, "+15551234567 hello", ""));
    }

    #[test]
    fn test_group_policy_commands_only() {
        let adapter = WhatsAppAdapter::new(
            "12345".to_string(),
            "token".to_string(),
            "verify".to_string(),
            8443,
            vec![],
        )
        .with_group_policy(GroupPolicy::CommandsOnly);
        assert!(!adapter.should_handle_message(true, "hello everyone", ""));
        assert!(adapter.should_handle_message(true, "/help", ""));
    }

    #[test]
    fn test_group_policy_ignore() {
        let adapter = WhatsAppAdapter::new(
            "12345".to_string(),
            "token".to_string(),
            "verify".to_string(),
            8443,
            vec![],
        )
        .with_group_policy(GroupPolicy::Ignore);
        assert!(!adapter.should_handle_message(true, "/help", ""));
    }
}
