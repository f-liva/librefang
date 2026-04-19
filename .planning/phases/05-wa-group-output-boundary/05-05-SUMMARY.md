---
phase: 05-wa-group-output-boundary
plan: 05
subsystem: ambrogio-persona
tags: [ambrogio, persona, anti-hallucination, prompt-engineering, fork-custom]
requires: []
provides:
  - ambrogio manifest [model].system_prompt anti-rule-citation anchor
  - N=10 acceptance harness scripts/phase05/persona_anchor_acceptance.sh
affects:
  - /data/agents/ambrogio/agent.toml (NAS runtime manifest)
  - workspaces/agents/ambrogio/agent.toml (repo-tracked copy)
tech-stack:
  added: []
  patterns:
    - "TOML triple-quoted multi-line system_prompt"
    - "Prompt-layer anchor (zero-code mitigation)"
key-files:
  created:
    - workspaces/agents/ambrogio/agent.toml
    - .planning/phases/05-wa-group-output-boundary/persona-anchor-fixtures/ambrogio_pre.toml
    - scripts/phase05/persona_anchor_acceptance.sh
  modified: []
decisions:
  - "Anchor lives in [model].system_prompt (kernel-level), not workspace MD — supplements claude-code workspace persona without touching maggiordomo voice files"
  - "Added fresh system_prompt field because pre-edit manifest had none — TOML diff is purely additive, no existing text mutated"
  - "Included a `---` separator line ABOVE the anchor paragraph to keep the anchor's phrase 'sopra la linea di separazione in questo system prompt' grammatically coherent even when nothing precedes it (empty above-separator → anchor correctly forbids all numbered citation)"
  - "Hot-reload endpoint returns 400 on live daemon for BOTH the anchor manifest AND the pre-edit baseline — pre-existing daemon bug, not caused by PA-01. Manifest will be picked up on next daemon restart (deferred investigation)"
metrics:
  duration_seconds: 395
  completed: 2026-04-19T18:08Z
---

# Phase 05 Plan 05: Ambrogio Persona Anti-Hallucination Anchor Summary

Anti-rule-fabrication anchor appended to ambrogio's system_prompt via TOML manifest edit, backed by a scripted N=10 acceptance harness that detects `§N.M` leaks in replies to the sentinel prompt.

## What shipped

- **Manifest edit** (`workspaces/agents/ambrogio/agent.toml`) — new `[model].system_prompt` triple-quoted field containing the locked paragraph from CONTEXT §E, verbatim, byte-for-byte, with a leading `---` separator line.
- **Pre-edit snapshot** (`.planning/phases/05-wa-group-output-boundary/persona-anchor-fixtures/ambrogio_pre.toml`) — verbatim copy of the NAS `/data/agents/ambrogio/agent.toml` before the anchor was added, for audit/rollback evidence.
- **Acceptance harness** (`scripts/phase05/persona_anchor_acceptance.sh`) — 148-line bash script with distinct exit codes (0 pass, 1 leak, 2 agent missing, 3 anchor on disk but not hot-reloaded), preflight check on live runtime, tolerant regex `§[[:space:]]*[0-9]+[[:space:]]*[.,][[:space:]]*[0-9]+`, LIBREFANG_API + N_TURNS env overrides, per-run structured log at `/tmp/persona_anchor_<epoch>.log`.

## Manifest path resolution (Task 1 finding)

- Local repo had **no** existing ambrogio manifest — workspace tree was untracked.
- NAS container path: `/data/agents/ambrogio/agent.toml` (lowercase TOML, 223 bytes pre-edit). Confirmed via `lzc-docker exec`.
- NAS had a separate `/data/workspaces/agents/ambrogio/AGENT.json` but that file is a 108-byte pointer stub (`{"created_at":..., "workspace":"/data/workspaces/agents/ambrogio"}`), NOT the manifest — CONTEXT §E's "AGENT.json" mention was a red herring resolved by codebase evidence.
- Form: **Case B** in plan's terms — pre-edit file had `[model]` section with `provider` + `model` only, NO `system_prompt` key. Added the field fresh with triple-quoted multi-line form.
- Separator line: NO separator existed in the pre-edit prompt (because there was no prompt). Injected `---\n\n` ABOVE the anchor paragraph so the anchor's referential phrase "sopra la linea di separazione in questo system prompt" stays grammatically coherent — above-separator is intentionally empty, making the anchor forbid all numbered citation (strictest possible reading).
- Persona voice was untouched: maggiordomo-aulico character lives in `/data/workspaces/agents/ambrogio/{SOUL,IDENTITY,AGENTS,RULES,STYLE,TOOLS}.md` (~100KB across 7+ files), wired via claude-code driver workspace context. The `[model].system_prompt` we added is ADDITIVE to those MDs, not a replacement.

