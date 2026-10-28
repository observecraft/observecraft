// In-browser scorecards for mock mode (Developer Portal → Scorecards). State lives in memory for the
// session. Services come from the mock APM services, monitors and SLOs from their mocks; versions,
// logs and CI deployments per service are fixed fictional facts. Evaluation, levels and validation
// follow the server (routes/scorecards.rs, routes/scorecard_rules.rs).
import { parseQuery } from '../../lib/searchQuery';
import { DAY, evaluateBuiltin, referencesService, score, serviceLevel, type ServiceFacts, type VersionSeen } from '../scorecards/logic';
import type {
  Scorecard,
  ScorecardOutcome,
  ScorecardOutcomesInput,
  ScorecardOutcomeState,
  ScorecardRecommendation,
  ScorecardRule,
  ScorecardRuleInput,
  ScorecardRuleUpdate,
  ScorecardService,
  ScorecardsOverview,
  ScorecardSetupInput,
  ServiceContact,
  ServiceMetadataInput,
} from '../scorecards/types';
import { listServices } from './apm';
import { versionAt, deployedAt } from './apmTraces';
import { matchesTerms } from './match';
import { searchMonitors } from './monitors';
import { searchSlos } from './slos';

const MAX_CUSTOM_RULES = 200;
const NO_OUTCOME = 'No outcome reported for this service';

const RECOMMENDED: (ScorecardRecommendation & { description: string })[] = [
  {
    key: 'deployment_tracking',
    name: 'Deployment tracking is active',
    level: 2,
    scorecard: 'Observability Best Practices',
    description: "The service's spans of the past 7 days carry service.version, so deployments can be compared.",
  },
  {
    key: 'logs_correlation',
    name: 'Logs correlation is active',
    level: 3,
    scorecard: 'Observability Best Practices',
    description: "The service's logs of the past 7 days carry trace ids, so logs and traces link to each other.",
  },
  { key: 'code_repos_defined', name: 'Code repos defined', level: 2, scorecard: 'Ownership & Documentation', description: "The service's metadata links to its code repository." },
  {
    key: 'contacts_defined',
    name: 'Contacts defined',
    level: 2,
    scorecard: 'Ownership & Documentation',
    description: "The service's metadata lists at least one contact (email, chat channel or link).",
  },
  { key: 'docs_defined', name: 'Docs defined', level: 2, scorecard: 'Ownership & Documentation', description: "The service's metadata links to its documentation or runbook." },
  { key: 'team_defined', name: 'Team defined', level: 1, scorecard: 'Ownership & Documentation', description: "The service's metadata names the team that owns it." },
  {
    key: 'deployed_recently',
    name: 'Deployed in the past 3 months',
    level: 3,
    scorecard: 'Production Readiness',
    description: 'A new version of the service was seen on spans, or a CI job deployed it, in the past 90 days.',
  },
  { key: 'monitors_defined', name: 'Monitors defined', level: 1, scorecard: 'Production Readiness', description: "At least one monitor's query or tags reference service:<name>." },
  { key: 'on_call_defined', name: 'On-call defined', level: 1, scorecard: 'Production Readiness', description: 'The owning team has an on-call schedule.' },
  { key: 'slos_defined', name: 'SLOs defined', level: 2, scorecard: 'Production Readiness', description: "At least one SLO's tags or queries reference service:<name>." },
];

const SCORECARD_DESCRIPTIONS: Record<string, string> = {
  'Observability Best Practices': 'Telemetry that makes a service easy to debug: versions on spans and logs tied to traces.',
  'Ownership & Documentation': 'Who owns a service, how to reach them, where its code and docs live.',
  'Production Readiness': 'What a service needs before it takes production traffic: monitors, SLOs, on-call and recent deploys.',
};

interface Metadata {
  team: string;
  contacts: ServiceContact[];
  repoUrl: string;
  docsUrl: string;
}

/** Logs of the past 7 days and those with a trace id, per service (fictional). */
const LOGS: Record<string, [logs: number, traced: number]> = {
  'api-gateway': [182_000, 171_500],
  'storefront-api': [96_400, 88_100],
  'order-service': [54_200, 51_900],
  billing: [12_800, 0],
  'inventory-service': [31_700, 12_400],
  'catalog-service': [44_000, 40_300],
  'workflow-engine': [7_900, 0],
};
/** Services whose spans don't carry service.version (fictional). */
const UNVERSIONED = new Set(['notification-worker']);
/** Last CI deployment job per service, days ago (fictional). */
const CI_DEPLOYS: Record<string, number> = { billing: 41 };

