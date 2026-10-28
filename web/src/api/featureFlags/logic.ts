// Feature flag helpers shared by the pages and the in-browser mock: keys, variant values, the
// Create Flag draft, splits and rollouts (as the server evaluates them, see the oc-flags crate),
// list search and facets, and the OpenFeature (OFREP) setup snippets.
import type { FeatureFlag, FeatureFlagInput, FlagEnvConfig, FlagLifecycle, FlagOperator, FlagRollout, FlagSdkAccess, FlagServe, FlagSplit, FlagType, FlagVariant } from './types';

export const FLAG_TYPES: FlagType[] = ['boolean', 'string', 'json', 'integer', 'number'];
export const TYPE_LABEL: Record<FlagType, string> = { boolean: 'Boolean', string: 'String', json: 'JSON', integer: 'Integer', number: 'Number' };

export const OPERATORS: { id: FlagOperator; label: string; values: 'many' | 'one' | 'none' }[] = [
  { id: 'one_of', label: 'is one of', values: 'many' },
  { id: 'not_one_of', label: 'is not one of', values: 'many' },
  { id: 'contains', label: 'contains', values: 'many' },
  { id: 'not_contains', label: 'does not contain', values: 'many' },
  { id: 'starts_with', label: 'starts with', values: 'many' },
  { id: 'ends_with', label: 'ends with', values: 'many' },
  { id: 'gt', label: '>', values: 'one' },
  { id: 'gte', label: '≥', values: 'one' },
  { id: 'lt', label: '<', values: 'one' },
  { id: 'lte', label: '≤', values: 'one' },
  { id: 'exists', label: 'is set', values: 'none' },
  { id: 'not_exists', label: 'is not set', values: 'none' },
];

/** Colour tokens for variants, in order (swatches, split bars, charts). */
export const VARIANT_TOKENS = ['--chart-1', '--chart-4', '--chart-3', '--chart-2', '--chart-7', '--chart-6', '--chart-5', '--chart-8'];
export const variantToken = (i: number) => VARIANT_TOKENS[((i % VARIANT_TOKENS.length) + VARIANT_TOKENS.length) % VARIANT_TOKENS.length];
export const variantColor = (i: number) => `var(${variantToken(i)})`;

// ---------------------------------------------------------------- keys

const MAX_KEY = 100;

/** A flag key from its name: lowercase ASCII words joined by `-`. */
export function flagKeyFromName(name: string): string {
  return name
    .normalize('NFKD')
    .replace(/\p{M}/gu, '')
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-+|-+$/g, '')
    .slice(0, MAX_KEY)
    .replace(/-+$/, '');
}

/** The server's rule: lowercase letters, digits, `-`, `_` and `.`, starting with a letter or digit. */
export const validKey = (key: string) => /^[a-z0-9][a-z0-9._-]{0,99}$/.test(key);

// ---------------------------------------------------------------- variant values

export function parseVariantValue(type: FlagType, text: string): { value: unknown; error?: undefined } | { value?: undefined; error: string } {
  const t = text.trim();
  switch (type) {
    case 'boolean':
      if (t === 'true' || t === 'false') return { value: t === 'true' };
      return { error: 'Use true or false' };
    case 'string':
      return { value: text };
    case 'integer':
      if (/^-?\d+$/.test(t) && Number.isSafeInteger(Number(t))) return { value: Number(t) };
      return { error: 'Enter a whole number' };
    case 'number':
      if (t !== '' && Number.isFinite(Number(t))) return { value: Number(t) };
      return { error: 'Enter a number' };
    case 'json': {
      try {
        const v: unknown = JSON.parse(t);
        if (v !== null && typeof v === 'object') return { value: v };
        return { error: 'A JSON value must be an object or an array' };
      } catch {
        return { error: 'Not valid JSON' };
      }
    }
  }
}

export function formatVariantValue(type: FlagType, value: unknown): string {
  if (type === 'json') return JSON.stringify(value, null, 2);
  return String(value);
}

/** A short one-line view of a value (lists, chips). */
export function valuePreview(value: unknown): string {
  const s = typeof value === 'string' ? JSON.stringify(value) : JSON.stringify(value);
  return s.length > 60 ? `${s.slice(0, 57)}…` : s;
}

