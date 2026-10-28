// In-browser standard attributes for mock mode (Logs → Configuration → Standard Attributes). The
// defaults are the server's (server/seeds/standard_attributes.json); like the server, only the
// session's changes are kept (in memory), merged onto them. Validation follows
// server/crates/oc-api/src/standard_attributes.rs.
import seed from '../../../../server/seeds/standard_attributes.json';
import type { StandardAttribute, StandardAttributeInput } from '../standardAttributes/types';

const MAX_ATTRIBUTES = 500;
const MAX_ALIASES = 20;
const MAX_PATH_CHARS = 200;
const MAX_DESCRIPTION_CHARS = 500;
const RESERVED = ['message', 'status', 'service', 'host', 'source', 'date', 'timestamp', 'trace_id', 'span_id'];

type Definition = Required<StandardAttributeInput>;

const DEFAULTS: Definition[] = (seed.attributes as Definition[]).map((d) => ({ ...d, preserveSource: d.preserveSource ?? true })).sort((a, b) => cmp(a.path, b.path));

/** The session's changes: a definition per added or edited path, `null` for a removed default. */
let overrides = new Map<string, Definition | null>();

const fail = (message: string): never => {
  throw new Error(message);
};
const copy = <T>(v: T): T => structuredClone(v);

function cmp(a: string, b: string): number {
  return a < b ? -1 : a > b ? 1 : 0;
}

const same = (a: Definition, b: Definition) => a.type === b.type && a.description === b.description && a.preserveSource === b.preserveSource && JSON.stringify(a.aliases) === JSON.stringify(b.aliases);

function merged(): StandardAttribute[] {
  const list: StandardAttribute[] = [];
  for (const d of DEFAULTS) {
    const own = overrides.get(d.path);
    if (own === null) continue;
    list.push({ ...(own ?? d), isDefault: true, modified: own ? !same(own, d) : false });
  }
  for (const [path, own] of overrides) if (own && !DEFAULTS.some((d) => d.path === path)) list.push({ ...own, isDefault: false, modified: false });
  return list.sort((a, b) => cmp(a.path, b.path));
}

const validPath = (p: string) => p.length > 0 && p.length <= MAX_PATH_CHARS && /^[A-Za-z0-9_.-]+$/.test(p) && !p.startsWith('.') && !p.endsWith('.') && !p.includes('..');
const contains = (outer: string, inner: string) => inner === outer || inner.startsWith(`${outer}.`);
const clean = (p: string) => p.trim().replace(/^@+/, '');

/** Trimmed and checked as the server does; throws with the server's messages. */
function validate(input: StandardAttributeInput, others: StandardAttribute[]): Definition {
  const path = clean(input.path);
  const description = (input.description ?? '').trim();
  const aliases = (input.aliases ?? []).map(clean);
  if (!validPath(path)) fail(`Attribute paths are up to ${MAX_PATH_CHARS} letters, digits, '_', '-' and dots (e.g. http.status_code)`);
  if (RESERVED.includes(path)) fail(`'${path}' is a reserved field of the log, not an attribute`);
  const owner = others.find((o) => o.aliases.includes(path));
  if (owner) fail(`'${path}' is an alias of ${owner.path}`);
  if (description.length > MAX_DESCRIPTION_CHARS) fail(`The description is at most ${MAX_DESCRIPTION_CHARS} characters`);
  if (aliases.length > MAX_ALIASES) fail(`At most ${MAX_ALIASES} aliases`);
  const seen = new Set<string>();
  for (const a of aliases) {
    if (!validPath(a)) fail(`'${a}' isn't a valid attribute path`);
    if (RESERVED.includes(a)) fail(`'${a}' is a reserved field of the log, not an attribute`);
    if (contains(a, path)) fail(`The alias '${a}' contains the attribute itself`);
    if (seen.has(a)) fail(`'${a}' is listed twice`);
    seen.add(a);
    if (others.some((o) => o.path === a)) fail(`'${a}' is a standard attribute itself`);
    const other = others.find((o) => o.aliases.includes(a));
    if (other) fail(`'${a}' is already an alias of ${other.path}`);
  }
  return { path, type: input.type, description, aliases, preserveSource: input.preserveSource ?? true };
}

function find(path: string): StandardAttribute {
  return merged().find((a) => a.path === path) ?? fail(`No standard attribute '${path}'`);
}

/** The paths of the session's standard attributes (the explorer's facets mark them). */
export function standardPaths(): Set<string> {
  return new Set(merged().map((a) => a.path));
}

export async function listStandardAttributes(): Promise<StandardAttribute[]> {
  return copy(merged());
}

export async function createStandardAttribute(input: StandardAttributeInput): Promise<StandardAttribute> {
  const list = merged();
  const path = clean(input.path);
  if (list.some((a) => a.path === path)) fail(`'${path}' is already a standard attribute`);
  if (list.length >= MAX_ATTRIBUTES) fail(`An organization can have at most ${MAX_ATTRIBUTES} standard attributes`);
  const def = validate(input, list);
  overrides.set(def.path, def);
  return copy(find(def.path));
}

export async function updateStandardAttribute(path: string, input: StandardAttributeInput): Promise<StandardAttribute> {
  const list = merged();
  find(path);
  if (clean(input.path) !== path) fail("A standard attribute can't be renamed: add a new one");
  const def = validate(
    input,
    list.filter((a) => a.path !== path),
  );
  overrides.set(path, def);
  return copy(find(path));
}

export async function deleteStandardAttribute(path: string): Promise<void> {
  const attribute = find(path);
  if (attribute.isDefault) overrides.set(path, null);
  else overrides.delete(path);
}

export async function resetStandardAttributes(): Promise<StandardAttribute[]> {
  overrides = new Map();
  return copy(merged());
}
