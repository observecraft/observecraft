// Runs a browser test: the start URL, then each step, in a fresh browser context whose every request goes through
// its own checking proxy (proxy.ts). Elements are found with the step's locators in order (the first visible match
// wins) and waited for up to the step's timeout; assertions are retried until they pass or time out. Each step
// reports its status, duration, URL, a JPEG screenshot, the page's errors and, when it fails, a Datadog failure code.
import { mkdtemp, readdir, readFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { chromium, errors, type Browser, type BrowserContext, type Locator as PwLocator, type Page } from 'playwright-core';
import { BLOCKED_HEADER, startProxy } from './proxy.ts';
import { compare, DEFAULT_STEP_TIMEOUT, fill, mask, type Element, type Step, type StepParams } from './steps.ts';

export interface Device {
  id: string;
  width: number;
  height: number;
  isMobile: boolean;
}

/** Datadog's default devices. */
export const DEVICES: Record<string, Device> = {
  'chrome.laptop_large': { id: 'chrome.laptop_large', width: 1440, height: 1100, isMobile: false },
  'chrome.tablet': { id: 'chrome.tablet', width: 768, height: 1020, isMobile: true },
  'chrome.mobile_small': { id: 'chrome.mobile_small', width: 320, height: 550, isMobile: true },
};

export type FailureCode =
  | 'LOCATE_ELEMENT'
  | 'ASSERTION_FAILURE'
  | 'ELEMENT_NOT_INTERACTABLE'
  | 'OPEN_URL'
  | 'STEP_TIMEOUT'
  | 'TEST_TIMEOUT'
  | 'EVALUATE_JAVASCRIPT'
  | 'EXTRACT_VARIABLE'
  | 'SELECT_OPTION'
  | 'PRESS_KEY'
  | 'UNKNOWN_VARIABLE'
  | 'INTERNAL_ERROR';

export interface BrowserError {
  type: 'js' | 'network';
  name: string;
  description: string;
  status?: number;
}

export interface StepResult {
  name: string;
  type: Step['type'];
  status: 'passed' | 'failed' | 'skipped';
  allowFailure: boolean;
  isCritical: boolean;
  durationMs: number;
  url: string;
  error?: string;
  failure?: { code: FailureCode; message: string };
  /** The extracted value (extract steps; masked when secure). */
  value?: string;
  /** JPEG, base64. */
  screenshot?: string;
  browserErrors: BrowserError[];
}

export interface RunRequest {
  startUrl: string;
  steps: Step[];
  /** Global variables, by name. */
  variables?: Record<string, string>;
  /** Values hidden in the results. */
  secrets?: string[];
  deviceId?: string;
  /** The whole run, at most (default 900 seconds). */
  timeoutSeconds?: number;
  /** When the run's video comes back (default: when it failed). */
  video?: 'never' | 'onFailure' | 'always';
}

export interface RunResult {
  passed: boolean;
  durationMs: number;
  startUrl: string;
  browserVersion: string;
  device: Device;
  error?: string;
  failure?: { code: FailureCode; message: string };
  steps: StepResult[];
  /** The run as a WebM video, base64 (see RunRequest.video). */
  video?: string;
}

export interface Runner {
  run: (request: RunRequest) => Promise<RunResult>;
  close: () => Promise<void>;
}

const MAX_RUN_SECONDS = 900;
const MAX_BROWSER_ERRORS = 20;
const POLL_MS = 200;

class StepError extends Error {
  readonly code: FailureCode;
  constructor(code: FailureCode, message: string) {
    super(message);
    this.code = code;
  }
}

const sleep = (ms: number) => new Promise((done) => setTimeout(done, ms));

function describe(el: Element): string {
  return el.description || el.userLocator.values.map((l) => l.value).join(' or ');
}

/**
 * The first element matching one of the locators, waited for until `deadline`: visible for steps that act on it or
 * check it's shown, merely in the page for steps that read it (an empty element has no size).
 */
async function locate(page: Page, el: Element, deadline: number, mustBeVisible = true): Promise<PwLocator> {
  for (;;) {
    for (const l of el.userLocator.values) {
      const candidate = page.locator(`${l.type}=${l.value}`).first();
      try {
        if (mustBeVisible ? await candidate.isVisible() : (await candidate.count()) > 0) return candidate;
      } catch (e) {
        throw new StepError('LOCATE_ELEMENT', `Invalid locator ${l.value}: ${(e as Error).message.split('\n')[0]}`);
      }
    }
    if (Date.now() >= deadline) throw new StepError('LOCATE_ELEMENT', `Cannot locate element ${describe(el)}`);
    await sleep(POLL_MS);
  }
}

/** Retries `attempt` (a failure message, or null when it passes) until it passes or `deadline`. */
async function until(deadline: number, code: FailureCode, attempt: () => Promise<string | null>): Promise<void> {
  for (;;) {
    let failure: string | null;
    let failCode = code;
    try {
      failure = await attempt();
    } catch (e) {
      failure = (e as Error).message;
      if (e instanceof StepError) failCode = e.code;
    }
    if (failure === null) return;
    if (Date.now() >= deadline) throw new StepError(failCode, failure);
    await sleep(POLL_MS);
  }
}

/** The element's text: an input's value, else its rendered text. */
function textOf(loc: PwLocator): Promise<string> {
  return loc.evaluate((e) => (e instanceof HTMLInputElement || e instanceof HTMLTextAreaElement || e instanceof HTMLSelectElement ? e.value : ((e as HTMLElement).innerText ?? e.textContent ?? '')));
}

/** Runs a step's JavaScript (a function body receiving `element`) and returns its result. */
async function runCode(page: Page, code: string, loc: PwLocator | null): Promise<unknown> {
  const fn = `async (element) => {\n${code}\n}`;
  try {
    return loc ? await loc.evaluate(fn) : await page.evaluate(`(${fn})(undefined)`);
  } catch (e) {
    throw new StepError('EVALUATE_JAVASCRIPT', `The JavaScript failed: ${(e as Error).message.split('\n')[0]}`);
  }
}

/** The step's parameters with `{{ VARIABLES }}` filled in. */
function filled(params: StepParams, variables: Record<string, string>): StepParams {
  const missing: string[] = [];
  const text = (s: string | undefined) => (s === undefined ? s : fill(s, variables, missing));
  const out: StepParams = {
    ...params,
    value: typeof params.value === 'string' ? text(params.value) : params.value,
    attribute: text(params.attribute),
    code: text(params.code),
    element: params.element && {
      ...params.element,
      userLocator: { ...params.element.userLocator, values: params.element.userLocator.values.map((l) => ({ ...l, value: fill(l.value, variables, missing) })) },
    },
  };
  if (missing.length) throw new StepError('UNKNOWN_VARIABLE', `Unknown variables: ${missing.join(', ')}`);
  return out;
}

function keyName(key: string, modifiers: string[] = []): string {
  return [...modifiers, key === 'Space' ? ' ' : key].join('+');
}

/** Runs one step; resolves with the extracted value, if any. */
async function execute(page: Page, step: Step, variables: Record<string, string>, secrets: string[]): Promise<string | undefined> {
  const timeoutMs = (step.timeout ?? DEFAULT_STEP_TIMEOUT) * 1000;
  const deadline = Date.now() + timeoutMs;
  const p = filled(step.params ?? {}, variables);
  const value = p.value === undefined ? '' : String(p.value);
  const element = (mustBeVisible = true) => {
    if (!p.element) throw new StepError('LOCATE_ELEMENT', 'The step has no element');
    return locate(page, p.element, deadline, mustBeVisible);
  };
  const remaining = () => Math.max(1, deadline - Date.now());
  const act = async (action: () => Promise<unknown>) => {
    try {
      await action();
    } catch (e) {
      if (e instanceof errors.TimeoutError) throw new StepError('ELEMENT_NOT_INTERACTABLE', `The element ${describe(p.element!)} could not be used: ${e.message.split('\n')[0]}`);
      throw e;
    }
  };
  switch (step.type) {
    case 'goToUrl':
    case 'refresh': {
      const response = step.type === 'goToUrl' ? await page.goto(value, { timeout: timeoutMs, waitUntil: 'load' }) : await page.reload({ timeout: timeoutMs, waitUntil: 'load' });
      if (response?.headers()[BLOCKED_HEADER]) throw new StepError('OPEN_URL', `Blocked: ${await response.text()}`);
      return;
    }
    case 'wait':
      await sleep(Number(value) * 1000);
      return;
    case 'click': {
      const loc = await element();
      const options = { timeout: remaining() };
      await act(() => (p.clickType === 'double' ? loc.dblclick(options) : loc.click({ ...options, button: p.clickType === 'contextual' ? 'right' : 'left' })));
      return;
    }
    case 'hover': {
      const loc = await element();
      await act(() => loc.hover({ timeout: remaining() }));
      return;
    }
    case 'typeText': {
      const loc = await element();
      await act(async () => {
        if (!p.appendToContent) await loc.fill('', { timeout: remaining() });
        await loc.pressSequentially(value, { timeout: remaining() });
      });
      return;
    }
    case 'selectOption': {
      const loc = await element();
      try {
        await loc.selectOption(value, { timeout: remaining() });
      } catch (e) {
        throw new StepError('SELECT_OPTION', `Cannot select '${value}': ${(e as Error).message.split('\n')[0]}`);
      }
      return;
    }
    case 'pressKey':
      try {
        await page.keyboard.press(keyName(value, p.modifiers));
      } catch (e) {
        throw new StepError('PRESS_KEY', `Cannot press ${value}: ${(e as Error).message.split('\n')[0]}`);
      }
      return;
    case 'scroll': {
      const by: [number, number] = [p.x ?? 0, p.y ?? 0];
      if (p.element) await (await element()).evaluate((e, [x, y]) => e.scrollBy(x, y), by);
      else await page.evaluate(([x, y]) => window.scrollBy(x, y), by);
      return;
    }
    case 'assertElementPresent':
      await element();
      return;
    case 'assertElementContent':
      await until(deadline, 'ASSERTION_FAILURE', async () => {
        const text = await textOf(await locate(page, p.element!, Date.now(), false));
        return compare(p.check!, text, value) ? null : `The element's content '${text}' doesn't ${p.check} '${value}'`;
      });
      return;
    case 'assertElementAttribute':
      await until(deadline, 'ASSERTION_FAILURE', async () => {
        const actual = (await (await locate(page, p.element!, Date.now(), false)).getAttribute(p.attribute!)) ?? '';
        return compare(p.check!, actual, value) ? null : `The attribute ${p.attribute} '${actual}' doesn't ${p.check} '${value}'`;
      });
      return;
    case 'assertPageContains':
    case 'assertPageLacks': {
      const wanted = step.type === 'assertPageContains';
      await until(deadline, 'ASSERTION_FAILURE', async () => {
        const text = await page.evaluate(() => document.body?.innerText ?? '');
        return text.includes(value) === wanted ? null : `The page ${wanted ? "doesn't contain" : 'contains'} '${value}'`;
      });
      return;
    }
    case 'assertCurrentUrl':
      await until(deadline, 'ASSERTION_FAILURE', async () => (compare(p.check!, page.url(), value) ? null : `The URL ${page.url()} doesn't ${p.check} '${value}'`));
      return;
    case 'assertFromJavascript':
      await until(deadline, 'ASSERTION_FAILURE', async () => {
        const loc = p.element ? await locate(page, p.element, Date.now(), false) : null;
        return (await runCode(page, p.code ?? '', loc)) ? null : 'The JavaScript assertion returned a falsy value';
      });
      return;
    case 'extractVariable':
    case 'extractFromJavascript': {
      const name = p.variable?.name ?? '';
      let extracted: string;
      try {
        if (step.type === 'extractVariable') extracted = (await textOf(await element(false))).trim();
        else {
          const loc = p.element ? await element(false) : null;
          const result = await runCode(page, p.code ?? '', loc);
          extracted = typeof result === 'string' ? result : JSON.stringify(result ?? '');
        }
      } catch (e) {
        if (e instanceof StepError && e.code === 'LOCATE_ELEMENT') throw e;
        throw new StepError('EXTRACT_VARIABLE', `Cannot extract ${name}: ${(e as Error).message}`);
      }
      variables[name] = extracted;
      if (p.variable?.secure) secrets.push(extracted);
      return extracted;
    }
  }
}

/** The failure code and message of an error thrown by a step. */
function failureOf(step: Step, e: unknown, blocked: string[]): { code: FailureCode; message: string } {
  if (e instanceof StepError) return { code: e.code, message: e.message };
  const message = (e as Error).message?.split('\n')[0] ?? String(e);
  if (step.type === 'goToUrl' || step.type === 'refresh') return { code: 'OPEN_URL', message: blocked.length ? `Blocked: ${blocked[blocked.length - 1]}` : message };
  if (e instanceof errors.TimeoutError) return { code: 'STEP_TIMEOUT', message };
  return { code: 'INTERNAL_ERROR', message };
}

async function screenshot(page: Page): Promise<string | undefined> {
  try {
    return (await page.screenshot({ type: 'jpeg', quality: 60, timeout: 5000 })).toString('base64');
  } catch {
    return undefined;
  }
}

export function userAgent(version: string): string {
  return `Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/${version} Safari/537.36 ObserveCraftSynthetics`;
}

async function runInContext(browser: Browser, context: BrowserContext, request: RunRequest, device: Device, blocked: string[]): Promise<RunResult> {
  const started = Date.now();
  const deadline = started + Math.min(request.timeoutSeconds ?? MAX_RUN_SECONDS, MAX_RUN_SECONDS) * 1000;
  const variables = { ...(request.variables ?? {}) };
  const secrets = [...(request.secrets ?? [])];
  const page = await context.newPage();
  let errorsNow: BrowserError[] = [];
  const note = (e: BrowserError) => errorsNow.length < MAX_BROWSER_ERRORS && errorsNow.push(e);
  page.on('pageerror', (e) => note({ type: 'js', name: e.name || 'Error', description: e.message }));
  page.on('console', (m) => m.type() === 'error' && note({ type: 'js', name: 'console', description: m.text() }));
  page.on('requestfailed', (r) => note({ type: 'network', name: r.url(), description: r.failure()?.errorText ?? 'failed' }));
  page.on('response', (r) => r.status() >= 400 && note({ type: 'network', name: r.url(), description: `${r.status()} ${r.statusText()}`, status: r.status() }));

  const all: Step[] = [{ name: `Navigate to ${request.startUrl}`, type: 'goToUrl', params: { value: request.startUrl } }, ...request.steps];
  const results: StepResult[] = [];
  let passed = true;
  let failure: RunResult['failure'];
  let stopped = false;
  for (const step of all) {
    const base = { name: step.name, type: step.type, allowFailure: !!step.allowFailure, isCritical: !!step.isCritical, browserErrors: [] as BrowserError[] };
    if (stopped) {
      results.push({ ...base, status: 'skipped', durationMs: 0, url: page.url() });
      continue;
    }
    if (Date.now() >= deadline) {
      failure ??= { code: 'TEST_TIMEOUT', message: 'The test ran out of time' };
      passed = false;
      stopped = true;
      results.push({ ...base, status: 'skipped', durationMs: 0, url: page.url() });
      continue;
    }
    errorsNow = [];
    const before = blocked.length;
    const stepStarted = Date.now();
    const result: StepResult = { ...base, status: 'passed', durationMs: 0, url: '' };
    try {
      result.value = await execute(page, step, variables, secrets);
    } catch (e) {
      const f = failureOf(step, e, blocked.slice(before));
      result.status = 'failed';
      result.failure = f;
      result.error = f.message;
      if (!step.allowFailure || step.isCritical) {
        passed = false;
        failure ??= f;
      }
      if (!step.allowFailure) stopped = true;
    }
    result.durationMs = Date.now() - stepStarted;
    result.url = page.url();
    result.browserErrors = errorsNow;
    if (!step.noScreenshot) result.screenshot = await screenshot(page);
    results.push(result);
  }
  await page.close().catch(() => {});

  // Secure values (given or extracted) never leave in clear.
  const hide = (s: string | undefined) => (s === undefined ? s : mask(s, secrets));
  for (const r of results) {
    r.name = hide(r.name)!;
    r.url = hide(r.url)!;
    r.error = hide(r.error);
    r.value = hide(r.value);
    if (r.failure) r.failure.message = hide(r.failure.message)!;
    r.browserErrors = r.browserErrors.map((b) => ({ ...b, name: hide(b.name)!, description: hide(b.description)! }));
  }
  return {
    passed,
    durationMs: Date.now() - started,
    startUrl: hide(request.startUrl)!,
    browserVersion: browser.version(),
    device,
    error: hide(failure?.message),
    failure: failure && { ...failure, message: hide(failure.message)! },
    steps: results,
  };
}

/** The context's video (one page per run), base64. */
async function readVideo(dir: string): Promise<string | undefined> {
  const file = (await readdir(dir)).find((f) => f.endsWith('.webm'));
  return file ? (await readFile(join(dir, file))).toString('base64') : undefined;
}

/** Launches Chromium; each run gets its own context and checking proxy. */
export async function createRunner(options: { allowPrivate: boolean; executablePath?: string }): Promise<Runner> {
  const browser = await chromium.launch({
    executablePath: options.executablePath,
    // WebRTC could reach the network around the proxy.
    args: ['--force-webrtc-ip-handling-policy=disable_non_proxied_udp', '--webrtc-ip-handling-policy=disable_non_proxied_udp'],
  });
  return {
    async run(request) {
      const device = DEVICES[request.deviceId ?? ''] ?? DEVICES['chrome.laptop_large'];
      const blocked: string[] = [];
      const proxy = await startProxy({ allowPrivate: options.allowPrivate, onBlocked: (_target, reason) => blocked.push(reason) });
      const videoMode = request.video ?? 'onFailure';
      const videoDir = videoMode === 'never' ? undefined : await mkdtemp(join(tmpdir(), 'oc-browser-'));
      const context = await browser.newContext({
        // `<-loopback>` sends loopback addresses through the proxy too (Chromium bypasses it for them by default).
        proxy: { server: `http://${proxy.host}:${proxy.port}`, bypass: '<-loopback>' },
        viewport: { width: device.width, height: device.height },
        isMobile: device.isMobile,
        userAgent: userAgent(browser.version()),
        serviceWorkers: 'block',
        acceptDownloads: false,
        recordVideo: videoDir ? { dir: videoDir, size: { width: Math.round(device.width / 2), height: Math.round(device.height / 2) } } : undefined,
      });
      try {
        const result = await runInContext(browser, context, request, device, blocked);
        // The video is complete once the context is closed.
        await context.close();
        if (videoDir && (videoMode === 'always' || !result.passed)) result.video = await readVideo(videoDir);
        return result;
      } finally {
        await context.close().catch(() => {});
        await proxy.close();
        if (videoDir) await rm(videoDir, { recursive: true, force: true });
      }
    },
    close: () => browser.close(),
  };
}
