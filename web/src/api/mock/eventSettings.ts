// In-browser Event Management settings for mock mode: correlators, maintenance windows and work item
// attributes (and their values), in memory for the session. Validation follows routes/event_settings.rs;
// the mock Triage Inbox (mock/events.ts) applies them like the API.
import type { EventCorrelator, EventCorrelatorInput, EventMaintenanceWindow, EventMaintenanceWindowInput, WorkItemAttribute, WorkItemAttributeInput } from '../events/client';

const fail = (message: string): never => {
  throw new Error(message);
};

const CORE_KEYS = ['source', 'host', 'status', 'title'];
const tagKeyOk = (k: string) => /^[A-Za-z0-9_.\-/]{1,100}$/.test(k);

interface State {
  correlators: EventCorrelator[];
  windows: EventMaintenanceWindow[];
  attributes: WorkItemAttribute[];
  /** work item id → attribute id → value */
  values: Record<string, Record<string, string>>;
  seq: number;
}

let state: State | null = null;

function db(): State {
  if (state) return state;
  const now = Date.now();
  state = {
    correlators: [],
    windows: [
      {
        id: 'mw-0001',
        name: 'Billing database upgrade',
        query: 'service:billing',
        startsAt: now - 6 * 3_600_000,
        endsAt: now + 3_600_000,
        createdBy: 'Sam Lee',
      },
    ],
    attributes: [
      { id: 'wa-0001', name: 'Severity', kind: 'select', options: ['SEV-1', 'SEV-2', 'SEV-3'], position: 0 },
      { id: 'wa-0002', name: 'Ticket', kind: 'text', options: [], position: 1 },
    ],
    values: {},
    seq: 2,
  };
  return state;
}

const id = (prefix: string) => `${prefix}-${String(db().seq++).padStart(4, '0')}`;
const unique = (list: { id: string; name: string }[], name: string, what: string, self?: string) =>
  list.some((x) => x.id !== self && x.name.toLowerCase() === name.toLowerCase()) && fail(`A ${what} named '${name}' already exists`);
const nameOk = (n: string) => (n.length === 0 || n.length > 200) && fail('The name is 1 to 200 characters');

function validCorrelator(i: EventCorrelatorInput): Omit<EventCorrelator, 'id' | 'updatedBy' | 'updatedAt'> {
  const name = i.name.trim();
  nameOk(name);
  const groupBy = [...new Set((i.groupBy ?? []).map((k) => k.trim()).filter(Boolean))];
  const bad = groupBy.find((k) => !CORE_KEYS.includes(k) && !tagKeyOk(k));
  if (bad) fail(`Can't group by '${bad}': use source, host, status, title or a tag key`);
  if (groupBy.length > 5) fail('Group by at most 5 keys');
  if (!(i.windowMinutes >= 1 && i.windowMinutes <= 1440)) fail('The time window is 1 minute to 24 hours');
  return { name, query: (i.query ?? '').trim(), groupBy, windowMinutes: i.windowMinutes, enabled: i.enabled ?? true, position: i.position ?? 0 };
}

const byPosition = (a: EventCorrelator, b: EventCorrelator) => a.position - b.position || a.name.localeCompare(b.name);

export async function listCorrelators(): Promise<EventCorrelator[]> {
  return [...db().correlators].sort(byPosition).map((c) => ({ ...c }));
}

/** The enabled correlators in order (the mock inbox). */
export function activeCorrelators(): EventCorrelator[] {
  return db()
    .correlators.filter((c) => c.enabled)
    .sort(byPosition);
}

export async function createCorrelator(i: EventCorrelatorInput): Promise<EventCorrelator> {
  const v = validCorrelator(i);
  unique(db().correlators, v.name, 'correlator');
  const c = { id: id('ec'), ...v, updatedBy: 'You', updatedAt: Date.now() };
  db().correlators.push(c);
  return { ...c };
}

