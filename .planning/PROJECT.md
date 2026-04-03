# Image Path References for Session Efficiency

## What This Is

Replace inline base64 image encoding in agent sessions with file path references. Channel images (Telegram, WhatsApp) are saved as temporary files on disk and referenced by path in the conversation, instead of embedding megabytes of base64 data that bloats session storage and causes context overflow.

## Core Value

Agent sessions must remain lightweight regardless of how many images are exchanged — images live on disk, sessions store only paths.

## Requirements

### Validated

- ✓ Images from channels are downloaded and passed to LLM for vision analysis — existing
- ✓ Claude Code CLI supports file path references for images — existing
- ✓ Image downscaling to 1024px max dimension — existing (crate `image` in Cargo.toml)
- ✓ Temporary upload directory at `/tmp/librefang_uploads/` — existing

### Active

- [ ] ContentBlock::ImageFile variant alongside existing Image variant
- [ ] Channel bridge saves images to disk instead of base64 encoding
- [ ] Claude Code driver handles ImageFile (direct file path, no temp copy)
- [ ] API-based drivers lazy-load base64 from file when needed
- [ ] Temp file cleanup with 24h TTL
- [ ] Retrocompatibility: ContentBlock::Image (base64) still works for API calls

### Out of Scope

- Permanent image storage (S3, cloud) — temp files are sufficient for session lifetime
- Image deduplication — same image sent twice creates two files
- Per-session file tracking — cleanup is time-based, not session-based
- Changes to the Session struct or DB schema

## Context

**Incident (2026-04-03):** 16 photos from Telegram were debounced into a single batch. Each photo was ~3MB, converted to ~4MB base64, totaling ~494k tokens in the session. The context overflow was "unrecoverable" even with aggressive compaction.

**Current flow:** `download_image_to_blocks()` in `bridge.rs` downloads image → base64 encodes → creates `ContentBlock::Image { media_type, data }` → stored inline in session messages → serialized to SQLite.

**Target flow:** `download_image_to_blocks()` downloads image → downscales to 1024px → saves to `/tmp/librefang_uploads/{uuid}.jpg` → creates `ContentBlock::ImageFile { media_type, path }` → only path stored in session → driver reads file when sending to LLM.

## Constraints

- **Retrocompat**: `ContentBlock::Image` (base64) must keep working for direct API calls
- **Serde**: `ImageFile` variant must deserialize without breaking existing sessions
- **Absolute paths**: File references must use absolute paths
- **File permissions**: Temp files readable by librefang process
- **Tests**: All existing tests pass, zero clippy warnings

## Key Decisions

| Decision | Rationale | Outcome |
|----------|-----------|---------|
| Add ImageFile variant vs modifying Image | Retrocompat — existing API callers send base64 inline | — Pending |
| Temp files with TTL vs session-linked cleanup | Simpler — no tracking per session, just age-based | — Pending |
| Lazy base64 loading for API drivers | Avoids storing base64 anywhere — generated on-the-fly from file | — Pending |

## Evolution

This document evolves at phase transitions and milestone boundaries.

---
*Last updated: 2026-04-04 after initialization*
