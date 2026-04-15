'use strict';

// ---------------------------------------------------------------------------
// lib/pending-group-history.js — Phase 3 completion: history-for-skipped.
//
// When the gateway silently skips a group message (mode=off, mention
// required but bot wasn't addressed, allowlist miss), the agent loses
// visibility of what happened in that group. When the bot is eventually
// activated for that group, it replies blind to a conversation it couldn't
// observe.
//
// This module buffers the skipped messages in SQLite keyed by group, with
// bounded retention. At forward-time we drain the buffer for the target
// group and prepend the entries as a "[Group context]" preamble so the
// agent sees what was said since its last response.
//
// Schema:
//   CREATE TABLE pending_group_history (
//     id          INTEGER PRIMARY KEY AUTOINCREMENT,
//     group_jid   TEXT NOT NULL,
//     sender_name TEXT,
//     sender_jid  TEXT,
//     text        TEXT,
//     skip_reason TEXT,
//     timestamp   INTEGER NOT NULL    -- unix ms
//   )
//
// Retention: bounded per-group (DEFAULT_MAX_PER_GROUP, oldest evicted on
// append) and globally by age (DEFAULT_MAX_AGE_MS, pruned by the caller
// via `pruneOlderThan`).
// ---------------------------------------------------------------------------

const DEFAULT_MAX_PER_GROUP = 50;
const DEFAULT_MAX_AGE_MS = 24 * 60 * 60 * 1000; // 24 h

function init(db) {
  db.exec(`
    CREATE TABLE IF NOT EXISTS pending_group_history (
      id          INTEGER PRIMARY KEY AUTOINCREMENT,
      group_jid   TEXT NOT NULL,
      sender_name TEXT,
      sender_jid  TEXT,
      text        TEXT,
      skip_reason TEXT,
      timestamp   INTEGER NOT NULL
    );
    CREATE INDEX IF NOT EXISTS idx_pending_group_history_group
      ON pending_group_history(group_jid, id);
    CREATE INDEX IF NOT EXISTS idx_pending_group_history_timestamp
      ON pending_group_history(timestamp);
  `);
}

function append(db, entry, opts) {
  if (!entry || !entry.group_jid) return;
  // Empty text or media-only messages: still record a placeholder so the
  // agent sees "[media]" activity happened.
  const maxPerGroup = (opts && typeof opts.maxPerGroup === 'number')
    ? opts.maxPerGroup
    : DEFAULT_MAX_PER_GROUP;

  db.prepare(`
    INSERT INTO pending_group_history
      (group_jid, sender_name, sender_jid, text, skip_reason, timestamp)
    VALUES (?, ?, ?, ?, ?, ?)
  `).run(
    entry.group_jid,
    entry.sender_name || null,
    entry.sender_jid || null,
    entry.text || '',
    entry.skip_reason || null,
    Number(entry.timestamp) || Date.now(),
  );

  // Evict the oldest rows beyond the cap, per group. Keeps the buffer
  // bounded even if a very chatty group never triggers a forward.
  db.prepare(`
    DELETE FROM pending_group_history
    WHERE id IN (
      SELECT id FROM pending_group_history
      WHERE group_jid = ?
      ORDER BY id DESC
      LIMIT -1 OFFSET ?
    )
  `).run(entry.group_jid, maxPerGroup);
}

function peek(db, groupJid, limit) {
  if (!groupJid) return [];
  const lim = typeof limit === 'number' && limit > 0 ? limit : DEFAULT_MAX_PER_GROUP;
  return db
    .prepare(`
      SELECT id, sender_name, sender_jid, text, skip_reason, timestamp
      FROM pending_group_history
      WHERE group_jid = ?
      ORDER BY id ASC
      LIMIT ?
    `)
    .all(groupJid, lim);
}

function drain(db, groupJid, limit) {
  if (!groupJid) return [];
  const rows = peek(db, groupJid, limit);
  if (rows.length === 0) return [];
  const ids = rows.map((r) => r.id);
  // better-sqlite3 doesn't bind arrays; template the placeholders.
  const placeholders = ids.map(() => '?').join(',');
  db.prepare(`DELETE FROM pending_group_history WHERE id IN (${placeholders})`).run(...ids);
  return rows;
}

function count(db, groupJid) {
  if (!groupJid) {
    const row = db.prepare('SELECT COUNT(*) AS c FROM pending_group_history').get();
    return row ? row.c : 0;
  }
  const row = db
    .prepare('SELECT COUNT(*) AS c FROM pending_group_history WHERE group_jid = ?')
    .get(groupJid);
  return row ? row.c : 0;
}

function pruneOlderThan(db, maxAgeMs) {
  const age = typeof maxAgeMs === 'number' && maxAgeMs > 0 ? maxAgeMs : DEFAULT_MAX_AGE_MS;
  const cutoff = Date.now() - age;
  const r = db
    .prepare('DELETE FROM pending_group_history WHERE timestamp < ?')
    .run(cutoff);
  return r.changes || 0;
}

// Build a plain-text preamble for the agent describing the skipped activity.
// Returns an empty string when the list is empty so the caller can safely
// concatenate unconditionally.
function formatPreamble(entries) {
  if (!Array.isArray(entries) || entries.length === 0) return '';
  const lines = ['[GROUP_CONTEXT — messages received while the bot was silent]'];
  for (const e of entries) {
    const name = e.sender_name || e.sender_jid || 'unknown';
    const body = (e.text && e.text.trim()) || '[no text]';
    lines.push(`• ${name}: ${body}`);
  }
  lines.push('[/GROUP_CONTEXT]');
  return lines.join('\n') + '\n\n';
}

module.exports = {
  init,
  append,
  peek,
  drain,
  count,
  pruneOlderThan,
  formatPreamble,
  DEFAULT_MAX_PER_GROUP,
  DEFAULT_MAX_AGE_MS,
};
