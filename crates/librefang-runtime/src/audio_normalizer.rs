//! Channel-aware audio normalizer.
//!
//! Reshape a local audio file into the bytes + MIME that the target
//! channel's adapter expects (Ogg/Opus PTT for WhatsApp, voice-vs-music
//! split for Telegram, pass-through elsewhere), so `channel_send` callers
//! never touch ffmpeg flags or PTT bits directly.

use std::path::{Path, PathBuf};
use std::time::Duration;

use tokio::io::AsyncReadExt;
use tracing::{debug, warn};

/// Hard cap on source file size we will accept before probing. Anything
/// larger than this almost certainly isn't a voice note and would blow
/// the channel's own upload limits anyway.
const MAX_SOURCE_BYTES: u64 = 64 * 1024 * 1024;

/// Wall-clock cap on the `ffmpeg` subprocess. 60 s is enough for a
/// several-minute song to transcode and short enough that a hung child
/// doesn't wedge the agent loop.
const FFMPEG_TIMEOUT: Duration = Duration::from_secs(60);

/// Channels we know about. Anything else falls through to
/// `ChannelProfile::Unknown`, which applies the safest-possible
/// pass-through policy (no transcode, no flag changes).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelProfile {
    /// WhatsApp. Voice notes require Ogg/Opus mono, flagged `ptt: true`.
    WhatsApp,
    /// Telegram. Voice notes use `sendVoice` (Ogg/Opus mono); other
    /// audio uses `sendAudio` (MP3/M4A/etc. with title/performer).
    Telegram,
    /// Signal (via signal-cli / Baileys-like bridge). Accepts Ogg/Opus
    /// voice notes; other audio uploads as attachment.
    Signal,
    /// Slack. No native voice-note concept — upload as file.
    Slack,
    /// Discord. Voice messages require specific headers; audio uploads
    /// as attachment. Currently treated as pass-through.
    Discord,
    /// Everything else. Pass through without normalisation.
    Unknown,
}

impl ChannelProfile {
    /// Parse a channel name (case-insensitive). Unknown values fall
    /// back to `ChannelProfile::Unknown` rather than erroring because
    /// the rest of the platform may well have an adapter registered
    /// under a name this module has no specific profile for, and a
    /// pass-through is almost always better than a hard failure.
    pub fn from_channel(channel: &str) -> Self {
        match channel.trim().to_ascii_lowercase().as_str() {
            "whatsapp" | "wa" | "whatsapp-web" | "whatsapp_web" => Self::WhatsApp,
            "telegram" | "tg" => Self::Telegram,
            "signal" => Self::Signal,
            "slack" => Self::Slack,
            "discord" => Self::Discord,
            _ => Self::Unknown,
        }
    }
}

/// The shape the caller wants. `Auto` lets the normalizer pick based
/// on duration / channel defaults; the other variants pin the
/// intended delivery form even when auto-detection would differ.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DeliveryMode {
    /// Let the profile decide: WhatsApp → always voice note; Telegram
    /// → voice note when the input is short/mono, otherwise music.
    #[default]
    Auto,
    /// Force a PTT voice note. Transcoded to Ogg/Opus mono 16 kHz.
    VoiceNote,
    /// Force delivery as a music/audio file (MP3 or source container).
    MusicFile,
}

/// Everything a channel adapter needs to route the normalised output.
#[derive(Debug, Clone)]
pub struct NormalizedAudio {
    /// Bytes ready to upload. Owned so the caller can drop the
    /// original input and the temp file on disk.
    pub data: Vec<u8>,
    /// MIME type shaped for the downstream API (e.g.
    /// `audio/ogg; codecs=opus` for a WhatsApp PTT).
    pub mime_type: String,
    /// File name to surface on platforms that require one
    /// (Slack, Telegram document fallback).
    pub filename: String,
    /// Duration rounded to seconds. Zero if we couldn't probe.
    pub duration_seconds: u32,
    /// True when the payload should be delivered via a voice-note
    /// endpoint (`ptt:true` on Baileys, `sendVoice` on Telegram, etc.).
    pub is_voice_note: bool,
    /// Profile that was applied — surfaced so the adapter can log it
    /// without re-parsing the channel name.
    pub profile: ChannelProfile,
    /// True when we actually ran `ffmpeg`. False means the source
    /// already matched the profile and we just read the bytes back.
    pub transcoded: bool,
    /// 64-byte RMS envelope, normalised 0–100, for WhatsApp voice
    /// notes. `None` when the profile doesn't use a waveform or when
    /// the extraction step failed — adapters treat `None` as "send
    /// without waveform" rather than as an error so a flat-bar
    /// voice note is still delivered.
    pub waveform: Option<Vec<u8>>,
    /// Temporary on-disk artefact, cleaned up when this guard drops.
    /// Present only when transcode ran.
    _temp: Option<TempFile>,
}

