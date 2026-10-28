// Self-service actions: template checks and rendering, form and run-input checks, secret redaction.
// They follow the server (routes/portal_action_steps.rs and routes/portal_actions.rs) so the mock
// accepts and refuses the same actions, and the editor can explain errors early.
import type { ActionHttpRequest, ActionInput, ActionStep, ActionStepKind, ActionValues, PortalActionCategory } from './types';

export const MAX_INPUTS = 30;
export const MAX_REQUESTS = 5;
export const MAX_HEADERS = 20;
export const MAX_TEMPLATE_CHARS = 20_000;
export const MAX_TEXT_INPUT_CHARS = 5000;
export const MAX_SECRETS = 10;
export const MAX_SECRET_CHARS = 4096;
export const REDACTED = '[REDACTED]';
export const METHODS = ['GET', 'POST', 'PUT', 'PATCH', 'DELETE'];

export const CATEGORY_LABEL: Record<PortalActionCategory, string> = {
  scaffolding: 'Scaffolding',
  release_management: 'Release Management',
  infrastructure_provisioning: 'Infrastructure Provisioning',
  incident_management: 'Incident Management',
  infrastructure_management: 'Infrastructure Management',
  other: 'Other',
};

export const STEP_LABEL: Record<ActionStepKind, string> = {
  http: 'HTTP request',
  create_incident: 'Declare an incident',
  page_on_call: 'Page an on-call team',
  incident_comment: 'Add a note to an incident',
  synthetic_run: 'Run a synthetic test',
};

/** The fields of an internal step, and whether each is required. */
export const STEP_FIELDS: Record<ActionStepKind, [string, boolean][]> = {
  http: [],
  create_incident: [
    ['title', true],
    ['severity', false],
    ['summary', false],
    ['teams', false],
    ['services', false],
  ],
  page_on_call: [
    ['team', true],
    ['title', true],
    ['description', false],
    ['urgency', false],
  ],
  incident_comment: [
    ['incident', true],
    ['body', true],
  ],
  synthetic_run: [['test', true]],
};

const chars = (s: string) => [...s].length;

// ---------------------------------------------------------------- templates

type Placeholder = { path: string; filter?: string };
type Piece = { literal: string } | { value: Placeholder };

function pieces(template: string): Piece[] {
  const out: Piece[] = [];
  let rest = template;
  for (;;) {
    const start = rest.indexOf('{{');
    if (start < 0) break;
    if (start > 0) out.push({ literal: rest.slice(0, start) });
    const after = rest.slice(start + 2);
    const end = after.indexOf('}}');
    if (end < 0) throw new Error('A placeholder is missing its closing }}');
    const inner = after.slice(0, end);
    const parts = inner.split('|');
    if (parts.length > 2) throw new Error(`{{${inner.trim()}}} has more than one filter`);
    out.push({ value: { path: parts[0].trim(), filter: parts[1]?.trim() } });
    rest = after.slice(end + 2);
  }
  if (rest) out.push({ literal: rest });
  return out;
}

function stepRef(path: string): [number, string] | null {
  if (!path.startsWith('steps.')) return null;
  const rest = path.slice(6);
  const dot = rest.indexOf('.');
  const n = dot < 0 ? rest : rest.slice(0, dot);
  if (!/^\d+$/.test(n)) return null;
  return [Number(n), dot < 0 ? '' : rest.slice(dot + 1)];
}

export type TemplateScope = { inputs: ActionInput[]; secrets: Set<string>; allowSecrets: boolean; priorSteps: number };

