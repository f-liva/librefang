'use strict';

const assert = require('node:assert/strict');
const { describe, it } = require('node:test');
const { spawnSync } = require('node:child_process');
const path = require('node:path');

const MODULE_PATH = path.resolve(__dirname, '..', 'lib', 'output_guard.js');
const { classifyOutput } = require(MODULE_PATH);

// ---------------------------------------------------------------------------
// CAPS-free proof (Q4 Signore push-back anchor — these MUST all trigger).
// Real-world hallucinated tokens observed in the 2026-04-18 Beeper screenshot
// are MIXED-CASE, not all-caps. If any of these three fail, the whole
// classifier is wrong.
// ---------------------------------------------------------------------------
describe('output_guard — CAPS-free regression proof (Q4 anchor)', () => {
  it('CAPS-1: [Response interrupted by user] → toxic bracket_token_solo', () => {
    const r = classifyOutput('[Response interrupted by user]');
    assert.equal(r.verdict, 'toxic');
    assert.equal(r.reason, 'bracket_token_solo');
  });

  it('CAPS-2: [User] → toxic bracket_token_solo (short mixed-case token)', () => {
    const r = classifyOutput('[User]');
    assert.equal(r.verdict, 'toxic');
    assert.equal(r.reason, 'bracket_token_solo');
  });

  it('CAPS-3: [EMPTY_OR_NO_REPLY_FROM_PREVIOUS_TURN] → toxic (screenshot string)', () => {
    const r = classifyOutput('[EMPTY_OR_NO_REPLY_FROM_PREVIOUS_TURN]');
    assert.equal(r.verdict, 'toxic');
    assert.equal(r.reason, 'bracket_token_solo');
  });
});

// ---------------------------------------------------------------------------
// Toxic — first-line markup-only or full-message structural.
// ---------------------------------------------------------------------------
describe('output_guard — toxic (first-line markup only)', () => {
  it('TOXIC-1: "## Sender" → markdown_header_solo', () => {
    const r = classifyOutput('## Sender');
    assert.equal(r.verdict, 'toxic');
    assert.equal(r.reason, 'markdown_header_solo');
  });

  it('TOXIC-2: JSON object open → json_object_open', () => {
    const r = classifyOutput('{"role": "assistant"');
    assert.equal(r.verdict, 'toxic');
    assert.equal(r.reason, 'json_object_open');
  });

  it('TOXIC-3: "<system>" → xml_tag_open', () => {
    const r = classifyOutput('<system>');
    assert.equal(r.verdict, 'toxic');
    assert.equal(r.reason, 'xml_tag_open');
  });

  it('TOXIC-4: "[foo]" → toxic bracket token', () => {
    const r = classifyOutput('[foo]');
    assert.equal(r.verdict, 'toxic');
    assert.equal(r.reason, 'bracket_token_solo');
  });

  it('TOXIC-5: "  ##  Note  " → toxic whitespace-tolerant markdown header', () => {
    const r = classifyOutput('  ##  Note  ');
    assert.equal(r.verdict, 'toxic');
    assert.equal(r.reason, 'markdown_header_solo');
  });

  it('TOXIC-6: multi-line all-structural (bracket + xml, no prose) → toxic cascade', () => {
    const r = classifyOutput('[foo]\n<bar>');
    assert.equal(r.verdict, 'toxic');
    // Two stacked structural leaders with no prose line — cascade shape
    // (regurgitation signature), reason prefixed cascade_.
    assert.ok(r.reason.startsWith('cascade_'), `expected cascade_ prefix, got ${r.reason}`);
  });
});

// ---------------------------------------------------------------------------
// Cascade — two+ stacked structural leaders → toxic full (NOT stripped).
// This is the 2026-04-19 Ambrogio group regurgitation signature.
// ---------------------------------------------------------------------------
describe('output_guard — cascade regurgitation (2+ leaders → toxic full)', () => {
  it('CASCADE-1: [EMPTY_...] + ## Response Style + bullets → toxic cascade', () => {
    const text = '[EMPTY_OR_NO_REPLY_FROM_PREVIOUS_TURN]\n\n## Response Style\n- Be concise and direct.\n- Use plain language.';
    const r = classifyOutput(text);
    assert.equal(r.verdict, 'toxic');
    assert.ok(r.reason.startsWith('cascade_'), `expected cascade_ prefix, got ${r.reason}`);
    assert.ok(r.reason.includes('bracket_token_solo'), `reason should mention bracket: ${r.reason}`);
    assert.ok(r.reason.includes('markdown_header_solo'), `reason should mention header: ${r.reason}`);
    // MUST NOT return stripped — cascade is full-suppress, not per-line strip.
    assert.equal(r.stripped, undefined);
  });

  it('CASCADE-2: bracket + bracket + prose → toxic cascade', () => {
    const r = classifyOutput('[foo]\n[bar]\n\nCiao Signore.');
    assert.equal(r.verdict, 'toxic');
    assert.ok(r.reason.startsWith('cascade_'));
    assert.equal(r.stripped, undefined);
  });

  it('CASCADE-3: header + JSON + prose → toxic cascade', () => {
    const r = classifyOutput('## Note\n{"foo": 1}\n\nHere is the answer.');
    assert.equal(r.verdict, 'toxic');
    assert.ok(r.reason.startsWith('cascade_'));
  });
});