/// File extensions we accept as audio without invoking ffprobe.
/// Shared by `tool_channel_send`'s routing guard and
/// [`looks_like_audio`]'s fast path so drift can't leave one in sync
/// while the other falls behind.
pub const AUDIO_EXTENSIONS: &[&str] = &["mp3", "ogg", "oga", "opus", "wav", "m4a", "aac", "flac"];

/// Return true when a MIME string describes an Opus / Ogg voice note,
/// i.e. the form that maps to WhatsApp's `ptt:true` bubble and
/// Telegram's `sendVoice`. Both adapters need the same heuristic —
/// centralised here so it can't drift between them.
pub fn mime_is_voice_note(mime: &str) -> bool {
    mime.contains("opus") || mime.contains("ogg")
}

/// Voice-note waveform: 64 bytes, one per window of RMS amplitude
/// scaled to 0..=100 with a small floor so no bar renders as a
/// completely invisible pixel.
const WAVEFORM_SAMPLES: usize = 64;
const WAVEFORM_FLOOR: u8 = 3;
const WAVEFORM_TIMEOUT: Duration = Duration::from_secs(3);

/// Compute the 64-byte WhatsApp voice-note waveform for an Ogg/Opus
/// payload. Pipes the buffer through ffmpeg to PCM mono s16le 8 kHz,
/// chunks the samples into 64 equal-size windows, takes the RMS of
/// each window, normalises the peak window to 100, clamps below
/// `WAVEFORM_FLOOR`. Returns `None` on any failure (missing ffmpeg,
/// corrupt input, timeout, empty output) so the caller can fall back
/// to sending without a waveform — a flat-bar voice note is still
/// better than a failed send.
pub async fn compute_voice_waveform(ogg_bytes: &[u8]) -> Option<Vec<u8>> {
    if ogg_bytes.is_empty() {
        return None;
    }

    use std::process::Stdio;
    use tokio::io::AsyncWriteExt;

    let mut child = match tokio::process::Command::new("ffmpeg")
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-f",
            "ogg",
            "-i",
            "pipe:0",
            "-vn",
            "-ac",
            "1",
            "-ar",
            "8000",
            "-f",
            "s16le",
            "pipe:1",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            warn!("audio_normalizer: waveform ffmpeg spawn failed: {e}");
            return None;
        }
    };

    // Feed stdin concurrently so ffmpeg's stdout pipe doesn't fill up
    // before we've written the whole input.
    if let Some(mut stdin) = child.stdin.take() {
        let bytes = ogg_bytes.to_vec();
        tokio::spawn(async move {
            let _ = stdin.write_all(&bytes).await;
            let _ = stdin.shutdown().await;
        });
    }

    let mut stdout = child.stdout.take().expect("stdout piped");
    let stdout_task = tokio::spawn(async move {
        let mut buf = Vec::new();
        let _ = stdout.read_to_end(&mut buf).await;
        buf
    });

    let status = match tokio::time::timeout(WAVEFORM_TIMEOUT, child.wait()).await {
        Ok(Ok(s)) => s,
        Ok(Err(e)) => {
            warn!("audio_normalizer: waveform ffmpeg wait failed: {e}");
            return None;
        }
        Err(_) => {
            let _ = child.start_kill();
            let _ = child.wait().await;
            warn!("audio_normalizer: waveform ffmpeg timed out");
            return None;
        }
    };

    if !status.success() {
        warn!("audio_normalizer: waveform ffmpeg exit {}", status);
        return None;
    }

    let pcm = stdout_task.await.unwrap_or_default();
    if pcm.len() < 2 {
        return None;
    }

    let sample_count = pcm.len() / 2;
    let window = std::cmp::max(1, sample_count / WAVEFORM_SAMPLES);
    let mut rms = [0f64; WAVEFORM_SAMPLES];
    let mut peak = 0f64;

    for (i, slot) in rms.iter_mut().enumerate() {
        let start = i * window;
        let end = if i == WAVEFORM_SAMPLES - 1 {
            sample_count
        } else {
            std::cmp::min(sample_count, start + window)
        };
        if start >= end {
            continue;
        }
        let mut sum_sq = 0f64;
        let mut count = 0usize;
        for j in start..end {
            let lo = pcm[j * 2] as i16;
            let hi = pcm[j * 2 + 1] as i16;
            let s = ((hi << 8) | (lo & 0xff)) as f64;
            sum_sq += s * s;
            count += 1;
        }
        let r = if count > 0 {
            (sum_sq / count as f64).sqrt()
        } else {
            0.0
        };
        *slot = r;
        if r > peak {
            peak = r;
        }
    }

    let mut out = vec![WAVEFORM_FLOOR; WAVEFORM_SAMPLES];
    if peak > 0.0 {
        for (i, &r) in rms.iter().enumerate() {
            let normalised = ((r / peak) * 100.0).round() as i32;
            out[i] = normalised.max(WAVEFORM_FLOOR as i32).min(100) as u8;
        }
    }
    Some(out)
}

