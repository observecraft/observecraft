// In-browser mock of Logs → Configuration beyond pipelines, indexes and metrics: archives and
// rehydrations, custom destinations, Flex Logs controls and pipeline usage. Session-only state.
import type { components } from '../generated/schema';

type S = components['schemas'];
/** The generated schema types JSON documents as empty objects. */
type Doc = Record<string, unknown>;
export type CustomDestination = Omit<S['CustomDestination'], 'destination'> & { destination: Doc };
export type CustomDestinationInput = Omit<S['CustomDestinationInput'], 'destination'> & { destination: Doc };

const fail = (message: string): never => {
  throw new Error(message);
};
const copy = <T>(v: T): T => structuredClone(v);
const HOUR = 3_600_000;
const DAY = 24 * HOUR;

interface State {
  archives: S['LogArchive'][];
  rehydrations: S['Rehydration'][];
  destinations: CustomDestination[];
  flex: { enabled: boolean; compute: string; size: string | null };
  seq: number;
}

let state: State | null = null;

function db(): State {
  if (state) return state;
  const now = Date.now();
  const archive = (id: string, name: string, query: string, prefix: string, rows: number, extra: Partial<S['LogArchive']> = {}): S['LogArchive'] => ({
    id,
    name,
    query,
    endpoint: 'http://rustfs:9000',
    region: 'us-east-1',
    bucket: 'oc-archives',
    prefix,
    pathStyle: true,
    accessKey: 'observecraft',
    enabled: true,
    createdAt: now - 60 * DAY,
    hoursDone: 1_430,
    hoursFailed: 0,
    rows,
    bytes: rows * 180,
    lastHour: now - 2 * HOUR,
    lastError: null,
    destinationType: 's3',
    hasCredentials: true,
    includeTags: true,
    rehydrationTags: [],
    rehydrationMaxScanSizeGb: null,
    ...extra,
  });
  state = {
    archives: [
      archive('ar-0001', 'all-logs', '', 'logs', 412_000_000, { rehydrationTags: ['rehydrated:true'], rehydrationMaxScanSizeGb: 200 }),
      archive('ar-0002', 'checkout-audit', 'service:(order-service OR billing)', 'audit/checkout', 38_200_000),
      archive('ar-0003', 'security-events', 'source:(cloudtrail OR okta)', 'security', 9_100_000, { includeTags: false }),
      archive('ar-0004', 'gcs-long-term', 'env:prod', 'long-term', 0, {
        endpoint: 'https://storage.googleapis.com',
        bucket: 'example-logs-archive',
        destinationType: 'gcs',
        hasCredentials: false,
        hoursDone: 0,
        lastHour: null,
        accessKey: '',
      }),
    ],
    rehydrations: [
      {
        id: 'rh-0001',
        archiveId: 'ar-0002',
        archiveName: 'checkout-audit',
        name: 'checkout-incident-oct-12',
        query: 'service:order-service status:error',
        from: now - 19 * DAY,
        to: now - 19 * DAY + 6 * HOUR,
        status: 'done',
        rows: 48_210,
        bytesScanned: 2_900_000_000,
        error: null,
        expiresAt: now + 4 * DAY,
        createdAt: now - 3 * DAY,
        createdBy: 'Sam Rivera',
      },
      {
        id: 'rh-0002',
        archiveId: 'ar-0001',
        archiveName: 'all-logs',
        name: 'billing-retries-q3',
        query: 'service:billing @retry:true',
        from: now - 40 * DAY,
        to: now - 39 * DAY,
        status: 'running',
        rows: 0,
        bytesScanned: 9_400_000_000,
        error: null,
        expiresAt: now + 7 * DAY,
        createdAt: now - 20 * 60_000,
        createdBy: 'Priya Patel',
      },
      {
        id: 'rh-0003',
        archiveId: 'ar-0001',
        archiveName: 'all-logs',
        name: 'catalog-sync-may',
        query: 'service:catalog-service',
        from: now - 160 * DAY,
        to: now - 159 * DAY,
        status: 'expired',
        rows: 1_204_330,
        bytesScanned: 14_800_000_000,
        error: null,
        expiresAt: now - 2 * DAY,
        createdAt: now - 12 * DAY,
        createdBy: 'Jane Doe',
      },
    ],
    destinations: [
      {
        id: 'cd-0001',
        name: 'Security team SIEM',
        query: 'source:(cloudtrail OR okta) OR @evt.category:authentication',
        enabled: true,
        forwardTags: true,
        tagsList: ['env', 'service', 'team'],
        tagsListType: 'ALLOW_LIST',
        destination: { type: 'splunk_hec', endpoint: 'https://splunk.example.com:8088', sourcetype: 'observecraft:logs' },
        forwarded: 18_420_331,
        lastError: null,
        lastErrorAt: null,
        createdBy: 'Alex Chen',
        updatedAt: now - 9 * DAY,
      },
      {
        id: 'cd-0002',
        name: 'Checkout errors to the data lake',
        query: 'service:(order-service OR billing) status:error',
        enabled: true,
        forwardTags: false,
        tagsList: [],
        tagsListType: 'ALLOW_LIST',
        destination: { type: 'elasticsearch', endpoint: 'https://search.example.com', index_name: 'checkout-errors', index_rotation: 'yyyy-MM-dd', auth: {} },
        forwarded: 211_904,
        lastError: null,
        lastErrorAt: null,
        createdBy: 'Sam Rivera',
        updatedAt: now - 30 * DAY,
      },
      {
        id: 'cd-0003',
        name: 'Partner webhook',
        query: 'service:storefront-api @http.status_code:>=500',
        enabled: false,
        forwardTags: true,
        tagsList: ['host'],
        tagsListType: 'BLOCK_LIST',
        destination: { type: 'http', endpoint: 'https://hooks.example.com/logs', auth: { type: 'custom_header', header_name: 'X-Api-Key' } },
        forwarded: 0,
        lastError: 'The destination answered 401 Unauthorized: invalid key',
        lastErrorAt: now - 2 * DAY,
        createdBy: 'Jane Doe',
        updatedAt: now - 2 * DAY,
      },
    ],
    flex: { enabled: false, compute: 'starter', size: null },
    seq: 10,
  };
  return state;
}

