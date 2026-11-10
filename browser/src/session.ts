// Agent sessions (AI Testing): a browser the agent drives one action at a time. Each answer is a text snapshot of
// the page whose interactive elements carry refs ([e12]); an action names a ref, and comes back as the browser test
// step that repeats it, with locators computed before the action (stable ones: id, test id, name, label, text, then
// position). Navigation may be kept on allowed hosts; every request still goes through the checking proxy.
import { randomUUID } from 'node:crypto';
import { chromium, type Browser, type BrowserContext, type Page } from 'playwright-core';
import { BLOCKED_HEADER, startProxy, type Proxy } from './proxy.ts';
import { DEVICES, userAgent } from './run.ts';
import { fill, type Locator, type Step } from './steps.ts';

export interface OpenRequest {
  startUrl: string;
  deviceId?: string;
  /** Hosts the top-level page may navigate to (subdomains included); none: anywhere public. */
  allowedHosts?: string[] | null;
}

export type SessionAction =
  | { type: 'snapshot' }
  | { type: 'goto'; url: string }
  | { type: 'click'; ref: string }
  | { type: 'type'; ref: string; text: string }
  | { type: 'select'; ref: string; value: string }
  | { type: 'press'; key: string }
  | { type: 'back' }
  | { type: 'scroll'; direction?: 'down' | 'up' }
  | { type: 'assert'; assert: 'pageContains' | 'urlContains' | 'elementVisible' | 'elementText'; ref?: string; text?: string };

export interface ActionResult {
  ok: boolean;
  error?: string;
  /** Where the page was before the action, and is after it. */
  urlBefore: string;
  url: string;
  title: string;
  snapshot: string;
  /** The browser test step that repeats the action (successful actions and assertions only). */
  step?: Step;
  /** API calls the page made during the action (fetch/XHR). */
  requests: { method: string; url: string; status: number }[];
}

export interface Sessions {
  open: (req: OpenRequest) => Promise<ActionResult & { id: string }>;
  /** `variables` fill `{{ NAME }}` in typed text; the step keeps the placeholder and values are masked. */
  act: (id: string, action: SessionAction, variables: Record<string, string>) => Promise<ActionResult>;
  end: (id: string) => Promise<void>;
  close: () => Promise<void>;
}

interface Session {
  context: BrowserContext;
  page: Page;
  proxy: Proxy;
  allowedHosts: string[] | null;
  requests: ActionResult['requests'];
  blocked: string[];
  touched: number;
}

const MAX_ELEMENTS = 150;
const MAX_TEXT = 1500;
const IDLE_MS = 10 * 60_000;
const ACTION_TIMEOUT = 15_000;

/** In the page: numbers the visible interactive elements (data-oc-ref) and describes the page. */
function describePage(max: number) {
  const visible = (el: Element) => {
    const r = el.getBoundingClientRect();
    const s = getComputedStyle(el);
    return r.width > 0 && r.height > 0 && s.visibility !== 'hidden' && s.display !== 'none';
  };
  const clean = (t: string | null | undefined) => (t ?? '').replace(/\s+/g, ' ').trim().slice(0, 80);
  const labelOf = (el: Element) => {
    const id = el.getAttribute('id');
    const forLabel = id ? document.querySelector(`label[for="${CSS.escape(id)}"]`) : null;
    return clean(
      el.getAttribute('aria-label') ||
        forLabel?.textContent ||
        el.closest('label')?.textContent ||
        el.getAttribute('placeholder') ||
        el.getAttribute('title') ||
        el.getAttribute('alt') ||
        (el as HTMLElement).innerText ||
        el.getAttribute('value') ||
        el.getAttribute('name'),
    );
  };
  const roleOf = (el: Element) => {
    const role = el.getAttribute('role');
    if (role) return role;
    const tag = el.tagName.toLowerCase();
    const type = (el.getAttribute('type') ?? '').toLowerCase();
    if (tag === 'a') return 'link';
    if (tag === 'button' || (tag === 'input' && ['submit', 'button', 'reset'].includes(type))) return 'button';
    if (tag === 'select') return 'combobox';
    if (tag === 'input' && type === 'checkbox') return 'checkbox';
    if (tag === 'input' && type === 'radio') return 'radio';
    return 'textbox';
  };
  document.querySelectorAll('[data-oc-ref]').forEach((el) => el.removeAttribute('data-oc-ref'));
  const selector = 'a[href], button, input:not([type=hidden]), select, textarea, [role=button], [role=link], [role=tab], [role=menuitem], [role=checkbox], [contenteditable=true]';
  const lines: string[] = [];
  let n = 0;
  for (const el of Array.from(document.querySelectorAll(selector))) {
    if (n >= max || !visible(el)) continue;
    n++;
    const ref = `e${n}`;
    el.setAttribute('data-oc-ref', ref);
    const role = roleOf(el);
    const type = (el.getAttribute('type') ?? '').toLowerCase();
    let line = `[${ref}] ${role} "${labelOf(el)}"`;
    if (type === 'password') line += ' (password)';
    if (el instanceof HTMLSelectElement)
      line += ` options: ${Array.from(el.options)
        .slice(0, 10)
        .map((o) => clean(o.text))
        .join(' | ')}`;
    const href = el.getAttribute('href');
    if (role === 'link' && href) line += ` → ${href.slice(0, 120)}`;
    lines.push(line);
  }
  const headings = Array.from(document.querySelectorAll('h1, h2, h3'))
    .filter(visible)
    .slice(0, 12)
    .map((h) => `${h.tagName.toLowerCase()} "${clean(h.textContent)}"`);
  return { headings, elements: lines, text: (document.body?.innerText ?? '').replace(/\s+/g, ' ').trim() };
}

