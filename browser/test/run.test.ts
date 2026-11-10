import { after, before, test } from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { createServer, type Server } from 'node:http';
import type { AddressInfo } from 'node:net';
import { createRunner, type Runner } from '../src/run.ts';
import type { Step } from '../src/steps.ts';

let site: Server;
let base = '';
let runner: Runner;

before(async () => {
  site = createServer(async (req, res) => {
    const path = new URL(req.url ?? '/', 'http://x').pathname;
    try {
      const body = await readFile(new URL(`./site${path}`, import.meta.url));
      res.writeHead(200, { 'content-type': 'text/html' }).end(body);
    } catch {
      res.writeHead(404).end('not found');
    }
  });
  await new Promise<void>((done) => site.listen(0, '127.0.0.1', () => done()));
  base = `http://127.0.0.1:${(site.address() as AddressInfo).port}`;
  runner = await createRunner({ allowPrivate: true });
});

after(async () => {
  await runner.close();
  site.close();
});

const css = (value: string) => ({ userLocator: { failTestOnCannotLocate: true, values: [{ type: 'css' as const, value }] } });

test('every step type runs against a page', async () => {
  const steps: Step[] = [
    { name: 'Title', type: 'assertElementContent', params: { element: css('#title'), check: 'equals', value: 'Example Shop' } },
    { name: 'Search', type: 'typeText', params: { element: css('#q'), value: 'mugs' } },
    { name: 'Submit', type: 'pressKey', params: { value: 'Enter' } },
    { name: 'Results', type: 'assertPageContains', params: { value: 'Results for mugs' } },
    { name: 'No error', type: 'assertPageLacks', params: { value: 'Something went wrong' } },
    { name: 'Size', type: 'selectOption', params: { element: css('#size'), value: 'Large' } },
    { name: 'Size shown', type: 'assertElementContent', params: { element: css('#chosen'), check: 'equals', value: 'Size l' } },
    { name: 'Menu', type: 'hover', params: { element: css('#menu') } },
    { name: 'Submenu', type: 'assertElementPresent', params: { element: css('#submenu') } },
    { name: 'Order', type: 'click', params: { element: { userLocator: { failTestOnCannotLocate: true, values: [{ type: 'xpath', value: "//button[normalize-space()='Place order']" }] } } } },
    { name: 'Order id', type: 'extractVariable', params: { element: css('#late'), variable: { name: 'ORDER', secure: false } } },
    { name: 'Escape', type: 'pressKey', params: { value: 'Escape' } },
    { name: 'Escaped', type: 'assertElementContent', params: { element: css('#result'), check: 'equals', value: 'escaped' } },
    { name: 'Scroll', type: 'scroll', params: { x: 0, y: 2500 } },
    { name: 'Empty', type: 'assertElementContent', params: { element: css('.empty'), check: 'isEmpty' } },
    { name: 'Product', type: 'click', params: { element: css('#product') } },
    { name: 'URL', type: 'assertCurrentUrl', params: { check: 'matchRegex', value: '/product\\.html\\?id=42$' } },
    { name: 'SKU', type: 'assertElementAttribute', params: { element: css('h1'), attribute: 'data-sku', check: 'startsWith', value: 'MUG' } },
    { name: 'Price', type: 'assertElementContent', params: { element: css('.price'), check: 'greater', value: '10' } },
    { name: 'Cart', type: 'assertFromJavascript', params: { code: 'return window.cartCount === 3;' } },
    { name: 'Cart count', type: 'extractFromJavascript', params: { code: 'return window.cartCount * 2;', variable: { name: 'DOUBLE', secure: false } } },
    { name: 'Reload', type: 'refresh' },
    { name: 'Wait', type: 'wait', params: { value: 1 } },
    { name: 'Back home', type: 'goToUrl', params: { value: `${base}/shop.html?order={{ ORDER }}&n={{ DOUBLE }}` } },
    { name: 'Carried', type: 'assertCurrentUrl', params: { check: 'contains', value: 'order=Order%201234%20placed&n=6' } },
  ];
  const r = await runner.run({ startUrl: `${base}/shop.html`, steps });
  const failed = r.steps.filter((s) => s.status !== 'passed');
  assert.deepEqual(failed, [], JSON.stringify(failed, null, 1));
  assert.equal(r.passed, true);
  assert.equal(r.steps.length, steps.length + 1, 'the start URL is the first step');
  assert.equal(r.steps[0].type, 'goToUrl');
  assert.equal(r.steps[11].value, 'Order 1234 placed');
  assert.ok(
    r.steps.every((s) => s.screenshot && s.screenshot.length > 100),
    'screenshots',
  );
  assert.ok(r.steps.every((s) => s.durationMs >= 0 && s.url.startsWith(base)));
  assert.match(r.browserVersion, /^\d+\./);
});

test('a missing element fails the test and skips the rest', async () => {
  const r = await runner.run({
    startUrl: `${base}/shop.html`,
    steps: [
      { name: 'Missing', type: 'click', params: { element: css('#nope') }, timeout: 1 },
      { name: 'Never', type: 'refresh' },
    ],
  });
  assert.equal(r.passed, false);
  assert.equal(r.steps[1].status, 'failed');
  assert.equal(r.steps[1].failure?.code, 'LOCATE_ELEMENT');
  assert.equal(r.steps[2].status, 'skipped');
  assert.equal(r.failure?.code, 'LOCATE_ELEMENT');
});