/** Checks every placeholder of a template against what it may refer to; throws with the reason. */
export function checkTemplate(template: string, scope: TemplateScope): void {
  if (chars(template) > MAX_TEMPLATE_CHARS) throw new Error(`A template is longer than ${MAX_TEMPLATE_CHARS} characters`);
  for (const p of pieces(template)) {
    if (!('value' in p)) continue;
    const { path, filter } = p.value;
    if (filter !== undefined && filter !== 'base64' && filter !== 'path') throw new Error(`Unknown filter ${filter} in {{${path}}} (use base64 or path)`);
    let known: boolean;
    const step = stepRef(path);
    if (path.startsWith('inputs.')) known = scope.inputs.some((i) => i.key === path.slice(7));
    else if (path.startsWith('secrets.')) {
      if (!scope.allowSecrets) throw new Error(`{{${path}}}: secrets can only be used in HTTP requests`);
      if (!scope.secrets.has(path.slice(8))) throw new Error(`There is no secret named ${path.slice(8)} ({{${path}}})`);
      known = true;
    } else if (step) {
      const [n, field] = step;
      if (n === 0 || n > scope.priorSteps || !field) throw new Error(`{{${path}}}: steps.${n} isn't available here; a request can read the JSON responses of the requests before it (steps.1.<field>)`);
      known = true;
    } else known = path === 'user.email' || path === 'user.name';
    if (!known) throw new Error(`Unknown value {{${path}}} (inputs.<key>, user.email, user.name, secrets.<name> or steps.<n>.<field>)`);
  }
}

export type RenderContext = { inputs: ActionValues; userEmail: string; userName: string; secrets: Record<string, string>; steps: unknown[] };

function asText(v: unknown): string {
  if (v === null || v === undefined) return '';
  if (typeof v === 'string') return v;
  if (typeof v === 'number' || typeof v === 'boolean') return String(v);
  return JSON.stringify(v);
}

/** UTF-8 safe base64. */
export function base64(s: string): string {
  let bin = '';
  for (const b of new TextEncoder().encode(s)) bin += String.fromCharCode(b);
  return btoa(bin);
}

function lookup(ctx: RenderContext, p: Placeholder): unknown {
  const { path } = p;
  let value: unknown;
  const step = stepRef(path);
  if (path.startsWith('inputs.')) value = ctx.inputs[path.slice(7)] ?? null;
  else if (path.startsWith('secrets.')) {
    const s = ctx.secrets[path.slice(8)];
    if (s === undefined) throw new Error(`There is no secret named ${path.slice(8)}`);
    value = s;
  } else if (step) {
    const [n, field] = step;
    const missing = () => new Error(`${path} is missing from the response of request ${n}`);
    let cur: unknown = ctx.steps[n - 1];
    if (cur === undefined) throw missing();
    for (const seg of field.split('.')) {
      if (Array.isArray(cur)) cur = /^\d+$/.test(seg) ? cur[Number(seg)] : undefined;
      else if (cur && typeof cur === 'object') cur = (cur as Record<string, unknown>)[seg];
      else cur = undefined;
      if (cur === undefined) throw missing();
    }
    value = cur;
  } else if (path === 'user.email') value = ctx.userEmail;
  else if (path === 'user.name') value = ctx.userName;
  else throw new Error(`Unknown value {{${path}}}`);
  return p.filter === 'base64' ? base64(asText(value)) : value;
}

/** Percent-encodes everything but RFC 3986 unreserved characters. */
export function percentEncode(s: string): string {
  let out = '';
  for (const b of new TextEncoder().encode(s)) {
    const c = String.fromCharCode(b);
    out += /[A-Za-z0-9\-._~]/.test(c) ? c : `%${b.toString(16).toUpperCase().padStart(2, '0')}`;
  }
  return out;
}

/** Plain text (headers, internal step fields); line breaks refused when `singleLine`. */
export function renderText(template: string, ctx: RenderContext, singleLine: boolean): string {
  let out = '';
  for (const p of pieces(template)) {
    if ('literal' in p) out += p.literal;
    else {
      const text = asText(lookup(ctx, p.value));
      if (singleLine && /[\r\n]/.test(text)) throw new Error(`{{${p.value.path}}} contains a line break, which a header can't carry`);
      out += text;
    }
  }
  return out;
}

