// AI Testing in mock mode: a job is a mock Assist run of kind `testing` (mock/assistRuns.ts) that plays browser
// actions over a few seconds; its proposals appear once it concludes. Accepting proposals creates mock synthetic tests
// and a suite (mock/synthetics.ts), as the server does (routes/ai_testing.rs).
import { cssElement } from '../synthetics/browserSteps';
import type { BrowserStep, SyntheticRunDetail } from '../synthetics/types';
import type { RunSummary } from '../assist/types';
import type { AiTestingAccepted, AiTestingDecision, AiTestingJobInput, AiTestingProfile, AiTestingProfileInput, AiTestingProposal, AiTestingProposalSearch } from '../aiTesting/types';
import { addTestingRun, getRun } from './assistRuns';
import { createSyntheticSuite, createSyntheticTest } from './synthetics';

const DAY = 86_400_000;
const DEVICES = ['chrome.laptop_large', 'chrome.tablet', 'chrome.mobile_small'];
const USAGES = ['username', 'email', 'password', 'otp', 'other'];

interface MockProposal extends AiTestingProposal {
  /** Shown once its job has concluded. */
  availableAt: number;
}

let proposals: MockProposal[] | null = null;
let profiles: AiTestingProfile[] = [];

const uuid = () => (globalThis.crypto?.randomUUID ? globalThis.crypto.randomUUID() : `id-${Math.random().toString(36).slice(2)}`);
const delay = <T>(v: T, ms = 150): Promise<T> => new Promise((r) => setTimeout(() => r(v), ms));
const fail = (message: string): never => {
  throw new Error(message);
};

function sample(startUrl: string, steps: BrowserStep[], passed: boolean): SyntheticRunDetail {
  const all = [{ name: `Navigate to ${startUrl}`, type: 'goToUrl' }, ...steps];
  const error = passed ? null : `Cannot locate element ${steps[steps.length - 1]?.params?.element?.userLocator?.values?.[0]?.value ?? ''}`;
  return {
    passed,
    error,
    durationMs: 900 + steps.length * 420,
    assertions: [],
    resolvedIp: null,
    method: null,
    url: startUrl,
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
    browser: {
      passed,
      durationMs: 900 + steps.length * 420,
      startUrl,
      browserVersion: '158.0.8127.18',
      device: { id: 'chrome.laptop_large', width: 1440, height: 1100, isMobile: false },
      error,
      failure: passed ? null : { code: 'LOCATE_ELEMENT', message: error! },
      hasVideo: false,
      steps: all.map((s, i) => ({
        name: s.name ?? '',
        type: s.type,
        status: passed || i < all.length - 1 ? 'passed' : 'failed',
        durationMs: i === 0 ? 900 : 420,
        url: startUrl,
        hasScreenshot: false,
        browserErrors: [],
        ...(passed || i < all.length - 1 ? {} : { error, failure: { code: 'LOCATE_ELEMENT', message: error! } }),
      })),
    },
  };
}

const click = (name: string, css: string): BrowserStep => ({ name, type: 'click', params: { element: cssElement(css) } });
const type = (name: string, css: string, value: string): BrowserStep => ({ name, type: 'typeText', params: { element: cssElement(css), value } });
const contains = (text: string): BrowserStep => ({ name: `Test page contains '${text}'`, type: 'assertPageContains', params: { value: text } });
const urlContains = (text: string): BrowserStep => ({ name: `Test current URL contains '${text}'`, type: 'assertCurrentUrl', params: { check: 'contains', value: text } });

interface Planned {
  suite: string;
  name: string;
  description: string;
  steps: BrowserStep[];
  passed: boolean;
}

/** What the agent finds on the fictional shop: journeys by what the job asked. */
function plan(prompt: string): Planned[] {
  const p = prompt.toLowerCase();
  if (/contact|form/.test(p)) {
    return [
      {
        suite: 'Contact',
        name: 'Fill in the contact form',
        description: 'A visitor fills in the contact form without sending it',
        steps: [click('Open Contact', 'a[href="/contact"]'), type('Type the name', '#name', 'QA Visitor'), type('Type the email', '#email', 'qa@example.com'), contains('Send message')],
        passed: true,
      },
    ];
  }
  const out: Planned[] = [
    {
      suite: 'Search',
      name: 'Search the catalog',
      description: 'A shopper searches for a product and sees results',
      steps: [type('Type the search', 'input[name="q"]', 'mug'), { name: 'Press Enter', type: 'pressKey', params: { value: 'Enter' } }, contains('Results for')],
      passed: true,
    },
    {
      suite: 'Search',
      name: 'Open a product from the results',
      description: 'A shopper opens a product page from the search results',
      steps: [
        type('Type the search', 'input[name="q"]', 'mug'),
        { name: 'Press Enter', type: 'pressKey', params: { value: 'Enter' } },
        click('Open Blue mug', "//a[normalize-space()='Blue mug']"),
        urlContains('/product'),
      ],
      passed: true,
    },
  ];
  if (!/navigat/.test(p)) {
    out.push({
      suite: 'Cart',
      name: 'Add a product to the cart',
      description: 'A shopper adds a product and sees it in the cart',
      steps: [click('Open Blue mug', "//a[normalize-space()='Blue mug']"), click('Add to cart', '[data-testid="add-to-cart"]'), click('Open the cart', '#cart-link')],
      passed: false,
    });
  }
  return out;
}

