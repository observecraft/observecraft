import assert from 'node:assert/strict';
import test from 'node:test';
import { registerHooks } from 'node:module';

// The web sources use extensionless relative imports (bundler resolution); resolve them to `.ts` here.
registerHooks({
  resolve(specifier, context, next) {
    try {
      return next(specifier, context);
    } catch (err) {
      if (specifier.startsWith('.')) return next(`${specifier}.ts`, context);
      throw err;
    }
  },
});
const mock = await import('../web/src/api/mock/assist.ts');
const { applyEvent } = await import('../web/src/pages/assist/chatState.ts');

/** Sends a message to the mock and folds the streamed events the way the chat does. */
async function send(messages, req) {
  let id;
  let done;
  let view = [...messages, { role: 'user', text: req.message, tools: [] }];
  await mock.chat(
    req,
    (e) => {
      if (e.type === 'conversation') id = e.id;
      if (e.type === 'done') done = e;
      view = applyEvent(view, e);
    },
    new AbortController().signal,
  );
  return { id, done, view };
}

test('the streamed reply matches the stored conversation', async () => {
  mock.resetAssistMock();
  const { id, done, view } = await send([], { conversationId: null, message: 'Which errors are the most frequent?', surface: 'chat', autoApprove: false });
  assert.equal(done.awaiting_approval, false);
  const stored = await mock.getConversation(id);
  assert.deepEqual(view, stored.messages);
  assert.deepEqual(
    view[1].tools.map((t) => [t.name, t.status]),
    [
      ['search_logs', 'ok'],
      ['search_error_issues', 'ok'],
    ],
  );
});

test('a change waits for approval, and a declined one is reported as declined', async () => {
  mock.resetAssistMock();
  const { id, done, view } = await send([], { conversationId: null, message: 'Create a monitor for checkout errors', surface: 'panel', autoApprove: false });
  assert.equal(done.awaiting_approval, true);
  const pending = view[1].tools.find((t) => t.status === 'pending');
  assert.equal(pending.name, 'create_monitor');
  await assert.rejects(
    mock.chat({ conversationId: id, message: 'and another', surface: 'panel', autoApprove: false }, () => {}, new AbortController().signal),
    /pending/,
  );

  let after = view;
  await mock.approve(id, [], (e) => (after = applyEvent(after, e)), new AbortController().signal);
  const tool = after[1].tools.find((t) => t.id === pending.id);
  assert.equal(tool.status, 'error');
  assert.match(tool.summary, /declined/);
  assert.deepEqual(after, (await mock.getConversation(id)).messages);
});

test('events that are not part of the reply leave the messages as they are', () => {
  const messages = [{ role: 'user', text: 'hi', tools: [] }];
  assert.equal(applyEvent(messages, { type: 'conversation', id: 'c', title: 'hi' }), messages);
  assert.equal(applyEvent(messages, { type: 'done', awaiting_approval: false, input_tokens: 1, output_tokens: 1 }), messages);
});

test('the usage report counts messages, conversations and surfaces in the range', async () => {
  mock.resetAssistMock();
  const now = Date.now();
  const before = await mock.report({ range: { from: now - 60_000, to: now + 60_000 }, surface: 'panel', userId: null });
  await send([], { conversationId: null, message: 'list services', surface: 'panel', autoApprove: false });
  const after = await mock.report({ range: { from: now - 60_000, to: Date.now() + 60_000 }, surface: 'panel', userId: null });
  assert.equal(after.totalMessages, before.totalMessages + 1);
  assert.equal(after.totalConversations, before.totalConversations + 1);
  assert.deepEqual(
    after.messagesBySurface.map((s) => s.surface),
    ['panel'],
  );
});

test("an organization's provider replaces the install's, and its key is never shown", async () => {
  mock.resetAssistMock();
  assert.equal((await mock.getProvider()).source, 'install');

  const settings = { provider: 'openai', model: 'gpt-5', baseUrl: 'https://llm.example.com/v1' };
  await mock.putProvider({ ...settings, apiKey: 'sk-org-secret' });
  const view = await mock.getProvider();
  assert.equal(view.source, 'org');
  assert.equal(view.org.hasKey, true);
  assert.doesNotMatch(JSON.stringify(view), /sk-org-secret/);
  assert.deepEqual([(await mock.status()).source, (await mock.status()).model], ['org', 'gpt-5']);

  await mock.putProvider({ ...settings, model: 'gpt-5-mini' });
  assert.equal((await mock.getProvider()).org.hasKey, true, 'saved without a key keeps it');
  assert.equal((await mock.testProvider({ ...settings, model: 'gpt-5-mini' })).ok, true);

  await assert.rejects(mock.putProvider({ ...settings, baseUrl: 'http://10.0.0.5/v1' }), /https/);
  await assert.rejects(mock.putProvider({ provider: 'anthropic', apiKey: '' }), /API key/);

  await mock.deleteProvider();
  assert.equal((await mock.getProvider()).source, 'install');
  assert.equal((await mock.status()).model, 'claude-opus-5');
});
