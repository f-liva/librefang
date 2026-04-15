'use strict';

// ---------------------------------------------------------------------------
// test/pending-group-history.test.js — Phase 3 completion (GA-03) unit tests.
// ---------------------------------------------------------------------------

const assert = require('node:assert/strict');
const { describe, it } = require('node:test');
const Database = require('better-sqlite3');

const pgh = require('../lib/pending-group-history');

const G1 = '120363100000000001@g.us';
const G2 = '120363100000000002@g.us';

function freshDb() {
  const db = new Database(':memory:');
  pgh.init(db);
  return db;
}

describe('pending-group-history', () => {
  describe('init', () => {
    it('creates the pending_group_history table', () => {
      const db = freshDb();
      const row = db
        .prepare("SELECT name FROM sqlite_master WHERE type='table' AND name='pending_group_history'")
        .get();
      assert.equal(row?.name, 'pending_group_history');
    });

    it('is idempotent', () => {
      const db = freshDb();
      assert.doesNotThrow(() => pgh.init(db));
    });
  });

  describe('append / peek / drain', () => {
    it('append records a row recoverable via peek', () => {
      const db = freshDb();
      pgh.append(db, {
        group_jid: G1,
        sender_name: 'Alice',
        sender_jid: '391230000001@s.whatsapp.net',
        text: 'hello',
        skip_reason: 'mention_required',
        timestamp: 1000,
      });
      const rows = pgh.peek(db, G1);
      assert.equal(rows.length, 1);
      assert.equal(rows[0].sender_name, 'Alice');
      assert.equal(rows[0].text, 'hello');
      assert.equal(rows[0].skip_reason, 'mention_required');
    });

    it('peek returns rows in ascending insertion order', () => {
      const db = freshDb();
      pgh.append(db, { group_jid: G1, sender_name: 'A', text: 'first' });
      pgh.append(db, { group_jid: G1, sender_name: 'B', text: 'second' });
      pgh.append(db, { group_jid: G1, sender_name: 'C', text: 'third' });
      const rows = pgh.peek(db, G1);
      assert.deepEqual(rows.map((r) => r.text), ['first', 'second', 'third']);
    });

    it('drain empties the group and returns the rows', () => {
      const db = freshDb();
      pgh.append(db, { group_jid: G1, text: 'a' });
      pgh.append(db, { group_jid: G1, text: 'b' });
      const drained = pgh.drain(db, G1);
      assert.equal(drained.length, 2);
      assert.equal(pgh.count(db, G1), 0);
    });

    it('drain of a group does not touch other groups', () => {
      const db = freshDb();
      pgh.append(db, { group_jid: G1, text: 'g1' });
      pgh.append(db, { group_jid: G2, text: 'g2' });
      pgh.drain(db, G1);
      assert.equal(pgh.count(db, G1), 0);
      assert.equal(pgh.count(db, G2), 1);
    });

    it('drain on empty group returns []', () => {
      const db = freshDb();
      assert.deepEqual(pgh.drain(db, G1), []);
    });

    it('append skipped when group_jid is missing', () => {
      const db = freshDb();
      pgh.append(db, { text: 'orphan' });
      assert.equal(pgh.count(db), 0);
    });
  });

  describe('per-group retention cap', () => {
    it('evicts oldest rows when above the cap', () => {
      const db = freshDb();
      const cap = 3;
      for (let i = 0; i < 5; i++) {
        pgh.append(db, { group_jid: G1, text: `m${i}` }, { maxPerGroup: cap });
      }
      const rows = pgh.peek(db, G1);
      assert.equal(rows.length, cap);
      // Oldest (m0, m1) should have been evicted; m2..m4 survive.
      assert.deepEqual(rows.map((r) => r.text), ['m2', 'm3', 'm4']);
    });
  });

  describe('pruneOlderThan', () => {
    it('deletes rows older than the cutoff', () => {
      const db = freshDb();
      const old = Date.now() - 7 * 24 * 60 * 60 * 1000; // 7 days ago
      pgh.append(db, { group_jid: G1, text: 'old', timestamp: old });
      pgh.append(db, { group_jid: G1, text: 'recent' });
      const removed = pgh.pruneOlderThan(db, 24 * 60 * 60 * 1000);
      assert.equal(removed, 1);
      assert.deepEqual(pgh.peek(db, G1).map((r) => r.text), ['recent']);
    });
  });

  describe('formatPreamble', () => {
    it('returns empty string for empty input', () => {
      assert.equal(pgh.formatPreamble([]), '');
      assert.equal(pgh.formatPreamble(null), '');
      assert.equal(pgh.formatPreamble(undefined), '');
    });

    it('formats entries into a GROUP_CONTEXT block', () => {
      const out = pgh.formatPreamble([
        { sender_name: 'Alice', text: 'hi' },
        { sender_name: 'Bob',   text: 'how are you' },
      ]);
      assert.ok(out.includes('[GROUP_CONTEXT'));
      assert.ok(out.includes('Alice: hi'));
      assert.ok(out.includes('Bob: how are you'));
      assert.ok(out.includes('[/GROUP_CONTEXT]'));
      assert.ok(out.endsWith('\n\n'));
    });

    it('falls back to [no text] for empty bodies', () => {
      const out = pgh.formatPreamble([{ sender_name: 'Ghost', text: '' }]);
      assert.ok(out.includes('Ghost: [no text]'));
    });
  });
});