/** A URL: every value percent-encoded (the `path` filter keeps slashes). */
export function renderUrl(template: string, ctx: RenderContext): string {
  let out = '';
  for (const p of pieces(template)) {
    if ('literal' in p) {
      out += p.literal;
      continue;
    }
    const text = asText(lookup(ctx, p.value));
    if (p.value.filter === 'path') {
      out += text
        .split('/')
        .map((seg) => {
          if (seg === '.' || seg === '..') throw new Error(`{{${p.value.path}}} can't contain the path segment “${seg}”`);
          return percentEncode(seg);
        })
        .join('/');
    } else {
      if (text === '.' || text === '..') throw new Error(`{{${p.value.path}}} can't be “${text}” in a URL`);
      out += percentEncode(text);
    }
  }
  return out;
}

function renderValue(v: unknown, ctx: RenderContext): unknown {
  if (typeof v === 'string') {
    const parts = pieces(v);
    if (parts.length === 1 && 'value' in parts[0]) {
      const value = lookup(ctx, parts[0].value);
      return value === null || value === undefined ? '' : value;
    }
    return renderText(v, ctx, false);
  }
  if (Array.isArray(v)) return v.map((x) => renderValue(x, ctx));
  if (v && typeof v === 'object') return Object.fromEntries(Object.entries(v).map(([k, x]) => [k, renderValue(x, ctx)]));
  return v;
}

/** A JSON body: placeholders filled inside the template's strings, so the structure never changes. */
export function renderJson(template: string, ctx: RenderContext): string {
  if (!template.trim()) return '';
  let doc: unknown;
  try {
    doc = JSON.parse(template);
  } catch (e) {
    throw new Error(`The body template isn't JSON: ${(e as Error).message}`);
  }
  return JSON.stringify(renderValue(doc, ctx));
}

// ---------------------------------------------------------------- validation

function trimmed(value: string | undefined, what: string, max: number, required: boolean): string {
  const v = (value ?? '').trim();
  if (required && !v) throw new Error(`${what} is required`);
  if (chars(v) > max) throw new Error(`${what} is longer than ${max} characters`);
  return v;
}