const newId = (prefix: string) => `${prefix}-${String(++db().seq).padStart(4, '0')}`;

// ---------------------------------------------------------------- archives

export async function listArchives(): Promise<S['LogArchive'][]> {
  return copy(db().archives);
}

export async function testArchive(t: S['ArchiveTarget']): Promise<S['ArchiveTestResult']> {
  if (!t.accessKey || !t.secretKey) return { ok: false, error: 'Access key and secret key are required' };
  return { ok: true, error: null };
}

export async function createArchive(a: S['CreateArchive']): Promise<S['LogArchive']> {
  if (db().archives.some((x) => x.name === a.name.trim())) fail(`An archive named '${a.name}' already exists`);
  const ar: S['LogArchive'] = {
    id: newId('ar'),
    name: a.name.trim(),
    query: a.query ?? '',
    endpoint: a.endpoint,
    region: a.region ?? 'us-east-1',
    bucket: a.bucket,
    prefix: a.prefix ?? '',
    pathStyle: a.pathStyle ?? true,
    accessKey: a.accessKey,
    enabled: true,
    createdAt: Date.now(),
    hoursDone: 0,
    hoursFailed: 0,
    rows: 0,
    bytes: 0,
    lastHour: null,
    lastError: null,
    destinationType: 's3',
    hasCredentials: true,
    includeTags: true,
    rehydrationTags: [],
    rehydrationMaxScanSizeGb: null,
  };
  db().archives.push(ar);
  return copy(ar);
}

export async function updateArchive(id: string, p: S['ArchivePatch']): Promise<S['LogArchive']> {
  const a = db().archives.find((x) => x.id === id) ?? fail('Archive not found');
  if (p.name) a.name = p.name;
  if (p.enabled != null) a.enabled = p.enabled;
  if (p.query != null) a.query = p.query;
  if (p.target) Object.assign(a, { endpoint: p.target.endpoint, bucket: p.target.bucket, prefix: p.target.prefix ?? '', accessKey: p.target.accessKey, hasCredentials: true });
  return copy(a);
}

export async function deleteArchive(id: string): Promise<void> {
  db().archives = db().archives.filter((a) => a.id !== id);
}

export async function archiveRuns(id: string): Promise<S['ArchiveRun'][]> {
  const a = db().archives.find((x) => x.id === id) ?? fail('Archive not found');
  const last = a.lastHour ?? Date.now();
  return Array.from({ length: a.hoursDone ? 12 : 0 }, (_, i) => ({
    hour: last - i * HOUR,
    status: 'done',
    rows: Math.round(a.rows / 1430),
    bytes: Math.round(a.bytes / 1430),
    attempts: 1,
    finishedAt: last - i * HOUR + 15 * 60_000,
  }));
}

// ---------------------------------------------------------------- rehydrations

export async function listRehydrations(): Promise<S['Rehydration'][]> {
  return copy([...db().rehydrations].sort((a, b) => b.createdAt - a.createdAt));
}

export async function rehydrate(r: { archiveId: string; name: string; query: string; range: { from: number; to: number }; retentionDays: number }): Promise<S['Rehydration']> {
  const a = db().archives.find((x) => x.id === r.archiveId) ?? fail('Archive not found');
  const hours = Math.max(1, (r.range.to - r.range.from) / HOUR);
  const rh: S['Rehydration'] = {
    id: newId('rh'),
    archiveId: a.id,
    archiveName: a.name,
    name: r.name,
    query: r.query,
    from: r.range.from,
    to: r.range.to,
    status: 'done',
    rows: Math.round(hours * 2_400),
    bytesScanned: Math.round(hours * 310_000_000),
    error: null,
    expiresAt: Date.now() + r.retentionDays * DAY,
    createdAt: Date.now(),
    createdBy: 'Jane Doe',
  };
  db().rehydrations.push(rh);
  return copy(rh);
}