/// RAII guard for the transcode output file.
#[derive(Debug, Clone)]
pub(crate) struct TempFile {
    path: PathBuf,
}

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Options passed from the caller (tool / skill / endpoint handler).
/// All fields are optional and have sensible defaults.
#[derive(Debug, Clone, Default)]
pub struct NormalizeOptions {
    /// Override the auto-picked delivery shape.
    pub delivery: DeliveryMode,
    /// Optional caption / filename hint passed down from the agent
    /// call site — used as a fallback when we can't derive one.
    pub caption: Option<String>,
    /// Explicit source filename hint (takes precedence over the one
    /// derived from the path).
    pub filename: Option<String>,
}

/// Normalise `src_path` for delivery on `channel`.
///
/// Returns a [`NormalizedAudio`] carrying bytes + metadata. The caller
/// is expected to hand the result to its adapter and let the
/// `_temp` guard drop to clean up.
pub async fn normalize_audio_for_channel(
    channel: &str,
    src_path: &Path,
    opts: &NormalizeOptions,
) -> Result<NormalizedAudio, String> {
    let profile = ChannelProfile::from_channel(channel);

    // Size guard before we touch ffprobe — cheap, and a 2 GB video
    // mis-classified as audio would otherwise eat our ffmpeg slot.
    let src_meta = tokio::fs::metadata(src_path)
        .await
        .map_err(|e| format!("audio normalize: cannot stat '{}': {e}", src_path.display()))?;
    if src_meta.len() > MAX_SOURCE_BYTES {
        return Err(format!(
            "audio normalize: source '{}' is {} bytes, exceeds cap of {} bytes",
            src_path.display(),
            src_meta.len(),
            MAX_SOURCE_BYTES
        ));
    }

    let probe = probe_audio(src_path).await?;
    let filename = opts
        .filename
        .clone()
        .or_else(|| {
            src_path
                .file_name()
                .and_then(|n| n.to_str())
                .map(|s| s.to_string())
        })
        .unwrap_or_else(|| "audio".to_string());

    let is_voice_note = match opts.delivery {
        DeliveryMode::VoiceNote => true,
        DeliveryMode::MusicFile => false,
        DeliveryMode::Auto => {
            matches!(profile, ChannelProfile::WhatsApp)
                || (matches!(profile, ChannelProfile::Telegram | ChannelProfile::Signal)
                    && probe.looks_like_voice_note())
        }
    };

    match (profile, is_voice_note) {
        (ChannelProfile::WhatsApp, _) => {
            normalize_whatsapp_voice_note(src_path, &probe, &filename).await
        }
        (ChannelProfile::Telegram, true) | (ChannelProfile::Signal, true) => {
            normalize_opus_voice_note(src_path, &probe, &filename, profile).await
        }
        (ChannelProfile::Telegram, false) => {
            // Music file: keep source as-is when it's already a sane
            // container; otherwise transcode to MP3 for widest device
            // compatibility.
            if probe.is_music_compatible() {
                pass_through(src_path, &probe, &filename, profile, false).await
            } else {
                transcode_to_mp3(src_path, &probe, &filename, profile).await
            }
        }
        (ChannelProfile::Signal, false)
        | (ChannelProfile::Slack, _)
        | (ChannelProfile::Discord, _)
        | (ChannelProfile::Unknown, _) => {
            pass_through(src_path, &probe, &filename, profile, is_voice_note).await
        }
    }
}