/** The run's browser actions, as the agent would take them. */
function actions(startUrl: string, planned: Planned[]): ([string, Record<string, unknown>, string] | string)[] {
  const out: ([string, Record<string, unknown>, string] | string)[] = [`Opening ${startUrl} to see what a visitor can do there.`, ['browser_snapshot', {}, 'Example Shop: 24 elements']];
  let n = 0;
  for (const t of planned) {
    for (const s of t.steps) {
      n++;
      const tool = s.type === 'typeText' ? 'browser_type' : s.type === 'pressKey' ? 'browser_press' : s.type.startsWith('assert') ? 'browser_check' : 'browser_click';
      out.push([tool, { ref: `e${(n % 20) + 1}` }, `Action #${n} recorded: ${s.name}`]);
    }
    out.push([
      'propose_test',
      { suite: t.suite, name: t.name },
      t.passed ? `Proposed "${t.name}". Its sample run passed.` : `Proposed "${t.name}". Its sample run failed: Cannot locate element #cart-link.`,
    ]);
  }
  return out;
}

function addJob(input: AiTestingJobInput, ago: number): { run: string; doneAt: number } {
  const host = new URL(input.startUrl).host;
  const title = input.prompt?.trim().split('\n')[0]?.slice(0, 120) || `Explore ${host}`;
  const planned = plan(input.prompt ?? '');
  const suites = [...new Set(planned.map((t) => t.suite))];
  const { id, doneAt } = addTestingRun(
    title,
    { startUrl: input.startUrl, restrictDomain: input.restrictDomain ?? true, deviceId: input.deviceId || DEVICES[0], prompt: input.prompt ?? '', profileId: input.profileId ?? null },
    actions(input.startUrl, planned),
    {
      summary: `Explored **${host}** and proposed ${planned.length} test${planned.length === 1 ? '' : 's'} in ${suites.length} suite${suites.length === 1 ? '' : 's'}: ${suites.join(', ')}.`,
      verdict: 'tests_proposed',
      confidence: 'high',
      evidence: [],
      nextSteps: ['Review the suites and start testing the ones you want'],
    },
    ago,
    ago ? 4 * 60_000 : undefined,
  );
  for (const t of planned) {
    db().unshift({
      id: uuid(),
      runId: id,
      suite: t.suite,
      name: t.name,
      description: t.description,
      startUrl: input.startUrl,
      deviceId: input.deviceId || DEVICES[0],
      steps: t.steps,
      verification: sample(input.startUrl, t.steps, t.passed),
      status: 'pending',
      testId: null,
      feedback: '',
      createdAt: doneAt,
      decidedAt: null,
      availableAt: doneAt,
    });
  }
  return { run: id, doneAt };
}

function db(): MockProposal[] {
  if (proposals) return proposals;
  proposals = [];
  addJob({ startUrl: 'https://shop.example.com/', restrictDomain: true, deviceId: DEVICES[0], prompt: '' }, 2 * DAY);
  profiles = [
    {
      id: uuid(),
      name: 'QA shopper',
      variables: [
        { usage: 'email', variable: 'SHOP_USER' },
        { usage: 'password', variable: 'CHECKOUT_API_TOKEN' },
      ],
      createdAt: Date.now() - 5 * DAY,
      updatedAt: Date.now() - 5 * DAY,
    },
  ];
  return proposals;
}

const view = ({ availableAt: _a, ...p }: MockProposal): AiTestingProposal => p;

export async function startAiTestingJob(input: AiTestingJobInput): Promise<RunSummary> {
  db();
  const url = input.startUrl.trim();
  if (!/^https?:\/\/[^/\s]+/.test(url)) fail("Enter the site's http or https starting URL, such as https://staging.shop.example.com");
  if (input.deviceId && !DEVICES.includes(input.deviceId)) fail(`Unknown device ${input.deviceId}`);
  if (input.profileId && !profiles.some((p) => p.id === input.profileId)) fail('Agent profile not found');
  const { run } = addJob({ ...input, startUrl: url }, 0);
  const { steps: _s, conclusion: _c, change: _ch, pullRequest: _p, ...summary } = await getRun(run);
  return delay(summary);
}

