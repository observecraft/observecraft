// In-browser notebooks for mock mode. State lives in memory for the session; the seed is a few
// notebooks about the demo shop by the people of the identity mock. Semantics follow the server
// (server/crates/oc-api/src/routes/notebooks.rs): the same cell validation and messages, per-user
// favorites and recents for the current user, soft delete with restore.
import type { CreateNotebookRequest, Notebook, NotebookCell, NotebookPatch, NotebookSearchPage, NotebookSearchRequest, NotebookSummary, NotebookTime } from '../notebooks/types';
import { NOTEBOOK_TIMES } from '../notebooks/types';
import { NOTEBOOK_TEMPLATES, fromTemplate } from '../notebooks/templates';
import { ME_ID, mockMe, mockMyTeams, mockUserNamed } from './identity';

const MAX_CELLS = 200;
const MAX_MARKDOWN_CHARS = 50_000;
const MAX_REQUESTS = 4;
const MAX_QUERY_CHARS = 2_000;
const MAX_CELLS_BYTES = 1_000_000;
const MAX_LABELS = 20;
const MAX_LABEL_CHARS = 100;
const CELL_TYPES = 'markdown, timeseries, query_value, toplist, log_stream';
const HOUR = 3_600_000;
const DAY = 24 * HOUR;

interface Row extends Omit<Notebook, 'viewedAt' | 'favorite' | 'cellCount'> {
  favorite: boolean;
  viewedAt: number | null;
}

const fail = (message: string): never => {
  throw new Error(message);
};
const copy = <T>(v: T): T => structuredClone(v);

// ================================================================ validation (as the server)

const chars = (s: string) => [...s].length;

function checkTime(t: unknown): void {
  if (!NOTEBOOK_TIMES.includes(t as NotebookTime)) fail(`unknown time range '${String(t)}' (expected one of ${NOTEBOOK_TIMES.join(', ')})`);
}

function checkSearch(q: unknown): void {
  if (typeof q !== 'string') fail('the query must be text');
  if (chars(q as string) > MAX_QUERY_CHARS) fail(`a query is at most ${MAX_QUERY_CHARS} characters`);
}

function checkMetricQuery(q: unknown): void {
  if (typeof q !== 'string' || !q.trim()) fail('a query cell needs a metric query');
  checkSearch(q);
}

function checkCell(cell: unknown): void {
  if (!cell || typeof cell !== 'object' || Array.isArray(cell)) fail('a cell is an object with an id and a definition');
  const c = cell as Record<string, unknown>;
  if (typeof c.id !== 'string' || !c.id || chars(c.id) > 64) fail('a cell needs an id of 1–64 characters');
  if (c.time !== undefined && c.time !== null) checkTime(c.time);
  const def = (c.definition ?? {}) as Record<string, unknown>;
  const request = (def.request ?? {}) as Record<string, unknown>;
  switch (def.type) {
    case 'markdown':
      if (typeof def.text !== 'string') fail('a Markdown cell needs its text');
      if (chars(def.text as string) > MAX_MARKDOWN_CHARS) fail(`a Markdown cell is at most ${MAX_MARKDOWN_CHARS} characters`);
      return;
    case 'timeseries': {
      const requests = Array.isArray(def.requests) ? (def.requests as Record<string, unknown>[]) : [];
      if (requests.length === 0) fail('a timeseries cell needs a metric query');
      if (requests.length > MAX_REQUESTS) fail(`a timeseries cell graphs at most ${MAX_REQUESTS} queries`);
      for (const r of requests) checkMetricQuery(r?.query);
      return;
    }
    case 'query_value':
    case 'toplist':
      return checkMetricQuery(request.query);
    case 'log_stream': {
      checkSearch(def.query);
      const limit = Number.isInteger(def.limit) ? (def.limit as number) : 0;
      if (limit < 1 || limit > 100) fail('a log stream shows a limit of 1–100 logs');
      return;
    }
    default:
      fail(`unsupported cell type '${String(def.type ?? '')}' (expected one of ${CELL_TYPES})`);
  }
}