async fn normalize_whatsapp_voice_note(
    src_path: &Path,
    probe: &AudioProbe,
    filename_hint: &str,
) -> Result<NormalizedAudio, String> {
    // WhatsApp PTT is strict: Ogg/Opus mono 16 kHz. If the source
    // already matches, we still re-mux through ffmpeg to strip any
    // stray metadata / channels that Baileys occasionally rejects.
    let needs_transcode = !probe.is_opus_mono_voice_compatible();

    debug!(
        src = %src_path.display(),
        src_codec = %probe.codec_name,
        src_container = %probe.container_name,
        src_channels = probe.channels,
        src_sample_rate = probe.sample_rate,
        needs_transcode,
        "audio_normalizer: WhatsApp PTT profile"
    );

    let (data, tmp) = if needs_transcode {
        let (bytes, guard) = run_ffmpeg_opus_mono(src_path, 16_000, 32_000).await?;
        (bytes, Some(guard))
    } else {
        let bytes = tokio::fs::read(src_path)
            .await
            .map_err(|e| format!("audio normalize: read source: {e}"))?;
        (bytes, None)
    };

    // WhatsApp Web renders a flat bar when `AudioMessage.waveform` is
    // absent. Compute the 64-byte RMS envelope from the Ogg we're
    // about to send; on any failure, log and deliver without a
    // waveform (better than a failed send).
    let waveform = compute_voice_waveform(&data).await;

    Ok(NormalizedAudio {
        data,
        mime_type: "audio/ogg; codecs=opus".to_string(),
        filename: force_extension(filename_hint, "ogg"),
        duration_seconds: probe.duration_seconds,
        is_voice_note: true,
        profile: ChannelProfile::WhatsApp,
        transcoded: needs_transcode,
        waveform,
        _temp: tmp,
    })
}

async fn normalize_opus_voice_note(
    src_path: &Path,
    probe: &AudioProbe,
    filename_hint: &str,
    profile: ChannelProfile,
) -> Result<NormalizedAudio, String> {
    let needs_transcode = !probe.is_opus_mono_voice_compatible();
    let (data, tmp) = if needs_transcode {
        let (bytes, guard) = run_ffmpeg_opus_mono(src_path, 48_000, 64_000).await?;
        (bytes, Some(guard))
    } else {
        let bytes = tokio::fs::read(src_path)
            .await
            .map_err(|e| format!("audio normalize: read source: {e}"))?;
        (bytes, None)
    };
    Ok(NormalizedAudio {
        data,
        mime_type: "audio/ogg; codecs=opus".to_string(),
        filename: force_extension(filename_hint, "ogg"),
        duration_seconds: probe.duration_seconds,
        is_voice_note: true,
        profile,
        transcoded: needs_transcode,
        waveform: None,
        _temp: tmp,
    })
}

async fn transcode_to_mp3(
    src_path: &Path,
    probe: &AudioProbe,
    filename_hint: &str,
    profile: ChannelProfile,
) -> Result<NormalizedAudio, String> {
    let (bytes, guard) = run_ffmpeg_mp3(src_path).await?;
    Ok(NormalizedAudio {
        data: bytes,
        mime_type: "audio/mpeg".to_string(),
        filename: force_extension(filename_hint, "mp3"),
        duration_seconds: probe.duration_seconds,
        is_voice_note: false,
        profile,
        transcoded: true,
        waveform: None,
        _temp: Some(guard),
    })
}

async fn pass_through(
    src_path: &Path,
    probe: &AudioProbe,
    filename_hint: &str,
    profile: ChannelProfile,
    is_voice_note: bool,
) -> Result<NormalizedAudio, String> {
    let bytes = tokio::fs::read(src_path)
        .await
        .map_err(|e| format!("audio normalize: read source: {e}"))?;
    let mime = probe.guessed_mime().to_string();
    Ok(NormalizedAudio {
        data: bytes,
        mime_type: mime,
        filename: filename_hint.to_string(),
        duration_seconds: probe.duration_seconds,
        is_voice_note,
        profile,
        transcoded: false,
        waveform: None,
        _temp: None,
    })
}

/// Minimal audio probe populated via `ffprobe`. When `ffprobe` is not
/// available we fall back to ext-based sniffing — the resulting
/// fields are best-effort and the `transcoded` flag on the returned
/// [`NormalizedAudio`] still reflects what actually happened.
#[derive(Debug, Clone)]
pub struct AudioProbe {
    pub container_name: String,
    pub codec_name: String,
    pub sample_rate: u32,
    pub channels: u32,
    pub duration_seconds: u32,
}

