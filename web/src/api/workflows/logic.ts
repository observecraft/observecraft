// Workflow Automation: the expression language, templates over the run's data, step-tree helpers
// and a run simulator. They follow the server (routes/workflow_expr.rs, routes/workflow_engine.rs) so
// the editor can explain mistakes early and the mock runs workflows the way the server does.
import type { JsonValue } from '../types';
import type { WorkflowSpec, WorkflowStep, WorkflowStepKind, WorkflowTrigger, WorkflowTriggerKind } from './types';

export const MAX_STEPS = 100;
export const MAX_LOOP_ITEMS = 100;
export const MAX_EXECUTED = 500;
export const ROOTS = ['Trigger', 'Steps', 'inputs', 'Loop', 'Workflow', 'user'];

export const STEP_LABEL: Record<WorkflowStepKind, string> = {
  http: 'Make an HTTP request',
  create_incident: 'Declare an incident',
  page_on_call: 'Page an on-call team',
  incident_comment: 'Add a note to an incident',
  synthetic_run: 'Run a synthetic test',
  notify: 'Send a notification',
  metric_query: 'Query a metric',
  log_count: 'Count logs',
  wait: 'Wait',
  if: 'If condition',
  for_each: 'For each',
};

export const TRIGGER_LABEL: Record<WorkflowTriggerKind, string> = {
  manual: 'Manual',
  monitor: 'Monitor',
  schedule: 'Schedule',
  incident: 'Incident',
  api: 'API',
  app: 'App',
};

/** Fields of each action step and whether each is required (http uses `requests`). */
export const STEP_FIELDS: Record<WorkflowStepKind, [string, boolean][]> = {
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
  notify: [
    ['recipients', true],
    ['title', false],
    ['message', true],
  ],
  metric_query: [
    ['query', true],
    ['minutes', false],
    ['aggregation', false],
  ],
  log_count: [
    ['query', true],
    ['minutes', false],
    ['group_by', false],
  ],
  wait: [['seconds', true]],
  if: [],
  for_each: [],
};

export const isControl = (k: WorkflowStepKind) => k === 'if' || k === 'for_each';

export function emptyTrigger(kind: WorkflowTriggerKind): WorkflowTrigger {
  return { kind, cron: kind === 'schedule' ? '0 9 * * *' : '', timezone: kind === 'schedule' ? 'UTC' : '', transitions: kind === 'monitor' ? ['alert'] : [], incidentEvents: [], severities: [] };
}

export function emptyStep(id: string, kind: WorkflowStepKind): WorkflowStep {
  const fields: Record<string, string> = {};
  if (kind === 'wait') fields.seconds = '60';
  return {
    id,
    kind,
    name: '',
    requests: kind === 'http' ? [{ method: 'GET', url: 'https://', headers: [], body: '' }] : [],
    fields,
    expression: kind === 'if' ? "Steps.x.status == 'succeeded'" : kind === 'for_each' ? 'Trigger.items' : '',
    steps: [],
    elseSteps: [],
    retries: 0,
    continueOnError: false,
  };
}

/** Fills the defaults the server leaves out of older documents. */
export function normalizeStep(s: Partial<WorkflowStep> & { id: string; kind: WorkflowStepKind }): WorkflowStep {
  return {
    name: '',
    requests: [],
    fields: {},
    expression: '',
    retries: 0,
    continueOnError: false,
    ...s,
    steps: (s.steps ?? []).map(normalizeStep),
    elseSteps: (s.elseSteps ?? []).map(normalizeStep),
  };
}

// ---------------------------------------------------------------- step tree

export type Branch = 'steps' | 'elseSteps';
/** Where a block of steps is: the top level, or a control step's branch. */
export type BlockRef = { parent: string | null; branch: Branch };

export function walk(list: WorkflowStep[], visit: (s: WorkflowStep, depth: number) => void, depth = 0): void {
  for (const s of list) {
    visit(s, depth);
    walk(s.steps, visit, depth + 1);
    walk(s.elseSteps, visit, depth + 1);
  }
}