/** Checks the cells of a notebook; throws with the server's messages. */
export function validateCells(cells: NotebookCell[]): void {
  if (cells.length > MAX_CELLS) fail(`a notebook holds at most ${MAX_CELLS} cells`);
  if (JSON.stringify(cells).length > MAX_CELLS_BYTES) fail(`the cells of a notebook take at most ${MAX_CELLS_BYTES / 1000} KB`);
  const ids = new Set<string>();
  cells.forEach((cell, i) => {
    try {
      checkCell(cell);
    } catch (e) {
      fail(`cell ${i + 1}: ${(e as Error).message}`);
    }
    if (ids.has(cell.id)) fail(`cell ${i + 1}: the id '${cell.id}' is used twice`);
    ids.add(cell.id);
  });
}

function checkTitle(t: string): string {
  const title = t.trim();
  if (!title || chars(title) > 200) fail('the title must be 1–200 characters');
  return title;
}

function cleanLabels(list: string[], what: string): string[] {
  const out: string[] = [];
  for (const raw of list) {
    const label = raw.trim();
    if (!label || out.includes(label)) continue;
    if (chars(label) > MAX_LABEL_CHARS) fail(`${what} are at most ${MAX_LABEL_CHARS} characters each`);
    out.push(label);
  }
  if (out.length > MAX_LABELS) fail(`a notebook has at most ${MAX_LABELS} ${what}`);
  return out;
}

// ================================================================ seed

let seq = 0;
const newId = () => `nb-${(++seq).toString(36).padStart(3, '0')}-${Date.now().toString(36).slice(-4)}`;

function seedCells(definitions: NotebookCell['definition'][], times: (NotebookTime | null)[] = []): NotebookCell[] {
  return definitions.map((definition, i) => ({ id: `c${i + 1}`, definition, ...(times[i] ? { time: times[i] } : {}) }));
}

