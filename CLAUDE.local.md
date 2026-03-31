# Fork-Specific Development Guidelines (f-liva/librefang)

> This file supplements the upstream `CLAUDE.md`. On conflicts, this file takes precedence
> for work on `fork/custom`. For PRs to upstream, follow `CLAUDE.md` conventions.

## User

Federico Liva (@f-liva). Speaks Italian — respond in Italian unless writing code,
commit messages, or PR descriptions (those stay in English).

## Branch strategy

### `fork/main` — PR staging area

- Always rebased on `upstream/main`
- **Every new feature or fix** starts as a branch from here
- PRs to upstream are opened from these feature branches → clean, minimal diffs
- **Never** put Lazycat-specific code here (Dockerfile, entrypoint, CI workflow)
- When upstream merges a PR, remove the corresponding commit from `fork/custom`

### `fork/custom` — production/deployment branch

- Base: `upstream/main` + all our patches + Lazycat infra
- This branch builds the Docker image deployed on the NAS
- Lazycat-specific files (never go upstream):
  - `Dockerfile`, `entrypoint.sh`, `DOCKER_README.md`
  - `.github/workflows/sync-build.yml`
  - `packages/whatsapp-gateway/ecosystem.config.cjs`
  - This file (`CLAUDE.local.md`)

### Why this structure

1. **PRs stay clean** — upstream sees only the feature diff, not deployment noise
2. **Easy merge** — when upstream accepts a PR, remove it from custom; base already has it
3. **Always deployable** — custom has everything for production
4. **Contributor-friendly** — clean PRs get reviewed and merged faster

### Visual

```
upstream/main ──────────────────────────────────►
       │
       ├── fork/main (mirror of upstream)
       │       │
       │       ├── feat/my-feature → PR to upstream
       │       └── fix/some-bug   → PR to upstream
       │
       └── fork/custom (upstream + our patches + Lazycat infra)
                └── deployed on NAS via Docker
```

## Remotes

| Name | URL | Purpose |
|------|-----|---------|
| `fork` | `git@github.com:f-liva/librefang.git` | Our fork (push here) |
| `upstream` | `https://github.com/librefang/librefang.git` | Official repo (pull only) |

**Never push to upstream** — always via PR from a branch on fork.

## CI workflow (on fork/custom)

`.github/workflows/sync-build.yml`:
- Every 6h: fetch upstream, rebase custom on top
- **Never `rebase --skip`** — on conflict: abort + Telegram notification
- Verifies no custom commits were lost
- Builds Docker → `fliva/librefang:latest`
- Telegram notifications (start/success/failure)
- Secrets: `PAT_TOKEN`, `DOCKERHUB_USERNAME`, `DOCKERHUB_TOKEN`, `TELEGRAM_BOT_TOKEN`, `TELEGRAM_CHAT_ID`

## Upstream sync checklist

Periodically:
1. Check if upstream absorbed our features (compare code, not just PRs)
2. If yes → remove that commit from custom
3. If a PR was merged → the branch can be deleted

## Lazycat NAS deployment

| Issue | Solution |
|-------|----------|
| Container forced to run as root | `entrypoint.sh` + `gosu` → drop to user `librefang` |
| Default listen 127.0.0.1 | Config: `api_listen = "0.0.0.0:4200"` |
| Auth rejects Docker IPs | Patch `is_loopback` → `is_trusted` (RFC1918) |
| npm packages lost on restart | `NPM_CONFIG_PREFIX=/data/npm-global` |
| Claude Code refuses --skip-permissions as root | gosu drops privileges; user has NOPASSWD sudo |

LZC package repo: `fede91it/lzc-librefang` (git.federicoliva.it)
Local: `/home/fede9/Progetti/lzc-librefang`

## Custom features (not yet upstream)

- **Multi-profile Claude Code**: token rotation on rate-limit via Anthropic OAuth usage API
- **WhatsApp gateway**: group chat, media, resilience, sender identity, markdown formatting
- **Channel identity**: `sender_id`, `sender_name`, `channel_type` propagated through full chain
- **Per-agent mutex**: serializes concurrent messages per agent

## Commit conventions

- **On custom branch**: `Co-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>`
- **On PR branches to upstream**: follow upstream CLAUDE.md — **no** Co-Authored-By footer
- Format: conventional commits (`feat:`, `fix:`, `docs:`, `refactor:`, `chore:`)
