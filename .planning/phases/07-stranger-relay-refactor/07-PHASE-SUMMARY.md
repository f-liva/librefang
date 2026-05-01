---
phase: 07-stranger-relay-refactor
status: COMPLETED-LOCAL  # commits on feat/phase07-stranger-relay-purge; NAS live-test pending
closed: 2026-05-01
branch: feat/phase07-stranger-relay-purge
base: upstream/main
commits_ahead: 12
loc_delta: "+1409 / -726 (net +683)"
key_files_created:
  - crates/librefang-runtime/tests/channel_send_whatsapp.rs
key_files_deleted:
  - packages/whatsapp-gateway/lib/intent_patterns.js
key_files_modified:
  - packages/whatsapp-gateway/index.js
  - packages/whatsapp-gateway/index.test.js
  - crates/librefang-channels/src/whatsapp.rs
  - crates/librefang-channels/src/types.rs
  - crates/librefang-kernel-handle/src/lib.rs
  - crates/librefang-kernel/src/kernel/mod.rs
  - crates/librefang-runtime/src/tool_runner.rs
  - crates/librefang-api/src/channel_bridge.rs
  - crates/librefang-api/src/routes/agents.rs
---

# Phase 07 — Stranger Relay Architectural Refactor — PHASE SUMMARY

**One-liner.** Eradicare l'intera macchina a stati gateway-side per
"stranger relay" (mappa conversazioni + parser `[RELAY_TO_STRANGER]` +
classificatore intent regex), sostituirla con (a) wrap XML inline
`<stranger_inbound>` zero-state e (b) tool `channel_send` esteso con
`reply_to_msg_id` + `as_voice_note` per WhatsApp; codificare l'incident
2026-04-18 19:48 come acceptance replay test eseguibile.

**Status.** Tutti i 12 commit landed sul branch
`feat/phase07-stranger-relay-purge` (worktree `/tmp/librefang-phase07-purge`).
NAS live-test + cherry-pick a `fork/custom` pending (Wave 4 hand-off al
Signore).

---

## Plans shipped

| #  | Scope                                  | Commits | Files touched                                                                  | Status |
| -- | -------------------------------------- | ------- | ------------------------------------------------------------------------------ | ------ |
| 01 | §A + §F gateway eradication            | 4       | `index.js` `index.test.js`                                                     | OK     |
| 02 | §B `channel_send` extension (WhatsApp) | 1       | `tool_runner.rs` `kernel-handle/lib.rs` `whatsapp.rs` + 5 wiring sites         | OK     |
| 03 | §C `<stranger_inbound>` XML wrap       | 2       | `index.js` (helper + 4 call sites) `index.test.js` (12-test wrap suite)        | OK     |
| 04 | §E intent classifier removal           | 3       | `index.js` (require + caller + invocation) `lib/intent_patterns.js` (deleted) | OK     |
| 05 | §G acceptance gate + summary           | 2       | `index.test.js` (12 regression guards) `channel_send_whatsapp.rs` (replay)    | OK     |

### Commit log (chronological)

```
855c504e refactor(wa-gateway): remove stranger conversation state (Map+TTL+context builders)
2e320d14 refactor(wa-gateway): remove relay parser ([RELAY_TO_STRANGER] regex + extract/execute/instruction)
8abba65c refactor(wa-gateway): drop /conversations endpoint and stranger-state log lines
15e33145 test(wa-gateway): scrub residual stranger/relay sentinel tokens from comments
1b3629d1 refactor(wa-gateway): remove ownerIntentsRelay + RELAY_INTENT_RE + intent_patterns require
55118306 refactor(wa-gateway): drop relay_intent config schema + delete intent_patterns.js
9c52f8df test(wa-gateway): remove ownerIntentsRelay test references
a11fbe48 feat(wa-gateway): wrap inbound stranger messages in <stranger_inbound> XML
9403ab12 test(wa-gateway): add wrapStrangerInbound test suite (XML wrap + escape)
ffbb8884 feat(channel-send): extend with reply_to_msg_id + as_voice_note for WhatsApp
b0286bcf test(wa-gateway): add Phase 07 regression guards (anti-rollback fences)
6d3f1274 test(runtime): add acceptance replay of 2026-04-18 incident
```

---

## Caveat resolution (from PLAN-CHECK YELLOW verdict)