export function allSteps(list: WorkflowStep[]): WorkflowStep[] {
  const out: WorkflowStep[] = [];
  walk(list, (s) => out.push(s));
  return out;
}

export function findStep(list: WorkflowStep[], id: string): WorkflowStep | undefined {
  return allSteps(list).find((s) => s.id === id);
}

/** A free step id: `restart`, `restart_2`, … */
export function freeId(list: WorkflowStep[], base: string): string {
  const taken = new Set(allSteps(list).map((s) => s.id));
  const clean =
    base
      .toLowerCase()
      .replace(/[^a-z0-9_]+/g, '_')
      .replace(/^_+|_+$/g, '')
      .slice(0, 32) || 'step';
  if (!taken.has(clean)) return clean;
  for (let i = 2; ; i++) if (!taken.has(`${clean}_${i}`)) return `${clean}_${i}`;
}

function mapBlock(list: WorkflowStep[], ref: BlockRef, f: (block: WorkflowStep[]) => WorkflowStep[]): WorkflowStep[] {
  if (ref.parent === null) return f(list);
  return list.map((s) => (s.id === ref.parent ? { ...s, [ref.branch]: f(s[ref.branch]) } : { ...s, steps: mapBlock(s.steps, ref, f), elseSteps: mapBlock(s.elseSteps, ref, f) }));
}

/** Inserts `step` at `index` of the block. */
export function insertStep(list: WorkflowStep[], ref: BlockRef, index: number, step: WorkflowStep): WorkflowStep[] {
  return mapBlock(list, ref, (b) => [...b.slice(0, index), step, ...b.slice(index)]);
}

export function removeStep(list: WorkflowStep[], id: string): WorkflowStep[] {
  return list.filter((s) => s.id !== id).map((s) => ({ ...s, steps: removeStep(s.steps, id), elseSteps: removeStep(s.elseSteps, id) }));
}

export function replaceStep(list: WorkflowStep[], id: string, next: WorkflowStep): WorkflowStep[] {
  return list.map((s) => (s.id === id ? next : { ...s, steps: replaceStep(s.steps, id, next), elseSteps: replaceStep(s.elseSteps, id, next) }));
}

// ---------------------------------------------------------------- expressions

type Tok = { t: 'num'; v: number } | { t: 'str'; v: string } | { t: 'id'; v: string } | { t: 'op'; v: string } | { t: '.' | '[' | ']' | '(' | ')' };
export type Expr = { k: 'lit'; v: JsonValue } | { k: 'path'; p: (string | number)[] } | { k: 'not'; e: Expr } | { k: 'neg'; e: Expr } | { k: 'bin'; op: string; a: Expr; b: Expr };

function tokenize(src: string): Tok[] {
  const out: Tok[] = [];
  let i = 0;
  while (i < src.length) {
    const c = src[i];
    if (/\s/.test(c)) {
      i++;
      continue;
    }
    const two = src.slice(i, i + 2);
    if (['==', '!=', '<=', '>=', '&&', '||'].includes(two)) {
      out.push({ t: 'op', v: two });
      i += 2;
      continue;
    }
    if ('<>!-'.includes(c)) out.push({ t: 'op', v: c });
    else if ('.[]()'.includes(c)) out.push({ t: c as '.' });
    else if (c === '"' || c === "'") {
      let s = '';
      i++;
      for (;;) {
        if (i >= src.length) throw new Error('A text value is missing its closing quote');
        const ch = src[i];
        if (ch === c) break;
        if (ch === '\\') {
          i++;
          if (i >= src.length) throw new Error('A text value is missing its closing quote');
          s += src[i] === 'n' ? '\n' : src[i] === 't' ? '\t' : src[i];
        } else s += ch;
        i++;
      }
      out.push({ t: 'str', v: s });
    } else if (/[0-9]/.test(c)) {
      const m = /^[0-9]+(\.[0-9]+)?/.exec(src.slice(i))!;
      out.push({ t: 'num', v: Number(m[0]) });
      i += m[0].length;
      continue;
    } else if (/[A-Za-z_]/.test(c)) {
      const m = /^[A-Za-z0-9_]+/.exec(src.slice(i))!;
      out.push({ t: 'id', v: m[0] });
      i += m[0].length;
      continue;
    } else throw new Error(`Unexpected character “${c}” in the expression`);
    i++;
  }
  return out;
}