export function defaultVariants(type: FlagType): FlagVariant[] {
  switch (type) {
    case 'boolean':
      return [
        { key: 'true', value: true },
        { key: 'false', value: false },
      ];
    case 'string':
      return [
        { key: 'control', value: 'control' },
        { key: 'treatment', value: 'treatment' },
      ];
    case 'integer':
      return [
        { key: 'low', value: 1 },
        { key: 'high', value: 10 },
      ];
    case 'number':
      return [
        { key: 'low', value: 0.1 },
        { key: 'high', value: 0.9 },
      ];
    case 'json':
      return [
        { key: 'default', value: {} },
        { key: 'alternative', value: { enabled: true } },
      ];
  }
}

// ---------------------------------------------------------------- Create Flag draft

export interface VariantDraft {
  id: number;
  key: string;
  /** The value as typed. */
  text: string;
}

export interface FlagDraft {
  name: string;
  /** Empty: derived from the name. */
  key: string;
  description: string;
  teams: string[];
  tags: string[];
  sdkAccess: FlagSdkAccess;
  lifecycle: FlagLifecycle;
  flagType: FlagType;
  variants: VariantDraft[];
  defaultIndex: number;
}

let seq = 0;
const draftVariants = (type: FlagType): VariantDraft[] => defaultVariants(type).map((v) => ({ id: ++seq, key: v.key, text: formatVariantValue(type, v.value) }));

export function newFlagDraft(): FlagDraft {
  return { name: '', key: '', description: '', teams: [], tags: [], sdkAccess: 'all', lifecycle: 'temporary', flagType: 'boolean', variants: draftVariants('boolean'), defaultIndex: 1 };
}

export function withFlagType(d: FlagDraft, flagType: FlagType): FlagDraft {
  return { ...d, flagType, variants: draftVariants(flagType), defaultIndex: flagType === 'boolean' ? 1 : 0 };
}

/** The draft of an existing flag (Edit). */
export function draftFromFlag(f: FeatureFlag): FlagDraft {
  return {
    name: f.name,
    key: f.key,
    description: f.description,
    teams: [...f.teams],
    tags: [...f.tags],
    sdkAccess: f.sdkAccess,
    lifecycle: f.lifecycle,
    flagType: f.flagType,
    variants: f.variants.map((v) => ({ id: ++seq, key: v.key, text: formatVariantValue(f.flagType, v.value) })),
    defaultIndex: Math.max(
      0,
      f.variants.findIndex((v) => v.key === f.defaultVariant),
    ),
  };
}

export const newVariantDraft = (d: FlagDraft): VariantDraft => {
  const n = d.variants.length + 1;
  const text = d.flagType === 'json' ? '{}' : d.flagType === 'string' ? `variant-${n}` : String(n);
  return { id: ++seq, key: `variant-${n}`, text };
};

export const draftKey = (d: FlagDraft) => d.key.trim() || flagKeyFromName(d.name);

/** Variant problems by draft id. */
export function variantProblems(d: FlagDraft): Map<number, string> {
  const out = new Map<number, string>();
  const seen = new Set<string>();
  for (const v of d.variants) {
    const k = v.key.trim();
    const parsed = parseVariantValue(d.flagType, v.text);
    if (!k) out.set(v.id, 'Name the variant');
    else if (seen.has(k)) out.set(v.id, `Variant '${k}' appears twice`);
    else if (parsed.error) out.set(v.id, parsed.error);
    seen.add(k);
  }
  return out;
}

/** The steps (1: details, 2: variants) still to complete. */
export function draftProblems(d: FlagDraft): number[] {
  const out: number[] = [];
  if (!d.name.trim() || !validKey(draftKey(d))) out.push(1);
  if (d.variants.length === 0 || variantProblems(d).size > 0) out.push(2);
  return out;
}

export function inputFromDraft(d: FlagDraft): FeatureFlagInput {
  const variants = d.variants.map((v) => ({ key: v.key.trim(), value: parseVariantValue(d.flagType, v.text).value }));
  return {
    key: draftKey(d),
    name: d.name.trim(),
    description: d.description.trim(),
    flagType: d.flagType,
    variants,
    defaultVariant: variants[Math.min(d.defaultIndex, variants.length - 1)]?.key ?? '',
    sdkAccess: d.sdkAccess,
    lifecycle: d.lifecycle,
    teams: d.teams,
    tags: d.tags,
  };
}

// ---------------------------------------------------------------- serving

export const splitTotal = (split: FlagSplit[]) => Math.round(split.reduce((s, p) => s + p.weight, 0) * 100) / 100;
export const splitValid = (split: FlagSplit[]) => split.length > 0 && Math.abs(splitTotal(split) - 100) <= 0.011 && split.every((p) => p.weight >= 0);

