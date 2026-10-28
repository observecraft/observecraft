// In-browser log restriction queries for mock mode (Logs → Configuration → Data Access). State lives
// in memory for the session; validation follows the server (routes/log_restrictions.rs and
// log_access.rs). Roles and teams come from the identity mock; previews count the mock logs.
import { parseQuery } from '../../lib/searchQuery';
import type { LogRestrictionQuery, RestrictionPreview, RestrictionQueryInput } from '../logRestrictions/types';
import { getProfile, listRoles, MOCK_ORG, searchTeams } from './identity';
import { mockHistogram } from './index';

const MAX_QUERIES = 100;
const MAX_NAME_CHARS = 100;
const MAX_DESCRIPTION_CHARS = 500;
const MAX_QUERY_CHARS = 2000;
const MAX_ASSIGNMENTS = 50;
const HOUR_MS = 3_600_000;
const ADMIN_ROLE = `${MOCK_ORG.id}-admin`;

const fail = (message: string): never => {
  throw new Error(message);
};

interface Row {
  id: string;
  name: string;
  description: string;
  query: string;
  roleIds: string[];
  teamHandles: string[];
  createdAt: number;
  updatedAt: number;
}

interface State {
  rows: Row[];
  seq: number;
}

let state: State | null = null;

function db(): State {
  if (state) return state;
  // Fixed timestamps keep the seed deterministic.
  const at = Date.UTC(2026, 8, 1);
  const row = (id: string, name: string, description: string, query: string, roleIds: string[], teamHandles: string[]): Row => ({
    id,
    name,
    description,
    query,
    roleIds,
    teamHandles,
    createdAt: at,
    updatedAt: at,
  });
  const org = MOCK_ORG.id;
  state = {
    rows: [
      row('lr-0001', 'Billing', 'Invoices and payment reconciliation logs.', 'service:billing', [`${org}-billing-developers`], ['billing', 'payments']),
      row('lr-0002', 'Checkout production', 'Orders and stock reservations in production.', 'env:prod service:(order-service OR inventory-service)', [`${org}-checkout-developers`], ['checkout']),
      row('lr-0003', 'Storefront', 'The storefront API and the gateway in front of it.', 'service:(storefront-api OR api-gateway)', [`${org}-storefront-developers`], ['storefront']),
      row('lr-0004', 'Support (no debug logs)', 'Support reads every service but not debug output.', '-status:debug', [`${org}-support-read-access`], []),
    ],
    seq: 5,
  };
  return state;
}

/** Quotes close, and parentheses outside quotes close in order. */
function balanced(q: string): boolean {
  let depth = 0;
  let quoted = false;
  for (const c of q) {
    if (c === '"') quoted = !quoted;
    else if (!quoted && c === '(') depth++;
    else if (!quoted && c === ')' && --depth < 0) return false;
  }
  return depth === 0 && !quoted;
}

/** Checks a restriction query as the server does. */
function checkQuery(query: string): string {
  const q = query.trim();
  if (q.length > MAX_QUERY_CHARS) fail(`The query is at most ${MAX_QUERY_CHARS} characters`);
  if (!balanced(q)) fail('Unbalanced quotes or parentheses');
  const terms = parseQuery(q);
  if (terms.length === 0) fail('Enter a search, e.g. service:billing');
  if (terms.some((t) => t.kind === 'tag' && t.key === 'index')) fail("Restriction queries can't filter on index");
  return q;
}

const unique = (values: string[]) => [...new Set(values.map((v) => v.trim()).filter(Boolean))];

async function knownRoles(): Promise<Map<string, string>> {
  return new Map((await listRoles()).map((r) => [r.id, r.name]));
}

async function knownTeams(): Promise<Map<string, string>> {
  return new Map((await searchTeams({ offset: 0, limit: 100, sort: 'name' })).teams.map((t) => [t.handle, t.name]));
}

async function checkRoles(ids: string[]): Promise<void> {
  if (ids.includes(ADMIN_ROLE)) fail("Admins always read every log: the admin role can't be restricted");
  const roles = await knownRoles();
  const unknown = ids.filter((id) => !roles.has(id));
  if (unknown.length) fail(`Unknown role: ${unknown.join(', ')}`);
}

async function checkTeams(handles: string[]): Promise<void> {
  const teams = await knownTeams();
  const unknown = handles.filter((h) => !teams.has(h));
  if (unknown.length) fail(`Unknown team: ${unknown.join(', ')}`);
}

