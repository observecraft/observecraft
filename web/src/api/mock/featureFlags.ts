// In-browser feature flags for mock mode (Software Delivery → Feature Flags). Flags, environments and
// history live in memory for the session and follow the server's rules (routes/feature_flags.rs):
// versions for concurrent edits, history entries, validation, and the Production and Staging
// environments created with the first flag. Evaluation counts are fictional but stable.
import { configProblem, validKey } from '../featureFlags/logic';
import type {
  FeatureFlag,
  FeatureFlagChange,
  FeatureFlagEnvironment,
  FeatureFlagEnvironmentInput,
  FeatureFlagEnvUpdate,
  FeatureFlagInput,
  FeatureFlagsPage,
  FlagEnvConfig,
  FlagEvaluations,
  FlagEvaluationSeries,
  FlagServe,
  FlagVariant,
} from '../featureFlags/types';
import type { TimeRange } from '../types';
import { listIngestKeys } from './identity';

const MIN = 60_000;
const HOUR = 60 * MIN;
const DAY = 24 * HOUR;
const ME = 'Alex Morgan';

interface StoredFlag extends Omit<FeatureFlag, 'environments'> {
  configs: Record<string, { config: FlagEnvConfig; updatedBy: string; updatedAt: number }>;
  /** Evaluations per hour, for the fictional counts. */
  rate: number;
}

interface StoredEnv extends Omit<FeatureFlagEnvironment, 'apiKeys'> {
  apiKeyIds: string[];
}

interface Db {
  envs: StoredEnv[];
  flags: StoredFlag[];
  history: FeatureFlagChange[];
  seq: number;
}

let state: Db | null = null;

const fail = (message: string): never => {
  throw new Error(message);
};

const uuid = (n: number) => `0f1a2b3c-4d5e-4f60-8a7b-${String(n).padStart(12, '0')}`;
const on = (variant: string): FlagServe => ({ variant });