export function serveLabel(s: FlagServe): string {
  if (s.variant) return s.variant;
  return (s.split ?? []).map((p) => `${p.variant} ${p.weight}%`).join(' · ');
}

/** An even split across the variants (the last one takes the rounding). */
export function evenSplit(keys: string[]): FlagSplit[] {
  const each = Math.floor(10_000 / Math.max(keys.length, 1)) / 100;
  return keys.map((variant, i) => ({ variant, weight: i === keys.length - 1 ? Math.round((100 - each * (keys.length - 1)) * 100) / 100 : each }));
}

/** The rollout's percent at `now` (0 before it starts, the last step's after). */
export function rolloutPercent(r: FlagRollout, now: number): number {
  const i = rolloutStepIndex(r, now);
  return i < 0 ? 0 : r.steps[i].percent;
}

/** The step running at `now` (-1 before the start or without steps). */
export function rolloutStepIndex(r: FlagRollout, now: number): number {
  const started = r.startedAt ?? 0;
  if (started <= 0 || now < started || r.steps.length === 0) return -1;
  let end = started;
  for (let i = 0; i < r.steps.length; i++) {
    end += r.steps[i].durationMinutes * 60_000;
    if (now < end) return i;
  }
  return r.steps.length - 1;
}

/** Whether an environment query (`staging-*`) matches an SDK's env value. */
export function envMatches(query: string, env: string): boolean {
  const q = query.trim().toLowerCase();
  if (!q) return false;
  const re = new RegExp(
    `^${q
      .split('*')
      .map((p) => p.replace(/[.+?^${}()|[\]\\]/g, '\\$&'))
      .join('.*')}$`,
  );
  return re.test(env.trim().toLowerCase());
}

// ---------------------------------------------------------------- evaluations

/** Evaluations in a series of per-second rates: each point's rate times its interval. */
export function totalFromRates(points: [number, number][], stepMs: number): number {
  return Math.round((points.reduce((s, [, v]) => s + v, 0) * stepMs) / 1000);
}

/** The interval the metrics API answers `points` over `range` in (routes/metrics.rs `step_ms`); series skip empty intervals. */
export function metricStepMs(range: { from: number; to: number }, points: number): number {
  const raw = Math.max((range.to - range.from) / Math.max(points, 1), 30_000);
  return [10_000, 20_000, 30_000, 60_000, 120_000, 300_000, 600_000, 1_800_000, 3_600_000, 10_800_000, 21_600_000].find((s) => s >= raw) ?? 43_200_000;
}

export function fmtCount(n: number): string {
  if (n >= 1e9) return `${(n / 1e9).toFixed(1)}B`;
  if (n >= 1e6) return `${(n / 1e6).toFixed(1)}M`;
  if (n >= 1e3) return `${(n / 1e3).toFixed(1)}K`;
  return String(Math.round(n));
}

// ---------------------------------------------------------------- list search and facets

type ListedFlag = Pick<FeatureFlag, 'key' | 'name' | 'flagType' | 'teams' | 'tags' | 'lifecycle'> & { environments: { config: { enabled?: boolean } }[] };

export type FacetKey = 'status' | 'type' | 'team' | 'lifecycle';
export type FacetSelection = Partial<Record<FacetKey, string[]>>;

const statusOf = (f: ListedFlag) => (f.environments.some((e) => e.config.enabled) ? 'on' : 'off');

function facetValues(f: ListedFlag, key: FacetKey): string[] {
  switch (key) {
    case 'status':
      return [statusOf(f)];
    case 'type':
      return [f.flagType];
    case 'team':
      return f.teams;
    case 'lifecycle':
      return [f.lifecycle];
  }
}

export const FACETS: { key: FacetKey; label: string }[] = [
  { key: 'status', label: 'Status' },
  { key: 'type', label: 'Type' },
  { key: 'team', label: 'Team' },
  { key: 'lifecycle', label: 'Lifecycle' },
];

export function filterFlags<T extends ListedFlag>(flags: T[], query: string, selected: FacetSelection): T[] {
  const words = query.toLowerCase().split(/\s+/).filter(Boolean);
  return flags.filter((f) => {
    const text = [f.key, f.name, ...f.tags, ...f.teams].join(' ').toLowerCase();
    if (!words.every((w) => text.includes(w))) return false;
    return FACETS.every(({ key }) => {
      const want = selected[key];
      return !want?.length || facetValues(f, key).some((v) => want.includes(v));
    });
  });
}