/** Trimmed and checked as the server does; throws with the server's messages. */
async function validate(input: RestrictionQueryInput, id: string | null): Promise<Omit<Row, 'id' | 'createdAt' | 'updatedAt'>> {
  const name = input.name.trim();
  const description = (input.description ?? '').trim();
  if (!name || name.length > MAX_NAME_CHARS) fail(`Name must be 1–${MAX_NAME_CHARS} characters`);
  if (description.length > MAX_DESCRIPTION_CHARS) fail(`The description is at most ${MAX_DESCRIPTION_CHARS} characters`);
  const query = checkQuery(input.query);
  const roleIds = unique(input.roleIds ?? []);
  const teamHandles = unique(input.teamHandles ?? []);
  if (roleIds.length > MAX_ASSIGNMENTS || teamHandles.length > MAX_ASSIGNMENTS) fail(`At most ${MAX_ASSIGNMENTS} roles and ${MAX_ASSIGNMENTS} teams per query`);
  await checkRoles(roleIds);
  await checkTeams(teamHandles);
  if (db().rows.some((r) => r.name === name && r.id !== id)) fail(`A restriction query named '${name}' already exists`);
  return { name, description, query, roleIds, teamHandles };
}

async function view(r: Row): Promise<LogRestrictionQuery> {
  const [roles, teams] = await Promise.all([knownRoles(), knownTeams()]);
  const byName = (a: { name: string }, b: { name: string }) => a.name.localeCompare(b.name);
  return {
    id: r.id,
    name: r.name,
    description: r.description,
    query: r.query,
    roles: r.roleIds
      .filter((id) => roles.has(id))
      .map((id) => ({ id, name: roles.get(id)! }))
      .sort(byName),
    teams: r.teamHandles
      .filter((h) => teams.has(h))
      .map((handle) => ({ handle, name: teams.get(handle)! }))
      .sort(byName),
    createdBy: 'Alex Morgan',
    createdAt: r.createdAt,
    updatedAt: r.updatedAt,
  };
}

const find = (id: string): Row => db().rows.find((r) => r.id === id) ?? fail('Restriction query not found');

export async function listRestrictionQueries(): Promise<LogRestrictionQuery[]> {
  const rows = [...db().rows].sort((a, b) => a.name.localeCompare(b.name));
  return Promise.all(rows.map(view));
}

export async function createRestrictionQuery(input: RestrictionQueryInput): Promise<LogRestrictionQuery> {
  const fields = await validate(input, null);
  const s = db();
  if (s.rows.length >= MAX_QUERIES) fail(`An organization can have at most ${MAX_QUERIES} restriction queries`);
  const now = Date.now();
  const row: Row = { id: `lr-${String(s.seq++).padStart(4, '0')}`, ...fields, createdAt: now, updatedAt: now };
  s.rows.push(row);
  return view(row);
}

export async function updateRestrictionQuery(id: string, input: RestrictionQueryInput): Promise<LogRestrictionQuery> {
  const row = find(id);
  Object.assign(row, await validate(input, id), { updatedAt: Date.now() });
  return view(row);
}

export async function deleteRestrictionQuery(id: string): Promise<void> {
  const row = find(id);
  db().rows = db().rows.filter((r) => r !== row);
}

export async function assignRole(id: string, roleId: string): Promise<LogRestrictionQuery> {
  const row = find(id);
  const role = roleId.trim();
  await checkRoles([role]);
  if (row.roleIds.length >= MAX_ASSIGNMENTS) fail(`At most ${MAX_ASSIGNMENTS} roles per query`);
  if (!row.roleIds.includes(role)) row.roleIds.push(role);
  return view(row);
}

export async function unassignRole(id: string, roleId: string): Promise<LogRestrictionQuery> {
  const row = find(id);
  row.roleIds = row.roleIds.filter((r) => r !== roleId);
  return view(row);
}

export async function assignTeam(id: string, handle: string): Promise<LogRestrictionQuery> {
  const row = find(id);
  const team = handle.trim();
  await checkTeams([team]);
  if (row.teamHandles.length >= MAX_ASSIGNMENTS) fail(`At most ${MAX_ASSIGNMENTS} teams per query`);
  if (!row.teamHandles.includes(team)) row.teamHandles.push(team);
  return view(row);
}

export async function unassignTeam(id: string, handle: string): Promise<LogRestrictionQuery> {
  const row = find(id);
  row.teamHandles = row.teamHandles.filter((h) => h !== handle);
  return view(row);
}

/** Counts the mock logs of the last hour matching a (valid) restriction query. */
export async function previewRestrictionQuery(query: string): Promise<RestrictionPreview> {
  const q = checkQuery(query);
  const to = Date.now();
  const buckets = mockHistogram({ range: { from: to - HOUR_MS, to }, query: q });
  return { count: buckets.reduce((n, b) => n + b.counts.error + b.counts.warn + b.counts.info + b.counts.debug, 0) };
}

/** Whether the mock viewer's Log Explorer is restricted: only when queries exist and they aren't an admin. */
export async function mockViewRestricted(): Promise<boolean> {
  if (db().rows.length === 0) return false;
  const me = await getProfile();
  return !me.roles.some((r) => r.id === ADMIN_ROLE);
}
