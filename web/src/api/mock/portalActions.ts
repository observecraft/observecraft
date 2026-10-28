// In-browser self-service actions for mock mode (Developer Portal → Actions). State lives in memory
// for the session. Validation follows the server (routes/portal_actions.rs and
// routes/portal_action_steps.rs, mirrored in portalActions/logic.ts); runs render every template like
// the server would, then answer with a fictional response instead of calling GitHub, GitLab or Jira.
import { checkInputs, checkRunInputs, checkStep, mergeSecrets, redact, renderJson, renderText, renderUrl, resultLink, type RenderContext } from '../portalActions/logic';
import type { ActionStep, ActionValues, PortalAction, PortalActionInput, PortalActionRun } from '../portalActions/types';

const ME = { name: 'Alex Morgan', email: 'alex.morgan@example.com' };
const HOUR = 3_600_000;
const MAX_ACTIONS = 500;

type Stored = Omit<PortalAction, 'secretNames' | 'runs' | 'lastRunStatus' | 'lastRunAt' | 'canRun'> & { secrets: Record<string, string> };

let actions: Stored[] | null = null;
let runs: PortalActionRun[] = [];
let nextRun = 1;
let nextIncident = 41;

const delay = <T>(v: T): Promise<T> => new Promise((r) => setTimeout(() => r(structuredClone(v)), 120));
const fail = (msg: string): never => {
  throw new Error(msg);
};
const uuid = () => (globalThis.crypto?.randomUUID ? globalThis.crypto.randomUUID() : `00000000-0000-4000-8000-${String(Date.now()).padStart(12, '0').slice(-12)}`);