export async function searchAiTestingProposals(q: AiTestingProposalSearch): Promise<AiTestingProposal[]> {
  const now = Date.now();
  return delay(
    db()
      .filter((p) => p.availableAt <= now && (!q.status || p.status === q.status) && (!q.runId || p.runId === q.runId))
      .map(view),
  );
}

function pending(ids: string[]): MockProposal[] {
  const rows = db().filter((p) => ids.includes(p.id) && p.status === 'pending' && p.availableAt <= Date.now());
  return rows.length ? rows : fail('No pending proposals among them');
}

export async function acceptAiTestingProposals(d: AiTestingDecision): Promise<AiTestingAccepted> {
  const rows = pending(d.ids);
  const groups = new Map<string, string[]>();
  const testIds: string[] = [];
  for (const p of rows) {
    const test = await createSyntheticTest({
      name: p.name,
      type: 'browser',
      request: {
        method: 'GET',
        url: p.startUrl,
        host: '',
        port: null,
        headers: [],
        body: '',
        bodyType: '',
        basicAuth: null,
        timeoutSeconds: 30,
        followRedirects: true,
        acceptSelfSigned: false,
        dnsServer: '',
        dnsServerPort: null,
        deviceId: p.deviceId,
      },
      steps: p.steps,
      frequencySeconds: 900,
      tags: ['source:ai-testing'],
      message: '{{#is_alert}}The test is failing.{{/is_alert}}\n{{#is_recovery}}The test passes again.{{/is_recovery}}',
    });
    Object.assign(p, { status: 'accepted', testId: test.id, decidedAt: Date.now() });
    testIds.push(test.id);
    const key = `${p.runId}\n${p.suite}`;
    groups.set(key, [...(groups.get(key) ?? []), test.id]);
  }
  const suiteIds: string[] = [];
  for (const [key, ids] of groups) {
    const s = await createSyntheticSuite({ name: key.split('\n')[1], description: 'Proposed by AI Testing', tags: ['source:ai-testing'], testIds: ids });
    suiteIds.push(s.id);
  }
  return delay({ testIds, suiteIds });
}

export async function rejectAiTestingProposals(d: AiTestingDecision): Promise<void> {
  for (const p of pending(d.ids)) Object.assign(p, { status: 'rejected', feedback: d.feedback ?? '', decidedAt: Date.now() });
  return delay(undefined);
}

function checkProfile(input: AiTestingProfileInput, id?: string): AiTestingProfileInput {
  const name = input.name.trim();
  if (!name || name.length > 100) fail("The profile's name is 1–100 characters");
  if (profiles.some((p) => p.name === name && p.id !== id)) fail(`An agent profile named ${name} exists`);
  const variables = (input.variables ?? []).map((v) => ({ usage: v.usage.trim().toLowerCase(), variable: v.variable.trim() }));
  if (!variables.length || variables.length > 10) fail('A profile lends 1–10 global variables');
  for (const v of variables) {
    if (!USAGES.includes(v.usage)) fail(`A variable's usage is one of ${USAGES.join(', ')}`);
    if (!/^[A-Z][A-Z0-9_]*$/.test(v.variable)) fail('Variable names are upper-case letters, digits and underscores, start with a letter');
  }
  return { name, variables };
}

export async function listAiTestingProfiles(): Promise<AiTestingProfile[]> {
  db();
  return delay([...profiles].sort((a, b) => a.name.localeCompare(b.name)));
}

export async function createAiTestingProfile(input: AiTestingProfileInput): Promise<AiTestingProfile> {
  db();
  const p = { id: uuid(), ...checkProfile(input), variables: checkProfile(input).variables ?? [], createdAt: Date.now(), updatedAt: Date.now() };
  profiles.push(p);
  return delay(p);
}

export async function updateAiTestingProfile(id: string, input: AiTestingProfileInput): Promise<AiTestingProfile> {
  db();
  const p = profiles.find((x) => x.id === id) ?? fail('Agent profile not found');
  Object.assign(p, checkProfile(input, id), { updatedAt: Date.now() });
  return delay(p);
}

export async function deleteAiTestingProfile(id: string): Promise<void> {
  db();
  if (!profiles.some((p) => p.id === id)) fail('Agent profile not found');
  profiles = profiles.filter((p) => p.id !== id);
  return delay(undefined);
}
