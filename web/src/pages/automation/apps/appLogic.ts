// App Builder: component defaults, checks (following routes/apps.rs check_spec), the 12-column
// layout, button inputs and the rows a table shows from a workflow run.
import type { AppComponent, AppComponentKind, AppSpec, AppValues } from '../../../api/apps/types';
import type { JsonValue } from '../../../api/types';
import { lookup, pathOf } from '../../../api/workflows/logic';
import type { WorkflowRun, WorkflowSpec, WorkflowStep } from '../../../api/workflows/types';

export const WIDTHS = [3, 4, 6, 8, 12];
export const AGGREGATIONS = ['avg', 'sum', 'min', 'max', 'last'];

export const KIND_LABEL: Record<AppComponentKind, string> = {
  heading: 'Heading',
  text: 'Text',
  text_input: 'Text input',
  number_input: 'Number input',
  select: 'Select',
  button: 'Button',
  table: 'Table',
  metric: 'Metric value',
};

export const PALETTE: { group: string; kinds: AppComponentKind[] }[] = [
  { group: 'Display', kinds: ['heading', 'text', 'metric', 'table'] },
  { group: 'Inputs', kinds: ['text_input', 'number_input', 'select'] },
  { group: 'Actions', kinds: ['button'] },
];

const isInput = (k: AppComponentKind) => k === 'text_input' || k === 'number_input' || k === 'select';

export function blankComponent(id: string, kind: AppComponentKind): AppComponent {
  return {
    id,
    kind,
    label: '',
    text: '',
    defaultValue: '',
    placeholder: '',
    options: [],
    workflowId: '',
    inputs: {},
    source: '',
    path: '',
    columns: [],
    query: '',
    aggregation: '',
    unit: '',
    width: 12,
  };
}

/** A free component id: `button`, `button_2`, … */
export function freeId(spec: AppSpec, base: string): string {
  const taken = new Set(spec.components.map((c) => c.id));
  const clean =
    base
      .toLowerCase()
      .replace(/[^a-z0-9_]+/g, '_')
      .replace(/^_+|_+$/g, '')
      .slice(0, 32) || 'component';
  if (!taken.has(clean)) return clean;
  for (let i = 2; ; i++) if (!taken.has(`${clean}_${i}`)) return `${clean}_${i}`;
}

/** A new component of `kind` with defaults that already pass the checks. */
export function newComponent(spec: AppSpec, kind: AppComponentKind): AppComponent {
  const c = blankComponent(freeId(spec, kind), kind);
  switch (kind) {
    case 'heading':
      return { ...c, text: 'New section' };
    case 'text':
      return { ...c, text: 'Describe what this app does.' };
    case 'text_input':
      return { ...c, label: 'Text', placeholder: 'Type a value', width: 4 };
    case 'number_input':
      return { ...c, label: 'Number', defaultValue: '1', width: 4 };
    case 'select':
      return { ...c, label: 'Option', options: ['staging', 'production'], defaultValue: 'staging', width: 4 };
    case 'button':
      return { ...c, label: 'Run', width: 3 };
    case 'table': {
      const button = spec.components.find((b) => b.kind === 'button');
      return { ...c, label: 'Results', source: button?.id ?? '' };
    }
    case 'metric':
      return { ...c, label: 'Requests/s', query: 'sum:trace.http.request.hits{service:storefront-api}', aggregation: 'avg', unit: 'req/s', width: 4 };
  }
}

/** Placeholders `{{ components.<id>.value }}` → the component ids read. */
export function referenced(template: string): string[] {
  return [...template.matchAll(/\{\{([^}]*)\}\}/g)].map((m) => m[1].trim()).flatMap((p) => (p.startsWith('components.') ? [p.slice('components.'.length).split('.')[0]] : []));
}

/** What is wrong with one component, or null. */
export function componentProblem(c: AppComponent, spec: AppSpec): string | null {
  if (!WIDTHS.includes(c.width)) return 'The width is 3, 4, 6, 8 or 12 columns';
  switch (c.kind) {
    case 'heading':
    case 'text':
      return c.text.trim() ? null : 'The text is required';
    case 'select': {
      const opts = c.options.map((o) => o.trim()).filter(Boolean);
      if (opts.length === 0 || opts.length > 50 || new Set(opts).size !== opts.length) return 'A select needs 1–50 distinct options';
      return null;
    }
    case 'number_input':
      return c.defaultValue.trim() && !Number.isFinite(Number(c.defaultValue)) ? 'The default value must be a number' : null;
    case 'button': {
      if (!c.workflowId.trim()) return 'Choose the workflow the button runs';
      const ids = new Set(spec.components.map((x) => x.id));
      for (const tpl of Object.values(c.inputs)) {
        for (const p of [...tpl.matchAll(/\{\{([^}]*)\}\}/g)].map((m) => m[1].trim())) {
          if (!p.startsWith('components.')) return `Unknown value {{${p}}}: buttons read components.<id>.value`;
        }
        const missing = referenced(tpl).find((id) => !ids.has(id));
        if (missing) return `There's no component ${missing}`;
      }
      return null;
    }
    case 'table':
      if (!spec.components.some((b) => b.kind === 'button' && b.id === c.source)) return "A table shows the latest run of one of the app's buttons";
      return c.path.trim() ? null : 'The path is required';
    case 'metric':
      if (!c.query.trim()) return 'The query is required';
      return c.aggregation && !AGGREGATIONS.includes(c.aggregation) ? 'The aggregation is avg, sum, min, max or last' : null;
    default:
      return null;
  }
}

