// Browser test runs in mock mode: each step "runs" against a fictional shop, failing where an outage says so,
// with screenshots drawn on a canvas (a sketch of the page and the step) instead of a real browser's.
import { stepName } from '../synthetics/browserSteps';
import type { BrowserRunResult, BrowserStep, BrowserStepResult, SyntheticRunDetail } from '../synthetics/types';

const DEVICE_SIZE: Record<string, [number, number]> = { 'chrome.laptop_large': [1440, 1100], 'chrome.tablet': [768, 1020], 'chrome.mobile_small': [320, 550] };

/** A stable pseudo-random number in [0, 1). */
function noise(seed: number, n: number): number {
  let h = (Math.imul(seed ^ n, 2_654_435_761) + n * 97) >>> 0;
  h = Math.imul(h ^ (h >>> 15), 2_246_822_507) >>> 0;
  return ((h ^ (h >>> 13)) >>> 0) / 4_294_967_296;
}

function emptyDetail(): SyntheticRunDetail {
  return {
    passed: false,
    error: null,
    durationMs: 0,
    assertions: [],
    resolvedIp: null,
    method: null,
    url: null,
    redirects: [],
    statusCode: null,
    responseHeaders: [],
    body: null,
    bodySize: null,
    bodyTruncated: false,
    certificate: null,
    dnsServer: null,
    records: [],
    timings: null,
    httpVersion: null,
    browser: null,
  };
}

/** The step that breaks during an outage: the first assertion, else the last step. */
function breakingStep(steps: BrowserStep[]): number {
  const i = steps.findIndex((s) => s.type.startsWith('assert'));
  return (i >= 0 ? i : steps.length - 1) + 1;
}

/** A browser run of `steps` from `startUrl`; `failing` breaks one step as an outage would. */
export function browserRun(startUrl: string, deviceId: string, steps: BrowserStep[], seed: number, failing: boolean, baseMs: number, keepVideo: boolean): SyntheticRunDetail {
  const [width, height] = DEVICE_SIZE[deviceId] ?? DEVICE_SIZE['chrome.laptop_large'];
  const all: BrowserStep[] = [{ name: `Navigate to ${startUrl}`, type: 'goToUrl', params: { value: startUrl } }, ...steps];
  const breaks = failing ? breakingStep(steps) : -1;
  const host = (() => {
    try {
      return new URL(startUrl).host;
    } catch {
      return '';
    }
  })();
  let url = startUrl;
  let stopped = false;
  let passed = true;
  let error: string | null = null;
  let failure: BrowserRunResult['failure'] = null;
  const results: BrowserStepResult[] = all.map((s, i) => {
    const base = { name: stepName(s), type: s.type, allowFailure: !!s.allowFailure, isCritical: !!s.isCritical, browserErrors: [], hasScreenshot: false };
    if (stopped) return { ...base, status: 'skipped', durationMs: 0, url };
    if (s.type === 'goToUrl') url = String(s.params?.value ?? url);
    const durationMs = Math.round((i === 0 ? baseMs * 4 : baseMs) * (0.6 + noise(seed, i) * 0.9));
    if (i === breaks) {
      const code = s.type.startsWith('assert') ? 'ASSERTION_FAILURE' : 'LOCATE_ELEMENT';
      const message = code === 'ASSERTION_FAILURE' ? `${stepName(s)}: the page shows "Something went wrong"` : `Cannot locate element ${s.params?.element?.userLocator?.values?.[0]?.value ?? ''}`;
      if (!s.allowFailure || s.isCritical) {
        passed = false;
        failure ??= { code, message };
        error ??= message;
      }
      if (!s.allowFailure) stopped = true;
      const browserErrors = [{ type: 'network', name: `https://${host}/api/cart`, description: '503 Service Unavailable', status: 503 }];
      return { ...base, status: 'failed', durationMs, url, error: message, failure: { code, message }, browserErrors, hasScreenshot: true };
    }
    const value = s.type === 'extractVariable' || s.type === 'extractFromJavascript' ? 'ORD-48213' : null;
    return { ...base, status: 'passed', durationMs, url, value, hasScreenshot: !s.noScreenshot };
  });
  const durationMs = results.reduce((sum, r) => sum + r.durationMs, 0);
  const browser: BrowserRunResult = {
    passed,
    durationMs,
    startUrl,
    browserVersion: '158.0.8127.18',
    device: { id: deviceId || 'chrome.laptop_large', width, height, isMobile: width < 1000 },
    error,
    failure,
    steps: results,
    hasVideo: false,
  };
  void keepVideo;
  return { ...emptyDetail(), passed, error, durationMs, url, browser };
}

/** A run that couldn't start (an unknown variable in the start URL). */
export function browserRunError(error: string): SyntheticRunDetail {
  return { ...emptyDetail(), error };
}

const shots = new Map<string, string>();

/** A JPEG data URL sketching the page at a step: the address bar, a header and the step's caption. */
export function mockScreenshot(key: string, step: Pick<BrowserStepResult, 'name' | 'url' | 'status'>, index: number): string {
  const cached = shots.get(key);
  if (cached) return cached;
  const canvas = document.createElement('canvas');
  canvas.width = 720;
  canvas.height = 450;
  const g = canvas.getContext('2d');
  if (!g) return '';
  g.fillStyle = '#f6f7f9';
  g.fillRect(0, 0, 720, 450);
  g.fillStyle = '#e3e6ea';
  g.fillRect(0, 0, 720, 36);
  g.fillStyle = '#ffffff';
  g.fillRect(70, 8, 580, 20);
  g.fillStyle = '#5b6470';
  g.font = '12px system-ui, sans-serif';
  g.fillText((step.url ?? '').slice(0, 90), 80, 22);
  g.fillStyle = '#2f3a4a';
  g.fillRect(0, 36, 720, 52);
  g.fillStyle = '#ffffff';
  g.font = 'bold 18px system-ui, sans-serif';
  g.fillText('Example Shop', 24, 68);
  for (let i = 0; i < 6; i++) {
    g.fillStyle = '#ffffff';
    g.fillRect(24 + (i % 3) * 228, 110 + Math.floor(i / 3) * 140, 212, 120);
    g.fillStyle = '#d5dae1';
    g.fillRect(36 + (i % 3) * 228, 122 + Math.floor(i / 3) * 140, 188, 60);
  }
  g.fillStyle = step.status === 'failed' ? 'rgba(214, 48, 49, 0.9)' : 'rgba(47, 58, 74, 0.85)';
  g.fillRect(0, 404, 720, 46);
  g.fillStyle = '#ffffff';
  g.font = '15px system-ui, sans-serif';
  g.fillText(`${index}. ${step.name}`.slice(0, 80), 18, 432);
  const url = canvas.toDataURL('image/jpeg', 0.7);
  shots.set(key, url);
  return url;
}