export async function expireRehydration(id: string): Promise<void> {
  const r = db().rehydrations.find((x) => x.id === id) ?? fail('Rehydration not found');
  r.status = 'expired';
}

// ---------------------------------------------------------------- custom destinations

export async function listDestinations(): Promise<CustomDestination[]> {
  return copy(db().destinations);
}

/** Secrets are kept server-side only: the mock drops them like the server does. */
function stripSecrets(d: Record<string, unknown>): Record<string, unknown> {
  const out = { ...d };
  delete out.access_token;
  const auth = (out.auth ?? null) as Record<string, unknown> | null;
  if (auth) out.auth = out.type === 'elasticsearch' ? {} : auth.type === 'custom_header' ? { type: 'custom_header', header_name: auth.header_name } : { type: auth.type };
  return out;
}

function checkDestination(i: CustomDestinationInput) {
  if (!i.name.trim()) fail('The name must be 1–200 characters');
  const d = i.destination;
  const endpoint = String(d.endpoint ?? '');
  if (d.type !== 'microsoft_sentinel' && !endpoint.startsWith('https://')) fail('Invalid custom destination configuration: The endpoint must be an https URL');
}

export async function createDestination(i: CustomDestinationInput): Promise<CustomDestination> {
  checkDestination(i);
  if (db().destinations.some((d) => d.name === i.name.trim())) fail(`A custom destination named '${i.name}' already exists`);
  const d: CustomDestination = {
    id: newId('cd'),
    name: i.name.trim(),
    query: i.query ?? '',
    enabled: i.enabled ?? true,
    forwardTags: i.forwardTags ?? true,
    tagsList: i.tagsList ?? [],
    tagsListType: i.tagsListType ?? 'ALLOW_LIST',
    destination: stripSecrets(i.destination),
    forwarded: 0,
    lastError: i.destination.type === 'microsoft_sentinel' ? "Forwarding to Microsoft Sentinel needs Azure credentials, which ObserveCraft doesn't hold yet: logs aren't sent" : null,
    lastErrorAt: null,
    createdBy: 'Jane Doe',
    updatedAt: Date.now(),
  };
  db().destinations.push(d);
  return copy(d);
}

export async function updateDestination(id: string, i: CustomDestinationInput): Promise<CustomDestination> {
  checkDestination(i);
  const d = db().destinations.find((x) => x.id === id) ?? fail('Custom destination not found');
  Object.assign(d, {
    name: i.name.trim(),
    query: i.query ?? '',
    enabled: i.enabled ?? d.enabled,
    forwardTags: i.forwardTags ?? d.forwardTags,
    tagsList: i.tagsList ?? [],
    tagsListType: i.tagsListType ?? 'ALLOW_LIST',
    destination: stripSecrets(i.destination),
    lastError: null,
    updatedAt: Date.now(),
  });
  return copy(d);
}

export async function setDestinationEnabled(id: string, enabled: boolean): Promise<CustomDestination> {
  const d = db().destinations.find((x) => x.id === id) ?? fail('Custom destination not found');
  d.enabled = enabled;
  return copy(d);
}

export async function deleteDestination(id: string): Promise<void> {
  db().destinations = db().destinations.filter((d) => d.id !== id);
}

export async function testDestination(id: string): Promise<S['DestinationTestResult']> {
  const d = db().destinations.find((x) => x.id === id) ?? fail('Custom destination not found');
  if (d.destination.type === 'microsoft_sentinel') return { ok: false, error: "Forwarding to Microsoft Sentinel needs Azure credentials, which ObserveCraft doesn't hold yet" };
  return { ok: true, error: null };
}

// ---------------------------------------------------------------- Flex Logs

export async function getFlex(flexIndexes: number): Promise<S['FlexSettings']> {
  const f = db().flex;
  return { enabled: f.enabled, compute: f.compute, size: f.size, flexIndexes, maxFlexDays: 450 };
}

export async function updateFlex(i: S['FlexSettingsInput'], flexIndexes: number): Promise<S['FlexSettings']> {
  if (i.compute === 'scalable' && !i.size) fail('Scalable compute needs a size: xs, xs+, s, m or l');
  db().flex = { enabled: i.enabled, compute: i.compute, size: i.compute === 'scalable' ? (i.size ?? null) : null };
  return getFlex(flexIndexes);
}

// ---------------------------------------------------------------- pipeline usage

export async function pipelinesUsage(ids: string[]): Promise<S['PipelineUsage'][]> {
  const shares = [0.62, 0.21, 0.09, 0.0, 0.34, 0.05, 0.14];
  return ids.map((id, i) => ({ id, share: shares[i % shares.length], matched: Math.round(shares[i % shares.length] * 2_300_000) }));
}