/** The first problem of an app (as the server words it), or null. */
export function specProblem(spec: AppSpec): string | null {
  if (spec.components.length > 60) return 'An app has at most 60 components';
  const seen = new Set<string>();
  for (const c of spec.components) {
    if (!/^[a-z0-9_]{1,40}$/.test(c.id)) return `${c.id} isn't a component id (a–z, 0–9 and _, up to 40)`;
    if (seen.has(c.id)) return `The component id ${c.id} is used twice`;
    seen.add(c.id);
  }
  for (const c of spec.components) {
    const p = componentProblem(c, spec);
    if (p) return `Component ${c.id}: ${p}`;
  }
  return null;
}

/** Components packed into rows of 12 columns, in order. */
export function layoutRows(components: AppComponent[]): AppComponent[][] {
  const rows: AppComponent[][] = [];
  let used = 12;
  for (const c of components) {
    const w = WIDTHS.includes(c.width) ? c.width : 12;
    if (used + w > 12) {
      rows.push([]);
      used = 0;
    }
    rows[rows.length - 1].push(c);
    used += w;
  }
  return rows;
}

/** Input values with defaults applied (numbers as numbers). */
export function initialValues(spec: AppSpec): AppValues {
  const out: AppValues = {};
  for (const c of spec.components) {
    if (!isInput(c.kind)) continue;
    if (c.kind === 'number_input') out[c.id] = c.defaultValue.trim() === '' ? '' : Number(c.defaultValue);
    else if (c.kind === 'select') out[c.id] = c.defaultValue || c.options[0] || '';
    else out[c.id] = c.defaultValue;
  }
  return out;
}

/** The workflow inputs a button would send now (as the server fills them). */
export function buttonInputsPreview(button: AppComponent, values: AppValues): Record<string, JsonValue> {
  const out: Record<string, JsonValue> = {};
  for (const [key, tpl] of Object.entries(button.inputs)) {
    const single = /^\{\{\s*components\.([a-z0-9_]+)\.value\s*\}\}$/.exec(tpl.trim());
    const value: JsonValue = single ? (values[single[1]] ?? null) : tpl.replace(/\{\{\s*components\.([a-z0-9_]+)\.value\s*\}\}/g, (_, id: string) => String(values[id] ?? '')).trim();
    if (value !== null && value !== '') out[key] = value;
  }
  return out;
}

/** The data a table reads from a run: Trigger, inputs and the latest result of each step. */
export function runData(run: WorkflowRun): JsonValue {
  const steps: Record<string, JsonValue> = {};
  for (const s of run.steps) steps[s.stepId] = { status: s.status, output: s.output, error: s.error ?? null };
  return { Trigger: run.trigger, inputs: run.inputs, Steps: steps };
}

/** The rows and columns a table shows: the list at `path` (an object counts as one row). */
export function tableRows(run: WorkflowRun | undefined, path: string, columns: string[]): { columns: string[]; rows: Record<string, JsonValue>[] } {
  if (!run || !path.trim()) return { columns, rows: [] };
  const value = lookup(runData(run), pathOf(path.trim()));
  const list: JsonValue[] = Array.isArray(value) ? value : value === undefined || value === null ? [] : [value];
  const rows = list.map((v) => (v !== null && typeof v === 'object' && !Array.isArray(v) ? (v as Record<string, JsonValue>) : { value: v }));
  const cols = columns.length ? columns : [...new Set(rows.flatMap((r) => Object.keys(r)))].slice(0, 12);
  return { columns: cols, rows };
}

export const cellText = (v: JsonValue | undefined): string => (v === null || v === undefined ? '' : typeof v === 'object' ? JSON.stringify(v) : String(v));

const OUTPUT_FIELDS: Partial<Record<WorkflowStep['kind'], string[]>> = {
  metric_query: ['groups', 'value'],
  log_count: ['groups', 'count'],
  http: ['body', 'statusCode'],
  notify: ['sent', 'failed'],
  create_incident: ['incident', 'title'],
  page_on_call: ['page', 'team'],
};

/** Paths a table can show for a workflow: `Steps.<step>.output.<field>` of its action steps. */
export function pathSuggestions(spec: WorkflowSpec | null | undefined): string[] {
  if (!spec) return [];
  const out: string[] = [];
  const visit = (list: WorkflowStep[]) => {
    for (const s of list) {
      for (const f of OUTPUT_FIELDS[s.kind] ?? []) out.push(`Steps.${s.id}.output.${f}`);
      visit(s.steps);
      visit(s.elseSteps);
    }
  };
  visit(spec.steps);
  return out;
}

/** Moves a component one place up (-1) or down (+1). */
export function moveComponent(spec: AppSpec, id: string, by: -1 | 1): AppSpec {
  const i = spec.components.findIndex((c) => c.id === id);
  const j = i + by;
  if (i < 0 || j < 0 || j >= spec.components.length) return spec;
  const list = [...spec.components];
  [list[i], list[j]] = [list[j], list[i]];
  return { components: list };
}

/** Renames a component, following the references of buttons and tables. */
export function renameComponent(spec: AppSpec, from: string, to: string): AppSpec {
  const re = new RegExp(`\\{\\{(\\s*)components\\.${from}\\.`, 'g');
  return {
    components: spec.components.map((c) => ({
      ...c,
      id: c.id === from ? to : c.id,
      source: c.source === from ? to : c.source,
      inputs: Object.fromEntries(Object.entries(c.inputs).map(([k, v]) => [k, v.replace(re, `{{$1components.${to}.`)])),
    })),
  };
}