function seedMetadata(): Map<string, Metadata> {
  const m = (team: string, contacts: ServiceContact[], repo: string, docs: string): Metadata => ({ team, contacts, repoUrl: repo, docsUrl: docs });
  const email = (value: string): ServiceContact => ({ type: 'email', name: 'On-call', value });
  const slack = (value: string): ServiceContact => ({ type: 'slack', name: '', value });
  return new Map([
    [
      'order-service',
      m('checkout', [email('checkout-oncall@example.com'), slack('#checkout')], 'https://git.example.com/example-org/order-service', 'https://docs.example.com/services/order-service'),
    ],
    ['inventory-service', m('checkout', [slack('#checkout')], 'https://git.example.com/example-org/inventory-service', '')],
    ['storefront-api', m('storefront', [email('storefront@example.com')], 'https://git.example.com/example-org/storefront-api', 'https://docs.example.com/services/storefront-api')],
    ['api-gateway', m('platform', [], 'https://git.example.com/example-org/api-gateway', '')],
    ['catalog-service', m('catalog', [slack('#catalog')], '', 'https://docs.example.com/services/catalog-service')],
  ]);
}

interface RuleRow extends ScorecardRule {
  manual: Map<string, { state: ScorecardOutcomeState; remarks: string }>;
}

interface Db {
  setUp: boolean;
  scorecards: Scorecard[];
  rules: RuleRow[];
  metadata: Map<string, Metadata>;
  outcomes: ScorecardOutcome[];
  evaluatedAt: number | null;
  telemetryAt: number | null;
}

let state: Db | null = null;
let seq = 0;

function db(): Db {
  state ??= { setUp: false, scorecards: [], rules: [], metadata: seedMetadata(), outcomes: [], evaluatedAt: null, telemetryAt: null };
  return state;
}

/** The services' metadata, shared with the Software Catalog mock like the server's service_metadata table. */
export function mockServiceMetadata(): ReadonlyMap<string, Metadata> {
  return db().metadata;
}

/** Stores a service's metadata from the Software Catalog mock (service definitions) and re-evaluates. */
export async function setMockServiceMetadata(service: string, meta: Metadata): Promise<void> {
  db().metadata.set(service, meta);
  if (db().setUp) await evaluate();
}

/** Starts the mock over (tests). */
export function resetScorecardsMock(): void {
  state = null;
}

const fail = (message: string): never => {
  throw new Error(message);
};
const uuid = () => `00000000-0000-4000-8000-${String(++seq).padStart(12, '0')}`;

function instrumentedServices(now: number): { service: string; version: string | null }[] {
  return listServices({ range: { from: now - 7 * DAY, to: now }, env: null })
    .filter((s) => s.instrumented)
    .map((s) => ({ service: s.service, version: s.version ?? null }));
}

function versionsOf(service: string, now: number): VersionSeen[] {
  // Spans are kept 15 days.
  const kept = now - 15 * DAY;
  if (UNVERSIONED.has(service)) return [{ version: '', firstSeen: kept, lastSeen: now, recentSpans: 1200 }];
  const current = versionAt(service, now);
  const deployed = deployedAt(service, kept + DAY);
  if (deployed === null) return [{ version: current, firstSeen: kept, lastSeen: now, recentSpans: 1200 }];
  const previous = versionAt(service, deployed - 1);
  return [
    { version: previous, firstSeen: kept, lastSeen: deployed, recentSpans: deployed > now - 7 * DAY ? 300 : 0 },
    { version: current, firstSeen: deployed, lastSeen: now, recentSpans: 900 },
  ];
}

async function evaluate(): Promise<void> {
  const s = db();
  const now = Date.now();
  const spans = instrumentedServices(now);
  const names = [...new Set([...spans.map((x) => x.service), ...s.metadata.keys()])].sort();
  const monitors = searchMonitors({
    query: '',
    statuses: [],
    muted: [],
    priorities: [],
    types: [],
    drafts: [false],
    recipients: [],
    creators: [],
    teams: [],
    sort: 'name',
    offset: 0,
    limit: 200,
  }).monitors;
  const slos = (await searchSlos({ query: '', offset: 0, limit: 200, burnRates: [], creators: [], envs: [], services: [], states: [], tags: [], teams: [], types: [], sort: null, window: null })).slos;
  const facts = new Map<string, ServiceFacts>(
    names.map((service) => {
      const meta = s.metadata.get(service);
      const mentions = (t: string) => referencesService(t, service);
      const mons = monitors.filter((m) => mentions(m.query) || m.tags.some(mentions));
      const monIds = new Set(mons.map((m) => m.id));
      const sloNames = slos.filter((o) => [o.goodQuery, o.totalQuery, o.sliceQuery, ...o.tags].some((t) => t && mentions(t)) || o.monitorIds.some((id) => monIds.has(id))).map((o) => o.name);
      const [logs, traced] = LOGS[service] ?? [0, 0];
      const ci = CI_DEPLOYS[service];
      return [
        service,
        {
          team: meta?.team ?? '',
          contacts: meta?.contacts.length ?? 0,
          repoUrl: meta?.repoUrl ?? '',
          docsUrl: meta?.docsUrl ?? '',
          versions: spans.some((x) => x.service === service) ? versionsOf(service, now) : [],
          logs,
          tracedLogs: traced,
          lastCiDeploy: ci === undefined ? null : now - ci * DAY,
          monitors: mons.map((m) => m.name),
          slos: sloNames,
        },
      ];
    }),
  );
  const outcomes: ScorecardOutcome[] = [];
  for (const rule of s.rules.filter((r) => r.enabled)) {
    const terms = parseQuery(rule.scope);
    for (const service of names) {
      const f = facts.get(service)!;
      const covered = matchesTerms((k) => (k === 'service' || k === 'name' ? service : k === 'team' ? f.team || undefined : k === 'kind' ? 'service' : undefined), service, terms);
      if (!covered) continue;
      const verdict = rule.builtin
        ? evaluateBuiltin(rule.builtin, f, now)
        : (() => {
            const m = rule.manual.get(service);
            return m ? { state: m.state, reason: m.remarks } : { state: 'skip' as const, reason: NO_OUTCOME };
          })();
      outcomes.push({ ruleId: rule.id, service, ...verdict });
    }
  }
  s.outcomes = outcomes;
  s.evaluatedAt = now;
  s.telemetryAt = now;
}

