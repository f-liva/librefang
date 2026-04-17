# `audio_normalizer` — channel-aware outbound audio

`channel_send` used to push whatever raw audio file the agent handed it
at the target channel. WhatsApp Web rejected anything that wasn't an
Ogg/Opus PTT voice note ("Unsupported content type in Web mode"),
Telegram split voice vs music and refused to render a music card for
ogg files, and the agent ended up hand-rolling ffmpeg + flag hacks in
every skill. This module owns that responsibility inside the transport
layer so the caller can say "send this audio to this recipient" and
nothing else.

## Public surface

```rust
use librefang_runtime::audio_normalizer::{
    ChannelProfile, DeliveryMode, NormalizeOptions, normalize_audio_for_channel,
};

let normalized = normalize_audio_for_channel(
    "whatsapp",
    &std::path::PathBuf::from("/tmp/ambrogio/greeting.mp3"),
    &NormalizeOptions::default(),
).await?;

// normalized.data          — bytes ready to upload
// normalized.mime_type     — "audio/ogg; codecs=opus"
// normalized.filename      — "greeting.ogg"
// normalized.is_voice_note — true
// normalized.profile       — ChannelProfile::WhatsApp
// normalized.transcoded    — true (MP3 -> Ogg/Opus mono)
```

The returned value owns an RAII `TempFile` guard. Drop it when the
payload has been uploaded and the scratch file cleans itself up.

## Profiles

| Channel   | Delivery shape              | Container / codec         | Flags                        | Endpoint used downstream |
|-----------|-----------------------------|---------------------------|------------------------------|--------------------------|
| WhatsApp  | always voice note (PTT)     | Ogg / Opus, mono, 16 kHz  | `ptt: true`                  | Baileys gateway `/message/send-audio` |
| Telegram  | voice note OR music         | Ogg/Opus (voice) or MP3   | `as_voice_note` override     | Bot API `sendVoice` / `sendAudio` |
| Signal    | voice note OR file          | Ogg/Opus (voice) else pass-through | caller hint   | Signal bridge upload     |
| Slack     | attachment                  | pass-through              | none                         | Slack `files.upload`     |
| Discord   | attachment                  | pass-through              | none                         | Discord REST upload      |
| *unknown* | pass-through                | source as-is              | none                         | generic file upload      |

`DeliveryMode::Auto` lets the module pick. `VoiceNote` and `MusicFile`
force one or the other even when auto-detection would disagree.

### WhatsApp voice note (the strict case)

WhatsApp Web only renders a voice bubble if the payload is Ogg / Opus
**mono** at one of the opus-supported sample rates (typically 16 or
48 kHz) with `ptt: true`. The normalizer re-muxes any non-compliant
source — `.mp3`, `.m4a`, `.wav`, even stereo `.ogg` — into a single-
channel Opus stream. Output bitrate defaults to 32 kbps, which matches
WhatsApp's own encoder and keeps the payload tiny.

### Telegram voice vs music

`sendVoice` requires Ogg/Opus and shows a voice bubble. `sendAudio`
accepts MP3 / M4A / FLAC and shows a music card with title and
performer. The heuristic is: mono + `duration <= 60 s` → voice note;
otherwise music. Callers can pin either form via
`DeliveryMode::VoiceNote` or `DeliveryMode::MusicFile`.

## Validation

The module rejects sources larger than 64 MB (before any transcode)
and any source that ffprobe cannot identify as an audio stream. The
second check degrades gracefully: if `ffprobe` isn't on `PATH`, the
normalizer falls back to an extension-only sniff and leaves the rest
to ffmpeg — so the binary is required for reliable validation but
not for the happy path on well-named files.

## Cache

Transcoded bytes live under `$TMPDIR/librefang_audio_norm/<uuid>.<ext>`
while the caller holds the returned `NormalizedAudio`. The `TempFile`
guard removes the scratch file on drop. No manual cleanup is required
from the caller.

## Testing

Unit tests cover profile selection, MIME mapping, voice-note heuristics
and extension guessing. An end-to-end test exercises the real `ffmpeg`
binary by feeding a generated PCM WAV through the WhatsApp profile and
asserting the returned bytes start with an `"OggS"` signature. The WAV
generator is inlined so the test doesn't rely on bundled fixtures.

Run just this module's tests with:

```sh
cargo test -p librefang-runtime --lib audio_normalizer
```

## Integration points (for reviewers)

- `tool_channel_send` (`crates/librefang-runtime/src/tool_runner.rs`) —
  detects audio by extension or ffprobe, invokes `normalize_audio_for_channel`,
  sends the normalised bytes via `send_channel_file_data` with the
  channel-specific MIME type.
- `WhatsAppAdapter::send` (`crates/librefang-channels/src/whatsapp.rs`) —
  in Web-gateway mode, spools `audio/*` `FileData` to
  `/tmp/librefang_audio_out/<uuid>.<ext>` and calls
  `/message/send-audio` with `file://...` so the Baileys gateway picks
  the bytes up from the shared volume.
- `TelegramAdapter::send` (`crates/librefang-channels/src/telegram.rs`)
  — routes `audio/*` `FileData` to `sendVoice` (Opus / Ogg) or
  `sendAudio` (anything else), uploading via multipart.
- `whatsapp-gateway/index.js` — `sendAudio()` now accepts both
  `http(s)://` URLs and local paths / `file://...` under
  `/tmp/librefang_*`.

## Out of scope

Video, image, and sticker payloads. See `media_understanding` for
inbound STT and the existing image-file pipeline for images. Adding
Signal / Slack / Discord voice-specific normalisation is a follow-up
once those adapters grow a bytes-aware voice endpoint; today they
pass through unchanged.