// ---------------------------------------------------------------- OpenFeature setup

export interface Snippet {
  id: 'browser' | 'node' | 'go' | 'python';
  label: string;
  note: string;
  code: string;
}

const accessor = (type: FlagType, lang: 'js' | 'go' | 'py') => {
  const js: Record<FlagType, [string, string]> = {
    boolean: ['getBooleanValue', 'false'],
    string: ['getStringValue', "'default'"],
    integer: ['getNumberValue', '0'],
    number: ['getNumberValue', '0'],
    json: ['getObjectValue', '{}'],
  };
  const go: Record<FlagType, [string, string]> = {
    boolean: ['BooleanValue', 'false'],
    string: ['StringValue', '"default"'],
    integer: ['IntValue', '0'],
    number: ['FloatValue', '0'],
    json: ['ObjectValue', 'map[string]any{}'],
  };
  const py: Record<FlagType, [string, string]> = {
    boolean: ['get_boolean_value', 'False'],
    string: ['get_string_value', '"default"'],
    integer: ['get_integer_value', '0'],
    number: ['get_float_value', '0.0'],
    json: ['get_object_value', '{}'],
  };
  return (lang === 'js' ? js : lang === 'go' ? go : py)[type];
};

/**
 * How OpenFeature SDKs evaluate a flag through this install's intake (OFREP), for the setup panel.
 * `{{intake}}` and `{{apiKey}}` are filled in by the page (setup/sources.ts `fill`).
 */
export function ofrepSnippets(flagKey: string, type: FlagType): Snippet[] {
  const [intake, apiKey] = ['{{intake}}', '{{apiKey}}'];
  const [jsGet, jsDefault] = accessor(type, 'js');
  const [goGet, goDefault] = accessor(type, 'go');
  const [pyGet, pyDefault] = accessor(type, 'py');
  return [
    {
      id: 'browser',
      label: 'Browser',
      note: 'Client SDKs fetch every flag the browser may see in one request and re-check it with the ETag; only flags open to client SDKs are answered. Their users can read the variant values.',
      code: [
        'npm install @openfeature/web-sdk @openfeature/ofrep-web-provider',
        '',
        "import { OpenFeature } from '@openfeature/web-sdk';",
        "import { OFREPWebProvider } from '@openfeature/ofrep-web-provider';",
        '',
        "await OpenFeature.setContext({ targetingKey: user.id, env: 'production' });",
        'await OpenFeature.setProviderAndWait(',
        `  new OFREPWebProvider({ baseUrl: '${intake}', headers: [['Authorization', 'Bearer ${apiKey}']], pollInterval: 30_000 }),`,
        ');',
        '',
        `const value = OpenFeature.getClient().${jsGet}('${flagKey}', ${jsDefault});`,
      ].join('\n'),
    },
    {
      id: 'node',
      label: 'Node.js',
      note: 'Server SDKs evaluate one flag per call with that request’s context.',
      code: [
        'npm install @openfeature/server-sdk @openfeature/ofrep-provider',
        '',
        "import { OpenFeature } from '@openfeature/server-sdk';",
        "import { OFREPProvider } from '@openfeature/ofrep-provider';",
        '',
        `await OpenFeature.setProviderAndWait(new OFREPProvider({ baseUrl: '${intake}', headers: [['Authorization', 'Bearer ${apiKey}']] }));`,
        'const client = OpenFeature.getClient();',
        '',
        `const value = await client.${jsGet}('${flagKey}', ${jsDefault}, { targetingKey: user.id, env: 'production', country: user.country });`,
      ].join('\n'),
    },
    {
      id: 'go',
      label: 'Go',
      note: 'The OFREP provider from the OpenFeature Go contrib repository.',
      code: [
        'go get github.com/open-feature/go-sdk github.com/open-feature/go-sdk-contrib/providers/ofrep',
        '',
        'provider := ofrep.NewProvider("' + intake + '", ofrep.WithBearerToken("' + apiKey + '"))',
        'openfeature.SetProviderAndWait(provider)',
        'client := openfeature.NewClient("checkout")',
        '',
        'ctx := openfeature.NewEvaluationContext(user.ID, map[string]any{"env": "production", "country": user.Country})',
        `value, err := client.${goGet}(context.Background(), "${flagKey}", ${goDefault}, ctx)`,
      ].join('\n'),
    },
    {
      id: 'python',
      label: 'Python',
      note: 'The OFREP provider from the OpenFeature Python contrib repository.',
      code: [
        'pip install openfeature-sdk openfeature-provider-ofrep',
        '',
        'from openfeature import api',
        'from openfeature.evaluation_context import EvaluationContext',
        'from openfeature.contrib.provider.ofrep import OFREPProvider',
        '',
        `api.set_provider(OFREPProvider(base_url="${intake}", headers_factory=lambda: {"Authorization": "Bearer ${apiKey}"}))`,
        'client = api.get_client()',
        '',
        'ctx = EvaluationContext(targeting_key=user.id, attributes={"env": "production", "country": user.country})',
        `value = client.${pyGet}("${flagKey}", ${pyDefault}, ctx)`,
      ].join('\n'),
    },
  ];
}

