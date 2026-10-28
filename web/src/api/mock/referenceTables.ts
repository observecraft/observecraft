// In-browser reference tables for mock mode (Integrations → Reference Tables). State lives in memory
// for the session; validation and searching follow the server (routes/reference_tables.rs).
import { checkFile, checkName, MAX_DESCRIPTION_CHARS } from '../referenceTables/file';
import type { ReferenceTable, ReferenceTableFile, ReferenceTableInput, ReferenceTableRows, ReferenceTableRowsQuery } from '../referenceTables/types';
import { getProfile } from './identity';

const MAX_TABLES = 100;
const MAX_PAGE = 500;

const fail = (message: string): never => {
  throw new Error(message);
};

interface Table {
  meta: ReferenceTable;
  rows: string[][];
}

let state: Table[] | null = null;

const CATEGORIES = ['kitchen', 'garden', 'apparel', 'electronics', 'toys'];
const ITEMS = ['Mug', 'Planter', 'T-shirt', 'Headphones', 'Puzzle', 'Kettle', 'Hose', 'Hoodie', 'Charger', 'Kite'];
const OWNERS = ['catalog', 'storefront', 'checkout'];

function db(): Table[] {
  if (state) return state;
  // Fixed timestamps keep the seed deterministic.
  const at = Date.UTC(2026, 8, 14, 9, 30);
  const table = (name: string, description: string, fileName: string, columns: string[], rows: string[][], createdBy: string, days: number): Table => ({
    meta: {
      name,
      description,
      source: 'local_file',
      fileName,
      columns,
      primaryKey: columns[0],
      rowCount: rows.length,
      createdBy,
      updatedBy: createdBy,
      createdAt: at,
      updatedAt: at + days * 86_400_000,
    },
    rows,
  });
  const products = Array.from({ length: 120 }, (_, i) => {
    const n = i + 1;
    const category = CATEGORIES[i % CATEGORIES.length];
    return [`SKU-${String(n).padStart(4, '0')}`, `${ITEMS[i % ITEMS.length]} ${Math.floor(i / ITEMS.length) + 1}`, category, (((n * 7.31) % 90) + 4.99).toFixed(2), OWNERS[i % OWNERS.length]];
  });
  const services = [
    ['api-gateway', 'platform', 'tier-1', 'platform-oncall@example.com', 'example-org/api-gateway'],
    ['billing', 'billing', 'tier-1', 'billing-oncall@example.com', 'example-org/billing'],
    ['catalog-service', 'catalog', 'tier-2', 'catalog-oncall@example.com', 'example-org/catalog-service'],
    ['inventory-service', 'checkout', 'tier-2', 'checkout-oncall@example.com', 'example-org/inventory-service'],
    ['order-service', 'checkout', 'tier-1', 'checkout-oncall@example.com', 'example-org/order-service'],
    ['storefront-api', 'storefront', 'tier-1', 'storefront-oncall@example.com', 'example-org/storefront-api'],
  ];
  state = [
    table('product_catalog', 'Product names, categories and owning teams by SKU.', 'product_catalog.csv', ['sku', 'product_name', 'category', 'price_usd', 'owner_team'], products, 'Avery Chen', 12),
    table('service_owners', 'Owning team, tier and on-call address of each service.', 'service_owners.csv', ['service', 'team', 'tier', 'on_call', 'repository'], services, 'Jordan Patel', 3),
  ];
  return state;
}

const clone = (t: Table): ReferenceTable => ({ ...t.meta, columns: [...t.meta.columns] });

function find(name: string): Table {
  return db().find((t) => t.meta.name === name) ?? fail(`Reference table '${name}' not found`);
}

async function me(): Promise<string> {
  return (await getProfile()).name;
}

export async function listReferenceTables(): Promise<ReferenceTable[]> {
  return db()
    .map(clone)
    .sort((a, b) => a.name.localeCompare(b.name));
}

export async function createReferenceTable(input: ReferenceTableInput): Promise<ReferenceTable> {
  const name = input.name.trim();
  const description = (input.description ?? '').trim();
  const columns = input.columns.map((c) => c.trim());
  const primaryKey = input.primaryKey.trim();
  const nameError = checkName(name);
  if (nameError) fail(nameError);
  if (description.length > MAX_DESCRIPTION_CHARS) fail(`The description is at most ${MAX_DESCRIPTION_CHARS} characters`);
  const fileError = checkFile({ columns, rows: input.rows, primaryKey });
  if (fileError) fail(fileError);
  if (db().length >= MAX_TABLES) fail(`An organization can have at most ${MAX_TABLES} reference tables`);
  if (db().some((t) => t.meta.name === name)) fail(`A reference table named '${name}' already exists`);
  const by = await me();
  const now = Date.now();
  const table: Table = {
    meta: {
      name,
      description,
      source: 'local_file',
      fileName: (input.fileName ?? '').trim(),
      columns,
      primaryKey,
      rowCount: input.rows.length,
      createdBy: by,
      updatedBy: by,
      createdAt: now,
      updatedAt: now,
    },
    rows: input.rows.map((r) => [...r]),
  };
  db().push(table);
  return clone(table);
}

/** Rows whose primary key or a value (or the value of `column`) contains the search, case-insensitively. */
export async function getReferenceTable(name: string, q: ReferenceTableRowsQuery): Promise<ReferenceTableRows> {
  const t = find(name);
  const column = q.column?.trim() || null;
  const ci = column === null ? -1 : t.meta.columns.indexOf(column);
  if (column !== null && ci < 0) fail(`The table has no column '${column}'`);
  const s = (q.query ?? '').trim().toLowerCase();
  const hit = (r: string[]) => !s || (ci < 0 ? r.some((v) => v.toLowerCase().includes(s)) : r[ci].toLowerCase().includes(s));
  const matching = t.rows.filter(hit);
  const limit = Math.min(Math.max(1, q.limit), MAX_PAGE);
  const offset = Math.max(0, q.offset);
  return { table: clone(t), rows: matching.slice(offset, offset + limit).map((r) => [...r]), total: matching.length };
}

/** Replaces every row; the primary key column stays the table's. */
export async function replaceReferenceTable(name: string, file: ReferenceTableFile): Promise<ReferenceTable> {
  const t = find(name);
  const columns = file.columns.map((c) => c.trim());
  const fileError = checkFile({ columns, rows: file.rows, primaryKey: t.meta.primaryKey });
  if (fileError) fail(fileError);
  t.rows = file.rows.map((r) => [...r]);
  t.meta = { ...t.meta, columns, fileName: (file.fileName ?? '').trim(), rowCount: file.rows.length, updatedBy: await me(), updatedAt: Math.max(Date.now(), t.meta.updatedAt) };
  return clone(t);
}

export async function deleteReferenceTable(name: string): Promise<void> {
  const t = find(name);
  state = db().filter((x) => x !== t);
}
