# LibreFang for Lazycat NAS

Custom [LibreFang](https://github.com/librefang/librefang) Docker image optimized for deployment on Lazycat LCMD Microserver.

**Automatically rebuilt on every new upstream release via GitHub Actions.**

## What's included

- **LibreFang Agent OS** — Rust-based autonomous AI agent daemon
- **Claude Code CLI** — Anthropic's CLI for Claude, as LLM provider
- **Node.js 22** — JavaScript runtime
- **Python 3** — Python runtime
- **Go** — via Homebrew
- **Homebrew** — package manager for additional tools
- **uv** — fast Python package manager
- **gh** — GitHub CLI
- **gog** — [Google Workspace CLI](https://gogcli.sh/) (Gmail, Calendar, Drive, Sheets, etc.)
- **ffmpeg** — multimedia processing
- **jq** — JSON processor
- **git, curl, wget** — standard utilities

## Non-root execution

The image uses `gosu` to drop root privileges to the `librefang` user at runtime. This is required because Claude Code's `--dangerously-skip-permissions` flag refuses to run as root.

The `librefang` user has passwordless `sudo` access, so it can still install system packages when needed.

## Usage

```bash
docker run -d \
  -p 4545:4545 \
  -v librefang-data:/data \
  -v librefang-home:/home/librefang \
  -e LIBREFANG_HOME=/data \
  fliva/librefang:latest
```

## Source

- **This fork**: [github.com/f-liva/librefang](https://github.com/f-liva/librefang)
- **Upstream**: [github.com/librefang/librefang](https://github.com/librefang/librefang)
