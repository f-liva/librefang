'use strict';

const assert = require('node:assert/strict');
const { describe, it, beforeEach, afterEach } = require('node:test');

// The helper reads `process.env.LIBREFANG_STRICT_CHANNEL_PREFIX` at module
// load, so every test that flips the flag must reload the module via
// `delete require.cache[require.resolve(...)]`. `helperPath` is constant
// across tests.
const helperPath = require.resolve('../lib/session-key');

function loadHelperWithFlag(flagValue) {
  delete require.cache[helperPath];
  if (flagValue === undefined) {
    delete process.env.LIBREFANG_STRICT_CHANNEL_PREFIX;
  } else {
    process.env.LIBREFANG_STRICT_CHANNEL_PREFIX = flagValue;
  }
  return require('../lib/session-key');
}

describe('channelTypeForChat (strict prefix enabled — default)', () => {
  let origFlag;
  beforeEach(() => {
    origFlag = process.env.LIBREFANG_STRICT_CHANNEL_PREFIX;
  });
  afterEach(() => {
    if (origFlag === undefined) {
      delete process.env.LIBREFANG_STRICT_CHANNEL_PREFIX;
    } else {
      process.env.LIBREFANG_STRICT_CHANNEL_PREFIX = origFlag;
    }
  });

  it('Test 1: DM JID produces whatsapp-dm:<jid>', () => {
    const { channelTypeForChat } = loadHelperWithFlag(undefined);
    assert.equal(
      channelTypeForChat('391234@s.whatsapp.net'),
      'whatsapp-dm:391234@s.whatsapp.net'
    );
  });

  it('Test 2: group JID produces whatsapp-group:<jid>', () => {
    const { channelTypeForChat } = loadHelperWithFlag(undefined);
    assert.equal(
      channelTypeForChat('120363abc@g.us'),
      'whatsapp-group:120363abc@g.us'
    );
  });

  it('Test 3: empty / undefined returns legacy "whatsapp" defensively', () => {
    const { channelTypeForChat } = loadHelperWithFlag(undefined);
    assert.equal(channelTypeForChat(''), 'whatsapp');
    assert.equal(channelTypeForChat(undefined), 'whatsapp');
    assert.equal(channelTypeForChat(null), 'whatsapp');
  });

  it('Test 5: DM and group channel_types differ (CS-01 invariant)', () => {
    const { channelTypeForChat } = loadHelperWithFlag(undefined);
    const dm = channelTypeForChat('391234@s.whatsapp.net');
    const group = channelTypeForChat('120363abc@g.us');
    assert.notEqual(dm, group, 'DM and group must never collapse to the same channel_type');
  });

  it('Test 6: LID-hosted groups still classified as group', () => {
    // Baileys `isJidGroup` returns true for `@g.us` — `@lid` peers are NOT
    // groups. We make this explicit to prevent a future regression that
    // treats LID peers as groups.
    const { channelTypeForChat } = loadHelperWithFlag(undefined);
    assert.equal(channelTypeForChat('abc@lid'), 'whatsapp-dm:abc@lid');
    assert.equal(channelTypeForChat('120363abc@g.us'), 'whatsapp-group:120363abc@g.us');
  });
});

describe('channelTypeForChat (strict prefix disabled via flag)', () => {
  let origFlag;
  beforeEach(() => {
    origFlag = process.env.LIBREFANG_STRICT_CHANNEL_PREFIX;
  });
  afterEach(() => {
    if (origFlag === undefined) {
      delete process.env.LIBREFANG_STRICT_CHANNEL_PREFIX;
    } else {
      process.env.LIBREFANG_STRICT_CHANNEL_PREFIX = origFlag;
    }
  });

  it('Test 4: flag=off → DM and group both return legacy whatsapp:<jid>', () => {
    const { channelTypeForChat } = loadHelperWithFlag('off');
    assert.equal(
      channelTypeForChat('391234@s.whatsapp.net'),
      'whatsapp:391234@s.whatsapp.net'
    );
    assert.equal(
      channelTypeForChat('120363abc@g.us'),
      'whatsapp:120363abc@g.us'
    );
  });

  it('Test 4b: flag=off still preserves CS-01 distinct-JIDs invariant', () => {
    const { channelTypeForChat } = loadHelperWithFlag('off');
    const dm = channelTypeForChat('391234@s.whatsapp.net');
    const group = channelTypeForChat('120363abc@g.us');
    // Legacy prefix still produced two distinct strings because the JIDs
    // differ — CS-01 held even before the strict prefix was introduced.
    assert.notEqual(dm, group);
  });

  it('Test 4c: flag=off sets STRICT_PREFIX_ENABLED to false', () => {
    const helper = loadHelperWithFlag('off');
    assert.equal(helper.STRICT_PREFIX_ENABLED, false);
  });
});

describe('runCs01SelfTest', () => {
  let origFlag;
  beforeEach(() => {
    origFlag = process.env.LIBREFANG_STRICT_CHANNEL_PREFIX;
  });
  afterEach(() => {
    if (origFlag === undefined) {
      delete process.env.LIBREFANG_STRICT_CHANNEL_PREFIX;
    } else {
      process.env.LIBREFANG_STRICT_CHANNEL_PREFIX = origFlag;
    }
  });

  it('reports ok=true with strict prefix applied (default)', () => {
    const { runCs01SelfTest } = loadHelperWithFlag(undefined);
    const result = runCs01SelfTest();
    assert.equal(result.ok, true);
    assert.equal(result.dm_type, 'whatsapp-dm:111@s.whatsapp.net');
    assert.equal(result.group_type, 'whatsapp-group:120363aaa@g.us');
  });

  it('reports ok=true with legacy prefix when flag=off', () => {
    const { runCs01SelfTest } = loadHelperWithFlag('off');
    const result = runCs01SelfTest();
    assert.equal(result.ok, true);
    assert.equal(result.dm_type, 'whatsapp:111@s.whatsapp.net');
    assert.equal(result.group_type, 'whatsapp:120363aaa@g.us');
  });

  it('returned object is JSON-serialisable for the boot log', () => {
    const { runCs01SelfTest } = loadHelperWithFlag(undefined);
    const result = runCs01SelfTest();
    const serialized = JSON.stringify({ event: 'cs01_self_test', ...result });
    assert.match(serialized, /"event":"cs01_self_test"/);
    assert.match(serialized, /"ok":true/);
    assert.match(serialized, /"dm_type":"whatsapp-dm:/);
    assert.match(serialized, /"group_type":"whatsapp-group:/);
  });
});