function seed(): Stored[] {
  if (actions) return actions;
  const now = Date.now();
  const gh = [
    { name: 'Authorization', value: 'Bearer {{secrets.github_token}}' },
    { name: 'Accept', value: 'application/vnd.github+json' },
  ];
  actions = [
    {
      id: '6c1f2a10-3b4d-4e5f-8a9b-0c1d2e3f4a01',
      name: 'Scaffold a storefront service',
      description: 'New repository from example-org/service-template with CI, Dockerfile and ownership files.',
      category: 'scaffolding',
      blueprint: 'github_scaffold',
      ownerTeam: 'platform',
      inputs: [
        { key: 'name', label: 'Repository name', kind: 'text', required: true, options: [], default: '', description: 'Lowercase, with dashes: storefront-search' },
        { key: 'private', label: 'Private repository', kind: 'boolean', required: false, options: [], default: 'true', description: '' },
      ],
      step: {
        kind: 'http',
        requests: [
          {
            method: 'POST',
            url: 'https://api.github.com/repos/example-org/service-template/generate',
            headers: gh,
            body: '{"owner": "example-org", "name": "{{inputs.name}}", "private": "{{inputs.private}}"}',
          },
        ],
        fields: {},
      },
      secrets: { github_token: 'ghp_mockTokenForExampleOrg0001' },
      allowedTeams: [],
      createdBy: 'Priya Natarajan',
      createdAt: now - 20 * 24 * HOUR,
      updatedAt: now - 3 * 24 * HOUR,
    },
    {
      id: '6c1f2a10-3b4d-4e5f-8a9b-0c1d2e3f4a02',
      name: 'Deploy order-service',
      description: 'Runs the deploy workflow of example-org/order-service for an environment.',
      category: 'release_management',
      blueprint: 'github_workflow',
      ownerTeam: 'checkout',
      inputs: [
        { key: 'ref', label: 'Branch or tag', kind: 'text', required: true, options: [], default: 'main', description: '' },
        { key: 'environment', label: 'Environment', kind: 'select', required: true, options: ['staging', 'production'], default: 'staging', description: '' },
      ],
      step: {
        kind: 'http',
        requests: [
          {
            method: 'POST',
            url: 'https://api.github.com/repos/example-org/order-service/actions/workflows/deploy.yml/dispatches',
            headers: gh,
            body: '{"ref": "{{inputs.ref}}", "inputs": {"environment": "{{inputs.environment}}"}}',
          },
        ],
        fields: {},
      },
      secrets: { github_token: 'ghp_mockTokenForExampleOrg0002' },
      allowedTeams: ['checkout', 'platform'],
      createdBy: 'Marcus Chen',
      createdAt: now - 12 * 24 * HOUR,
      updatedAt: now - 12 * 24 * HOUR,
    },
    {
      id: '6c1f2a10-3b4d-4e5f-8a9b-0c1d2e3f4a03',
      name: 'Declare a checkout incident',
      description: 'Declares an incident owned by @checkout with the severity you choose.',
      category: 'incident_management',
      blueprint: 'declare_incident',
      ownerTeam: 'checkout',
      inputs: [
        { key: 'title', label: 'What is happening?', kind: 'text', required: true, options: [], default: '', description: '' },
        { key: 'severity', label: 'Severity', kind: 'select', required: true, options: ['SEV-1', 'SEV-2', 'SEV-3'], default: 'SEV-3', description: '' },
      ],
      step: { kind: 'create_incident', requests: [], fields: { title: '{{inputs.title}}', severity: '{{inputs.severity}}', teams: 'checkout' } },
      secrets: {},
      allowedTeams: [],
      createdBy: 'Alex Morgan',
      createdAt: now - 5 * 24 * HOUR,
      updatedAt: now - 5 * 24 * HOUR,
    },
  ];
  runs = [
    run(actions[0], 'Dana Lee', { name: 'storefront-search', private: true }, 201, 'https://github.com/example-org/storefront-search', now - 2 * 24 * HOUR, 1840),
    run(actions[0], 'Priya Natarajan', { name: 'catalog-indexer', private: true }, 201, 'https://github.com/example-org/catalog-indexer', now - 6 * 24 * HOUR, 2210),
    run(actions[1], 'Marcus Chen', { ref: 'main', environment: 'staging' }, 204, '', now - 4 * HOUR, 640),
    { ...run(actions[1], 'Marcus Chen', { ref: 'release-2.14', environment: 'production' }, 422, '', now - 26 * HOUR, 410), status: 'failed', error: 'POST api.github.com answered 422' },
  ];
  return actions;
}

function run(a: Stored, userName: string, inputs: ActionValues, statusCode: number | null, link: string, createdAt: number, durationMs: number): PortalActionRun {
  return {
    id: nextRun++,
    actionId: a.id,
    userName,
    inputs,
    status: 'succeeded',
    statusCode,
    durationMs,
    responseExcerpt: link ? JSON.stringify({ html_url: link }) : '',
    error: null,
    link,
    createdAt,
  };
}

function view(a: Stored): PortalAction {
  const mine = runs.filter((r) => r.actionId === a.id).sort((x, y) => y.createdAt - x.createdAt);
  const { secrets, ...rest } = a;
  return {
    ...rest,
    secretNames: Object.keys(secrets).sort(),
    runs: mine.length,
    lastRunStatus: mine[0]?.status ?? null,
    lastRunAt: mine[0]?.createdAt ?? null,
    // The mock user is an admin.
    canRun: true,
  };
}

function find(id: string): Stored {
  return seed().find((a) => a.id === id) ?? fail('Action not found');
}

function teams(list: string[] | undefined): string[] {
  const out: string[] = [];
  for (const raw of list ?? []) {
    const t = raw.trim().replace(/^@/, '');
    if (!/^[A-Za-z0-9._-]{1,100}$/.test(t)) fail(`${t} isn't a team handle`);
    if (!out.includes(t)) out.push(t);
  }
  return out;
}