function services(): ScorecardService[] {
  const s = db();
  const now = Date.now();
  const instrumented = new Set(instrumentedServices(now).map((x) => x.service));
  const names = [...new Set([...instrumented, ...s.metadata.keys()])].sort();
  const ruleById = new Map(s.rules.map((r) => [r.id, r]));
  return names.map((service) => {
    const mine = s.outcomes.filter((o) => o.service === service && ruleById.has(o.ruleId)).map((o) => ({ rule: ruleById.get(o.ruleId)!, state: o.state, level: ruleById.get(o.ruleId)!.level }));
    const meta = s.metadata.get(service);
    const sc = score(mine);
    return {
      service,
      team: meta?.team ?? '',
      contacts: meta?.contacts ?? [],
      repoUrl: meta?.repoUrl ?? '',
      docsUrl: meta?.docsUrl ?? '',
      instrumented: instrumented.has(service),
      level: serviceLevel(mine),
      levels: Object.fromEntries(s.scorecards.map((c) => [c.id, serviceLevel(mine.filter((m) => m.rule.scorecardId === c.id))])),
      score: sc === null ? null : Math.round(sc * 10) / 10,
      pass: mine.filter((m) => m.state === 'pass').length,
      fail: mine.filter((m) => m.state === 'fail').length,
      skip: mine.filter((m) => m.state === 'skip').length,
    };
  });
}

function overview(): ScorecardsOverview {
  const s = db();
  const recommended = RECOMMENDED.map((r) => ({ ...r }));
  if (!s.setUp) return { setUp: false, evaluatedAt: null, telemetryAt: null, nextEvaluationAt: null, scorecards: [], rules: [], outcomes: [], services: [], recommended };
  const order = new Map(s.scorecards.map((c, i) => [c.id, i]));
  const rules = [...s.rules].sort((a, b) => order.get(a.scorecardId)! - order.get(b.scorecardId)! || a.level - b.level || a.name.localeCompare(b.name)).map(({ manual: _manual, ...r }) => ({ ...r }));
  return {
    setUp: true,
    evaluatedAt: s.evaluatedAt,
    telemetryAt: s.telemetryAt,
    nextEvaluationAt: (s.telemetryAt ?? Date.now()) + 3_600_000,
    scorecards: s.scorecards.map((c) => ({ ...c })),
    rules,
    outcomes: [...s.outcomes].sort((a, b) => a.service.localeCompare(b.service)),
    services: services(),
    recommended,
  };
}

function scorecardNamed(name: string, description = ''): Scorecard {
  const s = db();
  const found = s.scorecards.find((c) => c.name.toLowerCase() === name.toLowerCase());
  if (found) return found;
  const card = { id: uuid(), name, description };
  s.scorecards.push(card);
  return card;
}

function ruleRow(id: string): RuleRow {
  return db().rules.find((r) => r.id === id) ?? fail('No such scorecard rule');
}

function text(value: string | null | undefined, what: string, max: number, required: boolean): string {
  const v = (value ?? '').trim();
  if (required && !v) fail(`The ${what} is required`);
  if ([...v].length > max) fail(`The ${what} has at most ${max} characters`);
  return v;
}

function level(l: number): number {
  return Number.isInteger(l) && l >= 1 && l <= 3 ? l : fail('The level is 1, 2 or 3');
}

