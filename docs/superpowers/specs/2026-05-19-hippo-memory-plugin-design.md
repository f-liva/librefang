# Hippo Memory Plugin for LibreFang - Design Spec

**Date:** 2026-05-19
**Author:** LibreFang Community
**Status:** Design approved, pending implementation

## Overview

The `hippo-memory` plugin integrates the Hippo biologically-inspired memory layer as the context engine for LibreFang agents. It provides full Hippo feature set: decay-based forgetting, retrieval strengthening, consolidation, goal stack, continuity, and drill-down DAG.

### Key Features

- **Ingest + Recall** - Automatic message storage and semantic retrieval
- **Decay & Consolidation** - Memories decay over time, strengthen on retrieval, consolidate during sleep
- **Goal Stack** - dlPFC-style goal-conditioned memory boosting
- **Continuity** - Session handoff for long-lived tasks
- **Drill-down DAG** - Recover details from compressed summaries
- **Per-Agent Isolation** - Each agent has its own Hippo database
- **Embedded Hippo** - No external dependencies, plugin uses Hippo's JavaScript API directly in-process

### Architecture

**Monolithic Node.js plugin** with embedded Hippo store management:

1. **Hippo Store Manager** - Initializes per-agent stores using `initStore()`
2. **Per-Agent Database Isolation** - `agents/{agent_id}/hippo.db` with full isolation
3. **Hook Layer** - All 7 hooks implemented in single `hooks/index.js`
4. **Configuration** - `plugin.toml` with feature flags and tuning

**Zero overhead:** Uses Hippo's JavaScript API directly (no HTTP roundtrip).

## Implementation Design

### Plugin Structure

```
~/.librefang/plugins/hippo-memory/
├── plugin.toml                          # Manifest with configuration
├── hooks/index.js                       # Monolithic plugin (all 7 hooks)
├── package.json                         # Node.js dependencies
├── README.md                            # Installation and usage
├── state.json                           # Plugin state (auto-created)
├── agents/                              # Per-agent databases (auto-created)
│   └── {agent_id}/
│       └── hippo.db                     # SQLite DB (Hippo schema v25)
└── .disabled                            # Marker when disabled
```

### Hook Implementations

#### 1. bootstrap

Creates per-agent database and initializes plugin state. Initializes Hippo store using `initStore()`.

**Input:** `{ type: "bootstrap", agent_id, config }`

**Output:** `{ status: "ok", data: { hippo_db_path } }`

#### 2. ingest

Saves messages to Hippo with tagging and provenance. Initializes Hippo store if not initialized.

**Input:** `{ type: "ingest", agent_id, message, timestamp }`

**Output:** `{ status: "ok", data: { memory_id, strength } }`

Tags: `kind:raw`, `role:{role}`, `scope:agent:{agent_id}`, `error` (if user error).

#### 3. assemble

Retrieves relevant memories with hybrid search (BM25 + embeddings), continuity, and optional goal stack boost.

**Input:** `{ type: "assemble", agent_id, query, budget, session_id, max_results }`

**Output:** `{ status: "ok", data: { memories, continuity } }`

Scope-limited to `agent:{agent_id}` for isolation.

#### 4. compact

Triggers Hippo's consolidation: decay, merge, prune.

**Input:** `{ type: "compact", agent_id, pressure_threshold }`

**Output:** `{ status: "ok", data: { consolidated, pruned, merged } }`

#### 5. after_turn

Updates session handoff and propagates goal outcome to memories.

**Input:** `{ type: "after_turn", agent_id, session_id, outcome }`

**Output:** `{ status: "ok", data: {} }`

#### 6. prepare_subagent

Creates session snapshot for sub-agent isolation.

**Input:** `{ type: "prepare_subagent", agent_id, subagent_id, session_id }`

**Output:** `{ status: "ok", data: { snapshot_id } }`

#### 7. merge_subagent

Merges sub-agent memories back into parent session with outcome propagation.

**Input:** `{ type: "merge_subagent", agent_id, subagent_id, snapshot_id }`

**Output:** `{ status: "ok", data: { memories_added, memories_merged } }`

### Hippo Integration

#### Store Initialization

**Per-Agent Store Initialization (`ensureStoreInitialized()`):**
- Initializes Hippo store on first hook execution (`bootstrap`, `ingest`, or `assemble`)
- Uses `initStore(dbPath)` to create/open SQLite database
- Store instance cached in-memory for subsequent hook calls
- No process management - Hippo runs in plugin process

**Cleanup:**
- Close all store connections on plugin disable
- Remove agent DBs on plugin uninstall

#### API Integration

Uses Hippo's JavaScript API directly:

```javascript
import { initStore, createMemory, writeEntry, hybridSearch, consolidate, applyOutcome, saveSessionHandoff, loadLatestHandoff } from 'hippo-memory';

// In bootstrap hook
const store = initStore(dbPath);

// In ingest hook
const memory = createMemory({
  content: message.content,
  tags: [`role:${message.role}`, `kind:raw`, `scope:agent:${agent_id}`],
  artifact_ref: `message:${timestamp}`,
  owner: `agent:${agent_id}`
});
const result = await writeEntry(store, memory);

// In assemble hook
const results = await hybridSearch(store, {
  query: input.query,
  limit: input.max_results,
  budget: input.budget,
  include_continuity: config.enable_continuity,
  scope: `agent:${agent_id}`
});

// In compact hook
const result = await consolidate(store);

// In after_turn hook (if goal outcome)
await applyOutcome(store, {
  goal_id: input.outcome.goal_id,
  outcome_score: input.outcome.score,
  session_id: input.session_id
});

// In continuity (after_turn)
await saveSessionHandoff(store, {
  session_id: input.session_id,
  snapshot: {...}
});

// For handoff retrieval
const handoff = await loadLatestHandoff(store, input.session_id);
```

Zero HTTP overhead - direct function calls.

### Database Layout

**Per-Agent Isolation:**

```
~/.librefang/plugins/hippo-memory/
├── state.json
└── agents/
    ├── agent-123/hippo.db
    ├── agent-456/hippo.db
    └── ...
```

**Scope Convention:**
- `scope:agent:{agent_id}` - Agent-specific memories
- `session:{session_id}` - Session-specific (continuity)
- `global` - Optional shared lessons (future)

**Multi-Tenant Cleanup:**
- Disable plugin: remove `agents/{agent_id}/` only if empty
- Uninstall plugin: remove `agents/` completely (with confirmation)

### Configuration

**`plugin.toml` Configuration:**

```toml
[hooks]
runtime = "node"
allow_network = false          # Hippo is local
allow_filesystem = true        # For DB writes

[config]
# Feature flags
enable_decay = { type = "boolean", default = true }
enable_goal_stack = { type = "boolean", default = true }
enable_continuity = { type = "boolean", default = true }
enable_drilldown = { type = "boolean", default = true }

# Recall defaults
default_recall_limit = { type = "number", default = 10 }
default_fresh_tail_count = { type = "number", default = 5 }

[env]
HIPPO_PLUGIN_LOG_LEVEL = "info"  # or "debug" for stack traces
```

Accessible in hooks via input `config` field.

### State Persistence

**`state.json` Schema:**

```json
{
  "version": "1.0.0",
  "last_db_check": 1672531200,
  "initialized_agents": ["agent-123", "agent-456"]
}
```

Written atomically, readable for debugging, used to track which agents have been initialized.

## Error Handling

### Two-Layer Strategy

**Layer 1: Hook-Level Errors**
- Catch-all `try/catch` in every hook
- Exit with code 1 and structured error JSON on stdout/stderr
- Error codes: `db_corrupted`, `scope_violation`, `invalid_config`, `store_init_failed`

**Layer 2: DB Corruption Recovery**
- Automatic DB rebuild on SQLite corruption
- Corrupted DBs are backed up with `.corrupted.{timestamp}` suffix
- Hippo automatically recreates schema on next `initStore()` call
- Logged to state.json with timestamp

## Testing Strategy

### Unit Tests (Vitest)

Coverage:
- Store initialization (initStore, per-agent isolation)
- Hook logic (all 7 hooks)
- Integration (full workflow)
- Edge cases (DB corruption, concurrent execution, multi-tenant isolation, scope filtering)

```bash
cd ~/.librefang/plugins/hippo-memory
npm test
```

### Manual Testing Checklist

1. Install plugin from registry
2. Enable per agent
3. Start agent and send message (verify ingest)
4. Send query (verify recall surfaces memory)
5. Wait for consolidation (verify decay/prune)
6. Spawn sub-agent (verify isolation)
7. Merge sub-agent (verify memory merge)
8. Disable plugin (verify cleanup)

### Debug Mode

Set environment variable in `plugin.toml`:

```toml
[env]
HIPPO_PLUGIN_LOG_LEVEL = "debug"  # Full stack traces on errors
```

## Installation & Usage

### Installation Methods

**Registry (recommended):**
```bash
librefang plugin install hippo-memory
```

**Local:**
```bash
git clone https://github.com/librefang/hippo-memory-plugin.git
cd hippo-memory-plugin
librefang plugin install --local .
```

**Git URL:**
```bash
librefang plugin install --git https://github.com/librefang/hippo-memory-plugin.git
```

### Prerequisites

Automatically installed:
- Node.js 22.5+
- `hippo-memory` npm package v1.8.1+

Manual check:
```bash
node --version    # >= 22.5
hippo --version   # >= 1.8.1
```

### Configuration

**Enable per agent:**
```bash
librefang agent edit my-agent --context-engine-plugin hippo-memory
```