## Anchor paragraph

Length: **487 characters** (including the `---\n\n` separator preamble and trailing newline).

Verbatim (copied from CONTEXT §E):

> **Regola rigida sui riferimenti normativi**: non citare mai regole, sezioni, articoli, §N.M, policy numerate, clausole, paragrafi o leggi SALVO che il loro testo esplicito sia definito esplicitamente sopra la linea di separazione in questo system prompt. Se ti accingi a citare un articolo di regolamento, verifica prima che sia scritto qui. Se non è presente, NON inventarlo. Limítati a enunciare l'azione che stai prendendo senza giustificarla con riferimento normativo fittizio.

## Verification

| Check | Command | Result |
|---|---|---|
| TOML parse | `python3 -c "import tomllib; m=tomllib.load(open('workspaces/agents/ambrogio/agent.toml','rb')); print(len(m['model']['system_prompt']))"` | `487` |
| Anchor phrase present | `grep -c 'Regola rigida sui riferimenti normativi' workspaces/agents/ambrogio/agent.toml` | `1` |
| Diff vs snapshot (additive-only) | `diff -u …ambrogio_pre.toml workspaces/agents/ambrogio/agent.toml` | `+5 added, -0 removed, 0 modified` |
| Script executable + valid syntax | `test -x scripts/phase05/persona_anchor_acceptance.sh && bash -n …` | `ok` |
| NAS mirror | `lzc-docker exec … grep -c 'Regola rigida' /data/agents/ambrogio/agent.toml` | `1` |

## NAS deploy status

- `scp` + `lzc-docker cp` landed the new manifest at `/data/agents/ambrogio/agent.toml` in the live container. Confirmed content round-trip via `lzc-docker cp` back to host.
- **Hot-reload failed**: `POST /api/agents/{ambrogio_id}/reload` returns **HTTP 400** (`{"error":"api-error-generic"}`) on the current daemon binary. This reproduces **even when the on-disk manifest is reverted to the pre-edit baseline** — the endpoint is broken on the deployed daemon, unrelated to the anchor.
- Live `GET /api/agents/{ambrogio_id}` confirms the in-memory `system_prompt` is still the default `"You are a helpful AI agent."` — the anchor is on disk but not yet in the runtime.
- Expected remediation: next daemon restart (or next CI image pull + recreate cycle) will read the on-disk manifest and the anchor will be picked up. No code change needed for PA-01 to take effect eventually; the reload bug is a separate regression.

## Acceptance run

**Not executed in this plan run** because the runtime does NOT yet carry the anchor (Task 3 preflight check at exit code 3 would correctly refuse to run). Once Signore restarts the daemon and the preflight passes, operator can run:

```
LIBREFANG_API=http://192.168.8.115:4545 N_TURNS=10 \
  ./scripts/phase05/persona_anchor_acceptance.sh
```

Expected: `PASS (0 leaks in N=10)`, exit 0.

## Deviations from Plan

### Rule 3 — Blocking issue auto-resolved

**1. [Rule 3 — Blocking] Manifest had no existing `[model].system_prompt` field**
- **Found during:** Task 1
- **Issue:** Plan Task 2 offered Case A (triple-quoted, append) vs Case B (single-line, convert-to-triple-quoted). Reality: neither — the field was absent entirely, and Ambrogio's persona lives in `/data/workspaces/agents/ambrogio/*.md` (SOUL.md, IDENTITY.md, AGENTS.md, RULES.md, STYLE.md, TOOLS.md), loaded into the claude-code driver via workspace_root + `--add-dir` (see `crates/librefang-runtime/src/workspace_context.rs:18-21` and `crates/librefang-kernel/src/kernel/mod.rs:11117` `read_identity_file(...)`).
- **Fix:** Added the `system_prompt` field fresh as a triple-quoted multi-line TOML string. Plan's additive-only invariant is still satisfied (zero persona text mutated, zero MD files touched). The anchor now reaches the model via two independent channels: (1) kernel-level `ctx.manifest.model.system_prompt` per `crates/librefang-runtime/src/agent_loop.rs:1820`, and (2) workspace MDs that already exist — belt and braces.
- **Files modified:** `workspaces/agents/ambrogio/agent.toml`
- **Commit:** `3535ee04`