function db(): Db {
  if (state) return state;
  const now = Date.now();
  const s: Db = { envs: [], flags: [], history: [], seq: 100 };
  const env = (n: number, name: string, queries: string[], isProduction: boolean, requiresApproval: boolean, age: number): StoredEnv => ({
    id: uuid(n),
    name,
    queries,
    isProduction,
    requiresApproval,
    apiKeyIds: [],
    createdAt: now - age,
    updatedAt: now - age,
  });
  s.envs = [
    env(1, 'Production', ['production', 'prod'], true, true, 40 * DAY),
    env(2, 'Staging', ['staging', 'staging-*'], false, false, 40 * DAY),
    env(3, 'Development', ['dev-*', 'local'], false, false, 12 * DAY),
  ];
  const [prod, stg, dev] = s.envs.map((e) => e.id);
  const bool: FlagVariant[] = [
    { key: 'true', value: true },
    { key: 'false', value: false },
  ];
  const add = (f: Omit<StoredFlag, 'id' | 'version' | 'createdBy' | 'updatedBy' | 'updatedAt'> & { updatedAt?: number }, changes: [number, string, string, string, string][]) => {
    const id = uuid(10 + s.flags.length);
    const flag: StoredFlag = { ...f, id, version: changes.length + 1, createdBy: ME, updatedBy: changes.length ? changes[0][4] : ME, updatedAt: f.updatedAt ?? f.createdAt };
    s.flags.push(flag);
    s.history.push({
      id: ++s.seq,
      flagId: id,
      flagKey: f.key,
      flagName: f.name,
      environmentName: '',
      action: 'created',
      summary: `Created the ${f.flagType} flag ${f.name}`,
      actor: ME,
      version: 1,
      at: f.createdAt,
    });
    changes
      .slice()
      .reverse()
      .forEach(([ago, envName, action, summary, actor], i) =>
        s.history.push({ id: ++s.seq, flagId: id, flagKey: f.key, flagName: f.name, environmentName: envName, action, summary, actor, version: i + 2, at: now - ago }),
      );
  };
  const cfg = (config: FlagEnvConfig, ago: number, by = ME) => ({ config, updatedBy: by, updatedAt: now - ago });
  add(
    {
      key: 'checkout-new-payment-flow',
      name: 'New payment flow',
      description: 'The redesigned card form and wallet buttons on the checkout page of storefront-api.',
      flagType: 'boolean',
      variants: bool,
      defaultVariant: 'false',
      sdkAccess: 'all',
      lifecycle: 'temporary',
      teams: ['payments'],
      tags: ['service:storefront-api', 'service:billing'],
      createdAt: now - 21 * DAY,
      updatedAt: now - 3 * HOUR,
      rate: 5200,
      configs: {
        [prod]: cfg(
          {
            enabled: true,
            rules: [
              { name: 'Internal users', conditions: [{ attribute: 'email', operator: 'ends_with', values: ['@example.com'] }], serve: on('true') },
              {
                name: 'Beta countries',
                conditions: [{ attribute: 'country', operator: 'one_of', values: ['CA', 'NZ'] }],
                serve: {
                  split: [
                    { variant: 'true', weight: 50 },
                    { variant: 'false', weight: 50 },
                  ],
                },
              },
            ],
            defaultServe: on('false'),
            rollout: {
              variant: 'true',
              steps: [
                { percent: 5, durationMinutes: 240 },
                { percent: 25, durationMinutes: 720 },
                { percent: 50, durationMinutes: 1440 },
                { percent: 100, durationMinutes: 0 },
              ],
              startedAt: now - 9 * HOUR,
            },
          },
          3 * HOUR,
          'Priya Shah',
        ),
        [stg]: cfg({ enabled: true, rules: [], defaultServe: on('true') }, 6 * DAY),
        [dev]: cfg({ enabled: true, rules: [], defaultServe: on('true') }, 12 * DAY),
      },
    },
    [
      [3 * HOUR, 'Production', 'targeting', 'Targeting: 2 targeting rules', 'Priya Shah'],
      [9 * HOUR, 'Production', 'rollout', 'Started a rollout of true (4 steps)', 'Priya Shah'],
      [2 * DAY, 'Production', 'enabled', 'Turned the flag on', ME],
      [6 * DAY, 'Staging', 'targeting', 'Targeting: default serves true', ME],
      [6 * DAY, 'Staging', 'enabled', 'Turned the flag on', ME],
    ],
  );
  add(
    {
      key: 'catalog-search-v2',
      name: 'Catalog search v2',
      description: 'Ranking model for product search in catalog-service.',
      flagType: 'string',
      variants: [
        { key: 'classic', value: 'classic' },
        { key: 'semantic', value: 'semantic' },
        { key: 'hybrid', value: 'hybrid' },
      ],
      defaultVariant: 'classic',
      sdkAccess: 'client',
      lifecycle: 'temporary',
      teams: ['search'],
      tags: ['service:catalog-service'],
      createdAt: now - 9 * DAY,
      updatedAt: now - 26 * HOUR,
      rate: 14_800,
      configs: {
        [prod]: cfg(
          {
            enabled: true,
            rules: [
              {
                name: 'North America',
                conditions: [{ attribute: 'country', operator: 'one_of', values: ['US', 'CA'] }],
                serve: {
                  split: [
                    { variant: 'classic', weight: 34 },
                    { variant: 'semantic', weight: 33 },
                    { variant: 'hybrid', weight: 33 },
                  ],
                },
              },
            ],
            defaultServe: on('classic'),
          },
          26 * HOUR,
          'Daniel Kim',
        ),
        [stg]: cfg({ enabled: true, rules: [], defaultServe: on('hybrid') }, 4 * DAY, 'Daniel Kim'),
      },
    },
    [
      [26 * HOUR, 'Production', 'targeting', 'Targeting: 1 targeting rule', 'Daniel Kim'],
      [3 * DAY, 'Production', 'enabled', 'Turned the flag on', 'Daniel Kim'],
      [4 * DAY, 'Staging', 'enabled', 'Turned the flag on', 'Daniel Kim'],
    ],
  );
  add(
    {
      key: 'order-service-batch-size',
      name: 'Order export batch size',
      description: 'How many orders order-service sends to billing per export batch.',
      flagType: 'integer',
      variants: [
        { key: 'small', value: 50 },
        { key: 'large', value: 500 },
      ],
      defaultVariant: 'small',
      sdkAccess: 'server',
      lifecycle: 'permanent',
      teams: ['fulfillment'],
      tags: ['service:order-service'],
      createdAt: now - 34 * DAY,
      updatedAt: now - 8 * DAY,
      rate: 900,
      configs: {
        [prod]: cfg(
          { enabled: true, rules: [{ name: 'Large regions', conditions: [{ attribute: 'region', operator: 'one_of', values: ['us-east-1'] }], serve: on('large') }], defaultServe: on('small') },
          8 * DAY,
          'Sara Levi',
        ),
        [stg]: cfg({ enabled: true, rules: [], defaultServe: on('large') }, 20 * DAY, 'Sara Levi'),
      },
    },
    [[8 * DAY, 'Production', 'targeting', 'Targeting: 1 targeting rule', 'Sara Levi']],
  );
  add(
    {
      key: 'storefront-promo-banner',
      name: 'Holiday promo banner',
      description: 'Banner content on the storefront home page.',
      flagType: 'json',
      variants: [
        { key: 'hidden', value: { show: false } },
        { key: 'winter-sale', value: { show: true, headline: 'Winter sale', discount: 20 } },
      ],
      defaultVariant: 'hidden',
      sdkAccess: 'client',
      lifecycle: 'temporary',
      teams: ['growth'],
      tags: ['service:storefront-api', 'campaign:winter'],
      createdAt: now - 2 * DAY,
      rate: 0,
      configs: {},
    },
    [],
  );
  add(
    {
      key: 'billing-retry-backoff',
      name: 'Billing retry backoff',
      description: 'Multiplier between payment retries in billing.',
      flagType: 'number',
      variants: [
        { key: 'gentle', value: 1.5 },
        { key: 'aggressive', value: 3 },
      ],
      defaultVariant: 'gentle',
      sdkAccess: 'server',
      lifecycle: 'permanent',
      teams: ['payments'],
      tags: ['service:billing'],
      createdAt: now - 60 * DAY,
      updatedAt: now - 15 * DAY,
      rate: 320,
      configs: { [prod]: cfg({ enabled: false, rules: [], defaultServe: on('gentle') }, 15 * DAY, 'Omar Haddad') },
    },
    [[15 * DAY, 'Production', 'disabled', 'Turned the flag off', 'Omar Haddad']],
  );
  s.history.sort((a, b) => b.at - a.at || b.id - a.id);
  state = s;
  return s;
}