/** Checks an HTTP request (literal scheme and host, header names, JSON body); returns it normalised. */
export function checkRequest(r: ActionHttpRequest, scope: TemplateScope): ActionHttpRequest {
  const method = r.method.trim().toUpperCase();
  if (!METHODS.includes(method)) throw new Error(`Unknown method ${method}; use GET, POST, PUT, PATCH or DELETE`);
  const url = trimmed(r.url, 'The URL', 2048, true);
  const lower = url.toLowerCase();
  const scheme = lower.startsWith('https://') ? 8 : lower.startsWith('http://') ? 7 : 0;
  if (!scheme) throw new Error('The URL must start with http:// or https://');
  const after = lower.slice(scheme);
  const cut = after.search(/[/?#]/);
  const authority = cut < 0 ? after : after.slice(0, cut);
  if (!authority || authority.includes('{{')) throw new Error("Write the URL's host out: it can't come from a placeholder");
  checkTemplate(url, scope);
  const probe = pieces(url)
    .map((p) => ('literal' in p ? p.literal : 'x'))
    .join('');
  try {
    new URL(probe);
  } catch {
    throw new Error(`Invalid URL ${url}`);
  }
  const headers = r.headers ?? [];
  if (headers.length > MAX_HEADERS) throw new Error(`A request has at most ${MAX_HEADERS} headers`);
  const outHeaders = headers.map((h) => {
    const name = h.name.trim();
    if (!/^[A-Za-z0-9!#$%&'*+\-.^_`|~]{1,100}$/.test(name) || /^(host|content-length)$/i.test(name)) throw new Error(`${name} isn't a header name a request can set`);
    if (/[\r\n]/.test(h.value)) throw new Error(`The ${name} header has a line break`);
    checkTemplate(h.value, scope);
    return { name, value: h.value.trim() };
  });
  const body = (r.body ?? '').trim();
  if (body) {
    checkTemplate(body, scope);
    try {
      JSON.parse(body);
    } catch {
      throw new Error('The body must be JSON, with placeholders inside its strings ("name": "{{inputs.name}}")');
    }
  }
  return { method, url, headers: outHeaders, body };
}

const isNumber = (s: string) => s.trim() !== '' && Number.isFinite(Number(s));

/** Checks the form fields; returns them normalised. */
export function checkInputs(inputs: ActionInput[]): ActionInput[] {
  if (inputs.length > MAX_INPUTS) throw new Error(`An action has at most ${MAX_INPUTS} inputs`);
  const keys = new Set<string>();
  return inputs.map((i) => {
    const key = i.key.trim();
    if (!/^[a-z0-9_]{1,40}$/.test(key)) throw new Error(`${key} isn't an input key (a–z, 0–9 and _, up to 40)`);
    if (keys.has(key)) throw new Error(`The input key ${key} is used twice`);
    keys.add(key);
    const label = trimmed(i.label, `The label of ${key}`, 200, false) || key;
    const description = trimmed(i.description, `The description of ${key}`, 1000, false);
    const def = trimmed(i.default, `The default of ${key}`, MAX_TEXT_INPUT_CHARS, false);
    let options: string[] = [];
    if (i.kind === 'select') {
      options = (i.options ?? []).map((o) => o.trim());
      if (!options.length || options.length > 50 || new Set(options).size !== options.length || options.some((o) => !o || chars(o) > 200))
        throw new Error(`The select input ${key} needs 1–50 distinct options`);
      if (def && !options.includes(def)) throw new Error(`The default ${def} isn't an option of ${key}`);
    } else if (i.kind === 'boolean' && !['', 'true', 'false'].includes(def)) throw new Error(`The default of ${key} is true or false`);
    else if (i.kind === 'number' && def && !isNumber(def)) throw new Error(`The default of ${key} must be a number`);
    return { key, label, kind: i.kind, required: !!i.required, options, default: def, description };
  });
}

/** Checks a step against the inputs and secret names; returns it normalised. */
export function checkStep(step: ActionStep, inputs: ActionInput[], secrets: Set<string>): ActionStep {
  if (step.kind === 'http') {
    const requests = step.requests ?? [];
    if (!requests.length || requests.length > MAX_REQUESTS) throw new Error(`An HTTP step sends 1–${MAX_REQUESTS} requests`);
    return {
      kind: 'http',
      requests: requests.map((r, i) => {
        try {
          return checkRequest(r, { inputs, secrets, allowSecrets: true, priorSteps: i });
        } catch (e) {
          throw new Error(requests.length > 1 ? `Request ${i + 1}: ${(e as Error).message}` : (e as Error).message);
        }
      }),
      fields: {},
    };
  }
  const allowed = STEP_FIELDS[step.kind];
  const given = step.fields ?? {};
  const unknown = Object.keys(given).find((k) => !allowed.some(([f]) => f === k));
  if (unknown) throw new Error(`This step has no field ${unknown} (fields: ${allowed.map(([f]) => f).join(', ')})`);
  const fields: Record<string, string> = {};
  for (const [name, required] of allowed) {
    const value = trimmed(given[name], `The ${name} field`, MAX_TEMPLATE_CHARS, required);
    checkTemplate(value, { inputs, secrets, allowSecrets: false, priorSteps: 0 });
    if (value) fields[name] = value;
  }
  return { kind: step.kind, requests: [], fields };
}

function toNumber(s: string): number | null {
  return isNumber(s) ? Number(s.trim()) : null;
}

/** The values a run uses: given ones checked against the form, defaults for the others. */
export function checkRunInputs(inputs: ActionInput[], given: Record<string, unknown>): ActionValues {
  const unknown = Object.keys(given).find((k) => !inputs.some((i) => i.key === k));
  if (unknown) throw new Error(`Unknown input ${unknown}`);
  const out: ActionValues = {};
  for (const def of inputs) {
    const label = def.label;
    const raw = given[def.key];
    const present = raw !== null && raw !== undefined && !(typeof raw === 'string' && !raw.trim());
    let value: unknown;
    if (present) value = raw;
    else if (def.default) value = def.default;
    else if (def.required) throw new Error(`${label} is required`);
    else continue;
    switch (def.kind) {
      case 'text': {
        if (typeof value !== 'string') throw new Error(`${label} must be text`);
        const s = value.trim();
        if (chars(s) > MAX_TEXT_INPUT_CHARS) throw new Error(`${label} is longer than ${MAX_TEXT_INPUT_CHARS} characters`);
        out[def.key] = s;
        break;
      }
      case 'select': {
        const s = typeof value === 'string' ? value.trim() : '';
        if (!(def.options ?? []).includes(s)) throw new Error(`${s} isn't an option of ${label}`);
        out[def.key] = s;
        break;
      }
      case 'number': {
        const n = typeof value === 'number' ? value : typeof value === 'string' ? toNumber(value) : null;
        if (n === null || !Number.isFinite(n)) throw new Error(`${label} must be a number`);
        out[def.key] = n;
        break;
      }
      case 'boolean': {
        if (typeof value === 'boolean') out[def.key] = value;
        else if (value === 'true' || value === 'false') out[def.key] = value === 'true';
        else throw new Error(`${label} is true or false`);
        break;
      }
    }
  }
  return out;
}

/** The credentials after applying `changes` (value sets, null removes) to `current`. */
export function mergeSecrets(current: Record<string, string>, changes: Record<string, string | null>): Record<string, string> {
  const out = { ...current };
  for (const [name, value] of Object.entries(changes)) {
    if (value === null) {
      delete out[name];
      continue;
    }
    if (!/^[a-z0-9_]{1,40}$/.test(name)) throw new Error(`${name} isn't a secret name (a–z, 0–9 and _, up to 40)`);
    const v = value.trim();
    if (!v || chars(v) > MAX_SECRET_CHARS) throw new Error(`The secret ${name} must be 1–${MAX_SECRET_CHARS} characters`);
    out[name] = v;
  }
  if (Object.keys(out).length > MAX_SECRETS) throw new Error(`An action has at most ${MAX_SECRETS} secrets`);
  return out;
}

/** `text` with every secret value (raw, percent-encoded or base64) replaced by [REDACTED]. */
export function redact(text: string, secrets: Record<string, string>): string {
  const forms = [
    ...new Set(
      Object.values(secrets)
        .filter(Boolean)
        .flatMap((s) => [s, percentEncode(s), base64(s)]),
    ),
  ].sort((a, b) => b.length - a.length);
  let out = text;
  for (const f of forms) out = out.split(f).join(REDACTED);
  return out;
}

const webUrl = (v: unknown) => (typeof v === 'string' && /^https?:\/\//.test(v) ? v : null);

/** Link to what a response created: `html_url` (GitHub), `web_url` (GitLab) or a Jira issue's page. */
export function resultLink(response: unknown): string | null {
  if (!response || typeof response !== 'object' || Array.isArray(response)) return null;
  const o = response as Record<string, unknown>;
  const direct = webUrl(o.html_url) ?? webUrl(o.web_url);
  if (direct) return direct;
  const self = webUrl(o.self);
  if (typeof o.key !== 'string' || !self || !self.includes('/rest/api/')) return null;
  return `${self.split('/rest/api/')[0]}/browse/${percentEncode(o.key)}`;
}

/** Every `{{secrets.<name>}}` a step refers to. */
export function secretsUsed(step: ActionStep): string[] {
  const texts = (step.requests ?? []).flatMap((r) => [r.url, r.body ?? '', ...(r.headers ?? []).map((h) => h.value)]);
  const names = new Set<string>();
  for (const t of texts) for (const m of t.matchAll(/\{\{\s*secrets\.([a-z0-9_]+)\s*(\|[^}]*)?\}\}/g)) names.add(m[1]);
  return [...names].sort();
}

export function formatDuration(ms: number): string {
  if (ms < 1000) return `${Math.round(ms)} ms`;
  return `${(ms / 1000).toFixed(ms < 10_000 ? 2 : 1)} s`;
}