impl AudioProbe {
    fn is_opus_mono_voice_compatible(&self) -> bool {
        // Accept any Opus in an Ogg container where we actually have
        // one audio channel. Sample rate is left alone — both 16k and
        // 48k pass WhatsApp's own encoder path so re-muxing just to
        // change the rate wastes CPU.
        self.codec_name.eq_ignore_ascii_case("opus")
            && self.container_name.eq_ignore_ascii_case("ogg")
            && self.channels == 1
    }

    fn is_music_compatible(&self) -> bool {
        matches!(
            self.codec_name.to_ascii_lowercase().as_str(),
            "mp3" | "aac" | "m4a" | "alac" | "flac" | "wav"
        )
    }

    fn looks_like_voice_note(&self) -> bool {
        // Heuristic: mono + short (<= 60 s) + sample rate no higher
        // than 48 kHz. A stereo 3-min track is never a voice note.
        self.channels == 1 && self.duration_seconds > 0 && self.duration_seconds <= 60
    }

    fn guessed_mime(&self) -> &'static str {
        match self.codec_name.to_ascii_lowercase().as_str() {
            "opus" => "audio/ogg; codecs=opus",
            "vorbis" => "audio/ogg",
            "mp3" => "audio/mpeg",
            "aac" | "m4a" | "alac" => "audio/mp4",
            "flac" => "audio/flac",
            "wav" | "pcm_s16le" | "pcm_s24le" => "audio/wav",
            _ => "application/octet-stream",
        }
    }
}

async fn probe_audio(src_path: &Path) -> Result<AudioProbe, String> {
    match probe_with_ffprobe(src_path).await {
        Ok(p) => Ok(p),
        Err(why) => {
            warn!(
                src = %src_path.display(),
                reason = %why,
                "audio_normalizer: ffprobe unavailable, falling back to ext sniff"
            );
            Ok(probe_by_extension(src_path))
        }
    }
}