// ---------------------------------------------------------------------------
// Suspicious — exactly ONE structural leader + prose after → strip + ship.
// ---------------------------------------------------------------------------
describe('output_guard — suspicious (single leader, strip first line)', () => {
  it('SUSP-1: "## Sender\\nMessage...\\n\\nCiao!" → suspicious stripped', () => {
    const r = classifyOutput('## Sender\nMessage from: Federico\n\nCiao!');
    assert.equal(r.verdict, 'suspicious');
    assert.equal(r.stripped, 'Message from: Federico\n\nCiao!');
    assert.equal(r.reason, 'markdown_header_solo_with_prose');
  });

  it('SUSP-2: "[Response interrupted by user]\\n\\nSì Signore..." → stripped prose', () => {
    const r = classifyOutput('[Response interrupted by user]\n\nSì Signore, eccomi.');
    assert.equal(r.verdict, 'suspicious');
    assert.equal(r.stripped, 'Sì Signore, eccomi.');
    assert.equal(r.reason, 'bracket_token_solo_with_prose');
  });

  it('SUSP-3: JSON + prose → suspicious stripped', () => {
    const r = classifyOutput('{"foo": 1}\n\nHere is the answer.');
    assert.equal(r.verdict, 'suspicious');
    assert.ok(r.stripped.includes('Here is the answer.'));
    assert.equal(r.reason, 'json_object_open_with_prose');
  });
});

// ---------------------------------------------------------------------------
// Structural first-char fallback — char in {[<#§*>|\\, not a full pattern
// match. Catches fabricated rule citations like §N.M.
// ---------------------------------------------------------------------------
describe('output_guard — structural first-char fallback', () => {
  it('STRUCT-1: "§2.14 mi ha prescritto silenzio" → toxic (hallucinated rule)', () => {
    const r = classifyOutput('§2.14 mi ha prescritto silenzio');
    assert.equal(r.verdict, 'toxic');
    // Single-line §-prefix with no prose on another line — the whole message
    // is a structural leader. Reason is the first-line category so the log
    // is informative (NOT "cascade_", which requires 2+ leaders).
    assert.equal(r.reason, 'structural_first_char');
  });

  it('STRUCT-1b: "§2.14 rule\\nprose follows" → suspicious with prose', () => {
    const r = classifyOutput('§2.14 rule\nprose follows');
    assert.equal(r.verdict, 'suspicious');
    assert.equal(r.stripped, 'prose follows');
    assert.equal(r.reason, 'structural_first_char_with_prose');
  });

  it('STRUCT-2: "> quoted block\\nbody" → suspicious', () => {
    const r = classifyOutput('> quoted block\nbody');
    assert.equal(r.verdict, 'suspicious');
    assert.equal(r.stripped, 'body');
  });

  it('STRUCT-3: "*important*\\nbody" → suspicious (first char *)', () => {
    const r = classifyOutput('*important*\nbody');
    assert.equal(r.verdict, 'suspicious');
    assert.equal(r.stripped, 'body');
  });
});

// ---------------------------------------------------------------------------
// OK — natural prose.
// ---------------------------------------------------------------------------
describe('output_guard — ok (natural prose)', () => {
  it('OK-1: "Sì Signore, eccomi." → ok', () => {
    const r = classifyOutput('Sì Signore, eccomi.');
    assert.equal(r.verdict, 'ok');
    assert.equal(r.reason, 'prose');
  });

  it('OK-2: "Certo, Le confermo il volo." → ok', () => {
    assert.equal(classifyOutput('Certo, Le confermo il volo.').verdict, 'ok');
  });

  it('OK-3: emoji-prefixed prose → ok', () => {
    const r = classifyOutput('🎩 Sì, arrivo.');
    assert.equal(r.verdict, 'ok');
  });

  it('OK-4: numbered list "1. prima\\n2. seconda" → ok (digit first char)', () => {
    const r = classifyOutput('1. prima voce\n2. seconda');
    assert.equal(r.verdict, 'ok');
  });

  it('OK-5: multi-paragraph prose → ok', () => {
    const r = classifyOutput('Ho verificato il volo.\n\nConferma ricevuta per AF1234.');
    assert.equal(r.verdict, 'ok');
    assert.equal(r.reason, 'prose');
  });
});