/** Parses an expression (with or without `{{ }}`); throws an explanation. */
export function parseExpr(src: string): Expr {
  let inner = src.trim();
  if (inner.startsWith('{{') && inner.endsWith('}}')) inner = inner.slice(2, -2).trim();
  if (!inner) throw new Error('The expression is empty');
  const toks = tokenize(inner);
  let pos = 0;
  const peek = () => toks[pos];
  const isOp = (v: string) => peek()?.t === 'op' && (peek() as { v: string }).v === v;
  const isWord = (v: string) => peek()?.t === 'id' && (peek() as { v: string }).v === v;
  const or = (): Expr => {
    let l = and();
    while (isOp('||') || isWord('or')) {
      pos++;
      l = { k: 'bin', op: '||', a: l, b: and() };
    }
    return l;
  };
  const and = (): Expr => {
    let l = cmp();
    while (isOp('&&') || isWord('and')) {
      pos++;
      l = { k: 'bin', op: '&&', a: l, b: cmp() };
    }
    return l;
  };
  const cmp = (): Expr => {
    const l = unary();
    const t = peek();
    const op = t?.t === 'op' && ['==', '!=', '<', '<=', '>', '>='].includes(t.v) ? t.v : isWord('contains') ? 'contains' : null;
    if (!op) return l;
    pos++;
    const r = unary();
    const n = peek();
    if ((n?.t === 'op' && ['==', '!=', '<', '<=', '>', '>='].includes(n.v)) || isWord('contains')) throw new Error('Comparisons can’t be chained; join them with && or ||');
    return { k: 'bin', op, a: l, b: r };
  };
  const unary = (): Expr => {
    if (isOp('!') || isWord('not')) {
      pos++;
      return { k: 'not', e: unary() };
    }
    if (isOp('-')) {
      pos++;
      return { k: 'neg', e: unary() };
    }
    return primary();
  };
  const primary = (): Expr => {
    const t = toks[pos++];
    if (!t) throw new Error('The expression ends too early');
    if (t.t === 'num' || t.t === 'str') return { k: 'lit', v: t.v };
    if (t.t === '(') {
      const e = or();
      if (toks[pos++]?.t !== ')') throw new Error('A parenthesis isn’t closed');
      return e;
    }
    if (t.t === 'id') {
      if (t.v === 'true' || t.v === 'false') return { k: 'lit', v: t.v === 'true' };
      if (t.v === 'null') return { k: 'lit', v: null };
      if (['and', 'or', 'not', 'contains'].includes(t.v)) throw new Error(`“${t.v}” needs a value before and after it`);
      const p: (string | number)[] = [t.v];
      for (;;) {
        const n = peek();
        if (n?.t === '.') {
          pos++;
          const k = toks[pos++];
          if (k?.t === 'id') p.push(k.v);
          else if (k?.t === 'num' && Number.isInteger(k.v)) p.push(k.v);
          else throw new Error('A “.” must be followed by a name');
        } else if (n?.t === '[') {
          pos++;
          const k = toks[pos++];
          if (k?.t === 'num') p.push(k.v);
          else if (k?.t === 'str') p.push(k.v);
          else throw new Error('Brackets hold a position ([0]) or a quoted name (["key"])');
          if (toks[pos++]?.t !== ']') throw new Error('A “[” isn’t closed');
        } else break;
      }
      return { k: 'path', p };
    }
    if (t.t === 'op') throw new Error(`“${t.v}” needs a value before it`);
    throw new Error('Unexpected symbol: a path starts with a name (Trigger, Steps, inputs, Loop)');
  };
  const e = or();
  if (pos < toks.length) throw new Error('Unexpected text after the end of the expression; join conditions with && or ||');
  return e;
}