async fn probe_with_ffprobe(src_path: &Path) -> Result<AudioProbe, String> {
    use std::process::Stdio;

    let output = tokio::process::Command::new("ffprobe")
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-of",
            "json",
            "-show_streams",
            "-show_format",
            "-select_streams",
            "a:0",
        ])
        .arg(src_path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .map_err(|e| format!("spawn: {e}"))?;

    if !output.status.success() {
        return Err(format!(
            "ffprobe exit {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }

    let v: serde_json::Value =
        serde_json::from_slice(&output.stdout).map_err(|e| format!("ffprobe json parse: {e}"))?;

    let streams = v.get("streams").and_then(|s| s.as_array());
    let has_audio_stream = streams
        .map(|arr| {
            arr.iter()
                .any(|s| s.get("codec_type").and_then(|c| c.as_str()) == Some("audio"))
        })
        .unwrap_or(false);
    if !has_audio_stream {
        return Err("source has no audio stream".to_string());
    }

    let stream = streams
        .and_then(|arr| arr.first())
        .ok_or_else(|| "ffprobe returned no streams".to_string())?;

    let codec_name = stream
        .get("codec_name")
        .and_then(|s| s.as_str())
        .unwrap_or("")
        .to_string();
    let sample_rate = stream
        .get("sample_rate")
        .and_then(|s| s.as_str())
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or(0);
    let channels = stream.get("channels").and_then(|c| c.as_u64()).unwrap_or(0) as u32;

    let container_name = v
        .get("format")
        .and_then(|f| f.get("format_name"))
        .and_then(|s| s.as_str())
        .map(|s| {
            // ffprobe returns comma-separated candidate names like
            // "ogg" or "mov,mp4,m4a,3gp,3g2,mj2" — take the first.
            s.split(',').next().unwrap_or(s).to_string()
        })
        .unwrap_or_default();

    let duration_seconds = v
        .get("format")
        .and_then(|f| f.get("duration"))
        .and_then(|s| s.as_str())
        .and_then(|s| s.parse::<f64>().ok())
        .map(|f| f.round() as u32)
        .unwrap_or(0);

    Ok(AudioProbe {
        container_name,
        codec_name,
        sample_rate,
        channels,
        duration_seconds,
    })
}

fn probe_by_extension(src_path: &Path) -> AudioProbe {
    let ext = src_path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let (container, codec, channels, sr) = match ext.as_str() {
        "ogg" | "oga" | "opus" => ("ogg", "opus", 1, 48_000),
        "mp3" => ("mp3", "mp3", 2, 44_100),
        "m4a" | "aac" => ("mp4", "aac", 2, 44_100),
        "wav" => ("wav", "pcm_s16le", 1, 16_000),
        "flac" => ("flac", "flac", 2, 44_100),
        _ => ("", "", 0, 0),
    };
    AudioProbe {
        container_name: container.to_string(),
        codec_name: codec.to_string(),
        sample_rate: sr,
        channels,
        duration_seconds: 0,
    }
}

/// Run ffmpeg to re-encode `src_path` into a scratch file under
/// `$TMPDIR/librefang_audio_norm/` and return the output bytes plus
/// an RAII guard for the scratch path.
///
/// The guard is constructed immediately after spawn success so that
/// every subsequent `?` propagates cleanup via `Drop` even if the
/// read-back fails or the tokio task is cancelled.
async fn run_ffmpeg_to_tempfile(
    src_path: &Path,
    out_ext: &str,
    codec_args: &[&str],
) -> Result<(Vec<u8>, TempFile), String> {
    use std::process::Stdio;

    let tmp_dir = std::env::temp_dir().join("librefang_audio_norm");
    tokio::fs::create_dir_all(&tmp_dir)
        .await
        .map_err(|e| format!("create tmp dir: {e}"))?;
    let dst_path = tmp_dir.join(format!("{}.{out_ext}", uuid::Uuid::new_v4()));

    let mut child = tokio::process::Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-y", "-i"])
        .arg(src_path)
        .args(codec_args)
        .arg(&dst_path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("spawn ffmpeg: {e}"))?;

    // Guard the output path from the moment ffmpeg might have started
    // writing it; every `?` below will drop this and unlink the scratch
    // file, even on disk-read failure or timeout.
    let guard = TempFile {
        path: dst_path.clone(),
    };

    let mut stderr = child.stderr.take().expect("stderr piped");

    let status = match tokio::time::timeout(FFMPEG_TIMEOUT, child.wait()).await {
        Ok(Ok(s)) => s,
        Ok(Err(e)) => return Err(format!("ffmpeg wait failed: {e}")),
        Err(_) => {
            let _ = child.start_kill();
            let _ = child.wait().await;
            return Err("ffmpeg timed out while transcoding".to_string());
        }
    };

    let mut err_buf = Vec::new();
    let _ = stderr.read_to_end(&mut err_buf).await;

    if !status.success() {
        return Err(format!(
            "ffmpeg exited with {}: {}",
            status,
            String::from_utf8_lossy(&err_buf).trim()
        ));
    }

    let bytes = tokio::fs::read(&dst_path)
        .await
        .map_err(|e| format!("read ffmpeg output: {e}"))?;
    if bytes.is_empty() {
        return Err("ffmpeg produced an empty output".to_string());
    }
    Ok((bytes, guard))
}

async fn run_ffmpeg_opus_mono(
    src_path: &Path,
    sample_rate: u32,
    bitrate_bps: u32,
) -> Result<(Vec<u8>, TempFile), String> {
    let ar = sample_rate.to_string();
    let bitrate = format!("{}k", bitrate_bps / 1000);
    run_ffmpeg_to_tempfile(
        src_path,
        "ogg",
        &[
            "-vn", "-ac", "1", "-ar", &ar, "-c:a", "libopus", "-b:a", &bitrate, "-f", "ogg",
        ],
    )
    .await
}

async fn run_ffmpeg_mp3(src_path: &Path) -> Result<(Vec<u8>, TempFile), String> {
    run_ffmpeg_to_tempfile(
        src_path,
        "mp3",
        &["-vn", "-c:a", "libmp3lame", "-b:a", "128k", "-f", "mp3"],
    )
    .await
}

fn force_extension(name: &str, ext: &str) -> String {
    let stem = Path::new(name)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("audio");
    format!("{stem}.{ext}")
}

/// Best-effort audio-shape check used by `tool_channel_send` before
/// handing off to the normalizer. The extension allowlist runs first
/// so the hot path never spawns `ffprobe` for common audio inputs;
/// ffprobe only fires for extensionless or unrecognised names, where
/// paying a subprocess round-trip is genuinely the least-bad option.
pub async fn looks_like_audio(src_path: &Path) -> bool {
    let ext = src_path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if !ext.is_empty() {
        return AUDIO_EXTENSIONS.contains(&ext.as_str());
    }
    probe_with_ffprobe(src_path).await.is_ok()
}

// Simple in-process waveform writer, meant for tests. Produces a valid
// PCM WAV with the requested duration / channel count so tests can
// exercise the real ffmpeg path without bundling binary fixtures.
#[cfg(test)]
async fn write_sine_wav(path: &Path, seconds: u32, channels: u16, sample_rate: u32) {
    use std::f32::consts::TAU;
    use tokio::io::AsyncWriteExt;
    let total = (sample_rate as usize) * (seconds as usize);
    let bits_per_sample: u16 = 16;
    let byte_rate = sample_rate * u32::from(channels) * u32::from(bits_per_sample) / 8;
    let block_align = channels * bits_per_sample / 8;
    let data_bytes = (total * usize::from(channels) * usize::from(bits_per_sample) / 8) as u32;
    let riff_size = 36 + data_bytes;

    let mut buf = Vec::with_capacity(44 + data_bytes as usize);
    buf.extend_from_slice(b"RIFF");
    buf.extend_from_slice(&riff_size.to_le_bytes());
    buf.extend_from_slice(b"WAVE");
    buf.extend_from_slice(b"fmt ");
    buf.extend_from_slice(&16u32.to_le_bytes());
    buf.extend_from_slice(&1u16.to_le_bytes()); // PCM
    buf.extend_from_slice(&channels.to_le_bytes());
    buf.extend_from_slice(&sample_rate.to_le_bytes());
    buf.extend_from_slice(&byte_rate.to_le_bytes());
    buf.extend_from_slice(&block_align.to_le_bytes());
    buf.extend_from_slice(&bits_per_sample.to_le_bytes());
    buf.extend_from_slice(b"data");
    buf.extend_from_slice(&data_bytes.to_le_bytes());

    let mut t = 0f32;
    let step = TAU * 440.0 / sample_rate as f32;
    for _ in 0..total {
        let sample = (t.sin() * 0.25 * i16::MAX as f32) as i16;
        for _ in 0..channels {
            buf.extend_from_slice(&sample.to_le_bytes());
        }
        t += step;
    }

    let mut f = tokio::fs::File::create(path).await.unwrap();
    f.write_all(&buf).await.unwrap();
    f.sync_all().await.unwrap();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_from_channel_handles_common_aliases() {
        assert_eq!(
            ChannelProfile::from_channel("whatsapp"),
            ChannelProfile::WhatsApp
        );
        assert_eq!(
            ChannelProfile::from_channel("WhatsApp"),
            ChannelProfile::WhatsApp
        );
        assert_eq!(ChannelProfile::from_channel("wa"), ChannelProfile::WhatsApp);
        assert_eq!(
            ChannelProfile::from_channel("telegram"),
            ChannelProfile::Telegram
        );
        assert_eq!(
            ChannelProfile::from_channel("TG "),
            ChannelProfile::Telegram
        );
        assert_eq!(
            ChannelProfile::from_channel("signal"),
            ChannelProfile::Signal
        );
        assert_eq!(ChannelProfile::from_channel("slack"), ChannelProfile::Slack);
        assert_eq!(
            ChannelProfile::from_channel("discord"),
            ChannelProfile::Discord
        );
        assert_eq!(
            ChannelProfile::from_channel("mastodon"),
            ChannelProfile::Unknown
        );
        assert_eq!(ChannelProfile::from_channel(""), ChannelProfile::Unknown);
    }

    #[test]
    fn probe_opus_mono_is_ptt_compatible() {
        let p = AudioProbe {
            container_name: "ogg".into(),
            codec_name: "opus".into(),
            sample_rate: 48_000,
            channels: 1,
            duration_seconds: 4,
        };
        assert!(p.is_opus_mono_voice_compatible());
        assert!(p.looks_like_voice_note());
    }

    #[test]
    fn probe_stereo_opus_is_not_ptt_compatible() {
        let p = AudioProbe {
            container_name: "ogg".into(),
            codec_name: "opus".into(),
            sample_rate: 48_000,
            channels: 2,
            duration_seconds: 4,
        };
        assert!(!p.is_opus_mono_voice_compatible());
        assert!(!p.looks_like_voice_note());
    }

    #[test]
    fn probe_mp3_is_music_compatible_not_ptt() {
        let p = AudioProbe {
            container_name: "mp3".into(),
            codec_name: "mp3".into(),
            sample_rate: 44_100,
            channels: 2,
            duration_seconds: 180,
        };
        assert!(!p.is_opus_mono_voice_compatible());
        assert!(!p.looks_like_voice_note());
        assert!(p.is_music_compatible());
    }

    #[test]
    fn probe_by_extension_maps_known_formats() {
        let p = probe_by_extension(Path::new("note.ogg"));
        assert_eq!(p.codec_name, "opus");
        assert_eq!(p.channels, 1);

        let p = probe_by_extension(Path::new("song.MP3"));
        assert_eq!(p.codec_name, "mp3");
        assert_eq!(p.channels, 2);

        let p = probe_by_extension(Path::new("blob.bin"));
        assert_eq!(p.codec_name, "");
        assert_eq!(p.channels, 0);
    }

    #[test]
    fn force_extension_preserves_stem() {
        assert_eq!(force_extension("note.wav", "ogg"), "note.ogg");
        assert_eq!(force_extension("no_ext", "mp3"), "no_ext.mp3");
        assert_eq!(force_extension("path.tar.gz", "ogg"), "path.tar.ogg");
    }

    #[test]
    fn guessed_mime_covers_common_codecs() {
        let opus = AudioProbe {
            container_name: "ogg".into(),
            codec_name: "opus".into(),
            sample_rate: 48_000,
            channels: 1,
            duration_seconds: 0,
        };
        assert_eq!(opus.guessed_mime(), "audio/ogg; codecs=opus");

        let mp3 = AudioProbe {
            container_name: "mp3".into(),
            codec_name: "mp3".into(),
            sample_rate: 44_100,
            channels: 2,
            duration_seconds: 0,
        };
        assert_eq!(mp3.guessed_mime(), "audio/mpeg");

        let unknown = AudioProbe {
            container_name: "bin".into(),
            codec_name: "xyz".into(),
            sample_rate: 0,
            channels: 0,
            duration_seconds: 0,
        };
        assert_eq!(unknown.guessed_mime(), "application/octet-stream");
    }

    #[tokio::test]
    async fn normalize_whatsapp_transcodes_wav_to_ogg_opus_mono() {
        if tokio::process::Command::new("ffmpeg")
            .arg("-version")
            .output()
            .await
            .is_err()
        {
            eprintln!("skipping: ffmpeg not installed");
            return;
        }

        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("tone.wav");
        write_sine_wav(&src, 2, 1, 16_000).await;

        let out = normalize_audio_for_channel("whatsapp", &src, &NormalizeOptions::default())
            .await
            .expect("normalize");

        assert!(out.is_voice_note);
        assert_eq!(out.profile, ChannelProfile::WhatsApp);
        assert_eq!(out.mime_type, "audio/ogg; codecs=opus");
        assert!(
            out.filename.ends_with(".ogg"),
            "filename = {}",
            out.filename
        );
        assert!(out.transcoded);
        // First 4 bytes of an Ogg stream are always "OggS".
        assert_eq!(&out.data[..4], b"OggS", "output is not an Ogg container");

        // Waveform must be attached for WhatsApp voice notes: 64 bytes,
        // every value in 0..=100, peak == 100, floor respected.
        let waveform = out.waveform.as_ref().expect("waveform present for WA");
        assert_eq!(waveform.len(), 64, "waveform is always 64 bytes");
        for (i, &v) in waveform.iter().enumerate() {
            assert!(v <= 100, "waveform[{i}] = {v} > 100");
            assert!(
                v >= WAVEFORM_FLOOR,
                "waveform[{i}] = {v} < floor {WAVEFORM_FLOOR}"
            );
        }
        let peak = waveform.iter().copied().max().unwrap_or(0);
        assert_eq!(
            peak, 100,
            "peak window must be normalised to 100, got {peak}"
        );
    }

    #[tokio::test]
    async fn compute_voice_waveform_returns_none_on_empty_input() {
        let out = compute_voice_waveform(&[]).await;
        assert!(out.is_none(), "empty input must produce no waveform");
    }

    #[tokio::test]
    async fn normalize_rejects_oversized_source() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("big.bin");
        // Sparse-allocate a 128 MB file so we hit the size cap without
        // burning disk on a real write.
        let f = tokio::fs::File::create(&src).await.unwrap();
        f.set_len(MAX_SOURCE_BYTES + 1).await.unwrap();
        drop(f);

        let err = normalize_audio_for_channel("whatsapp", &src, &NormalizeOptions::default())
            .await
            .unwrap_err();
        assert!(err.contains("exceeds cap"), "err = {err}");
    }

    #[tokio::test]
    async fn pass_through_unknown_channel_keeps_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("blob.wav");
        write_sine_wav(&src, 1, 1, 8_000).await;
        let original = tokio::fs::read(&src).await.unwrap();

        let out = normalize_audio_for_channel("mastodon", &src, &NormalizeOptions::default())
            .await
            .expect("normalize");
        assert_eq!(out.profile, ChannelProfile::Unknown);
        assert!(!out.transcoded);
        assert_eq!(out.data, original);
    }
}