function seed(): Row[] {
  const now = Date.now();
  const person = (name: string) => mockUserNamed(name) ?? mockMe();
  const me = mockMe();
  const template = (id: string, subject: string) =>
    fromTemplate(
      NOTEBOOK_TEMPLATES.find((t) => t.id === id)!,
      subject,
    );
  const row = (r: Partial<Row> & Pick<Row, 'title' | 'author' | 'cells'>, ageDays: number, modifiedHours: number): Row => ({
    id: newId(),
    teams: [],
    tags: [],
    status: 'published',
    time: '1h',
    createdAt: now - ageDays * DAY,
    modifiedAt: now - modifiedHours * HOUR,
    viewedAt: null,
    favorite: false,
    deletedAt: null,
    ...r,
  });
  const postmortem = template('postmortem', 'billing invoice job failures');
  const runbook = template('runbook', 'api-gateway 5xx spike');
  return [
    row(
      {
        title: 'Checkout latency investigation',
        author: me,
        teams: ['checkout'],
        tags: ['service:order-service', 'investigation'],
        time: '4h',
        viewedAt: now - 20 * 60_000,
        cells: seedCells(
          [
            {
              type: 'markdown',
              text: '# Checkout latency investigation\nCustomers report slow order confirmation since this morning. **order-service** is the first suspect: its p95 latency and error rate went up together.\n\n- Started around the 09:40 deployment\n- Only `env:prod` is affected',
            },
            {
              type: 'timeseries',
              title: 'order-service latency (p95)',
              display: 'line',
              requests: [{ query: 'avg:trace.latency.p95{service:order-service,env:prod}' }, { query: 'avg:trace.latency.p50{service:order-service,env:prod}' }],
            },
            {
              type: 'query_value',
              title: 'Error rate (%)',
              request: { query: 'avg:trace.error_rate{service:order-service,env:prod}' },
              aggregator: 'avg',
              precision: 2,
              unit: '%',
              formats: [{ comparator: '>', value: 5, palette: 'red' }],
            },
            { type: 'log_stream', title: 'order-service errors', query: 'service:order-service status:error', limit: 20 },
            { type: 'toplist', title: 'Errors by service', request: { query: 'sum:trace.errors{env:prod} by {service}' }, aggregator: 'sum', limit: 10 },
            {
              type: 'markdown',
              text: '## Findings\n1. The latency rise lines up with the order-service deployment.\n2. The errors are payment authorization timeouts towards **billing**.\n\n## Next steps\n- Compare with the previous version\n- Ask the payments team about provider latency',
            },
          ],
          [null, null, '1h', '1h'],
        ),
      },
      2,
      1,
    ),
    row(
      {
        ...postmortem,
        author: person('Priya Shah'),
        teams: ['billing', 'payments'],
        tags: [...postmortem.tags, 'incident', 'service:billing'],
        favorite: true,
        viewedAt: now - 2 * DAY,
        cells: seedCells(postmortem.cells.map((c) => c.definition)),
      },
      6,
      30,
    ),
    row(
      {
        ...runbook,
        author: person('Omar Haddad'),
        teams: ['platform', 'sre'],
        tags: [...runbook.tags, 'service:api-gateway'],
        favorite: true,
        cells: seedCells(runbook.cells.map((c) => c.definition)),
      },
      40,
      9 * 24,
    ),
    row(
      {
        title: 'Weekly storefront performance review',
        author: person('Sara Levi'),
        teams: ['storefront'],
        tags: ['service:storefront-api', 'weekly'],
        status: 'draft',
        time: '1w',
        cells: seedCells([
          { type: 'markdown', text: '# Storefront this week\nTraffic, latency and errors of **storefront-api**, reviewed every Monday.' },
          { type: 'timeseries', title: 'Requests', display: 'bars', requests: [{ query: 'sum:trace.requests{service:storefront-api}' }] },
          { type: 'toplist', title: 'Slowest services (p95)', request: { query: 'avg:trace.latency.p95{env:prod} by {service}' }, aggregator: 'avg', limit: 5 },
        ]),
      },
      12,
      3 * 24,
    ),
    row(
      {
        title: 'Catalog cache warm-up notes',
        author: me,
        teams: ['catalog', 'platform'],
        tags: ['service:catalog-service'],
        cells: seedCells([
          { type: 'markdown', text: '# Catalog cache warm-up\nThe catalog-service product query is slow for the first minutes after a deployment, until the cache is warm.' },
          { type: 'timeseries', title: 'catalog-service latency (p95)', display: 'line', requests: [{ query: 'avg:trace.latency.p95{service:catalog-service}' }] },
        ]),
      },
      20,
      5 * 24,
    ),
    row(
      {
        title: 'Inventory reservation timeouts',
        author: person('Daniel Kim'),
        teams: ['checkout'],
        tags: ['service:inventory-service'],
        deletedAt: now - 3 * DAY,
        cells: seedCells([
          { type: 'markdown', text: '# Inventory reservation timeouts\nSuperseded by the checkout latency investigation.' },
          { type: 'log_stream', title: 'inventory-service warnings', query: 'service:inventory-service status:warn', limit: 20 },
        ]),
      },
      15,
      4 * 24,
    ),
  ];
}

let store: Row[] | null = null;
const db = (): Row[] => (store ??= seed());

function summary(r: Row): NotebookSummary {
  const { cells, ...rest } = r;
  return copy({ ...rest, cellCount: cells.length });
}
const full = (r: Row): Notebook => ({ ...summary(r), cells: copy(r.cells) });

function find(id: string): Row {
  return db().find((r) => r.id === id) ?? fail(`Notebook ${id} not found`);
}

// ================================================================ API

const includes = (hay: string, needle: string) => hay.toLowerCase().includes(needle.toLowerCase());