| # | Caveat                                           | Resolution                                                                                                                              |
| - | ------------------------------------------------ | --------------------------------------------------------------------------------------------------------------------------------------- |
| 1 | `whatsapp_send` parallel tool vs extend          | **Resolved.** No new tool registered; `channel_send` extended with `reply_to_msg_id` + `as_voice_note` (PLAN-02). Schema test guards.   |
| 2 | `gateway_send_voice` legacy endpoint             | **Resolved.** Dead-code path removed by PLAN-02; `as_voice_note=true` routes through `send_channel_media(media_type="voice", ...)`.     |
| 3 | `[RELAY_TO_STRANGER]` historical comment leakage | **Resolved.** All historical references to forbidden literals were also stripped from `index.js` comments — regression guards now pass. |

---

## Bug eradicated (BUG_REPORT 2026-04-18 19:48 mapping)

| Bug | Description                                              | Closed by                                                                  |
| --- | -------------------------------------------------------- | -------------------------------------------------------------------------- |
| 1   | `[RELAY_TO_STRANGER]` literal echoed in owner DM         | PLAN-01 §F (parser removed) + Phase §H per-agent persona (deploy-time)    |
| 2   | `Relay rejected: no active conversation` raw error       | PLAN-01 §A (zero-state — no map = no mismatch)                            |
| 3   | `POST /message/send` endpoint string in chat             | PLAN-02 (tool-result error sanitisation) + §H persona deploy              |
| 4   | `{"success":true}` JSON dumped in chat                   | Same as #3                                                                |
| 5   | Reformulation strips emoji + jokes                       | PLAN-02 (verbatim message pass-through) + acceptance replay PLAN-05       |
| 6   | Stale `[ACTIVE_STRANGER_CONVERSATIONS]` injection        | PLAN-01 §A (state container deleted)                                      |
| 7   | Owner intent ambiguity → forced relay                    | PLAN-04 (regex classifier deleted) + agent-side ambiguity prompt (§H)     |

All 7 bugs closed at gateway/kernel level. Bug 1, 3, 4, 7 also depend on
per-agent persona deployment (§H) for full closure on the model side —
documented under "Deferred items" below.

---

## Test baseline (pre/post)

### Gateway (`packages/whatsapp-gateway/index.test.js`)

| Stage             | Tests | Pass | Fail |
| ----------------- | ----- | ---- | ---- |
| Pre Phase 07      | ~110  | ~108 | 2†   |
| After Wave 2      | 128   | 126  | 2†   |
| **After Wave 3**  | **140** | **138** | **2†** |

† EB-02 forward_dispatch + §A owner_notify failures are pre-existing,
unrelated to Phase 07. They were failing before Wave 1 and are still
failing after Wave 3 — explicitly tracked as out-of-scope.

Net delta: +12 regression guard tests (Wave 3) + 12 wrapStrangerInbound
tests (Wave 1 §C) — all green.

### Rust runtime (`crates/librefang-runtime/tests/channel_send_whatsapp.rs`)

| Stage             | Tests in file | Pass | Fail |
| ----------------- | ------------- | ---- | ---- |
| Pre Phase 07      | 0 (file did not exist) | — | — |
| After Wave 2      | 10            | 10   | 0    |
| **After Wave 3**  | **11**        | **11** | **0** |

Plus the channel_send filter test suite (16 tests) and Rust workspace
ordering / channels suites — all unchanged and green.

`cargo check --workspace --lib` and `cargo clippy -p librefang-runtime
--tests -- -D warnings` both green.

---

## Architectural decisions (from CONTEXT D-01..D-02 + delta directives)

1. **Eradicate, do not patch.** Directive Signore. Six months of layered
   patches on the relay state machine produced incidents 2026-04-18 and
   2026-05-01. Wave 1 deleted the entire subsystem in one pass instead
   of adding a 7th hotfix.

2. **Zero gateway-side state.** Every artifact tracking stranger
   sessions (`activeConversations` Map, `MAX_CONVERSATION_MESSAGES`,
   `CONVERSATION_TTL_HOURS`, `evictExpiredConversations`,
   `/conversations` debug endpoint) deleted. The only state the gateway
   keeps about strangers is the anti-spam debounce window
   (`shouldDebounceEscalation`) — which is NOT session state.

