// In-browser stand-in for Logs → Configuration → Migration (routes/log_migration.rs). The real
// translator is the server's (spl.rs); here the three examples have canned translations and any
// other search gets a rough field=value → field:value rewrite.
import type { CreateImport, ImportAsset, ImportCounts, ImportSearch, LogImport, LogImportDetail, SplTranslation } from '../logMigration/types';

const delay = <T>(v: T, ms = 200) => new Promise<T>((r) => setTimeout(() => r(structuredClone(v)), ms));

const CANNED: Record<string, SplTranslation> = {
  'index=web sourcetype=nginx status=500 NOT host=web-test*': {
    query: 'index:web source:nginx @http.status_code:500 -host:web-test*',
    aggregation: null,
    columns: [],
    explanation: [
      { spl: 'index=web', result: 'index:web', note: 'Splunk indexes map to log indexes.' },
      { spl: 'sourcetype=nginx', result: 'source:nginx', note: 'The source type becomes the log source.' },
      { spl: 'status=500', result: '@http.status_code:500', note: '`status` with a numeric value is read as the HTTP status code.' },
      { spl: 'NOT host=web-test*', result: '-host:web-test*', note: '' },
    ],
    warnings: [],
    complete: true,
  },
  'index=web service=checkout | stats count, avg(duration) by host': {
    query: 'index:web service:checkout',
    aggregation: {
      viz: 'table',
      measures: [
        { function: 'count', field: null, name: 'count' },
        { function: 'avg', field: '@duration', name: 'avg(duration)' },
      ],
      groupBy: ['host'],
      interval: null,
      limit: null,
      ascending: false,
      condition: null,
    },
    columns: [],
    explanation: [
      { spl: 'index=web', result: 'index:web', note: 'Splunk indexes map to log indexes.' },
      { spl: 'service=checkout', result: 'service:checkout', note: '' },
      { spl: 'stats count, avg(duration) by host', result: 'table — count, avg(@duration) by host', note: "Aggregation: shown in the Log Explorer's analytics views and used by log monitors." },
    ],
    warnings: [],
    complete: true,
  },
  'index=web log_level=ERROR | timechart span=5m count by service': {
    query: 'index:web status:error',
    aggregation: { viz: 'timeseries', measures: [{ function: 'count', field: null, name: 'count' }], groupBy: ['service'], interval: '5m', limit: null, ascending: false, condition: null },
    columns: [],
    explanation: [
      { spl: 'index=web', result: 'index:web', note: 'Splunk indexes map to log indexes.' },
      { spl: 'log_level=ERROR', result: 'status:error', note: 'The log level becomes the log status.' },
      { spl: 'timechart span=5m count by service', result: 'timeseries — count by service every 5m', note: "Aggregation: shown in the Log Explorer's analytics views and used by log monitors." },
    ],
    warnings: [],
    complete: true,
  },
};

function rough(spl: string): SplTranslation {
  const [search, ...cmds] = spl.split('|');
  const terms = search
    .trim()
    .replace(/^search\s+/i, '')
    .split(/\s+/)
    .filter(Boolean)
    .map((w) => w.replace(/^([\w.]+)!=(.+)$/, '-$1:$2').replace(/^([\w.]+)=(.+)$/, '$1:$2'));
  const warnings = cmds.map((c) => `\`${c.trim()}\`: the in-browser preview only rewrites the search; connect the server for the full translation`);
  return {
    query: terms.join(' '),
    aggregation: null,
    columns: [],
    explanation: terms.map((t) => ({ spl: t, result: t, note: '' })),
    warnings,
    complete: warnings.length === 0,
  };
}

export async function translate(query: string): Promise<SplTranslation> {
  return delay(CANNED[query.trim()] ?? rough(query.trim()), 350);
}

const t = (spl: string, title: string): ImportSearch => ({ title, spl, translation: CANNED[spl] ?? rough(spl) });