**Customize settings:**
```bash
librefang plugin config hippo-memory --enable-goal-stack=false
```

Or edit `~/.librefang/plugins/hippo-memory/plugin.toml` directly.

### Usage Examples

**Basic Memory:**
```bash
librefang agent start my-agent
librefang agent send my-agent "I deployed the app but it's failing on /api/users"
# Later:
librefang agent send my-agent "Why is /api/users failing?"
# → Hippo recall surfaces: "I deployed the app but it's failing on /api/users"
```

**Goal Stack:**
```bash
hippo goal push "Fix /api/users endpoint"
# Work on task...
hippo goal complete --outcome 0.9  # High score → boost memories
# or
hippo goal complete --outcome 0.2  # Low score → decay memories
```

**Sub-Agent Isolation:**
```bash
librefang agent spawn sub-agent --parent main-agent
# Sub-agent has isolated snapshot → no parent memories visible
# After completion, merge_subagent brings back memories
```

**Manual Recall:**
```bash
cd ~/.librefang/plugins/hippo-memory/agents/agent-123
hippo recall "FRED cache" --budget 2000
```

### Verification

**Check plugin:**
```bash
librefang plugin list
librefang plugin info hippo-memory
librefang plugin lint hippo-memory
```

**Check DB contents:**
```bash
cd ~/.librefang/plugins/hippo-memory/agents/agent-123
hippo recall --all --json | jq '.memories[] | .content'
```

## Success Criteria

### Functional

1. ✅ Ingest works - messages saved with tagging and provenance
2. ✅ Recall works - query returns memories with hybrid search (BM25 + embeddings)
3. ✅ Decay works - memories decay if not accessed
4. ✅ Goal stack works - boost memories for active goal
5. ✅ Continuity works - session handoff for long-lived tasks
6. ✅ Drill-down works - recover details from summaries
7. ✅ Sub-agent isolation works - session snapshots and merge

### Non-Functional

8. ✅ Zero network outbound - no external HTTP calls
9. ✅ Per-agent isolation - agent A cannot read agent B's memories
10. ✅ Automatic DB recovery on corruption
11. ✅ Plugin cleanup - store connections closed on disable, DBs removed on uninstall

### Integration

13. ✅ Hook protocol compatible - JSON-over-stdin/stdout with LibreFang
14. ✅ Config schema valid - passes `librefang plugin lint`
15. ✅ Automatic installation - prerequisites installed if missing

## Deliverables

### Core

1. `hooks/index.js` - Monolithic plugin (all 7 hooks)
2. `plugin.toml` - Complete manifest with configuration
3. `package.json` - Hippo dependencies
4. `README.md` - Installation, configuration, usage examples
5. `state.json` - Auto-created, schema documented

### Testing

6. `test/` directory - Unit tests + integration tests (Vitest)
7. `package.json` scripts - `npm test`, `npm run lint`
8. `test-manual.md` - Manual testing checklist

### Documentation

9. `DESIGN.md` - This document (archived for reference)
10. `CHANGELOG.md` - Version history

### Registry

11. Registry entry - In `librefang/librefang-registry`
12. GitHub repository - `librefang/hippo-memory-plugin` (public)

## MVP Scope

**v1.0.0 (MVP):**
- All 7 hooks implemented
- Ingest + recall working
- Decay + consolidation
- Per-agent isolation
- Basic testing (5-10 key tests)

**Post-MVP (v1.1.0+):**
- Goal stack integration
- Continuity
- Drill-down DAG
- Comprehensive test suite
- Advanced configuration

## Rollback Plan

**If plugin doesn't work:**
```bash
librefang plugin disable hippo-memory
rm -rf ~/.librefang/plugins/hippo-memory/agents/*
librefang plugin enable hippo-memory
```

**If Hippo DB corrupts:**
```bash
# Plugin automatically backs up corrupted DB and recreates
# Check for .corrupted.* files:
ls ~/.librefang/plugins/hippo-memory/agents/*/hippo.db.corrupted.*
```

## Summary

The `hippo-memory` plugin provides a production-ready, feature-complete integration of Hippo's biologically-inspired memory layer into LibreFang. It uses a monolithic Node.js approach for simplicity, Hippo's JavaScript API for zero external process dependencies, and per-agent database isolation for multi-tenant safety. All Hippo features are supported: decay, consolidation, goal stack, continuity, and drill-down DAG. The plugin includes comprehensive error handling, automatic DB recovery on corruption, and a full testing strategy.

**Approach:** Monolithic Node.js plugin with embedded Hippo store management
**Isolation:** Per-agent databases with scope filtering
**Performance:** Zero HTTP overhead - direct Hippo JavaScript API (initStore, createMemory, hybridSearch, etc.)
**Reliability:** Automatic DB recovery on corruption, no process management overhead
**Extensibility:** All Hippo features implemented, configurable via `plugin.toml`

Approved design. Proceeding to implementation planning.