/** An explanation of what is wrong with an expression, or null when it is fine. */
export function exprError(src: string, roots: string[] = ROOTS): string | null {
  try {
    const e = parseExpr(src);
    const bad: string[] = [];
    const visit = (x: Expr) => {
      if (x.k === 'path' && !roots.includes(String(x.p[0]))) bad.push(String(x.p[0]));
      if (x.k === 'not' || x.k === 'neg') visit(x.e);
      if (x.k === 'bin') {
        visit(x.a);
        visit(x.b);
      }
    };
    visit(e);
    return bad.length ? `Unknown name ${bad[0]}: start a path with ${roots.join(', ')}` : null;
  } catch (err) {
    return (err as Error).message;
  }
}

export function lookup(vars: JsonValue, path: (string | number)[]): JsonValue | undefined {
  let cur: JsonValue | undefined = vars;
  for (const seg of path) {
    if (cur === null || cur === undefined || typeof cur !== 'object') return undefined;
    cur = Array.isArray(cur) ? cur[Number(seg)] : (cur as Record<string, JsonValue>)[String(seg)];
  }
  return cur;
}

export const pathOf = (dotted: string): (string | number)[] => dotted.split('.').map((s) => (/^\d+$/.test(s) ? Number(s) : s));

export function truthy(v: JsonValue | undefined): boolean {
  if (v === null || v === undefined || v === false || v === 0 || v === '') return false;
  if (Array.isArray(v)) return v.length > 0;
  if (typeof v === 'object') return Object.keys(v).length > 0;
  return true;
}

const num = (v: JsonValue): number | null => (typeof v === 'number' ? v : typeof v === 'string' && v.trim() !== '' && Number.isFinite(Number(v)) ? Number(v) : null);
const eq = (a: JsonValue, b: JsonValue): boolean => {
  if (JSON.stringify(a) === JSON.stringify(b)) return true;
  if ((typeof a === 'number' || typeof b === 'number') && (typeof a === 'string' || typeof b === 'string' || typeof a === typeof b)) {
    const x = num(a);
    const y = num(b);
    return x !== null && x === y;
  }
  return false;
};
const order = (a: JsonValue, b: JsonValue): number | null => {
  const x = num(a);
  const y = num(b);
  if (x !== null && y !== null) return x - y;
  if (typeof a === 'string' && typeof b === 'string') return a < b ? -1 : a > b ? 1 : 0;
  return null;
};

export function evalExpr(e: Expr, vars: JsonValue): JsonValue {
  switch (e.k) {
    case 'lit':
      return e.v;
    case 'path':
      return lookup(vars, e.p) ?? null;
    case 'not':
      return !truthy(evalExpr(e.e, vars));
    case 'neg': {
      const n = num(evalExpr(e.e, vars));
      return n === null ? null : -n;
    }
    case 'bin': {
      const a = evalExpr(e.a, vars);
      if (e.op === '&&') return truthy(a) && truthy(evalExpr(e.b, vars));
      if (e.op === '||') return truthy(a) || truthy(evalExpr(e.b, vars));
      const b = evalExpr(e.b, vars);
      const o = order(a, b);
      switch (e.op) {
        case '==':
          return eq(a, b);
        case '!=':
          return !eq(a, b);
        case '<':
          return o !== null && o < 0;
        case '<=':
          return o !== null && o <= 0;
        case '>':
          return o !== null && o > 0;
        case '>=':
          return o !== null && o >= 0;
        case 'contains':
          if (typeof a === 'string') return typeof b === 'string' || typeof b === 'number' || typeof b === 'boolean' ? a.includes(String(b)) : false;
          if (Array.isArray(a)) return a.some((x) => eq(x, b));
          if (a && typeof a === 'object') return typeof b === 'string' && b in a;
          return false;
      }
    }
  }
  return null;
}

// ---------------------------------------------------------------- templates

const asText = (v: JsonValue | undefined) => (v === null || v === undefined ? '' : typeof v === 'string' ? v : typeof v === 'object' ? JSON.stringify(v) : String(v));