test('allowed failures continue; critical ones still fail the test', async () => {
  const lax = await runner.run({
    startUrl: `${base}/shop.html`,
    steps: [
      { name: 'Optional', type: 'assertPageContains', params: { value: 'Free shipping' }, timeout: 1, allowFailure: true },
      { name: 'Title', type: 'assertElementPresent', params: { element: css('#title') } },
    ],
  });
  assert.equal(lax.passed, true);
  assert.equal(lax.steps[1].status, 'failed');
  assert.equal(lax.steps[1].failure?.code, 'ASSERTION_FAILURE');
  assert.equal(lax.steps[2].status, 'passed');
  const strict = await runner.run({
    startUrl: `${base}/shop.html`,
    steps: [
      { name: 'Critical', type: 'assertPageContains', params: { value: 'Free shipping' }, timeout: 1, allowFailure: true, isCritical: true },
      { name: 'Title', type: 'assertElementPresent', params: { element: css('#title') } },
    ],
  });
  assert.equal(strict.passed, false);
  assert.equal(strict.steps[2].status, 'passed', 'the next step still ran');
});

test('assertions wait for the page; unknown variables fail the step', async () => {
  const r = await runner.run({
    startUrl: `${base}/shop.html`,
    steps: [
      { name: 'Order', type: 'click', params: { element: css('#later') } },
      { name: 'Placed', type: 'assertPageContains', params: { value: 'Order 1234 placed' }, timeout: 5 },
      { name: 'Unknown', type: 'typeText', params: { element: css('#q'), value: '{{ NOPE }}' } },
    ],
  });
  assert.equal(r.steps[2].status, 'passed');
  assert.equal(r.steps[3].status, 'failed');
  assert.match(r.steps[3].error ?? '', /NOPE/);
});

test('secure values are masked in results', async () => {
  const r = await runner.run({
    startUrl: `${base}/shop.html`,
    variables: { PASSWORD: 'hunter2' },
    secrets: ['hunter2'],
    steps: [
      { name: 'Type', type: 'typeText', params: { element: css('#q'), value: '{{ PASSWORD }}' } },
      { name: 'Wrong', type: 'assertElementContent', params: { element: css('#title'), check: 'equals', value: '{{ PASSWORD }}' }, timeout: 1 },
    ],
  });
  assert.doesNotMatch(JSON.stringify({ ...r, steps: r.steps.map((s) => ({ ...s, screenshot: undefined })) }), /hunter2/);
});

test('failed runs keep a video; passing ones only when asked', async () => {
  const failing = await runner.run({ startUrl: `${base}/shop.html`, steps: [{ name: 'Missing', type: 'assertElementPresent', params: { element: css('#nope') }, timeout: 1 }] });
  assert.equal(failing.passed, false);
  const video = Buffer.from(failing.video ?? '', 'base64');
  assert.ok(video.length > 1000, `video of ${video.length} bytes`);
  assert.equal(video.subarray(0, 4).toString('hex'), '1a45dfa3', 'a WebM (EBML) file');
  const passing = await runner.run({ startUrl: `${base}/shop.html`, steps: [] });
  assert.equal(passing.video, undefined);
  const asked = await runner.run({ startUrl: `${base}/shop.html`, steps: [], video: 'always' });
  assert.ok((asked.video ?? '').length > 1000);
  const never = await runner.run({ startUrl: `${base}/shop.html`, steps: [{ name: 'Missing', type: 'assertElementPresent', params: { element: css('#nope') }, timeout: 1 }], video: 'never' });
  assert.equal(never.video, undefined);
});

test('the step timeout bounds waiting', async () => {
  const started = Date.now();
  const r = await runner.run({ startUrl: `${base}/shop.html`, steps: [{ name: 'Never', type: 'assertElementPresent', params: { element: css('#never') }, timeout: 1 }] });
  assert.equal(r.passed, false);
  assert.ok(Date.now() - started < 8000, `took ${Date.now() - started} ms`);
});

test('a blocked or unreachable start URL fails as OPEN_URL', async () => {
  const r = await runner.run({ startUrl: 'http://127.0.0.1:1/nothing', steps: [] });
  assert.equal(r.passed, false);
  assert.equal(r.steps[0].failure?.code, 'OPEN_URL');
});

test('internal addresses are refused when private targets are disabled', async () => {
  const guarded = await createRunner({ allowPrivate: false });
  try {
    const r = await guarded.run({ startUrl: `${base}/shop.html`, steps: [] });
    assert.equal(r.passed, false);
    assert.equal(r.steps[0].failure?.code, 'OPEN_URL');
    assert.match(r.steps[0].error ?? '', /loopback/);
    const named = await guarded.run({ startUrl: base.replace('127.0.0.1', 'localhost') + '/shop.html', steps: [] });
    assert.match(named.steps[0].error ?? '', /loopback/);
  } finally {
    await guarded.close();
  }
});
