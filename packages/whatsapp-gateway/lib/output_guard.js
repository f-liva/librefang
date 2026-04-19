'use strict';

/**
 * OutputGuard — gateway-side hallucination classifier (OB-05, Phase 5 §A).
 *
 * Blocks or strips model-generated meta-text (markdown headers, bracket tokens,
 * JSON/XML openers, fabricated rule citations) BEFORE it reaches WhatsApp.
 *
 * Decisions (see .planning/phases/05-wa-group-output-boundary/05-CONTEXT.md §A):
 * - Categorial first-char + structural shape detection, zero CAPS dependency
 *   (Q4 locked — Signore explicit push-back on CAPS matching because real
 *   hallucinated tokens like `[Response interrupted by user]` and `[User]` are
 *   mixed-case, not all-caps).
 * - Cascade-aware: two or more structural leaders stacked before any prose
 *   line is the signature of a system-prompt regurgitation (evidence
 *   2026-04-19 Ambrogio group leak: `[EMPTY_OR_NO_REPLY_FROM_PREVIOUS_TURN]`
 *   + `## Response Style` + 6 bullets of Claude Code CLI defaults). In that
 *   case the full message is suppressed — NOT stripped per line — because
 *   each leader is its own leak category.
 * - `LIBREFANG_OUTPUT_GUARD=off` short-circuits to `{verdict:'ok',
 *   reason:'disabled'}` for instant revert without redeploy.
 *
 * API:
 *   classifyOutput(text) -> { verdict, reason, stripped? }
 *   verdict is one of: 'ok' | 'suspicious' | 'toxic'
 *   - 'ok'         : natural prose, ship as-is
 *   - 'suspicious' : exactly one structural leader + prose after; `stripped`
 *                    contains the text with the leader line removed
 *   - 'toxic'      : two+ stacked leaders (cascade) OR full-message markup
 *                    OR no prose at all; suppress / delete
 */

const OUTPUT_GUARD_ENABLED = !['off', '0', 'false', 'no'].includes(
  String(process.env.LIBREFANG_OUTPUT_GUARD || '').toLowerCase(),
);

// First-line markup-only patterns (CAPS-free — case-insensitive where alpha).
// Each regex tests a single line in isolation.
const TOXIC_FIRST_LINE_PATTERNS = [
  { re: /^\s*##\s+\S/, reason: 'markdown_header_solo' },          // "## Sender", "## Note" alone
  { re: /^\s*\[[^\]]{2,}\]\s*$/, reason: 'bracket_token_solo' },  // "[anything here]" alone on line
  { re: /^\s*\{["']/, reason: 'json_object_open' },               // {"role": ...
  { re: /^\s*<[a-zA-Z]+[>\s/]/, reason: 'xml_tag_open' },         // <tag ...> or <tag/>
];

// First-char structural set — if a line starts with one of these chars after
// trimming, treat it as a structural leader. The cascade scan walks lines
// from the top and counts leaders until the first prose line.
const STRUCTURAL_FIRST_CHARS = new Set(['[', '{', '<', '#', '§', '*', '>', '|', '\\']);

function isStructuralLeader(line) {
  for (const { re } of TOXIC_FIRST_LINE_PATTERNS) {
    if (re.test(line)) return true;
  }
  const firstChar = line.trimStart()[0];
  if (!firstChar) return false;
  return STRUCTURAL_FIRST_CHARS.has(firstChar);
}

function firstLineReason(line) {
  for (const { re, reason } of TOXIC_FIRST_LINE_PATTERNS) {
    if (re.test(line)) return reason;
  }
  return 'structural_first_char';
}

function classifyOutput(text) {
  if (!OUTPUT_GUARD_ENABLED) {
    return { verdict: 'ok', reason: 'disabled' };
  }
  if (!text || typeof text !== 'string') {
    return { verdict: 'ok', reason: 'empty' };
  }
  const trimmed = text.trim();
  if (!trimmed) return { verdict: 'ok', reason: 'whitespace_only' };

  // Cascade scan: walk lines from the top counting structural leaders until
  // the first prose line. Two or more structural leaders before any prose is
  // the signature of a system-prompt regurgitation leak and MUST be
  // suppressed in full — each leader is its own category of tech content the
  // operator does not want in chat.
  const lines = trimmed.split('\n');
  let structuralCount = 0;
  let firstProseIdx = -1;
  const reasons = [];

  for (let i = 0; i < lines.length; i++) {
    const line = lines[i];
    if (line.trim().length === 0) continue; // skip blank separators
    if (isStructuralLeader(line)) {
      structuralCount++;
      reasons.push(firstLineReason(line));
      continue;
    }
    firstProseIdx = i;
    break;
  }

  if (firstProseIdx === -1) {
    // Every non-blank line was structural — full-message leak.
    // - Single structural line with no prose → toxic with the specific
    //   first-line reason (e.g. bracket_token_solo) so downstream log +
    //   metrics stay informative.
    // - Two or more stacked leaders → cascade regurgitation (2026-04-19
    //   Ambrogio group signature); full suppress with cascade_ reason.
    if (structuralCount === 1) {
      return { verdict: 'toxic', reason: reasons[0] };
    }
    if (structuralCount >= 2) {
      return { verdict: 'toxic', reason: `cascade_${reasons.join('_then_')}` };
    }
    return { verdict: 'toxic', reason: 'all_structural_or_empty' };
  }

  if (structuralCount >= 2) {
    // Two or more stacked structural leaders → system-prompt regurgitation
    // (evidence: 2026-04-19 Ambrogio group leak `[EMPTY_...]` + `## Response
    // Style` + bullets). Bracket + markdown header is 2 leaders, toxic full.
    return { verdict: 'toxic', reason: `cascade_${reasons.join('_then_')}` };
  }

  if (structuralCount === 1) {
    // Single structural leader + prose after → suspicious, strip the leader
    // and ship the prose tail. Preserve internal blank-line formatting from
    // the prose start onward, but drop any leading whitespace.
    const stripped = lines.slice(firstProseIdx).join('\n').replace(/^\s+/, '');
    return {
      verdict: 'suspicious',
      reason: `${reasons[0]}_with_prose`,
      stripped,
    };
  }

  return { verdict: 'ok', reason: 'prose' };
}

module.exports = {
  classifyOutput,
  OUTPUT_GUARD_ENABLED,
  // Exposed for tests only:
  TOXIC_FIRST_LINE_PATTERNS,
  STRUCTURAL_FIRST_CHARS,
};