/** Fills `{{ path }}` placeholders from the run's data (as text). */
export function renderTemplate(template: string, vars: JsonValue): string {
  return template.replace(/\{\{([^}]*)\}\}/g, (_, inner: string) => asText(lookup(vars, pathOf(inner.split('|')[0].trim()))));
}

/** The placeholder paths of a template. */
export function placeholders(template: string): string[] {
  return [...template.matchAll(/\{\{([^}]*)\}\}/g)].map((m) => m[1].split('|')[0].trim());
}

/** Placeholders that don't start at a known name, or read inputs the form lacks. */
export function templateError(template: string, inputKeys: string[]): string | null {
  for (const p of placeholders(template)) {
    const [root, key] = p.split('.');
    if (root === 'inputs') {
      if (!inputKeys.includes(key)) return `Unknown value {{${p}}}: the form has no input ${key}`;
    } else if (root === 'secrets' || root === 'steps') continue;
    else if (root === 'user') {
      if (key !== 'email' && key !== 'name') return `Unknown value {{${p}}} (user.email or user.name)`;
    } else if (!ROOTS.includes(root)) return `Unknown value {{${p}}}: start with ${ROOTS.join(', ')}`;
  }
  return null;
}

// ---------------------------------------------------------------- checks

/** The first problem of a workflow's steps, or null (ids, required fields, expressions, nesting). */
export function specError(spec: WorkflowSpec): string | null {
  const ids = new Set<string>();
  const keys = spec.inputs.map((i) => i.key);
  let count = 0;
  let problem: string | null = null;
  const check = (list: WorkflowStep[], loops: number) => {
    for (const s of list) {
      if (problem) return;
      count++;
      if (!/^[a-z0-9_]{1,40}$/.test(s.id)) problem = `${s.id} isn't a step id (a–z, 0–9 and _, up to 40)`;
      else if (ids.has(s.id)) problem = `The step id ${s.id} is used twice`;
      ids.add(s.id);
      if (problem) return;
      if (isControl(s.kind)) {
        const e = exprError(s.expression);
        if (e) problem = `Step ${s.id}: ${e}`;
        const inner = loops + (s.kind === 'for_each' ? 1 : 0);
        if (inner > 2) problem = `Step ${s.id}: “for each” steps can be nested 2 deep at most`;
        check(s.steps, inner);
        check(s.elseSteps, loops);
      } else {
        for (const [f, required] of STEP_FIELDS[s.kind]) {
          const v = (s.fields[f] ?? '').trim();
          if (required && !v) problem = `Step ${s.id}: the ${f} field is required`;
          const t = templateError(v, keys);
          if (!problem && t) problem = `Step ${s.id}: ${t}`;
        }
        if (s.kind === 'http' && !problem) {
          if (s.requests.length === 0) problem = `Step ${s.id}: an HTTP step sends 1–5 requests`;
          for (const r of s.requests) if (!problem && !/^https?:\/\/[^/{]+/.test(r.url.trim())) problem = `Step ${s.id}: the URL must start with http(s):// and a literal host`;
        }
      }
    }
  };
  check(spec.steps, 0);
  if (!problem && count > MAX_STEPS) problem = `A workflow has at most ${MAX_STEPS} steps`;
  return problem;
}

// ---------------------------------------------------------------- run simulation (mock mode)

export type SimStep = {
  stepId: string;
  kind: WorkflowStepKind;
  name: string;
  iteration: number | null;
  ok: boolean;
  inputs: Record<string, JsonValue>;
  output: JsonValue;
  error: string | null;
  waitSeconds?: number;
};

/**
 * Runs a workflow the way the server's state machine does, asking `perform` for each action step's
 * result. Returns what each step did and how the run ended.
 */
export function simulate(
  spec: WorkflowSpec,
  base: Record<string, JsonValue>,
  perform: (step: WorkflowStep, vars: JsonValue) => { ok: boolean; inputs: Record<string, JsonValue>; output: JsonValue; error?: string },
): { steps: SimStep[]; ok: boolean; error: string | null } {
  const results: Record<string, JsonValue> = {};
  const out: SimStep[] = [];
  let executed = 0;
  const loop: { item: JsonValue; index: number }[] = [];
  const vars = (): JsonValue => ({ ...base, Steps: results, ...(loop.length ? { Loop: loop[loop.length - 1] } : {}) });
  const iteration = () => (loop.length ? loop[loop.length - 1].index : null);
  const run = (list: WorkflowStep[]): string | null => {
    for (const s of list) {
      if (s.kind === 'if' || s.kind === 'for_each') {
        let value: JsonValue;
        try {
          value = evalExpr(parseExpr(s.expression), vars());
        } catch (e) {
          return `Step ${s.id}: ${(e as Error).message}`;
        }
        if (s.kind === 'if') {
          const holds = truthy(value);
          const output = { result: holds, branch: holds ? 'then' : 'else' };
          out.push({ stepId: s.id, kind: s.kind, name: s.name, iteration: iteration(), ok: true, inputs: {}, output, error: null });
          results[s.id] = { status: 'succeeded', output };
          const err = run(holds ? s.steps : s.elseSteps);
          if (err) return err;
        } else {
          const items = value === null ? [] : value;
          if (!Array.isArray(items)) return `Step ${s.id}: “for each” needs a list`;
          if (items.length > MAX_LOOP_ITEMS) return `Step ${s.id}: “for each” runs over ${MAX_LOOP_ITEMS} items at most (${items.length} given)`;
          out.push({ stepId: s.id, kind: s.kind, name: s.name, iteration: iteration(), ok: true, inputs: {}, output: { items: items.length }, error: null });
          results[s.id] = { status: 'succeeded', output: { items: items.length } };
          for (let i = 0; i < items.length; i++) {
            loop.push({ item: items[i], index: i });
            const err = run(s.steps);
            loop.pop();
            if (err) return err;
          }
        }
        continue;
      }
      if (executed >= MAX_EXECUTED) return `The run stopped after ${MAX_EXECUTED} steps`;
      executed++;
      const r = perform(s, vars());
      out.push({ stepId: s.id, kind: s.kind, name: s.name, iteration: iteration(), ok: r.ok, inputs: r.inputs, output: r.output, error: r.error ?? null });
      results[s.id] = { status: r.ok ? 'succeeded' : 'failed', output: r.output, error: r.error ?? null };
      if (!r.ok && !s.continueOnError) return `Step ${s.id}: ${r.error ?? 'failed'}`;
    }
    return null;
  };
  const error = run(spec.steps);
  return { steps: out, ok: error === null, error };
}

// ---------------------------------------------------------------- schedules

/** A short description of a 5-field cron schedule (`0 9 * * 1-5` → `At 09:00, Monday through Friday`). */
export function describeCron(cron: string, timezone = 'UTC'): string {
  const parts = cron.trim().split(/\s+/);
  if (parts.length !== 5) return cron;
  const [m, h, dom, mon, dow] = parts;
  const zone = timezone && timezone !== 'UTC' ? ` (${timezone})` : ' UTC';
  const days: Record<string, string> = { '*': 'every day', '1-5': 'Monday through Friday', 'MON-FRI': 'Monday through Friday', '0,6': 'on weekends', '1': 'on Mondays', MON: 'on Mondays' };
  if (/^\d+$/.test(m) && /^\d+$/.test(h) && dom === '*' && mon === '*' && days[dow.toUpperCase()]) return `At ${h.padStart(2, '0')}:${m.padStart(2, '0')}, ${days[dow.toUpperCase()]}${zone}`;
  const every = /^\*\/(\d+)$/.exec(m);
  if (every && h === '*' && dom === '*' && mon === '*' && dow === '*') return `Every ${every[1]} minutes`;
  if (m === '0' && h === '*' && dom === '*' && mon === '*' && dow === '*') return 'Every hour';
  return `${cron}${zone}`;
}