/** Back to the demo data (tests). */
export function resetFeatureFlags() {
  state = null;
}

const initial = (defaultVariant: string): FlagEnvConfig => ({ enabled: false, rules: [], defaultServe: { variant: defaultVariant } });
const clone = <T>(v: T): T => structuredClone(v);

async function envView(e: StoredEnv): Promise<FeatureFlagEnvironment> {
  const keys = await listIngestKeys();
  const { apiKeyIds, ...rest } = e;
  return { ...rest, apiKeys: keys.filter((k) => apiKeyIds.includes(k.id)).map((k) => ({ id: k.id, name: k.name, prefix: k.prefix })) };
}

const sortedEnvs = (s: Db) => [...s.envs].sort((a, b) => Number(b.isProduction) - Number(a.isProduction) || a.name.toLowerCase().localeCompare(b.name.toLowerCase()));

function flagView(s: Db, f: StoredFlag): FeatureFlag {
  const { configs, rate: _rate, ...rest } = f;
  return clone({
    ...rest,
    environments: sortedEnvs(s).map((e) => ({
      environmentId: e.id,
      environmentName: e.name,
      isProduction: e.isProduction,
      config: configs[e.id]?.config ?? initial(f.defaultVariant),
      updatedBy: configs[e.id]?.updatedBy ?? null,
      updatedAt: configs[e.id]?.updatedAt ?? null,
    })),
  });
}

function findFlag(s: Db, id: string): StoredFlag {
  return s.flags.find((f) => f.id === id) ?? fail('feature flag not found');
}

function record(s: Db, f: StoredFlag, environmentName: string, action: string, summary: string) {
  s.history.unshift({ id: ++s.seq, flagId: f.id, flagKey: f.key, flagName: f.name, environmentName, action, summary, actor: ME, version: f.version, at: Date.now() });
}

function checkInput(input: FeatureFlagInput) {
  if (!validKey(input.key.trim())) fail("The flag key is 1–100 characters: lowercase letters, digits, '-', '_' and '.', starting with a letter or digit");
  if (!input.name.trim()) fail('The flag name is 1–200 characters');
  if (!input.variants.length) fail('A flag needs at least one variant');
  const seen = new Set<string>();
  for (const v of input.variants) {
    if (seen.has(v.key)) fail(`Variant '${v.key}' appears twice`);
    seen.add(v.key);
  }
  if (!seen.has(input.defaultVariant)) fail(`The default variant '${input.defaultVariant}' is not one of the variants`);
}

export async function listFeatureFlags(): Promise<FeatureFlagsPage> {
  const s = db();
  const flags = [...s.flags].sort((a, b) => b.updatedAt - a.updatedAt).map((f) => flagView(s, f));
  return { flags, environments: await Promise.all(sortedEnvs(s).map(envView)) };
}

export async function getFeatureFlag(id: string): Promise<FeatureFlag> {
  const s = db();
  return flagView(s, findFlag(s, id));
}

