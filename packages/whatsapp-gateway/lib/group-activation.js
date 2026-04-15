'use strict';

// ---------------------------------------------------------------------------
// lib/group-activation.js — Phase 5 §A: per-group activation mode persistence.
//
// Pure functional module, same shape as lib/lid-cache.js. The caller owns
// the better-sqlite3 handle; this module only exposes SQL helpers.
//
// Schema:
//   CREATE TABLE group_activation (
//     group_jid   TEXT PRIMARY KEY,                       -- '<digits>@g.us'
//     mode        TEXT NOT NULL CHECK(mode IN ('always','mention','off')),
//     updated_at  INTEGER NOT NULL                        -- unix ms
//   )
//
// Modes:
//   - 'always'  : bot answers every group message (noisy; opt-in only)
//   - 'mention' : bot answers only when @mentioned or addressed by name
//                 (default for groups without a stored row)
//   - 'off'     : bot stays silent in this group regardless of mentions
//
// Functions:
//   - init(db)                 — idempotent CREATE TABLE IF NOT EXISTS
//   - get(db, groupJid)        — returns stored mode or null
//   - set(db, groupJid, mode)  — INSERT OR REPLACE; validates mode
//   - remove(db, groupJid)     — DELETE a row (back to default)
//   - list(db)                 — all rows { group_jid, mode, updated_at }
//
// Validation is strict: `set` throws on unknown modes so callers catch typos
// before they hit the CHECK constraint.
// ---------------------------------------------------------------------------

const MODES = Object.freeze(['always', 'mention', 'off']);
const DEFAULT_MODE = 'mention';

function init(db) {
  db.exec(`
    CREATE TABLE IF NOT EXISTS group_activation (
      group_jid  TEXT PRIMARY KEY,
      mode       TEXT NOT NULL CHECK(mode IN ('always','mention','off')),
      updated_at INTEGER NOT NULL
    );
  `);
}

function get(db, groupJid) {
  if (!groupJid) return null;
  const row = db
    .prepare('SELECT mode FROM group_activation WHERE group_jid = ?')
    .get(groupJid);
  return row ? row.mode : null;
}

function set(db, groupJid, mode) {
  if (!groupJid) throw new Error('group_activation.set: empty groupJid');
  if (!MODES.includes(mode)) {
    throw new Error(`group_activation.set: invalid mode ${JSON.stringify(mode)}`);
  }
  db.prepare(
    'INSERT OR REPLACE INTO group_activation (group_jid, mode, updated_at) VALUES (?, ?, ?)'
  ).run(groupJid, mode, Date.now());
}

function remove(db, groupJid) {
  if (!groupJid) return;
  db.prepare('DELETE FROM group_activation WHERE group_jid = ?').run(groupJid);
}

function list(db) {
  return db
    .prepare('SELECT group_jid, mode, updated_at FROM group_activation ORDER BY updated_at DESC')
    .all();
}

// Parse `/activation [always|mention|off]` from a message body. Returns
// { mode } on a valid command, { query: true } when the owner asked for the
// current mode with a bare `/activation`, or null when the text isn't an
// activation command at all. Case-insensitive; tolerates surrounding
// whitespace and trailing punctuation.
function parseCommand(text) {
  if (typeof text !== 'string') return null;
  const m = text.trim().match(/^\/activation(?:\s+(\S+))?\s*$/i);
  if (!m) return null;
  const arg = (m[1] || '').toLowerCase();
  if (!arg) return { query: true };
  if (!MODES.includes(arg)) return { error: 'invalid_mode', arg };
  return { mode: arg };
}

module.exports = {
  init,
  get,
  set,
  remove,
  list,
  parseCommand,
  MODES,
  DEFAULT_MODE,
};