/** In the page: CSS and XPath locators of an element, the most stable first, each unique in the page. */
function locatorsOf(el: Element): { type: 'css' | 'xpath'; value: string }[] {
  const out: { type: 'css' | 'xpath'; value: string }[] = [];
  const tag = el.tagName.toLowerCase();
  const uniqueCss = (css: string) => {
    try {
      return document.querySelectorAll(css).length === 1;
    } catch {
      return false;
    }
  };
  const count = (xpath: string) => document.evaluate(`count(${xpath})`, document, null, XPathResult.NUMBER_TYPE, null).numberValue;
  const quote = (s: string) => (s.includes("'") ? `concat('${s.split("'").join(`', "'", '`)}')` : `'${s}'`);
  const id = el.getAttribute('id');
  // Generated ids (react-select-3, ember1234, :r5:) change between builds.
  if (id && /^[A-Za-z][\w-]*$/.test(id) && !/\d{3,}|^(ember|react|radix|mui)/i.test(id) && uniqueCss(`#${id}`)) out.push({ type: 'css', value: `#${id}` });
  for (const attr of ['data-testid', 'data-test', 'data-qa', 'data-cy']) {
    const v = el.getAttribute(attr);
    if (v && uniqueCss(`[${attr}="${CSS.escape(v)}"]`)) out.push({ type: 'css', value: `[${attr}="${v}"]` });
  }
  const name = el.getAttribute('name');
  if (name && /^(input|select|textarea)$/.test(tag) && uniqueCss(`${tag}[name="${CSS.escape(name)}"]`)) out.push({ type: 'css', value: `${tag}[name="${name}"]` });
  const aria = el.getAttribute('aria-label');
  if (aria && uniqueCss(`${tag}[aria-label="${CSS.escape(aria)}"]`)) out.push({ type: 'css', value: `${tag}[aria-label="${aria}"]` });
  const text = ((el as HTMLElement).innerText ?? '').replace(/\s+/g, ' ').trim();
  if (text && text.length <= 60 && !/^(input|select|textarea)$/.test(tag)) {
    const xpath = `//${tag}[normalize-space()=${quote(text)}]`;
    const n = count(xpath);
    if (n === 1) out.push({ type: 'xpath', value: xpath });
    else if (n > 1) {
      const all = document.evaluate(xpath, document, null, XPathResult.ORDERED_NODE_SNAPSHOT_TYPE, null);
      for (let i = 0; i < all.snapshotLength; i++) if (all.snapshotItem(i) === el) out.push({ type: 'xpath', value: `(${xpath})[${i + 1}]` });
    }
  }
  // Last resort: the element's path from the nearest ancestor with an id (or the body).
  const parts: string[] = [];
  let node: Element | null = el;
  while (node && node !== document.body && parts.length < 8) {
    const nodeId = node.getAttribute('id');
    if (node !== el && nodeId && /^[A-Za-z][\w-]*$/.test(nodeId)) {
      parts.unshift(`#${nodeId}`);
      break;
    }
    const parent: Element | null = node.parentElement;
    const same = parent ? Array.from(parent.children).filter((c) => c.tagName === node!.tagName) : [];
    parts.unshift(same.length > 1 ? `${node.tagName.toLowerCase()}:nth-of-type(${same.indexOf(node) + 1})` : node.tagName.toLowerCase());
    node = parent;
  }
  out.push({ type: 'css', value: parts.join(' > ') });
  return out.slice(0, 4);
}

function hostAllowed(url: string, hosts: string[] | null): boolean {
  if (!hosts) return true;
  try {
    const h = new URL(url).hostname;
    return hosts.some((a) => h === a || h.endsWith(`.${a}`));
  } catch {
    return false;
  }
}

export async function createSessions(options: { allowPrivate: boolean; max: number; executablePath?: string }): Promise<Sessions> {
  const browser: Browser = await chromium.launch({
    executablePath: options.executablePath,
    args: ['--force-webrtc-ip-handling-policy=disable_non_proxied_udp', '--webrtc-ip-handling-policy=disable_non_proxied_udp'],
  });
  const live = new Map<string, Session>();

  const end = async (id: string) => {
    const s = live.get(id);
    if (!s) return;
    live.delete(id);
    await s.context.close().catch(() => {});
    await s.proxy.close();
  };
  const sweep = setInterval(() => {
    for (const [id, s] of live) if (Date.now() - s.touched > IDLE_MS) void end(id);
  }, 30_000);
  sweep.unref();

  const snapshot = async (s: Session, urlBefore: string, secrets: string[]): Promise<ActionResult> => {
    const page = s.page;
    await page.waitForLoadState('domcontentloaded', { timeout: 5000 }).catch(() => {});
    const d = await page.evaluate(describePage, MAX_ELEMENTS).catch(() => ({ headings: [] as string[], elements: [] as string[], text: '' }));
    const title = await page.title().catch(() => '');
    let text = [`URL: ${page.url()}`, `Title: ${title}`, `Headings: ${d.headings.join('; ') || '(none)'}`, 'Elements:', ...d.elements, `Text: ${d.text.slice(0, MAX_TEXT)}`].join('\n');
    for (const secret of secrets) if (secret) text = text.split(secret).join('******');
    const requests = s.requests.splice(0);
    return { ok: true, urlBefore, url: page.url(), title, snapshot: text, requests };
  };

  /** The element behind a ref of the last snapshot, with its locators (computed before acting on it). */
  const element = async (s: Session, ref: string) => {
    const handle = s.page.locator(`[data-oc-ref="${ref.replace(/[^\w]/g, '')}"]`).first();
    if ((await handle.count()) === 0) throw new Error(`No element ${ref} on the page: take a new snapshot and use its refs`);
    const values = (await handle.evaluate(locatorsOf)) as Locator[];
    const description = await handle.evaluate((el) =>
      ((el as HTMLElement).innerText || el.getAttribute('aria-label') || el.getAttribute('placeholder') || el.getAttribute('name') || '').replace(/\s+/g, ' ').trim().slice(0, 60),
    );
    return { handle, element: { userLocator: { failTestOnCannotLocate: true, values }, ...(description ? { description } : {}) } };
  };

  const act = async (id: string, action: SessionAction, variables: Record<string, string>): Promise<ActionResult> => {
    const s = live.get(id);
    if (!s) throw new Error(`Unknown or expired session ${id}`);
    s.touched = Date.now();
    const page = s.page;
    const before = page.url();
    const secrets = Object.values(variables);
    const missing: string[] = [];
    const filled = (t: string) => fill(t, variables, missing);
    let step: Step | undefined;
    let failure: string | undefined;
    try {
      switch (action.type) {
        case 'snapshot':
          break;
        case 'goto': {
          if (!hostAllowed(action.url, s.allowedHosts)) throw new Error(`${action.url} is outside the hosts this job may visit (${s.allowedHosts?.join(', ')})`);
          const res = await page.goto(action.url, { timeout: 30_000, waitUntil: 'load' });
          if (res?.headers()[BLOCKED_HEADER]) throw new Error(`Blocked: ${await res.text()}`);
          step = { name: `Navigate to ${action.url}`, type: 'goToUrl', params: { value: action.url } };
          break;
        }
        case 'click': {
          const { handle, element: el } = await element(s, action.ref);
          await handle.click({ timeout: ACTION_TIMEOUT });
          step = { name: `Click on ${el.description ?? action.ref}`, type: 'click', params: { element: el } };
          break;
        }
        case 'type': {
          const { handle, element: el } = await element(s, action.ref);
          const text = filled(action.text);
          if (missing.length) throw new Error(`Unknown variables: ${missing.join(', ')}`);
          await handle.fill('', { timeout: ACTION_TIMEOUT });
          await handle.pressSequentially(text, { timeout: ACTION_TIMEOUT });
          step = { name: `Type text on ${el.description ?? action.ref}`, type: 'typeText', params: { element: el, value: action.text } };
          break;
        }
        case 'select': {
          const { handle, element: el } = await element(s, action.ref);
          await handle.selectOption(action.value, { timeout: ACTION_TIMEOUT });
          step = { name: `Select option '${action.value}' in ${el.description ?? action.ref}`, type: 'selectOption', params: { element: el, value: action.value } };
          break;
        }
        case 'press':
          await page.keyboard.press(action.key === 'Space' ? ' ' : action.key);
          step = { name: `Press ${action.key}`, type: 'pressKey', params: { value: action.key } };
          break;
        case 'back':
          await page.goBack({ timeout: 15_000 });
          break;
        case 'scroll':
          await page.mouse.wheel(0, action.direction === 'up' ? -800 : 800);
          break;
        case 'assert': {
          const text = action.text ?? '';
          if (action.assert === 'pageContains') {
            if (!(await page.evaluate(() => document.body?.innerText ?? '')).includes(text)) throw new Error(`The page doesn't contain '${text}'`);
            step = { name: `Test page contains '${text}'`, type: 'assertPageContains', params: { value: text } };
          } else if (action.assert === 'urlContains') {
            if (!page.url().includes(text)) throw new Error(`The URL ${page.url()} doesn't contain '${text}'`);
            step = { name: `Test current URL contains '${text}'`, type: 'assertCurrentUrl', params: { check: 'contains', value: text } };
          } else {
            const { handle, element: el } = await element(s, action.ref ?? '');
            if (!(await handle.isVisible())) throw new Error(`${action.ref} isn't visible`);
            if (action.assert === 'elementVisible') step = { name: `Test ${el.description ?? action.ref} is present`, type: 'assertElementPresent', params: { element: el } };
            else {
              const actual = (await handle.innerText()).trim();
              if (!actual.includes(text)) throw new Error(`${action.ref} shows '${actual.slice(0, 80)}', not '${text}'`);
              step = { name: `Test ${el.description ?? action.ref} content`, type: 'assertElementContent', params: { element: el, check: 'contains', value: text } };
            }
          }
          break;
        }
      }
      // A click may lead off the allowed hosts: come back.
      if (!hostAllowed(page.url(), s.allowedHosts)) {
        const away = page.url();
        await page.goBack({ timeout: 15_000 }).catch(() => page.goto(before));
        throw new Error(`That led to ${away}, outside the hosts this job may visit; went back`);
      }
    } catch (e) {
      failure = (e as Error).message.split('\n')[0];
      step = undefined;
    }
    const result = await snapshot(s, before, secrets);
    const mask = (t: string) => secrets.reduce((acc, x) => (x ? acc.split(x).join('******') : acc), t);
    return { ...result, ok: !failure, ...(failure ? { error: mask(failure) } : {}), ...(step ? { step } : {}) };
  };

  return {
    async open(req) {
      if (live.size >= options.max) throw new Error('Every browser session is busy');
      const device = DEVICES[req.deviceId ?? ''] ?? DEVICES['chrome.laptop_large'];
      const blocked: string[] = [];
      const proxy = await startProxy({ allowPrivate: options.allowPrivate, onBlocked: (_t, reason) => blocked.push(reason) });
      const context = await browser.newContext({
        proxy: { server: `http://${proxy.host}:${proxy.port}`, bypass: '<-loopback>' },
        viewport: { width: device.width, height: device.height },
        isMobile: device.isMobile,
        userAgent: userAgent(browser.version()),
        serviceWorkers: 'block',
        acceptDownloads: false,
      });
      const page = await context.newPage();
      const id = randomUUID();
      const s: Session = { context, page, proxy, allowedHosts: req.allowedHosts?.length ? req.allowedHosts : null, requests: [], blocked, touched: Date.now() };
      page.on('response', (r) => {
        const kind = r.request().resourceType();
        if ((kind === 'fetch' || kind === 'xhr') && s.requests.length < 30) s.requests.push({ method: r.request().method(), url: r.url(), status: r.status() });
      });
      live.set(id, s);
      const opened = await act(id, { type: 'goto', url: req.startUrl }, {});
      if (!opened.ok) {
        await end(id);
        throw new Error(opened.error ?? 'The start URL could not be opened');
      }
      return { ...opened, step: undefined, id };
    },
    act,
    end,
    async close() {
      clearInterval(sweep);
      for (const id of [...live.keys()]) await end(id);
      await browser.close();
    },
  };
}
