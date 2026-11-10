import { after, before, test } from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { createServer, type Server } from 'node:http';
import type { AddressInfo } from 'node:net';
import { createSessions, type Sessions } from '../src/session.ts';

let site: Server;
let base = '';
let sessions: Sessions;

before(async () => {
  site = createServer(async (req, res) => {
    const path = new URL(req.url ?? '/', 'http://x').pathname;
    try {
      res.writeHead(200, { 'content-type': 'text/html' }).end(await readFile(new URL(`./site${path}`, import.meta.url)));
    } catch {
      res.writeHead(404).end('not found');
    }
  });
  await new Promise<void>((done) => site.listen(0, '127.0.0.1', () => done()));
  base = `http://127.0.0.1:${(site.address() as AddressInfo).port}`;
  sessions = await createSessions({ allowPrivate: true, max: 2 });
});

after(async () => {
  await sessions.close();
  site.close();
});

/** The ref of the snapshot line naming `text`. */
function ref(snapshot: string, text: string): string {
  const line = snapshot.split('\n').find((l) => l.includes(text));
  assert.ok(line, `no element "${text}" in:\n${snapshot}`);
  return /\[(e\d+)\]/.exec(line!)![1];
}

test('a session opens the start URL and lists the page with refs', async () => {
  const s = await sessions.open({ startUrl: `${base}/login.html` });
  try {
    assert.match(s.snapshot, /Title: Sign in - Example Shop/);
    assert.match(s.snapshot, /Headings: .*h1 "Sign in"/);
    assert.match(s.snapshot, /\[e\d+\] textbox "Email"/);
    assert.match(s.snapshot, /\[e\d+\] textbox "Password" \(password\)/);
    assert.match(s.snapshot, /\[e\d+\] button "Sign in"/);
    assert.match(s.snapshot, /\[e\d+\] link "Shop" → \/shop\.html/);
  } finally {
    await sessions.end(s.id);
  }
});

test('actions on refs come back as steps with stable locators', async () => {
  const s = await sessions.open({ startUrl: `${base}/login.html` });
  try {
    const typed = await sessions.act(s.id, { type: 'type', ref: ref(s.snapshot, 'textbox "Email"'), text: '{{ SHOP_USER }}' }, { SHOP_USER: 'qa@example.com' });
    assert.equal(typed.ok, true, typed.error);
    assert.equal(typed.step?.type, 'typeText');
    assert.equal(typed.step?.params?.value, '{{ SHOP_USER }}', 'the step keeps the variable');
    assert.deepEqual(typed.step?.params?.element?.userLocator.values[0], { type: 'css', value: '#email' });
    const pw = await sessions.act(s.id, { type: 'type', ref: ref(typed.snapshot, 'Password'), text: '{{ SHOP_PASSWORD }}' }, { SHOP_PASSWORD: 'hunter2' });
    assert.doesNotMatch(JSON.stringify(pw), /hunter2/, 'secrets never come back');
    const clicked = await sessions.act(s.id, { type: 'click', ref: ref(pw.snapshot, 'button "Sign in"') }, {});
    assert.equal(clicked.ok, true, clicked.error);
    assert.equal(clicked.step?.type, 'click');
    assert.deepEqual(clicked.step?.params?.element?.userLocator.values[0], { type: 'css', value: '[data-testid="sign-in"]' });
    assert.equal(clicked.urlBefore, `${base}/login.html`);
    assert.match(clicked.url, /\/account\.html$/);
    assert.match(clicked.snapshot, /Welcome back/);
    const checked = await sessions.act(s.id, { type: 'assert', assert: 'pageContains', text: 'You have 2 orders' }, {});
    assert.equal(checked.ok, true);
    assert.deepEqual(checked.step, { name: "Test page contains 'You have 2 orders'", type: 'assertPageContains', params: { value: 'You have 2 orders' } });
    const missing = await sessions.act(s.id, { type: 'assert', assert: 'pageContains', text: 'Free shipping' }, {});
    assert.equal(missing.ok, false);
    assert.equal(missing.step, undefined);
  } finally {
    await sessions.end(s.id);
  }
});

test('locators fall back to text, then position, when nothing unique names the element', async () => {
  const s = await sessions.open({ startUrl: `${base}/login.html` });
  try {
    const forgot = await sessions.act(s.id, { type: 'assert', assert: 'elementVisible', ref: ref(s.snapshot, 'Forgot password?') }, {});
    assert.deepEqual(forgot.step?.params?.element?.userLocator.values[0], { type: 'xpath', value: "//button[normalize-space()='Forgot password?']" });
    const details = await sessions.act(s.id, { type: 'assert', assert: 'elementVisible', ref: ref(s.snapshot, 'Details') }, {});
    assert.deepEqual(details.step?.params?.element?.userLocator.values[0], { type: 'xpath', value: "(//button[normalize-space()='Details'])[1]" });
  } finally {
    await sessions.end(s.id);
  }
});

test('navigation stays on the allowed hosts', async () => {
  const s = await sessions.open({ startUrl: `${base}/login.html`, allowedHosts: ['127.0.0.1'] });
  try {
    const away = await sessions.act(s.id, { type: 'goto', url: 'https://elsewhere.example.net/' }, {});
    assert.equal(away.ok, false);
    assert.match(away.error ?? '', /outside/);
    assert.match(away.url, /login\.html$/);
    const shop = await sessions.act(s.id, { type: 'goto', url: `${base}/shop.html` }, {});
    assert.equal(shop.ok, true);
    assert.equal(shop.step?.type, 'goToUrl');
    const back = await sessions.act(s.id, { type: 'back' }, {});
    assert.match(back.url, /login\.html$/);
  } finally {
    await sessions.end(s.id);
  }
});

test('unknown refs and sessions are errors, and the session limit holds', async () => {
  const s = await sessions.open({ startUrl: `${base}/login.html` });
  const t = await sessions.open({ startUrl: `${base}/login.html` });
  try {
    await assert.rejects(sessions.open({ startUrl: `${base}/login.html` }), /busy/);
    const bad = await sessions.act(s.id, { type: 'click', ref: 'e999' }, {});
    assert.equal(bad.ok, false);
    assert.match(bad.error ?? '', /e999/);
    await assert.rejects(sessions.act('nope', { type: 'snapshot' }, {}), /session/);
  } finally {
    await sessions.end(s.id);
    await sessions.end(t.id);
  }
});
