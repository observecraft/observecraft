// In-browser spreadsheets for mock mode. State lives in memory for the session; the seed is a few
// spreadsheets about the demo shop. Validation and messages follow the server
// (server/crates/oc-api/src/routes/sheets.rs).
import type { CreateSpreadsheetRequest, Sheet, Spreadsheet, SpreadsheetDefinition, SpreadsheetPatch, SpreadsheetSearchPage, SpreadsheetSearchRequest } from '../sheets/types';
import { normalizeSheet } from '../sheets/types';
import { mockMe, mockUserNamed } from './identity';

const MAX_SHEETS = 20;
const MAX_NAME_CHARS = 200;
const MAX_TEAM_CHARS = 100;
const SOURCES = ['metrics', 'logs', 'reference_table', 'none'];
const HOUR = 3_600_000;
const DAY = 24 * HOUR;

const fail = (message: string): never => {
  throw new Error(message);
};
const copy = <T>(v: T): T => structuredClone(v);
const chars = (s: string) => [...s].length;

function checkName(name: string): string {
  const n = name.trim();
  if (!n || chars(n) > MAX_NAME_CHARS) fail(`The name must be 1–${MAX_NAME_CHARS} characters`);
  return n;
}

function checkTeam(team: string | null | undefined): string | null {
  const t = team?.trim() ?? '';
  if (!t) return null;
  if (chars(t) > MAX_TEAM_CHARS) fail(`A team is at most ${MAX_TEAM_CHARS} characters`);
  return t;
}

function checkDefinition(def: SpreadsheetDefinition): void {
  if (!def || !Array.isArray(def.sheets)) fail('A spreadsheet definition has a list of sheets');
  if (def.sheets.length > MAX_SHEETS) fail(`A spreadsheet has at most ${MAX_SHEETS} sheets`);
  def.sheets.forEach((s, i) => {
    const n = i + 1;
    if (!s.id) fail(`Sheet ${n} needs an id`);
    if (!s.name?.trim() || chars(s.name) > MAX_NAME_CHARS) fail(`Sheet ${n} needs a name of 1–${MAX_NAME_CHARS} characters`);
    const kind = (s.source as { type?: string } | undefined)?.type ?? '';
    if (!SOURCES.includes(kind)) fail(`Sheet ${n}: unknown data source '${kind}' (use ${SOURCES.join(', ')})`);
  });
}

let seq = 0;
const newId = () => `ss-${(++seq).toString(36).padStart(3, '0')}-${Date.now().toString(36).slice(-4)}`;

const sheet = (id: string, name: string, s: Partial<Sheet>): Sheet => normalizeSheet({ id, name, ...s });

function seed(): Spreadsheet[] {
  const now = Date.now();
  const me = mockMe();
  const person = (name: string) => mockUserNamed(name) ?? me;
  let n = 0;
  // Stable ids, so a reloaded page finds the seeded spreadsheet again.
  const row = (name: string, author: { id: string; name: string }, team: string | null, sheets: Sheet[], ageDays: number, modifiedHours: number): Spreadsheet => ({
    id: `ss-seed-${++n}`,
    name,
    author,
    team,
    sheetCount: sheets.length,
    createdAt: now - ageDays * DAY,
    modifiedAt: now - modifiedHours * HOUR,
    definition: { sheets },
  });
  return [
    row(
      'Service latency review',
      me,
      'checkout',
      [
        sheet('s1', 'p95 by service', {
          source: { type: 'metrics', query: 'avg:trace.latency.p95{env:prod} by {service}', aggregators: ['avg', 'max'] },
          calculated: [
            { id: 'c1', name: 'spread', formula: 'max - avg' },
            { id: 'c2', name: 'verdict', formula: 'IF(max > 300, "slow", "ok")' },
          ],
          sort: { column: 'max', dir: 'desc' },
        }),
        sheet('s2', 'Errors by service', { time: '1d', source: { type: 'logs', query: 'status:error', groupBy: 'service', limit: 25 }, sort: { column: 'count', dir: 'desc' } }),
      ],
      14,
      3,
    ),
    row(
      'Catalog pricing by category',
      person('Priya Shah'),
      'catalog',
      [
        sheet('s1', 'Products', {
          source: { type: 'reference_table', table: 'product_catalog' },
          calculated: [{ id: 'c1', name: 'price with tax', formula: 'ROUND(price_usd * 1.2, 2)' }],
          pivot: {
            groupBy: 'category',
            aggregations: [
              { column: 'price_usd', fn: 'avg' },
              { column: 'sku', fn: 'count' },
            ],
          },
        }),
      ],
      30,
      40,
    ),
    row(
      'Checkout traffic and errors',
      person('Omar Haddad'),
      'checkout',
      [
        sheet('s1', 'Requests', {
          source: { type: 'metrics', query: 'sum:trace.requests{service:order-service} by {env}', aggregators: ['avg', 'sum'] },
          calculated: [{ id: 'c1', name: 'per minute', formula: 'ROUND(avg * 60, 0)' }],
        }),
      ],
      9,
      26,
    ),
    row('Billing reconciliation notes', person('Sara Levi'), 'billing', [sheet('s1', 'Sheet 1', {})], 4, 70),
  ];
}

let store: Spreadsheet[] | null = null;
const db = () => (store ??= seed());

function find(id: string): Spreadsheet {
  return db().find((s) => s.id === id) ?? fail(`Spreadsheet ${id} not found`);
}

const summary = ({ definition: _definition, ...s }: Spreadsheet) => copy(s);

export async function searchSpreadsheets(req: SpreadsheetSearchRequest): Promise<SpreadsheetSearchPage> {
  const q = req.query.trim().toLowerCase();
  const me = mockMe();
  const rows = db()
    .filter((s) => !q || s.name.toLowerCase().includes(q) || (s.author?.name ?? '').toLowerCase().includes(q))
    .filter((s) => req.view !== 'mine' || s.author?.id === me.id)
    .filter((s) => !req.team || s.team === req.team)
    .sort((a, b) => b.modifiedAt - a.modifiedAt);
  const teams = [...new Set(db().flatMap((s) => (s.team ? [s.team] : [])))].sort();
  return { spreadsheets: rows.map(summary), total: rows.length, teams };
}

export async function getSpreadsheet(id: string): Promise<Spreadsheet> {
  return copy(find(id));
}

export async function createSpreadsheet(req: CreateSpreadsheetRequest): Promise<Spreadsheet> {
  const name = checkName(req.name);
  const team = checkTeam(req.team);
  const definition = req.definition ?? { sheets: [sheet('sheet1', 'Sheet 1', {})] };
  checkDefinition(definition);
  const now = Date.now();
  const s: Spreadsheet = { id: newId(), name, team, author: mockMe(), sheetCount: definition.sheets.length, createdAt: now, modifiedAt: now, definition: copy(definition) };
  db().push(s);
  return copy(s);
}

export async function updateSpreadsheet(id: string, p: SpreadsheetPatch): Promise<Spreadsheet> {
  const s = find(id);
  const name = p.name === undefined ? s.name : checkName(p.name);
  const team = p.team === undefined ? s.team : checkTeam(p.team);
  if (p.definition) checkDefinition(p.definition);
  Object.assign(s, { name, team, modifiedAt: Date.now() });
  if (p.definition) {
    s.definition = copy(p.definition);
    s.sheetCount = p.definition.sheets.length;
  }
  return copy(s);
}

export async function deleteSpreadsheet(id: string): Promise<void> {
  find(id);
  store = db().filter((s) => s.id !== id);
}
