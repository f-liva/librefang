# Phase 1: ImageFile Type and Channel Bridge - Context

**Gathered:** 2026-04-04
**Status:** Ready for planning
**Source:** Auto mode (from idea document)

## Phase Boundary

Add `ContentBlock::ImageFile` variant to the types crate and convert the channel bridge's `download_image_to_blocks()` to save images as files instead of base64 encoding. This phase does NOT modify any LLM drivers — that's Phase 2.

## Implementation Decisions

### ContentBlock::ImageFile variant
- Add `ImageFile { media_type: String, path: String }` to the `ContentBlock` enum in `crates/librefang-types/src/message.rs`
- The existing `Image { media_type: String, data: String }` variant stays unchanged for retrocompatibility
- Serde: use the default externally-tagged enum serialization (already used by ContentBlock) — no custom deserializer needed
- Old sessions with `Image` blocks will deserialize correctly since we're adding a new variant, not changing existing ones

### File storage location and naming
- Save to `/tmp/librefang_uploads/` (directory already exists and is used for API uploads)
- Filename format: `{uuid}.{ext}` where ext is derived from media_type (image/jpeg → jpg, image/png → png, image/webp → webp, default → jpg)
- Use absolute paths in the `path` field
- Create directory if it doesn't exist (tokio::fs::create_dir_all)

### Image downscaling
- Keep the existing `image` crate downscaling logic (max 1024px, Triangle filter, re-encode as JPEG)
- Apply downscaling BEFORE saving to disk (so the file on disk is already small)
- Threshold: only downscale if original > 200KB (already implemented)

### download_image_to_blocks changes
- After downloading and optional downscaling, write bytes to disk instead of base64-encoding
- Return `ContentBlock::ImageFile { media_type, path }` instead of `ContentBlock::Image { media_type, data }`
- On write failure: fall back to the existing base64 approach (graceful degradation)

### Claude's Discretion
- Error handling strategy for disk write failures
- Whether to log file sizes for monitoring
- Exact image quality settings for JPEG re-encoding

## Canonical References

### Types
- `crates/librefang-types/src/message.rs` — ContentBlock enum definition

### Channel Bridge
- `crates/librefang-channels/src/bridge.rs` — download_image_to_blocks() function (~line 2350)
- `crates/librefang-channels/Cargo.toml` — image crate dependency

### Existing patterns
- `crates/librefang-api/src/routes/media.rs` — existing /tmp/librefang_uploads/ usage pattern

## Specific Ideas

- The `image` crate (v0.25) is already a dependency of librefang-channels — no new dep needed
- base64 crate is already imported in bridge.rs — keep for fallback path
- uuid crate is already available for filename generation

## Deferred Ideas

- Driver support for ImageFile (Phase 2)
- Temp file cleanup (Phase 2)
- Stripping image blocks during session compaction (v2)

---

*Phase: 01-imagefile-type-and-channel-bridge*
*Context gathered: 2026-04-04 via auto mode*