export async function searchNotebooks(req: NotebookSearchRequest): Promise<NotebookSearchPage> {
  const myTeams = mockMyTeams();
  const text = (req.query ?? '').trim();
  const rows = db().filter((r) => {
    if ((req.view === 'deleted') !== (r.deletedAt !== null)) return false;
    if (req.view === 'mine' && r.author?.id !== ME_ID) return false;
    if (req.view === 'team' && !r.teams.some((t) => myTeams.includes(t))) return false;
    if (req.view === 'recents' && r.viewedAt === null) return false;
    if (req.view === 'favorites' && !r.favorite) return false;
    if (req.author && r.author?.id !== req.author) return false;
    if (req.team && !r.teams.includes(req.team)) return false;
    return !text || includes(r.title, text) || includes(r.author?.name ?? '', text) || includes(r.tags.join(' '), text) || includes(JSON.stringify(r.cells), text);
  });
  const key = (r: Row): string | number => (req.sort === 'title' ? r.title.toLowerCase() : req.sort === 'created' ? r.createdAt : r.modifiedAt);
  const dir = req.dir === 'asc' ? 1 : -1;
  rows.sort((a, b) => (req.view === 'recents' ? (b.viewedAt ?? 0) - (a.viewedAt ?? 0) : key(a) < key(b) ? -dir : key(a) > key(b) ? dir : a.id.localeCompare(b.id)));
  const live = db().filter((r) => r.deletedAt === null);
  const authors = new Map(live.flatMap((r) => (r.author ? [[r.author.id, r.author] as const] : [])));
  const limit = Math.min(200, Math.max(1, req.limit));
  const offset = Math.max(0, req.offset);
  return {
    notebooks: rows.slice(offset, offset + limit).map(summary),
    total: rows.length,
    authors: [...authors.values()].sort((a, b) => a.name.localeCompare(b.name)).map(copy),
    teams: [...new Set(live.flatMap((r) => r.teams))].sort(),
  };
}

/** Opens a notebook and records the visit (Recents). */
export async function getNotebook(id: string): Promise<Notebook> {
  const r = find(id);
  r.viewedAt = Date.now();
  return full(r);
}

export async function createNotebook(req: CreateNotebookRequest): Promise<Notebook> {
  const title = checkTitle(req.title);
  const teams = cleanLabels(req.teams ?? [], 'teams');
  const tags = cleanLabels(req.tags ?? [], 'tags');
  const time = req.time ?? '1h';
  checkTime(time);
  const cells = req.cells ?? [];
  validateCells(cells);
  const now = Date.now();
  const row: Row = {
    id: newId(),
    title,
    author: mockMe(),
    teams,
    tags,
    status: req.status ?? 'published',
    time,
    cells: copy(cells),
    createdAt: now,
    modifiedAt: now,
    viewedAt: now,
    favorite: false,
    deletedAt: null,
  };
  db().unshift(row);
  return full(row);
}

export async function updateNotebook(id: string, p: NotebookPatch): Promise<Notebook> {
  const r = find(id);
  if (r.deletedAt !== null) fail('This notebook is deleted: restore it to edit it');
  const title = p.title === undefined ? r.title : checkTitle(p.title);
  const teams = p.teams === undefined ? r.teams : cleanLabels(p.teams, 'teams');
  const tags = p.tags === undefined ? r.tags : cleanLabels(p.tags, 'tags');
  if (p.time !== undefined) checkTime(p.time);
  if (p.cells !== undefined) validateCells(p.cells);
  Object.assign(r, { title, teams, tags, status: p.status ?? r.status, time: p.time ?? r.time, cells: p.cells ? copy(p.cells) : r.cells, modifiedAt: Date.now() });
  return full(r);
}

export async function cloneNotebook(id: string): Promise<Notebook> {
  const src = find(id);
  if (src.deletedAt !== null) fail(`Notebook ${id} not found`);
  const now = Date.now();
  const row: Row = { ...copy(src), id: newId(), title: `Clone of ${src.title}`.slice(0, 200), author: mockMe(), createdAt: now, modifiedAt: now, viewedAt: now, favorite: false };
  db().unshift(row);
  return full(row);
}

export async function deleteNotebook(id: string): Promise<void> {
  const r = find(id);
  if (r.deletedAt !== null) fail(`Notebook ${id} not found`);
  r.deletedAt = Date.now();
}

export async function restoreNotebook(id: string): Promise<Notebook> {
  const r = find(id);
  if (r.deletedAt === null) fail(`Notebook ${id} is not in Recently Deleted`);
  r.deletedAt = null;
  return full(r);
}

export async function setNotebookFavorite(id: string, favorite: boolean): Promise<void> {
  find(id).favorite = favorite;
}