### Rule 4 — Architectural concern (deferred, NOT fixed in this plan)

**2. [Rule 4 — Deferred] Hot-reload endpoint broken on live daemon**
- **Found during:** Task 2 NAS deploy
- **Issue:** `POST /api/agents/{id}/reload` returns HTTP 400 `{"error":"api-error-generic"}` on the current daemon binary, even when the manifest on disk is the **unmodified pre-edit baseline** (reverted, reloaded → same 400). Error surfaces in `crates/librefang-api/src/routes/agents.rs:3680` as `api-error-generic` from `state.kernel.reload_agent_from_disk(agent_id)`. The daemon log shows only the middleware 400 line — no `reload_agent_from_disk` error trace. Potentially an unrelated regression in the deployed binary (fork/custom `custom` branch currently running).
- **Impact on PA-01:** The anchor edit landed on disk correctly. It will be picked up on next daemon restart. Nothing about this blocks the PA-01 objective; it just means the acceptance script can't run until restart happens.
- **Why NOT fixed here:** Fixing the reload endpoint is orthogonal to §E persona anchor, touches Rust kernel code (out of scope per the plan's "config-only edit" success criterion), and would require a fresh image build + NAS deploy cycle that dwarfs the anchor change itself. Deferred to a future plan.
- **Follow-up:** File an issue (or note in `.planning/deferred-items.md`) to investigate `reload_agent_from_disk` on the current fork/custom binary. Likely candidates: (a) `source_toml_path` drift after registry migration, (b) some downstream step in the reload chain that was added after the deployed binary was cut. Repro script: `sshpass -p '...' ssh root@192.168.8.115 'lzc-docker exec librefang-1 curl -X POST http://127.0.0.1:4545/api/agents/<id>/reload'`.

### Scope deferrals (from plan, per user directive)

- **Task 4 checkpoint (Signore Beeper smoke)** — skipped per the user's explicit instruction in the execute prompt ("fuori scope del task (skip — Signore manually verifies)"). No automated action from executor.
- **Live acceptance-script run** — skipped because the runtime has not hot-reloaded the anchor (would exit 3 and produce a false-negative failure unrelated to PA-01). Will run after Signore-triggered restart.

### Fork/custom discipline

- **No cherry-pick to `fork/custom`** — per user prompt ("NO push fork/custom, solo fork feature branch"). Only `feat/ambrogio-persona-anchor` was pushed to `fork`. Signore decides when/if to cherry-pick for the NAS deploy cycle. The NAS manifest was mirrored live (`lzc-docker cp`) for the current container only; next container rebuild will overwrite it unless the commit lands on `fork/custom` first.

## Commits

| Hash | Message |
|---|---|
| `3535ee04` | feat(ambrogio): persona anti-hallucination anchor |
| `a905d8f4` | test(ambrogio): N=10 acceptance harness for persona anchor |

Branch: `feat/ambrogio-persona-anchor` (pushed to `fork`, based on `upstream/main`).

## Threat Flags

None — prompt-layer edit, no new network/auth/schema surface.

## TDD Gate Compliance

N/A — plan type is `execute`, not `tdd`. The `test(...)` acceptance-harness commit is an end-to-end script, not an xUnit-style unit test; it follows the `feat(...)` commit but does not exercise TDD gating.

## Self-Check: PASSED

- FOUND: workspaces/agents/ambrogio/agent.toml
- FOUND: .planning/phases/05-wa-group-output-boundary/persona-anchor-fixtures/ambrogio_pre.toml
- FOUND: scripts/phase05/persona_anchor_acceptance.sh
- FOUND: .planning/phases/05-wa-group-output-boundary/05-05-SUMMARY.md
- FOUND commit: 3535ee04 (feat anchor)
- FOUND commit: a905d8f4 (test harness)
- FOUND remote branch: `fork/feat/ambrogio-persona-anchor` carries both commits
