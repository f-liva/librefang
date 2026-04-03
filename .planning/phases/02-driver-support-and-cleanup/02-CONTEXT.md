# Phase 2: Driver Support and Cleanup - Context

**Gathered:** 2026-04-04
**Status:** Ready for planning
**Source:** Auto mode

## Phase Boundary

All LLM drivers handle ContentBlock::ImageFile natively. Background cleanup removes expired temp files. Phase 1 added the ImageFile variant and channel bridge saves files to disk — this phase wires the drivers and adds housekeeping.

## Implementation Decisions

### Claude Code driver: direct file path
- For ContentBlock::ImageFile, pass the file path directly to the CLI (no temp copy needed)
- The CLI already supports @file references for images
- For ContentBlock::Image (base64), keep existing behavior (write temp file, pass path)
- File: `crates/librefang-runtime/src/drivers/claude_code.rs`

### API-based drivers: lazy base64 loading
- For ContentBlock::ImageFile, read the file from disk and base64-encode at call time
- For ContentBlock::Image (base64), use existing data directly
- If file is missing (deleted by cleanup before LLM call), log warning and skip the image block
- Applies to: anthropic.rs, openai.rs, gemini.rs, chatgpt.rs, vertex_ai.rs
- Pattern: add a helper function `resolve_image_base64(block: &ContentBlock) -> Option<(String, String)>` that returns (media_type, base64_data) for both Image and ImageFile

### Temp file cleanup
- Background task in kernel that runs every hour
- Removes files in /tmp/librefang_uploads/ older than 24 hours
- Uses tokio::fs for async file operations
- Runs as a spawned task alongside existing heartbeat/supervisor tasks
- File: `crates/librefang-kernel/src/kernel.rs` (add to boot sequence)

### Claude's Discretion
- Exact placement of cleanup task in kernel boot sequence
- Whether to log each deleted file or just a summary count
- Error handling for files that can't be deleted (permissions)

## Canonical References

### Drivers
- `crates/librefang-runtime/src/drivers/claude_code.rs` — image preparation for CLI
- `crates/librefang-runtime/src/drivers/anthropic.rs` — API image handling
- `crates/librefang-runtime/src/drivers/openai.rs` — API image handling
- `crates/librefang-runtime/src/drivers/gemini.rs` — API image handling

### Types
- `crates/librefang-types/src/message.rs` — ContentBlock::ImageFile (from Phase 1)

### Kernel
- `crates/librefang-kernel/src/kernel.rs` — boot sequence, background tasks

## Deferred Ideas

- Strip image blocks during session compaction (v2)
- Configurable TTL per channel

---
*Phase: 02-driver-support-and-cleanup*
*Context gathered: 2026-04-04 via auto mode*