export async function createFeatureFlag(input: FeatureFlagInput): Promise<FeatureFlag> {
  const s = db();
  checkInput(input);
  const key = input.key.trim();
  if (s.flags.some((f) => f.key === key)) fail(`A flag with the key '${key}' already exists`);
  if (!s.envs.length) {
    const now = Date.now();
    s.envs.push({ id: uuid(++s.seq), name: 'Production', queries: ['production', 'prod'], isProduction: true, requiresApproval: false, apiKeyIds: [], createdAt: now, updatedAt: now });
    s.envs.push({ id: uuid(++s.seq), name: 'Staging', queries: ['staging', 'staging-*'], isProduction: false, requiresApproval: false, apiKeyIds: [], createdAt: now, updatedAt: now });
  }
  const now = Date.now();
  const f: StoredFlag = {
    id: uuid(++s.seq),
    key,
    name: input.name.trim(),
    description: (input.description ?? '').trim(),
    flagType: input.flagType,
    variants: clone(input.variants),
    defaultVariant: input.defaultVariant,
    sdkAccess: input.sdkAccess,
    lifecycle: input.lifecycle,
    teams: input.teams ?? [],
    tags: input.tags ?? [],
    version: 1,
    createdBy: ME,
    updatedBy: ME,
    createdAt: now,
    updatedAt: now,
    configs: {},
    rate: 0,
  };
  s.flags.push(f);
  record(s, f, '', 'created', `Created the ${f.flagType} flag ${f.name}`);
  return flagView(s, f);
}

export async function updateFeatureFlag(id: string, input: FeatureFlagInput): Promise<FeatureFlag> {
  const s = db();
  const f = findFlag(s, id);
  checkInput({ ...input, key: f.key });
  if (input.version !== f.version) fail(`This flag was changed by someone else meanwhile (now version ${f.version}). Reload it and make the change again.`);
  for (const e of flagView(s, f).environments) {
    const p = configProblem(e.config, input.variants);
    if (p) fail(`${e.environmentName}: ${p}`);
  }
  Object.assign(f, {
    name: input.name.trim(),
    description: (input.description ?? '').trim(),
    variants: clone(input.variants),
    defaultVariant: input.defaultVariant,
    sdkAccess: input.sdkAccess,
    lifecycle: input.lifecycle,
    teams: input.teams ?? [],
    tags: input.tags ?? [],
    version: f.version + 1,
    updatedBy: ME,
    updatedAt: Date.now(),
  });
  record(s, f, '', 'updated', 'Changed the flag details');
  return flagView(s, f);
}

export async function deleteFeatureFlag(id: string): Promise<void> {
  const s = db();
  const f = findFlag(s, id);
  record(s, f, '', 'deleted', `Deleted ${f.name}`);
  for (const h of s.history) if (h.flagId === id) h.flagId = null;
  s.flags = s.flags.filter((x) => x.id !== id);
}

export async function updateFeatureFlagEnvironment(id: string, environmentId: string, update: FeatureFlagEnvUpdate): Promise<FeatureFlag> {
  const s = db();
  const f = findFlag(s, id);
  const env = s.envs.find((e) => e.id === environmentId) ?? fail('environment not found');
  const p = configProblem(update.config, f.variants);
  if (p) fail(p);
  if (update.version !== f.version) fail(`This flag was changed by someone else meanwhile (now version ${f.version}). Reload it and make the change again.`);
  const before = f.configs[environmentId]?.config ?? initial(f.defaultVariant);
  const config = clone(update.config);
  if (config.rollout && !(config.rollout.startedAt && config.rollout.startedAt > 0)) config.rollout.startedAt = Date.now();
  f.version += 1;
  f.updatedAt = Date.now();
  f.updatedBy = ME;
  f.configs[environmentId] = { config, updatedBy: ME, updatedAt: Date.now() };
  if (!!before.enabled !== !!config.enabled) record(s, f, env.name, config.enabled ? 'enabled' : 'disabled', config.enabled ? 'Turned the flag on' : 'Turned the flag off');
  if (JSON.stringify(before.rules) !== JSON.stringify(config.rules) || JSON.stringify(before.defaultServe) !== JSON.stringify(config.defaultServe))
    record(s, f, env.name, 'targeting', `Targeting: ${(config.rules ?? []).length} targeting rule${(config.rules ?? []).length === 1 ? '' : 's'}`);
  if (JSON.stringify(before.rollout ?? null) !== JSON.stringify(config.rollout ?? null))
    record(s, f, env.name, 'rollout', config.rollout ? `Rollout of ${config.rollout.variant} (${config.rollout.steps.length} steps)` : 'Stopped the rollout');
  return flagView(s, f);
}

export async function listFeatureFlagHistory(flagId?: string): Promise<FeatureFlagChange[]> {
  const s = db();
  return clone(s.history.filter((h) => !flagId || h.flagId === flagId));
}