// ---------------------------------------------------------------------------
// Edge cases.
// ---------------------------------------------------------------------------
describe('output_guard — edge cases', () => {
  it('EDGE-1: empty string → ok empty', () => {
    const r = classifyOutput('');
    assert.equal(r.verdict, 'ok');
    assert.equal(r.reason, 'empty');
  });

  it('EDGE-2: null → ok empty', () => {
    const r = classifyOutput(null);
    assert.equal(r.verdict, 'ok');
    assert.equal(r.reason, 'empty');
  });

  it('EDGE-3: undefined → ok empty', () => {
    const r = classifyOutput(undefined);
    assert.equal(r.verdict, 'ok');
    assert.equal(r.reason, 'empty');
  });

  it('EDGE-4: whitespace-only → ok whitespace_only', () => {
    const r = classifyOutput('   \n\t  ');
    assert.equal(r.verdict, 'ok');
    assert.equal(r.reason, 'whitespace_only');
  });

  it('EDGE-5: number input → ok empty (non-string safe)', () => {
    const r = classifyOutput(42);
    assert.equal(r.verdict, 'ok');
    assert.equal(r.reason, 'empty');
  });

  it('EDGE-6: object input → ok empty', () => {
    const r = classifyOutput({ foo: 'bar' });
    assert.equal(r.verdict, 'ok');
    assert.equal(r.reason, 'empty');
  });
});

// ---------------------------------------------------------------------------
// Flag off — LIBREFANG_OUTPUT_GUARD=off short-circuits to ok.
// Classifier captures the flag at module-load time, so we spawn a fresh
// Node process with the env mutation rather than fight require-cache.
// ---------------------------------------------------------------------------
describe('output_guard — LIBREFANG_OUTPUT_GUARD=off bypass', () => {
  it('FLAG-1: disabled flag → ok reason=disabled even for toxic input', () => {
    const probe = `
      const { classifyOutput, OUTPUT_GUARD_ENABLED } = require(${JSON.stringify(MODULE_PATH)});
      const inputs = ['## Sender', '[User]', '{"role": "x"', '<system>', '[EMPTY_OR_NO_REPLY_FROM_PREVIOUS_TURN]'];
      const out = { OUTPUT_GUARD_ENABLED, results: inputs.map((t) => classifyOutput(t)) };
      process.stdout.write(JSON.stringify(out));
    `;
    const res = spawnSync(process.execPath, ['-e', probe], {
      env: { ...process.env, LIBREFANG_OUTPUT_GUARD: 'off' },
      encoding: 'utf8',
    });
    assert.equal(res.status, 0, `spawn failed: ${res.stderr}`);
    const parsed = JSON.parse(res.stdout);
    assert.equal(parsed.OUTPUT_GUARD_ENABLED, false);
    for (const r of parsed.results) {
      assert.equal(r.verdict, 'ok', `expected ok when disabled, got ${JSON.stringify(r)}`);
      assert.equal(r.reason, 'disabled');
    }
  });

  it('FLAG-2: explicit "no" also disables', () => {
    const probe = `
      const { classifyOutput, OUTPUT_GUARD_ENABLED } = require(${JSON.stringify(MODULE_PATH)});
      process.stdout.write(JSON.stringify({ flag: OUTPUT_GUARD_ENABLED, r: classifyOutput('## toxic') }));
    `;
    const res = spawnSync(process.execPath, ['-e', probe], {
      env: { ...process.env, LIBREFANG_OUTPUT_GUARD: 'no' },
      encoding: 'utf8',
    });
    assert.equal(res.status, 0, `spawn failed: ${res.stderr}`);
    const parsed = JSON.parse(res.stdout);
    assert.equal(parsed.flag, false);
    assert.equal(parsed.r.verdict, 'ok');
    assert.equal(parsed.r.reason, 'disabled');
  });

  it('FLAG-3: default env (unset) keeps classifier active', () => {
    const probe = `
      const { classifyOutput, OUTPUT_GUARD_ENABLED } = require(${JSON.stringify(MODULE_PATH)});
      process.stdout.write(JSON.stringify({ flag: OUTPUT_GUARD_ENABLED, r: classifyOutput('## toxic') }));
    `;
    const cleanEnv = { ...process.env };
    delete cleanEnv.LIBREFANG_OUTPUT_GUARD;
    const res = spawnSync(process.execPath, ['-e', probe], {
      env: cleanEnv,
      encoding: 'utf8',
    });
    assert.equal(res.status, 0, `spawn failed: ${res.stderr}`);
    const parsed = JSON.parse(res.stdout);
    assert.equal(parsed.flag, true);
    assert.equal(parsed.r.verdict, 'toxic');
  });
});