function checked(input: PortalActionInput, current: Record<string, string>) {
  const name = input.name.trim();
  if (!name) fail('The name is required');
  if ([...name].length > 200) fail('The name is longer than 200 characters');
  const ownerTeam = (input.ownerTeam ?? '').trim().replace(/^@/, '');
  if (ownerTeam && !/^[A-Za-z0-9._-]{1,100}$/.test(ownerTeam)) fail(`${ownerTeam} isn't a team handle`);
  const inputs = checkInputs(input.inputs ?? []);
  const secrets = mergeSecrets(current, input.secrets ?? {});
  if (input.step.kind !== 'http' && Object.keys(secrets).length) fail('Secrets are only used by HTTP requests');
  const step = checkStep(input.step, inputs, new Set(Object.keys(secrets)));
  return {
    name,
    description: (input.description ?? '').trim(),
    category: input.category,
    blueprint: (input.blueprint ?? '').trim(),
    ownerTeam,
    inputs,
    step,
    secrets,
    allowedTeams: teams(input.allowedTeams),
  };
}

export async function listActions(): Promise<PortalAction[]> {
  return delay(
    seed()
      .map(view)
      .sort((a, b) => a.name.localeCompare(b.name)),
  );
}

export async function getAction(id: string): Promise<PortalAction> {
  return delay(view(find(id)));
}

export async function createAction(input: PortalActionInput): Promise<PortalAction> {
  const list = seed();
  const c = checked(input, {});
  if (list.length >= MAX_ACTIONS) fail(`An organization has at most ${MAX_ACTIONS} actions`);
  if (list.some((a) => a.name === c.name)) fail(`An action named ${c.name} exists already`);
  const now = Date.now();
  const a: Stored = { id: uuid(), ...c, createdBy: ME.name, createdAt: now, updatedAt: now };
  list.push(a);
  return delay(view(a));
}

export async function updateAction(id: string, input: PortalActionInput): Promise<PortalAction> {
  const a = find(id);
  const c = checked(input, a.secrets);
  if (seed().some((o) => o.id !== id && o.name === c.name)) fail(`An action named ${c.name} exists already`);
  Object.assign(a, c, { updatedAt: Date.now() });
  return delay(view(a));
}

export async function deleteAction(id: string): Promise<void> {
  find(id);
  actions = seed().filter((a) => a.id !== id);
  runs = runs.filter((r) => r.actionId !== id);
  await delay(null);
}

export async function listRuns(id: string): Promise<PortalActionRun[]> {
  find(id);
  return delay(runs.filter((r) => r.actionId === id).sort((a, b) => b.createdAt - a.createdAt || b.id - a.id));
}

/** A fictional answer for a request, shaped like the service it names. */
function fakeResponse(method: string, url: string, body: string): { status: number; json: unknown } {
  const u = new URL(url);
  const sent = body ? (JSON.parse(body) as Record<string, unknown>) : {};
  const path = decodeURIComponent(u.pathname);
  if (u.hostname === 'api.github.com') {
    if (path.endsWith('/generate')) return { status: 201, json: { full_name: `${sent.owner}/${sent.name}`, html_url: `https://github.com/${sent.owner}/${sent.name}` } };
    if (path.endsWith('/dispatches')) return { status: 204, json: null };
    if (method === 'GET' && path.includes('/git/ref/')) return { status: 200, json: { ref: path.split('/git/')[1], object: { sha: '9f2c4e1a7b3d5f60a8c2e4b6d8f0a1c3e5b7d9f1', type: 'commit' } } };
    if (path.endsWith('/git/refs')) return { status: 201, json: { ref: sent.ref, object: { sha: sent.sha } } };
    if (path.includes('/contents/')) return { status: 201, json: { content: { path: path.split('/contents/')[1] }, commit: { sha: '1b3d5f7a9c2e4b6d8f0a1c3e5b7d9f2c4e6a8b0d' } } };
    if (path.endsWith('/pulls')) return { status: 201, json: { number: 128, html_url: `https://github.com${path.replace('/repos', '').replace('/pulls', '/pull/128')}` } };
  }
  if (u.hostname.includes('gitlab')) {
    if (path.endsWith('/pipeline'))
      return { status: 201, json: { id: 90412, status: 'created', web_url: `https://gitlab.com/${path.split('/projects/')[1].replace('/pipeline', '')}/-/pipelines/90412` } };
    if (path.endsWith('/projects')) return { status: 201, json: { id: 5531, web_url: `https://gitlab.com/example-org/${sent.name}` } };
  }
  if (path.endsWith('/rest/api/3/issue'))
    return { status: 201, json: { id: '10412', key: `${(sent.fields as { project?: { key?: string } })?.project?.key ?? 'SHOP'}-412`, self: `${u.origin}/rest/api/3/issue/10412` } };
  return { status: 200, json: { ok: true } };
}