export async function listFeatureFlagEnvironments(): Promise<FeatureFlagEnvironment[]> {
  return Promise.all(sortedEnvs(db()).map(envView));
}

function checkEnv(s: Db, input: FeatureFlagEnvironmentInput, id?: string) {
  const name = input.name.trim();
  if (!name || name.length > 100) fail('The environment name is 1–100 characters');
  if (s.envs.some((e) => e.id !== id && e.name.toLowerCase() === name.toLowerCase())) fail(`An environment named '${name}' already exists`);
  const queries = [...new Set((input.queries ?? []).map((q) => q.trim().toLowerCase()).filter(Boolean))];
  if (queries.some((q) => /\s/.test(q))) fail('Queries are env values without spaces (use * as a wildcard)');
  return { name, queries };
}

export async function createFeatureFlagEnvironment(input: FeatureFlagEnvironmentInput): Promise<FeatureFlagEnvironment> {
  const s = db();
  const { name, queries } = checkEnv(s, input);
  const now = Date.now();
  const e: StoredEnv = { id: uuid(++s.seq), name, queries, isProduction: !!input.isProduction, requiresApproval: !!input.requiresApproval, apiKeyIds: [], createdAt: now, updatedAt: now };
  if (input.apiKeyIds) setKeys(s, e, input.apiKeyIds);
  s.envs.push(e);
  return envView(e);
}

function setKeys(s: Db, e: StoredEnv, ids: string[]) {
  for (const other of s.envs) other.apiKeyIds = other.apiKeyIds.filter((k) => !ids.includes(k));
  e.apiKeyIds = [...ids];
}

export async function updateFeatureFlagEnvironmentSettings(id: string, input: FeatureFlagEnvironmentInput): Promise<FeatureFlagEnvironment> {
  const s = db();
  const e = s.envs.find((x) => x.id === id) ?? fail('environment not found');
  const { name, queries } = checkEnv(s, input, id);
  Object.assign(e, { name, queries, isProduction: !!input.isProduction, requiresApproval: !!input.requiresApproval, updatedAt: Date.now() });
  if (input.apiKeyIds) setKeys(s, e, input.apiKeyIds);
  return envView(e);
}

export async function deleteFeatureFlagEnvironment(id: string): Promise<void> {
  const s = db();
  if (!s.envs.some((e) => e.id === id)) fail('environment not found');
  s.envs = s.envs.filter((e) => e.id !== id);
  for (const f of s.flags) delete f.configs[id];
}

/** A stable pseudo-random number in [0, 1) for (seed, t). */
function noise(seed: number, t: number): number {
  let h = (Math.imul(seed, 2_654_435_761) + Math.floor(t / 1000)) >>> 0;
  h = Math.imul(h ^ (h >>> 16), 2_246_822_507) >>> 0;
  h = Math.imul(h ^ (h >>> 13), 3_266_489_909) >>> 0;
  return ((h ^ (h >>> 16)) >>> 0) / 4_294_967_296;
}

function seedOf(s: string): number {
  let h = 0;
  for (const c of s) h = (h * 31 + c.charCodeAt(0)) >>> 0;
  return h;
}

/** Evaluations of every flag over the range. */
export async function featureFlagEvaluations(range: TimeRange): Promise<FlagEvaluations[]> {
  const hours = Math.max(0, (range.to - range.from) / HOUR);
  return db().flags.map((f) => ({ flag: f.key, total: Math.round(f.rate * hours * (0.85 + 0.3 * noise(seedOf(f.key), range.from))) }));
}

/** One flag's evaluations per variant over the range, in `points` intervals. */
export async function featureFlagEvaluationSeries(flagKey: string, range: TimeRange, points = 48): Promise<FlagEvaluationSeries[]> {
  const f = db().flags.find((x) => x.key === flagKey);
  if (!f || !f.rate) return [];
  const step = Math.max(MIN, Math.floor((range.to - range.from) / points));
  const shares = f.variants.map((_, i) => (i === 0 ? 0.6 : 0.4 / Math.max(1, f.variants.length - 1)));
  return f.variants.map((v, i) => ({
    variant: v.key,
    points: Array.from({ length: points }, (_, j) => {
      const t = range.from + j * step;
      if (t < f.createdAt) return [t, 0] as [number, number];
      const daily = 0.75 + 0.5 * Math.sin(((t % DAY) / DAY) * 2 * Math.PI);
      return [t, Math.round(((f.rate * step) / HOUR) * shares[i] * daily * (0.8 + 0.4 * noise(seedOf(v.key) + i, t)))] as [number, number];
    }),
  }));
}