export async function updateCorrelator(cid: string, i: EventCorrelatorInput): Promise<EventCorrelator> {
  const v = validCorrelator(i);
  unique(db().correlators, v.name, 'correlator', cid);
  const c = db().correlators.find((x) => x.id === cid) ?? fail('Correlator not found');
  Object.assign(c, v, { updatedBy: 'You', updatedAt: Date.now() });
  return { ...c };
}

export async function deleteCorrelator(cid: string): Promise<void> {
  const s = db();
  if (!s.correlators.some((c) => c.id === cid)) fail('Correlator not found');
  s.correlators = s.correlators.filter((c) => c.id !== cid);
}

export async function listWindows(): Promise<EventMaintenanceWindow[]> {
  return [...db().windows].sort((a, b) => b.startsAt - a.startsAt).map((w) => ({ ...w }));
}

/** Windows overlapping the range (the mock inbox). */
export function windowsBetween(from: number, to: number): EventMaintenanceWindow[] {
  return db().windows.filter((w) => w.endsAt >= from && w.startsAt <= to);
}

export async function createWindow(i: EventMaintenanceWindowInput): Promise<EventMaintenanceWindow> {
  const name = i.name.trim();
  nameOk(name);
  if (i.endsAt <= i.startsAt) fail('The window ends after it starts');
  if (i.endsAt - i.startsAt > 31 * 86_400_000) fail('A maintenance window is at most 31 days');
  const w = { id: id('mw'), name, query: (i.query ?? '').trim(), startsAt: i.startsAt, endsAt: i.endsAt, createdBy: 'You' };
  db().windows.push(w);
  return { ...w };
}

export async function deleteWindow(wid: string): Promise<void> {
  const s = db();
  if (!s.windows.some((w) => w.id === wid)) fail('Maintenance window not found');
  s.windows = s.windows.filter((w) => w.id !== wid);
}

function validAttribute(i: WorkItemAttributeInput): Omit<WorkItemAttribute, 'id'> {
  const name = i.name.trim();
  nameOk(name);
  const options = i.kind === 'text' ? [] : [...new Set((i.options ?? []).map((o) => o.trim()).filter(Boolean))];
  if (options.some((o) => o.length > 100)) fail('Values are at most 100 characters');
  if (i.kind === 'select' && options.length === 0) fail('A select lists at least one value');
  return { name, kind: i.kind, options, position: i.position ?? 0 };
}

export async function listAttributes(): Promise<WorkItemAttribute[]> {
  return [...db().attributes].sort((a, b) => a.position - b.position || a.name.localeCompare(b.name)).map((a) => ({ ...a, options: [...a.options] }));
}

export async function createAttribute(i: WorkItemAttributeInput): Promise<WorkItemAttribute> {
  const v = validAttribute(i);
  unique(db().attributes, v.name, 'attribute');
  const a = { id: id('wa'), ...v };
  db().attributes.push(a);
  return { ...a };
}

export async function updateAttribute(aid: string, i: WorkItemAttributeInput): Promise<WorkItemAttribute> {
  const v = validAttribute(i);
  unique(db().attributes, v.name, 'attribute', aid);
  const a = db().attributes.find((x) => x.id === aid) ?? fail('Attribute not found');
  Object.assign(a, v);
  return { ...a };
}

export async function deleteAttribute(aid: string): Promise<void> {
  const s = db();
  if (!s.attributes.some((a) => a.id === aid)) fail('Attribute not found');
  s.attributes = s.attributes.filter((a) => a.id !== aid);
  for (const v of Object.values(s.values)) delete v[aid];
}

export async function setValue(workItemId: string, attributeId: string, value: string): Promise<void> {
  const a = db().attributes.find((x) => x.id === attributeId) ?? fail('Attribute not found');
  const v = value.trim();
  if (a.kind === 'select' && v && !a.options.includes(v)) fail(`'${v}' isn't a value of ${a.name}`);
  const values = (db().values[workItemId] ??= {});
  if (v) values[attributeId] = v;
  else delete values[attributeId];
}

/** A work item's attribute values. */
export function valuesOf(workItemId: string): Record<string, string> {
  return { ...(db().values[workItemId] ?? {}) };
}