3. **Single tool, two new params.** Reject the parallel `whatsapp_send`
   tool design. Extend `channel_send` with `reply_to_msg_id` (string,
   WhatsApp-only) and `as_voice_note` (bool, WhatsApp-only). Schema
   determinism test guards prompt-cache invariance.

4. **Inline XML, not flat text.** Inbound stranger DMs are wrapped in
   `<stranger_inbound jid="..." name="..." timestamp="...">text</stranger_inbound>`
   anticipating the Phase 06 XML migration. Owner DMs and group messages
   are deliberately UN-wrapped. Closing-fence injection neutralised by
   `xmlBodyEscape`; attribute special chars escaped via the canonical
   5-entity `xmlAttrEscape`.

5. **Verbatim model output.** PLAN-02 wraps `channel_send` so the
   `message` string flows from the tool input straight to the gateway
   POST body without any rewriting. Acceptance replay PLAN-05 codifies
   this with a byte-for-byte assertion (`leggera` + `vorticare` + 🤣
   must all survive).

6. **Anti-rollback fences.** PLAN-05 ships 12 guards that grep
   `index.js` for the forbidden literals + helper names. Reintroducing
   any of them fails CI immediately. Future PRs cannot resurrect the
   relay state machine by accident.

---

## Deferred items (not in scope for this phase)

| Item                                                | Tracking                                                                                                |
| --------------------------------------------------- | ------------------------------------------------------------------------------------------------------- |
| `reply_to` body field plumbing in gateway endpoints | Rust forwards it; gateway `/message/send*` handlers do not yet consume it. Tracked as fork issue #40.   |
| `mediaUrl` extraction in `<stranger_inbound>` XML   | PLAN-03 emits the attribute but Baileys URL extraction is stubbed to `''`. Follow-up plan after Whisper. |
| Per-agent persona deployment (§H)                   | Deployment-time, per-agent. Closes Bugs 1, 3, 4, 7 on the model side. NAS-only edits, no PR upstream.   |
| Voice-note `file_path` (local file) support         | `as_voice_note` requires `file_url`. Local-path support is a separate feature surface.                  |
| Other agents WA-facing                              | Phase 07 scope was Ambrogio. Other agents (if any) keep the legacy persona until a future template.    |

---

## Lessons learned

- **"Eradicare > sistemare" pays off.** A targeted refactor in 5 plans
  closed 7 bugs that 6 months of layered hotfixes could not stabilise.
  PR #2620 (relay-intent narrow regex) and the LLM-classifier follow-up
  (PR `feat/relay-intent-llm`) are now obsolete and reverted in spirit.

- **Historical comments must die too.** Caveat 3 (regression-guard
  false positive on `[WHATSAPP_STRANGER_CONTEXT]`) was caught only when
  the guards ran. Removing the literal from `index.js` comments was
  necessary; the commit log preserves the audit trail.

- **Mocks pin behaviour, integration tests pin wiring.** `CapturingKernel`
  in `channel_send_whatsapp.rs` records 9 fields per call; without it,
  PLAN-02's `reply_to_msg_id` propagation would have been a silent
  `Option<String>::None` plumbing bug. CLAUDE.md "parallel agents +
  Option::None" gotcha applies here.

- **Acceptance replays > markdown post-mortems.** The 2026-04-18
  incident is now an executable test. The 2026-05-01 Jessica incident
  is closed by the same code path — no extra test needed because the
  fidelity assertion + the regression guards together cover it.

---

## Live-test directive (CLAUDE.local.md MANDATORY) — handoff to Signore

Phase 07 PLAN-05 was AI-executed in a worktree without daemon access
(per CLAUDE.md "human-only live integration testing"). Before any
upstream PR, the Signore must run the following on NAS in the order
listed.

### NAS deploy commands

