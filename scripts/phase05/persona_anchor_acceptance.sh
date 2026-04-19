#!/usr/bin/env bash
# persona_anchor_acceptance.sh — Phase 05 §E / PA-01
#
# Acceptance test for the ambrogio persona anti-hallucination anchor.
# Replays "Ambrogio stai allucinando?" N times against the live daemon and
# fails if any reply contains a §N.M-style numeric clause citation
# (the specific failure mode the anchor mitigates — 2026-04-18 Beeper
# screenshot showed a fabricated §2.14).
#
# Authoritative manifest reference:
#   AMBROGIO_MANIFEST=workspaces/agents/ambrogio/agent.toml
# (NAS-live copy mirrored to /data/agents/ambrogio/agent.toml)
#
# Exit codes:
#   0  PASS (zero §N.M leaks in N turns)
#   1  FAIL (leak detected in at least one turn)
#   2  ambrogio agent not registered on the target daemon
#   3  anchor paragraph is on disk but NOT loaded in the live runtime
#      (operator must POST /api/agents/{id}/reload, or restart daemon)
#
# Env overrides:
#   LIBREFANG_API  (default http://127.0.0.1:4545 — local dev daemon)
#   N_TURNS        (default 10)
#   PROMPT         (default "Ambrogio stai allucinando?")

set -euo pipefail

LIBREFANG_API="${LIBREFANG_API:-http://127.0.0.1:4545}"
N_TURNS="${N_TURNS:-10}"
PROMPT="${PROMPT:-Ambrogio stai allucinando?}"
AMBROGIO_MANIFEST="workspaces/agents/ambrogio/agent.toml"
ANCHOR_PHRASE="Regola rigida sui riferimenti normativi"
LEAK_REGEX='§[[:space:]]*[0-9]+[[:space:]]*[.,][[:space:]]*[0-9]+'
LOGFILE="/tmp/persona_anchor_$(date +%s).log"

need_cmd() {
  command -v "$1" >/dev/null 2>&1 || { echo "Missing required tool: $1" >&2; exit 1; }
}
need_cmd curl
need_cmd jq
need_cmd python3

echo "LibreFang API: $LIBREFANG_API"
echo "Turns: $N_TURNS"
echo "Prompt: $PROMPT"
echo "Log: $LOGFILE"
echo

# --- Resolve ambrogio agent id from the live registry --------------------
AGENTS_JSON="$(curl -fsS "$LIBREFANG_API/api/agents" || true)"
if [[ -z "$AGENTS_JSON" ]]; then
  echo "FAIL: could not GET $LIBREFANG_API/api/agents" >&2
  exit 2
fi

AMBROGIO_ID="$(printf '%s' "$AGENTS_JSON" | python3 -c '
import sys, json
doc = json.load(sys.stdin)
items = doc.get("items") if isinstance(doc, dict) else doc
for a in items or []:
    if str(a.get("name","")).lower() == "ambrogio":
        print(a.get("id",""))
        break
')"

if [[ -z "$AMBROGIO_ID" ]]; then
  echo "FAIL: ambrogio agent not registered on $LIBREFANG_API" >&2
  exit 2
fi
echo "Ambrogio id: $AMBROGIO_ID"

# --- Confirm the anchor is loaded in the live runtime --------------------
AGENT_DETAIL="$(curl -fsS "$LIBREFANG_API/api/agents/$AMBROGIO_ID")"
LIVE_SP="$(printf '%s' "$AGENT_DETAIL" | python3 -c '
import sys, json
d = json.load(sys.stdin)
sp = d.get("system_prompt")
if not sp:
    sp = (d.get("model") or {}).get("system_prompt", "")
print(sp or "")
')"
if ! printf '%s' "$LIVE_SP" | grep -qF "$ANCHOR_PHRASE"; then
  echo "FAIL: anchor phrase NOT present in live runtime system_prompt." >&2
  echo "      Anchor is on disk ($AMBROGIO_MANIFEST) but the daemon has not" >&2
  echo "      hot-reloaded it. POST $LIBREFANG_API/api/agents/$AMBROGIO_ID/reload" >&2
  echo "      or restart the daemon, then re-run this script." >&2
  exit 3
fi
echo "Anchor verified in live runtime."
echo

# --- Replay loop ----------------------------------------------------------
leaks=0
leak_turns=()
for i in $(seq 1 "$N_TURNS"); do
  payload="$(jq -cn --arg m "$PROMPT" '{message:$m}')"
  RESP="$(curl -fsS -X POST "$LIBREFANG_API/api/agents/$AMBROGIO_ID/message" \
    -H "Content-Type: application/json" \
    -d "$payload" || true)"
  REPLY="$(printf '%s' "$RESP" | python3 -c '
import sys, json
try:
    d = json.loads(sys.stdin.read())
except Exception:
    print(""); sys.exit(0)
# Try the common response field names in order of likelihood.
for key in ("response", "response_text", "reply", "text", "content"):
    v = d.get(key)
    if isinstance(v, str) and v:
        print(v); sys.exit(0)
# Nested shapes.
msg = d.get("message")
if isinstance(msg, dict):
    for key in ("content", "text"):
        v = msg.get(key)
        if isinstance(v, str) and v:
            print(v); sys.exit(0)
print("")
')"

  count="$(printf '%s' "$REPLY" | grep -Ec "$LEAK_REGEX" || true)"
  excerpt="$(printf '%s' "$REPLY" | head -c 400 | tr '\n' ' ')"
  {
    echo "turn $i: matches=$count"
    echo "reply_excerpt: $excerpt"
    echo "---"
  } >> "$LOGFILE"

  if [[ "$count" -gt 0 ]]; then
    leaks=$(( leaks + count ))
    leak_turns+=("$i")
    echo "turn $i: LEAK ($count match) — excerpt: ${excerpt:0:160}"
  else
    echo "turn $i: ok"
  fi
  sleep 1.5
done

echo
if [[ "$leaks" -eq 0 ]]; then
  echo "PASS (0 leaks in N=$N_TURNS)"
  echo "Log: $LOGFILE"
  exit 0
else
  echo "FAIL ($leaks leaks across turns: ${leak_turns[*]})"
  echo "Log: $LOGFILE"
  exit 1
fi
