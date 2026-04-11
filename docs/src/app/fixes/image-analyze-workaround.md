# Image Analyze Workaround

## Problem

The `image_analyze` tool is registered in `tool_runner.rs` but is **not functional** in Qwen Code agent runtimes. The tool appears in the tool list but fails silently or returns errors when called.

## Root Cause

Qwen Code agents do not have a vision-capable LLM backend connected to `image_analyze`. The tool reads image files and encodes them as base64, but there is no downstream vision model to interpret them.

## Workaround (Working)

The `read_file` tool **natively supports image files** (JPG, PNG, WebP, SVG, BMP, GIF). When an image is passed to `read_file`, it renders the image content directly in the agent's context.

### Procedure

```bash
# 1. Download the image
curl -s -o /tmp/img.jpg "IMAGE_URL"

# 2. Read it with read_file (supports JPG/PNG/WebP/SVG/BMP/GIF)
read_file /tmp/img.jpg
```

This works because `read_file` has built-in image rendering support, while `image_analyze` requires an external vision model that isn't available.

## Affected Code

- `crates/librefang-runtime/src/tool_runner.rs` — `tool_image_analyze()` function
- Agent workspace: `TOOLS.md` — documented workaround

## Long-term Fix

Integrate a vision-capable model (e.g., GPT-4V, Claude Vision) with `image_analyze`, or deprecate `image_analyze` in favor of making `read_file` the primary image handling path for agents without vision backends.