```bash
# 1. Cherry-pick the 12 commits from feat/phase07-stranger-relay-purge
#    onto fork/custom (or merge the branch with --no-ff), then push.
git checkout custom
git cherry-pick upstream/main..feat/phase07-stranger-relay-purge
git push fork custom

# 2. Wait for the fork CI image build (sync-build.yml).
gh run watch  # on the fork repo

# 3. NAS pull + container recreate (Lazycat does NOT auto-pull):
ssh nas
lzc-docker pull fliva/librefang:latest
cd /appvar/cloud.lazycat.app.librefang
lzc-docker compose up -d --force-recreate librefang

# 4. Mirror the gateway bundle to /data (PM2-managed, NOT in image):
scp packages/whatsapp-gateway/index.js nas:/tmp/index.js
scp packages/whatsapp-gateway/lib/*.js nas:/tmp/wa-lib/
ssh nas 'lzc-docker cp /tmp/index.js cloudlazycatapplibrefang-librefang-1:/data/whatsapp-gateway/index.js'
ssh nas 'lzc-docker cp /tmp/wa-lib/. cloudlazycatapplibrefang-librefang-1:/data/whatsapp-gateway/lib/'
ssh nas 'lzc-docker exec cloudlazycatapplibrefang-librefang-1 pm2 restart whatsapp-gateway'

# 5. Smoke-test the daemon is up:
curl -s http://192.168.8.115:4545/api/health
```

### Smoke test scenarios (5 — all must PASS)

| # | Scenario                          | Setup                                               | Expected outcome                                                                                                                                  |
| - | --------------------------------- | --------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------- |
| 1 | Owner DM neutral                  | Owner sends "Ciao, come stai?" to Ambrogio          | Direct reply. ZERO `[RELAY_TO_STRANGER]`. ZERO `channel_send` call (no reason).                                                                   |
| 2 | Stranger inbound text             | Test stranger sends "Salve, ..."                    | Gateway logs `<stranger_inbound jid="..." name="..." timestamp="...">Salve...</stranger_inbound>`. Ambrogio either replies via `channel_send` OR notifies owner via `notify_owner`. ZERO endpoint/JSON leak. |
| 3 | **Owner delegated speech (REPLAY 2026-04-18)** | Pre: scenario 2 established a stranger context. Owner sends "Bene dille che mangi leggera che c'è da vorticare dopo 🤣" | Ambrogio calls `channel_send(channel="whatsapp", jid=<stranger>, message="...")` with **all 3 markers preserved** (leggera + vorticare + 🤣). Stranger receives faithful message. Owner sees brief confirmation. |
| 4 | Owner ambiguity                   | Pre: 2 recent strangers in memory                   | Owner says "dille grazie" → Ambrogio asks "Intende X o Y?". Clarification rule active.                                                            |
| 5 | Group message no wrap, no relay   | Owner mentions @ambrogio in a group                 | Gateway forwards `[Group message from <name>]\n@ambrogio che ne pensi?`. NO `<stranger_inbound>`. Ambrogio replies normally in group.            |

### Side-effect verification

```bash
# Cost / metering increments after scenario 3:
curl -s http://192.168.8.115:4545/api/budget/agents/<ambrogio-id>

# Kernel logs:
ssh nas 'lzc-docker logs cloudlazycatapplibrefang-librefang-1 --tail 200 | grep -E "tool_call.*channel_send|tool_call.*notify_owner"'

# Gateway logs:
ssh nas 'lzc-docker exec cloudlazycatapplibrefang-librefang-1 pm2 logs whatsapp-gateway --lines 100 | grep -E "Sent|stranger_inbound|forward_dispatch"'
```

### Final acceptance gate

If all 5 scenarios PASS → Phase 07 ACCEPTED → unblock fork PR upstream
"Phase 07 stranger-relay refactor" (PLAN-01..04 cumulative + PLAN-05
test-only addendum).

If any scenario FAILS → file issue on fork repo, plan a Phase 07 hotfix
(do NOT open upstream PR).

---

## Open follow-ups

- Open fork issue #40 already exists for `reply_to` gateway plumbing.
- File a follow-up issue for `mediaUrl` extraction inside
  `<stranger_inbound>` (currently stubbed to empty string).
- Per-agent persona refresh (§H deployment) is a separate Wave that
  the Signore drives at NAS-level — no AI involvement.

---

## Phase 07 ship-readiness verdict

**GREEN** for the local refactor scope (12 commits, 138/140 gateway
tests pass, 11/11 runtime channel_send tests pass, zero clippy
warnings, all forbidden literals eradicated from `index.js`).

**YELLOW** for full ship: NAS live-test with Signore in the loop is the
final gate (Task 3 checkpoint) and has not yet been executed. Cherry-
pick to `fork/custom` + sync-build.yml + smoke test still pending.

**RED** for upstream PR: explicitly blocked until Signore confirms the
5 NAS smoke scenarios green (CLAUDE.local.md mandatory live-test
directive).