function httpUrl(url: string, what: string): string {
  const u = url.trim();
  if (!u) return '';
  if (!/^https?:\/\/\S+$/i.test(u)) fail(`The ${what} must be an http(s) URL`);
  return u;
}

const requireSetUp = () => db().setUp || fail('Set up scorecards first');

export async function getScorecards(): Promise<ScorecardsOverview> {
  return overview();
}

export async function setupScorecards(input: ScorecardSetupInput): Promise<ScorecardsOverview> {
  const unknown = input.rules.find((k) => !RECOMMENDED.some((r) => r.key === k));
  if (unknown) fail(`Unknown rule ${unknown}`);
  const s = db();
  s.setUp = true;
  for (const r of RECOMMENDED) {
    const card = scorecardNamed(r.scorecard, SCORECARD_DESCRIPTIONS[r.scorecard]);
    const existing = s.rules.find((x) => x.builtin === r.key);
    if (existing) existing.enabled = input.rules.includes(r.key);
    else
      s.rules.push({ id: uuid(), scorecardId: card.id, builtin: r.key, name: r.name, description: r.description, level: r.level, scope: '', enabled: input.rules.includes(r.key), manual: new Map() });
  }
  await evaluate();
  return overview();
}

export async function evaluateScorecards(): Promise<ScorecardsOverview> {
  requireSetUp();
  await evaluate();
  return overview();
}

export async function createScorecardRule(input: ScorecardRuleInput): Promise<ScorecardsOverview> {
  requireSetUp();
  const s = db();
  if (s.rules.filter((r) => !r.builtin).length >= MAX_CUSTOM_RULES) fail(`An organization can have at most ${MAX_CUSTOM_RULES} custom rules`);
  const name = text(input.name, 'rule name', 200, true);
  const card = scorecardNamed(text(input.scorecard, 'scorecard name', 100, true));
  s.rules.push({
    id: uuid(),
    scorecardId: card.id,
    builtin: null,
    name,
    description: text(input.description, 'description', 2000, false),
    level: level(input.level),
    scope: text(input.scope, 'scope', 500, false),
    enabled: true,
    manual: new Map(),
  });
  await evaluate();
  return overview();
}

export async function updateScorecardRule(id: string, input: ScorecardRuleUpdate): Promise<ScorecardsOverview> {
  const rule = ruleRow(id);
  if (rule.builtin && (input.name != null || input.description != null)) fail('Built-in rules keep their name and description');
  if (input.name != null) rule.name = text(input.name, 'rule name', 200, true);
  if (input.description != null) rule.description = text(input.description, 'description', 2000, false);
  if (input.level != null) rule.level = level(input.level);
  if (input.scope != null) rule.scope = text(input.scope, 'scope', 500, false);
  if (input.enabled != null) rule.enabled = input.enabled;
  await evaluate();
  return overview();
}

export async function deleteScorecardRule(id: string): Promise<void> {
  const rule = ruleRow(id);
  if (rule.builtin) fail("Built-in rules can't be deleted; disable them instead");
  const s = db();
  s.rules = s.rules.filter((r) => r.id !== id);
  await evaluate();
}

export async function setScorecardOutcomes(id: string, input: ScorecardOutcomesInput): Promise<ScorecardsOverview> {
  const rule = ruleRow(id);
  if (rule.builtin) fail("Built-in rules are evaluated from the organization's data; report outcomes for custom rules");
  if (!input.outcomes.length || input.outcomes.length > 1000) fail('Send 1 to 1000 outcomes');
  for (const o of input.outcomes) rule.manual.set(text(o.service, 'service name', 200, true), { state: o.state, remarks: text(o.remarks, 'remarks', 500, false) });
  await evaluate();
  return overview();
}

export async function updateServiceMetadata(service: string, input: ServiceMetadataInput): Promise<ScorecardsOverview> {
  const name = text(service, 'service name', 200, true);
  const contacts: ServiceContact[] = [];
  for (const c of input.contacts ?? []) {
    const value = c.value.trim();
    if (!value) continue;
    if (c.type === 'email' && !/^\S+@\S+$/.test(value)) fail(`${value} is not an email address`);
    if (c.type === 'link') httpUrl(value, 'contact link');
    if (!['email', 'slack', 'link'].includes(c.type)) fail(`Unknown contact type ${c.type}: use email, slack or link`);
    contacts.push({ type: c.type, name: (c.name ?? '').trim(), value });
  }
  if (contacts.length > 20) fail('A service can have at most 20 contacts');
  const meta: Metadata = {
    team: text((input.team ?? '').replace(/^\s*@/, ''), 'team handle', 100, false),
    contacts,
    repoUrl: httpUrl(input.repoUrl ?? '', 'code repository'),
    docsUrl: httpUrl(input.docsUrl ?? '', 'documentation link'),
  };
  db().metadata.set(name, meta);
  if (db().setUp) await evaluate();
  return overview();
}
