#!/usr/bin/env bash
# Sweep placeholder-leak rows from the LibreFang episodic memory bank.
#
# Background: the kernel formerly stored interactions as
#   "User asked: <user_message>\nI responded: <agent_response>"
# When the model occasionally emitted a placeholder ("<empty>", "<response>",
# "<silent>", "<no_reply>") instead of a real reply, the kernel persisted
# rows like
#   "User asked: ...\nI responded: <empty>"
# These rows are then retrieved by `proactive_memory::auto_retrieve` and
# rendered as bullet items in the system prompt's Memory section, which
# (a) wastes context budget and (b) trains the model to imitate the
# placeholder pattern on subsequent turns.
#
# The behavioural fix lives in the runtime (gate empty/silent responses out
# of `remember_interaction_best_effort`, mandate `NO_REPLY` only as the
# silent sentinel, defensive output guard for system-prompt regurgitation).
# Existing rows still need a one-shot cleanup — that's what this script does.
#
# Usage:
#   sweep_placeholder_memory.sh [--apply] [--db <path>]
#
# Default mode is dry-run: prints how many rows MATCH but does not modify the
# database. Pass --apply to soft-delete (memories.deleted = 1) the matching
# rows. The matching predicate is intentionally narrow:
#
#   * scope = 'episodic'
#   * deleted = 0
#   * content matches one of the known placeholder leak shapes
#
# The default --db is `/data/librefang.db` (the path used by the LibreFang
# Docker container under Lazycat NAS). On a host filesystem you can pass
# the explicit path. Inside a container without sqlite3 installed, run from
# the host using the /proc/<PID>/root/ pivot:
#
#   PID=$(lzc-docker inspect cloudlazycatapplibrefang-librefang-1 \
#         --format '{{.State.Pid}}')
#   sqlite3 /proc/$PID/root/data/librefang.db < <( ... )
#
# Idempotent: re-running after --apply finds zero matches.

set -euo pipefail

DB="/data/librefang.db"
APPLY=0

while [[ $# -gt 0 ]]; do
    case "$1" in
        --apply)  APPLY=1; shift ;;
        --db)     DB="$2"; shift 2 ;;
        -h|--help)
            sed -n '2,40p' "$0" | sed 's/^# \{0,1\}//'
            exit 0
            ;;
        *)
            echo "unknown arg: $1" >&2
            exit 2
            ;;
    esac
done

if ! command -v sqlite3 >/dev/null 2>&1; then
    echo "sqlite3 not found in PATH" >&2
    exit 3
fi
if [[ ! -f "$DB" ]]; then
    echo "database not found: $DB" >&2
    exit 4
fi

# Predicate is one-line: easier to inspect, no SQL injection surface
# (no user-supplied input ever interpolated).
PREDICATE="
    scope = 'episodic'
    AND deleted = 0
    AND (
        content LIKE '%I responded: <empty>%'
        OR content LIKE '%I responded: <response>%'
        OR content LIKE '%I responded: <silent>%'
        OR content LIKE '%I responded: <no_reply>%'
        OR content LIKE '%I responded: <answer>%'
        OR content LIKE '%</answer>%'
        OR content LIKE '%</response>%'
    )
"

MATCH_COUNT=$(sqlite3 "$DB" "SELECT COUNT(*) FROM memories WHERE ${PREDICATE};")
TOTAL_EPISODIC=$(sqlite3 "$DB" "SELECT COUNT(*) FROM memories WHERE scope = 'episodic' AND deleted = 0;")

echo "Database:           $DB"
echo "Episodic memories:  $TOTAL_EPISODIC"
echo "Placeholder leaks:  $MATCH_COUNT"

if [[ "$APPLY" -eq 0 ]]; then
    echo
    echo "(dry-run — no changes written. Re-run with --apply to soft-delete.)"
    exit 0
fi

if [[ "$MATCH_COUNT" -eq 0 ]]; then
    echo "Nothing to do."
    exit 0
fi

echo
echo "Applying soft-delete to $MATCH_COUNT rows..."
sqlite3 "$DB" "UPDATE memories SET deleted = 1 WHERE ${PREDICATE};"
REMAINING=$(sqlite3 "$DB" "SELECT COUNT(*) FROM memories WHERE ${PREDICATE};")
echo "Remaining matching rows: $REMAINING (expected: 0)"
if [[ "$REMAINING" -ne 0 ]]; then
    echo "WARN: some rows still match — re-inspect predicate." >&2
    exit 5
fi
echo "Done."