// ---------------------------------------------------------------- environment configuration checks

function serveProblem(s: FlagServe, keys: string[], what: string): string | null {
  const split = s.split ?? [];
  if (s.variant && split.length) return `${what} serves either a variant or a split, not both`;
  if (s.variant) return keys.includes(s.variant) ? null : `${what} serves '${s.variant}', which is not a variant of this flag`;
  if (!split.length) return `${what} must serve a variant or a split`;
  const unknown = split.find((p) => !keys.includes(p.variant));
  if (unknown) return `${what} splits to '${unknown.variant}', which is not a variant of this flag`;
  if (!splitValid(split)) return `${what}: split weights add up to ${splitTotal(split)}, not 100`;
  return null;
}

/** What the server would refuse in an environment configuration (the oc-flags `validate_config` rules), or null. */
export function configProblem(cfg: FlagEnvConfig, variants: FlagVariant[]): string | null {
  const keys = variants.map((v) => v.key);
  for (const [i, r] of (cfg.rules ?? []).entries()) {
    const what = `Rule ${i + 1}`;
    for (const c of r.conditions ?? []) {
      if (!c.attribute.trim()) return `${what}: every condition needs an attribute`;
      const op = OPERATORS.find((o) => o.id === c.operator);
      if (op?.values !== 'none' && !(c.values ?? []).some((v) => v !== '')) return `${what}: the condition on '${c.attribute}' needs a value`;
      if (op?.values === 'one' && !Number.isFinite(Number((c.values ?? [])[0]))) return `${what}: the condition on '${c.attribute}' compares with a number`;
    }
    const p = serveProblem(r.serve, keys, what);
    if (p) return p;
  }
  const p = serveProblem(cfg.defaultServe ?? {}, keys, 'The default rule');
  if (p) return p;
  const r = cfg.rollout;
  if (r) {
    if (!keys.includes(r.variant)) return `The rollout serves '${r.variant}', which is not a variant of this flag`;
    if (!r.steps.length) return 'A rollout has 1–20 steps';
    if (r.steps.some((s) => !(s.percent >= 0 && s.percent <= 100))) return 'Rollout percentages are between 0 and 100';
    if (r.steps.some((s, i) => i > 0 && s.percent < r.steps[i - 1].percent)) return 'Rollout steps must not decrease';
  }
  return null;
}

/** The list toolbar's filters (the facets of the reference's flag list). */
export interface ToolbarFilter {
  /** Only flags I created. */
  mine?: boolean;
  team?: string;
  /** Temporary flags nobody changed for 30 days: candidates for removal. */
  stale?: boolean;
  /** Enabled or not in the production environment(s). */
  prod?: 'on' | 'off';
}

const STALE_MS = 30 * 86_400_000;

type ToolbarFlag = { createdBy?: string | null; teams: string[]; lifecycle: string; updatedAt: number; environments: { isProduction: boolean; config: { enabled?: boolean } }[] };

export function toolbarFilter<T extends ToolbarFlag>(flags: T[], f: ToolbarFilter, me: string | undefined, now: number): T[] {
  return flags.filter((x) => {
    if (f.mine && (!me || x.createdBy !== me)) return false;
    if (f.team && !x.teams.includes(f.team)) return false;
    if (f.stale && !(x.lifecycle === 'temporary' && now - x.updatedAt > STALE_MS)) return false;
    if (f.prod) {
      const on = x.environments.some((e) => e.isProduction && e.config.enabled);
      if ((f.prod === 'on') !== on) return false;
    }
    return true;
  });
}