const ASSETS: ImportAsset[] = [
  {
    id: 'a1',
    kind: 'alert',
    title: 'Checkout 5xx spike',
    description: 'More than 25 server errors on checkout in 10 minutes.',
    status: 'ready',
    searches: [
      {
        title: 'Checkout 5xx spike',
        spl: 'index=web service=checkout status=500 | stats count by host',
        translation: {
          query: 'index:web service:checkout @http.status_code:500',
          aggregation: { viz: 'table', measures: [{ function: 'count', field: null, name: 'count' }], groupBy: ['host'], interval: null, limit: null, ascending: false, condition: null },
          columns: [],
          explanation: [
            { spl: 'index=web', result: 'index:web', note: 'Splunk indexes map to log indexes.' },
            { spl: 'service=checkout', result: 'service:checkout', note: '' },
            { spl: 'status=500', result: '@http.status_code:500', note: '`status` with a numeric value is read as the HTTP status code.' },
            { spl: 'stats count by host', result: 'table — count by host', note: 'Aggregation.' },
          ],
          warnings: [],
          complete: true,
        },
      },
    ],
    alert: { comparator: 'above', threshold: 25, windowMinutes: 10, recipients: ['oncall@example.com'], schedule: '*/5 * * * *' },
    issues: [],
  },
  {
    id: 'a2',
    kind: 'alert',
    title: 'Payment declines by gateway',
    description: null,
    status: 'migrated',
    searches: [t('index=web sourcetype=nginx status=500 NOT host=web-test*', 'Payment declines by gateway')],
    alert: { comparator: 'above', threshold: 50, windowMinutes: 15, recipients: [], schedule: '*/15 * * * *' },
    issues: [],
    targetKind: 'monitor',
    targetId: '1',
    migratedAt: Date.now() - 3_600_000,
  },
  {
    id: 'a3',
    kind: 'alert',
    title: 'Inventory sync lag',
    description: 'Average sync duration above 2 seconds.',
    status: 'partial',
    searches: [
      {
        title: 'Inventory sync lag',
        spl: 'index=app service=inventory-service | eval secs=duration/1000 | stats avg(secs)',
        translation: {
          query: 'index:app service:inventory-service',
          aggregation: null,
          columns: [],
          explanation: [
            { spl: 'index=app', result: 'index:app', note: 'Splunk indexes map to log indexes.' },
            { spl: 'service=inventory-service', result: 'service:inventory-service', note: '' },
            { spl: 'eval secs=duration/1000', result: '', note: "computed fields aren't supported at search time: compute them at ingestion with a pipeline processor (Arithmetic, String Builder)" },
          ],
          warnings: ["`eval secs=duration/1000`: computed fields aren't supported at search time"],
          complete: false,
        },
      },
    ],
    alert: { comparator: 'above', threshold: 2, windowMinutes: 15, recipients: [], schedule: null },
    issues: ['log monitors count events: this alert measures another value, so it needs a log-based metric and a metric monitor'],
  },
  {
    id: 'a4',
    kind: 'search',
    title: 'Order service errors',
    status: 'ready',
    searches: [t('index=web log_level=ERROR | timechart span=5m count by service', 'Order service errors')],
    alert: null,
    issues: [],
  },
  {
    id: 'a5',
    kind: 'dashboard',
    title: 'Storefront overview',
    status: 'partial',
    searches: [
      t('index=web service=checkout | stats count, avg(duration) by host', 'Checkout latency by host'),
      t('index=web log_level=ERROR | timechart span=5m count by service', 'Errors over time'),
      t('index=web sourcetype=nginx status=500 NOT host=web-test*', 'Latest server errors'),
    ],
    alert: null,
    issues: ["charted panels need log-based metrics: they're migrated as log streams with a note describing the aggregation"],
  },
];

function counts(assets: ImportAsset[]): ImportCounts {
  const c: ImportCounts = { total: assets.length, alerts: 0, dashboards: 0, searches: 0, ready: 0, partial: 0, unsupported: 0, migrated: 0, failed: 0 };
  for (const a of assets) {
    if (a.kind === 'alert') c.alerts++;
    else if (a.kind === 'dashboard') c.dashboards++;
    else c.searches++;
    c[a.status]++;
  }
  return c;
}

const imports: LogImportDetail[] = [
  { id: 'imp-demo', name: 'Splunk production export', source: 'splunk', createdAt: Date.now() - 2 * 86_400_000, createdBy: 'Avery Example', macros: 4, counts: counts(ASSETS), assets: ASSETS },
];

export async function listImports(): Promise<LogImport[]> {
  return delay(imports.map(({ assets: _a, ...i }) => ({ ...i, counts: counts(_a) })));
}

export async function getImport(id: string): Promise<LogImportDetail> {
  const i = imports.find((x) => x.id === id);
  if (!i) throw new Error(`Import ${id} not found`);
  return delay({ ...i, counts: counts(i.assets) });
}

/** Rows of the uploaded CSV with a `search` column become searches (no full CSV parsing here). */
export async function createImport(input: CreateImport): Promise<LogImportDetail> {
  const lines = (input.alerts ?? '')
    .split(/\r?\n/)
    .slice(1)
    .filter((l) => l.trim());
  const assets: ImportAsset[] = lines.map((l, i) => {
    const [title, spl] = l.split(',');
    return { id: `n${i}`, kind: 'search', title: title || `Search ${i + 1}`, status: 'ready', searches: [t(spl ?? '', title ?? '')], alert: null, issues: [] };
  });
  if (input.dashboards?.trim()) assets.push({ ...ASSETS[4], id: `d${Date.now()}` });
  if (!assets.length) throw new Error('No searches, alerts or dashboards found: upload the CSV files exported with the searches shown');
  const d: LogImportDetail = {
    id: `imp-${Date.now()}`,
    name: input.name,
    source: 'splunk',
    createdAt: Date.now(),
    createdBy: 'You',
    macros: (input.macros ?? '').split(/\r?\n/).length - 1,
    counts: counts(assets),
    assets,
  };
  imports.unshift(d);
  return delay(d);
}

export async function migrate(id: string, assetIds: string[]): Promise<LogImportDetail> {
  const i = imports.find((x) => x.id === id);
  if (!i) throw new Error(`Import ${id} not found`);
  for (const a of i.assets) {
    const wanted = assetIds.length ? assetIds.includes(a.id) : a.status === 'ready' || a.status === 'partial';
    if (!wanted || a.kind === 'search' || a.status === 'migrated' || a.status === 'unsupported') continue;
    if (a.kind === 'alert' && a.searches[0]?.translation.aggregation?.measures.some((m) => m.function !== 'count')) {
      a.status = 'failed';
      a.error = 'log monitors count events; this alert measures another value';
    } else if (a.kind === 'alert' && a.issues.length && !a.searches[0]?.translation.aggregation) {
      a.status = 'failed';
      a.error = 'log monitors count events; this alert measures another value';
    } else {
      a.status = 'migrated';
      a.targetKind = a.kind === 'alert' ? 'monitor' : 'dashboard';
      a.targetId = a.kind === 'alert' ? '1' : 'storefront-overview';
      a.migratedAt = Date.now();
    }
  }
  return getImport(id);
}

export async function deleteImport(id: string): Promise<void> {
  const i = imports.findIndex((x) => x.id === id);
  if (i >= 0) imports.splice(i, 1);
  await delay(undefined);
}
