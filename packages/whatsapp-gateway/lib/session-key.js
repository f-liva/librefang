'use strict';

// ---------------------------------------------------------------------------
// Session key / channel_type derivation (Phase 05 §B.1, CS-01 strict prefix)
// ---------------------------------------------------------------------------
// Single source of truth for the `channel_type` string forwarded to the
// kernel. The kernel derives its `SessionId` from an opaque hash of
// `channel_type`, so two different strings yield two different sessions.
//
// Before this helper, every WhatsApp chat — DM or group — forwarded
// `channel_type = "whatsapp:<jid>"`. That string already distinguished one
// chat from another (JIDs are unique), but it gave the kernel no structural
// way to tell a DM from a group. Defense-in-depth modules downstream
// (claude_code driver scope, memory recall filter) need that structural
// distinction to apply group-vs-DM policies.
//
// Flag `LIBREFANG_STRICT_CHANNEL_PREFIX=off` reverts to the legacy
// `whatsapp:<jid>` form so the mitigation can be disabled in the field
// without a redeploy. Default ON.
//
// Migration note: NO database migration is performed. Existing kernel
// sessions keyed on `whatsapp:<jid>` remain alive; new post-deploy traffic
// opens new sessions under `whatsapp-dm:<jid>` / `whatsapp-group:<jid>`.
// `channel_bridge` creates sessions on-demand so this is safe.

// Reuse the gateway's own `isGroupJid` helper instead of Baileys' `isJidGroup`.
// The gateway helper is narrower (matches `@g.us` exactly) and already
// validated by the identity.test.js suite — Baileys' implementation is an
// internal of @whiskeysockets and has drifted across versions.
const { isGroupJid } = require('./identity');

const STRICT_PREFIX_ENABLED =
  process.env.LIBREFANG_STRICT_CHANNEL_PREFIX !== 'off';

/**
 * Derive the `channel_type` string for a given WhatsApp chat JID.
 *
 * - DM (`<phone>@s.whatsapp.net`)      → `whatsapp-dm:<jid>`
 * - Group (`<id>@g.us`)                → `whatsapp-group:<jid>`
 * - Empty/undefined (defensive)        → `'whatsapp'` (CS-01 already throws
 *   upstream of every call site, so this branch is unreachable in practice)
 * - Flag off (`LIBREFANG_STRICT_CHANNEL_PREFIX=off`) → `whatsapp:<jid>`
 *   (legacy behavior — single source of truth for the revert path)
 *
 * @param {string} chatJid e.g. `39123@s.whatsapp.net` or `120363abc@g.us`
 * @returns {string}
 */
function channelTypeForChat(chatJid) {
  if (!chatJid) {
    // Unreachable post CS-01 — every forward path throws on empty chatJid
    // before we ever reach here. Kept as a belt-and-braces fallback so the
    // helper is total.
    return 'whatsapp';
  }
  if (!STRICT_PREFIX_ENABLED) {
    return `whatsapp:${chatJid}`;
  }
  return isGroupJid(chatJid)
    ? `whatsapp-group:${chatJid}`
    : `whatsapp-dm:${chatJid}`;
}

/**
 * Boot self-test for the CS-01 invariant + strict prefix wiring.
 *
 * Called once at gateway startup, BEFORE the Baileys socket connects.
 * Emits a structured log on success and crashes the process on regression
 * (the boot-self-test pattern from Phase 3 §B — faster and louder than
 * discovering the regression via cross-chat leak in production).
 *
 * Regressions detected:
 *  - DM JID and group JID produce the same channel_type (CS-01 broken).
 *  - Strict prefix enabled but `whatsapp-dm:` / `whatsapp-group:` not
 *    actually applied (helper returned legacy string despite flag).
 *
 * @returns {{ok: true, dm_type: string, group_type: string} | {ok: false, reason: string}}
 */
function runCs01SelfTest() {
  const dmSample = '111@s.whatsapp.net';
  const groupSample = '120363aaa@g.us';
  const dm = channelTypeForChat(dmSample);
  const group = channelTypeForChat(groupSample);

  if (dm === group) {
    return {
      ok: false,
      reason: `cs01 regression: dm=${dm} group=${group} (same string for DM and group)`,
    };
  }

  if (STRICT_PREFIX_ENABLED) {
    if (!dm.startsWith('whatsapp-dm:') || !group.startsWith('whatsapp-group:')) {
      return {
        ok: false,
        reason: `strict prefix enabled but not applied: dm=${dm} group=${group}`,
      };
    }
  } else {
    if (!dm.startsWith('whatsapp:') || !group.startsWith('whatsapp:')) {
      return {
        ok: false,
        reason: `legacy mode but non-legacy prefix seen: dm=${dm} group=${group}`,
      };
    }
  }

  return { ok: true, dm_type: dm, group_type: group };
}

module.exports = {
  channelTypeForChat,
  runCs01SelfTest,
  STRICT_PREFIX_ENABLED,
};