function internalRun(step: ActionStep, ctx: RenderContext): { response: string; link: string } {
  const field = (name: string) => renderText(step.fields?.[name] ?? '', ctx, false).trim();
  switch (step.kind) {
    case 'create_incident': {
      const id = nextIncident++;
      return { response: `Declared incident IR-${id}: ${field('title')}`, link: `/incidents/${id}` };
    }
    case 'page_on_call':
      return { response: `Paged @${field('team').replace(/^@/, '')}: ${field('title')}`, link: `/on-call/pages/${700 + nextRun}` };
    case 'incident_comment': {
      const raw = field('incident');
      const id = Number(raw.replace(/^#/, '').replace(/^IR-/i, ''));
      if (!Number.isInteger(id) || id <= 0) throw new Error(`${raw} isn't an incident number`);
      return { response: `Added a note to incident IR-${id}`, link: `/incidents/${id}` };
    }
    case 'synthetic_run':
      return { response: 'The synthetic test passed', link: `/synthetics/tests/${field('test')}` };
    default:
      throw new Error('Not an internal step');
  }
}

export async function runAction(id: string, given: ActionValues): Promise<PortalActionRun> {
  const a = find(id);
  const inputs = checkRunInputs(a.inputs, given);
  const ctx: RenderContext = { inputs, userEmail: ME.email, userName: ME.name, secrets: a.secrets, steps: [] };
  const started = performance.now();
  let status: 'succeeded' | 'failed' = 'succeeded';
  let statusCode: number | null = null;
  let response = '';
  let error: string | null = null;
  let link = '';
  try {
    if (a.step.kind === 'http') {
      const requests = a.step.requests ?? [];
      requests.forEach((r, i) => {
        const which = requests.length > 1 ? `Request ${i + 1}: ` : '';
        try {
          const url = renderUrl(r.url, ctx);
          for (const h of r.headers ?? []) renderText(h.value, ctx, true);
          const body = renderJson(r.body ?? '', ctx);
          const res = fakeResponse(r.method, url, body);
          statusCode = res.status;
          response = res.json === null ? '' : JSON.stringify(res.json);
          link = resultLink(res.json) ?? link;
          ctx.steps.push(res.json);
        } catch (e) {
          throw new Error(which + (e as Error).message);
        }
      });
    } else {
      const out = internalRun(a.step, ctx);
      response = out.response;
      link = out.link;
    }
  } catch (e) {
    status = 'failed';
    error = (e as Error).message;
  }
  const r: PortalActionRun = {
    id: nextRun++,
    actionId: a.id,
    userName: ME.name,
    inputs,
    status,
    statusCode,
    durationMs: Math.round(performance.now() - started) + 180 + Math.round(Math.random() * 400),
    responseExcerpt: redact(response, a.secrets).slice(0, 2000),
    error: error && redact(error, a.secrets),
    link,
    createdAt: Date.now(),
  };
  runs.push(r);
  return delay(r);
}

/** Test hook: forget every change. */
export function resetMock() {
  actions = null;
  runs = [];
  nextRun = 1;
  nextIncident = 41;
}
