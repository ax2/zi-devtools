const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const test = require('node:test');
const vm = require('node:vm');

const html = fs.readFileSync(path.join(__dirname, '..', 'views', 'main.html'), 'utf8');
const inlineScripts = [...html.matchAll(/<script(?:\s[^>]*)?>([\s\S]*?)<\/script>/gi)].map((match) => match[1]);
assert.equal(/<script\s+src=/i.test(html), false, 'the plugin View must not load external scripts');
const helperScript = inlineScripts.find((script) => script.includes('window.ZiAgentAssistLimits'));
assert.ok(helperScript, 'the byte-limit helper must be embedded in main.html');
const context = { window: {}, TextEncoder };
vm.runInNewContext(helperScript, context, { filename: 'main.html#limits' });
const limits = context.window.ZiAgentAssistLimits;
const model = { modelRef: 'model-opaque-ref', displayName: 'Fixture model' };
const material = (text, sourceId = 'source-1') => ({
  sourceId,
  displayName: '手动资料',
  sourceKind: 'user_text',
  trust: 'untrusted_user_material',
  text,
});
const draft = (overrides = {}) => ({
  goal: '请分析这份资料。',
  materials: [],
  model,
  maxOutputTokens: 1024,
  deadlineMs: 60000,
  ...overrides,
});

test('counts UTF-8 bytes for the 2 KiB goal boundary', () => {
  assert.equal(limits.bytes('é'.repeat(1024)), 2048);
  assert.equal(limits.validateDraft(draft({ goal: 'é'.repeat(1024) })), null);
  assert.equal(limits.validateDraft(draft({ goal: 'é'.repeat(1025) })), 'goal_too_large');
});

test('counts aggregate UTF-8 material bytes without trusting source labels', () => {
  assert.equal(limits.validateDraft(draft({ materials: [material('x'.repeat(32768))] })), null);
  assert.equal(limits.validateDraft(draft({ materials: [material('x'.repeat(32769))] })), 'materials_too_large');
  assert.equal(limits.validateDraft(draft({ materials: [material('x', 'a'), material('y', 'a')] })), 'invalid_material_id');
  assert.equal(limits.validateDraft(draft({ materials: [{ ...material('x'), trust: 'verified_file' }] })), 'invalid_material_source');
});

test('caps materials at 32 and validates model, token and deadline bounds', () => {
  const thirtyTwo = Array.from({ length: 32 }, (_, index) => material('', `source-${index}`));
  assert.equal(limits.validateDraft(draft({ materials: thirtyTwo })), null);
  assert.equal(limits.validateDraft(draft({ materials: [...thirtyTwo, material('', 'source-33')] })), 'too_many_materials');
  assert.equal(limits.validateDraft(draft({ model: null })), 'model_required');
  assert.equal(limits.validateDraft(draft({ model: { modelRef: 'x'.repeat(129) } })), 'model_required');
  assert.equal(limits.validateDraft(draft({ maxOutputTokens: 4097 })), 'invalid_output_limit');
  assert.equal(limits.validateDraft(draft({ deadlineMs: 120001 })), 'invalid_deadline');
});
