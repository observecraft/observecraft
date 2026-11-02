// Generate the integrations catalog from integrations-core (BSD-3-Clause): every integration's
// manifest.json (tile, classifier tags, metric prefix, process signatures, dashboards, monitors, log
// source) and its configuration spec (assets/configuration/spec.yaml) with the shared templates
// resolved (agentSpecs/<check>.json), so the UI renders the Agent configuration form from it.
// Template resolution follows datadog_checks_dev/.../configuration/{template,spec}.py.
// integrations-extras (BSD-3-Clause, --extras) adds its community Agent checks and the integrations with log pipelines;
// its partner listings and documentation-only tiles send their data to Datadog itself and are left out.
// node scripts/import-integration-catalog.mjs [--source <integrations-core checkout>] [--extras <integrations-extras checkout at EXTRAS_REVISION>] [commit SHA]
import { mkdir, readFile, rm, writeFile } from 'node:fs/promises';
import { createRequire } from 'node:module';
import { join } from 'node:path';
import { pathToFileURL } from 'node:url';
import { EXTRAS_REVISION, PINNED_REVISION } from './import-integration-dashboards.mjs';

const yaml = createRequire(new URL('../web/package.json', import.meta.url))('js-yaml');
const TEMPLATES_DIR = 'datadog_checks_dev/datadog_checks/dev/tooling/templates/configuration';

const clone = (value) => structuredClone(value);
const isMapping = (value) => value !== null && typeof value === 'object' && !Array.isArray(value);

/** Splits `Category::Caching`-style tags into the catalog's facets. */
export function classifierTags(tags = []) {
  const facets = { categories: [], supportedOs: [], offerings: [], dataTypes: [] };
  const target = { Category: 'categories', 'Supported OS': 'supportedOs', Offering: 'offerings', 'Submitted Data Type': 'dataTypes', 'Queried Data Type': 'dataTypes' };
  for (const tag of tags) {
    const [kind, value] = tag.split('::');
    const facet = target[kind];
    if (facet && value && !facets[facet].includes(value)) facets[facet].push(value);
  }
  return facets;
}

/**
 * Configuration templates read through `read(path)` (e.g. "instances/http" → parsed YAML, or undefined).
 * `load("instances/http.username")` returns a copy of the template or of the named element in it.
 */
export function createTemplates(read) {
  function load(template) {
    const parts = template.split('/');
    const branches = parts.pop().split('.');
    parts.push(branches.shift());
    const path = parts.join('/');
    const data = read(path);
    if (data === undefined) throw new Error(`Template \`${path}\` does not exist`);
    let node = clone(data);
    for (const branch of branches) {
      const next = Array.isArray(node) ? node.find((item) => isMapping(item) && item.name === branch) : isMapping(node) ? node[branch] : undefined;
      if (next === undefined) throw new Error(`Template \`${path}\` has no element \`${branch}\``);
      node = next;
    }
    return node;
  }

  /** Finds a named item, splicing in the nested templates it lives in. */
  function expandAndFind(items, name, visited = new Set()) {
    const direct = items.find((item) => isMapping(item) && item.name === name);
    if (direct) return direct;
    for (let index = 0; index < items.length; index++) {
      const item = items[index];
      if (!isMapping(item) || 'name' in item || !('template' in item) || visited.has(item.template)) continue;
      visited.add(item.template);
      let nested;
      try {
        nested = load(item.template);
      } catch {
        continue;
      }
      if (isMapping(nested) && nested.name === name) {
        items[index] = nested;
        return nested;
      }
      if (Array.isArray(nested)) {
        const found = expandAndFind(nested, name, visited);
        if (found) {
          items.splice(index, 1, ...nested);
          return found;
        }
      }
    }
    return undefined;
  }

  /** Applies `{"a.b.c": value}` overrides to a loaded template; applied ones are removed from `overrides`. */
  function applyOverrides(template, overrides) {
    for (const key of Object.keys(overrides).sort()) {
      const keys = key.split('.');
      const last = keys.pop();
      let root = template;
      let ok = true;
      for (const [i, part] of keys.entries()) {
        if (isMapping(root)) {
          if (i === 0 && root.name === part) continue;
          if (!(part in root)) {
            ok = false;
            break;
          }
          root = root[part];
        } else if (Array.isArray(root)) {
          const found = expandAndFind(root, part);
          if (!found) {
            ok = false;
            break;
          }
          root = found;
        } else {
          ok = false;
          break;
        }
      }
      if (!ok) continue;
      if (isMapping(root)) root[last] = overrides[key];
      else if (Array.isArray(root)) {
        const found = expandAndFind(root, last);
        if (!found) continue;
        root[root.indexOf(found)] = overrides[key];
      } else continue;
      delete overrides[key];
    }
  }

  return { load, applyOverrides };
}

/** Expands every `template:` item in place; a template's `hidden` carries over to following plain items. */
function expandItems(items, templates) {
  const overrides = {};
  let pendingHidden = false;
  for (let i = 0; i < items.length; i++) {
    let item = items[i];
    if (isMapping(item) && 'template' in item) {
      while (isMapping(item) && 'template' in item) {
        pendingHidden = item.hidden ?? false;
        Object.assign(overrides, item.overrides ?? {});
        delete item.overrides;
        const name = item.template;
        delete item.template;
        const template = templates.load(name);
        if (isMapping(template) && 'name' in item) template.name = item.name;
        templates.applyOverrides(template, overrides);
        if (isMapping(template)) {
          item = Object.assign(template, item);
          items[i] = item;
        } else if (Array.isArray(template) && template.length > 0) {
          items.splice(i, 1, ...template);
          item = template[0];
        } else {
          throw new Error(`Template \`${name}\` must be a mapping or a non-empty list`);
        }
      }
    }
    if (isMapping(item)) {
      item.hidden ??= pendingHidden;
      if (Array.isArray(item.options)) expandItems(item.options, templates);
    }
  }
}

/** A copy of the spec with every option template resolved. */
export function resolveSpec(spec, templates) {
  const resolved = clone(spec);
  for (const file of resolved.files ?? []) expandItems(file.options ?? [], templates);
  return resolved;
}

/** The form's view of an option: visible options only, without authoring fields. */
function compactOption(option) {
  const out = { name: option.name };
  if (option.description) out.description = option.description.trim();
  if (option.required) out.required = true;
  if (option.secret) out.secret = true;
  if (option.multiple) out.multiple = true;
  if (option.value) out.value = option.value;
  if (Array.isArray(option.options)) out.options = option.options.filter((o) => !o.hidden).map(compactOption);
  return out;
}

/** Tiles we name differently from their manifest: Datadog's only Traefik integration is `traefik_mesh`, whose check
 *  reads any Traefik Proxy (the Kubernetes ingress controller too), so it is found and shown as Traefik. */
const TILES = {
  'traefik-mesh': { name: 'Traefik', description: 'Monitor Traefik Proxy, as an ingress controller or on its own, and Traefik Mesh: entrypoints, routers, services and logs.' },
};

const metricChecks = (check) => (check === undefined ? [] : Array.isArray(check) ? check : [check]);

/** A catalog entry from an integration's directory name and manifest.json. */
export function catalogEntry(check, manifest, repository = 'integrations-core') {
  const integration = manifest.assets?.integration ?? {};
  return {
    id: manifest.app_id,
    check,
    name: manifest.tile.title,
    publisher: manifest.author?.name ?? 'Datadog',
    description: manifest.tile.description ?? '',
    ...classifierTags(manifest.tile.classifier_tags),
    metricPrefix: integration.metrics?.prefix ?? null,
    checkMetrics: metricChecks(integration.metrics?.check),
    processSignatures: integration.process_signatures ?? [],
    logSource: manifest.assets?.logs?.source ?? null,
    dashboards: Object.keys(manifest.assets?.dashboards ?? {}),
    monitors: Object.keys(manifest.assets?.monitors ?? {}),
    configurable: Boolean(integration.configuration?.spec),
    repository,
    ...TILES[manifest.app_id],
  };
}

// Metric namespaces shared by many Agent checks: a prefix there does not identify one integration.
const GENERIC_PREFIX = /^(system|network|process|container|containerd|datadog|kubernetes|runtime|trace|http)\./;

/**
 * Unique ids (the Agent's core checks share app id "system": they get their check name instead) and the
 * metric prefix that identifies each integration in received telemetry (null when shared or generic).
 */
export function finalizeCatalog(entries) {
  const count = (values) => values.reduce((map, v) => map.set(v, (map.get(v) ?? 0) + 1), new Map());
  // Repeated until stable: a check name can collide with another integration's app id.
  let ids = entries.map((e) => e.id);
  for (let changed = true; changed;) {
    const counts = count(ids);
    const next = ids.map((id, i) => (counts.get(id) > 1 ? entries[i].check.replace(/_/g, '-') : id));
    changed = next.some((id, i) => id !== ids[i]);
    ids = next;
  }
  const prefixes = count(entries.map((e) => e.metricPrefix));
  return entries.map((e, i) => ({
    ...e,
    id: ids[i],
    detectionPrefix: e.metricPrefix && prefixes.get(e.metricPrefix) === 1 && !GENERIC_PREFIX.test(e.metricPrefix) ? e.metricPrefix : null,
  }));
}

/** Reads integrations-core from a local checkout or from GitHub at a revision. */
function source(dir, revision) {
  if (dir) {
    return {
      read: (path) => readFile(join(dir, path), 'utf8').catch(() => undefined),
      list: async () => {
        const { readdir } = await import('node:fs/promises');
        return (await readdir(dir, { withFileTypes: true })).filter((d) => d.isDirectory()).map((d) => d.name);
      },
    };
  }
  const base = `https://raw.githubusercontent.com/DataDog/integrations-core/${revision}/`;
  return {
    read: async (path) => {
      const response = await fetch(base + path);
      if (response.status === 404) return undefined;
      if (!response.ok) throw new Error(`${path}: HTTP ${response.status}`);
      return response.text();
    },
    list: async () => {
      const response = await fetch(`https://api.github.com/repos/DataDog/integrations-core/git/trees/${revision}`);
      if (!response.ok) throw new Error(`tree ${revision}: HTTP ${response.status}`);
      return (await response.json()).tree.filter((t) => t.type === 'tree').map((t) => t.path);
    },
  };
}

/** Runs `fn` over `items` with at most `limit` in flight. */
async function mapLimit(items, limit, fn) {
  const results = new Array(items.length);
  let next = 0;
  await Promise.all(
    Array.from({ length: limit }, async () => {
      while (next < items.length) {
        const index = next++;
        results[index] = await fn(items[index]);
      }
    }),
  );
  return results;
}

async function main() {
  const args = process.argv.slice(2);
  const sourceIndex = args.indexOf('--source');
  const dir = sourceIndex >= 0 ? args.splice(sourceIndex, 2)[1] : null;
  const revision = args[0] ?? PINNED_REVISION;
  const repo = source(dir, revision);

  const templateFiles = new Map();
  const readTemplate = async (path) => {
    if (!templateFiles.has(path)) {
      const text = await repo.read(`${TEMPLATES_DIR}/${path}.yaml`);
      templateFiles.set(path, text === undefined ? undefined : yaml.load(text));
    }
    return templateFiles.get(path);
  };

  const extrasIndex = args.indexOf('--extras');
  const extrasDir = extrasIndex >= 0 ? args.splice(extrasIndex, 2)[1] : null;
  const extras = extrasDir ? source(extrasDir, EXTRAS_REVISION) : null;
  // integrations-extras: what runs in the Agent (a check) or arrives as logs (a pipeline); its specs use core's templates.
  const { existsSync, readdirSync } = await import('node:fs');
  const extrasUseful = (check) =>
    existsSync(join(extrasDir, check, 'datadog_checks')) ||
    (existsSync(join(extrasDir, check, 'assets/logs')) && readdirSync(join(extrasDir, check, 'assets/logs')).some((f) => f.endsWith('.yaml') && !f.endsWith('_tests.yaml')));
  const dirs = [
    ...(await repo.list()).map((check) => [repo, check, 'integrations-core']),
    ...(extras ? (await extras.list()).filter(extrasUseful).map((check) => [extras, check, 'integrations-extras']) : []),
  ];
  const loaded = await mapLimit(dirs, 16, async ([from, check, repository]) => {
    const text = await from.read(`${check}/manifest.json`);
    if (!text) return null;
    const manifest = JSON.parse(text);
    if (!manifest.tile || manifest.display_on_public_website === false) return null;
    const entry = catalogEntry(check, manifest, repository);
    const specPath = manifest.assets?.integration?.configuration?.spec;
    const specText = specPath ? await from.read(`${check}/${specPath}`) : undefined;
    // Some manifests name a spec the repository doesn't have (profile-only SNMP tiles, gke, …).
    if (!specText) entry.configurable = false;
    // A few community specs aren't valid YAML (duplicated keys): their integration has no form, the rest stays.
    let spec = null;
    try {
      spec = specText ? yaml.load(specText) : null;
    } catch (error) {
      console.warn(`${check}: ${error.reason ?? error.message}`);
      entry.configurable = false;
    }
    return { entry, spec };
  });

  // Templates are fetched up front (the resolver is synchronous): every path a spec can reference.
  const referenced = new Set();
  const collect = (node) => {
    if (Array.isArray(node)) node.forEach(collect);
    else if (isMapping(node)) {
      if (typeof node.template === 'string') referenced.add(node.template.split('.')[0]);
      Object.values(node).forEach(collect);
    }
  };
  loaded.forEach((item) => item?.spec && collect(item.spec));
  for (let size = 0; size !== referenced.size;) {
    size = referenced.size;
    for (const path of [...referenced]) collect(await readTemplate(path));
  }
  const templates = createTemplates((path) => templateFiles.get(path));

  const integrations = [];
  const specs = {};
  for (const item of loaded) {
    if (!item) continue;
    integrations.push(item.entry);
    if (!item.spec) continue;
    if (specs[item.entry.check]) throw new Error(`${item.entry.check}: in both repositories`);
    try {
      const resolved = resolveSpec(item.spec, templates);
      const file = resolved.files.find((f) => f.name === `${item.entry.check}.yaml`) ?? resolved.files[0];
      specs[item.entry.check] = { name: file.name, options: file.options.filter((o) => !o.hidden).map(compactOption) };
    } catch (error) {
      console.warn(`${item.entry.check}: ${error.message}`);
      item.entry.configurable = false;
    }
  }
  const catalog = finalizeCatalog(integrations).sort((a, b) => a.name.localeCompare(b.name));

  const out = new URL('../web/src/api/integrations/', import.meta.url);
  const sources = [`DataDog/integrations-core@${revision}`, ...(extras ? [`DataDog/integrations-extras@${EXTRAS_REVISION}`] : [])].join(', ');
  await writeFile(new URL('catalog.generated.json', out), JSON.stringify({ source: sources, integrations: catalog }) + '\n');
  // One file per check, loaded when its integration is opened (import.meta.glob in the client).
  const specDir = new URL('agentSpecs/', out);
  await rm(specDir, { recursive: true, force: true });
  await mkdir(specDir);
  for (const [check, spec] of Object.entries(specs)) await writeFile(new URL(`${check}.json`, specDir), JSON.stringify(spec) + '\n');
  await writeFile(new URL('integrations-core.LICENSE', out), await repo.read('LICENSE'));
  if (extras) await writeFile(new URL('integrations-extras.LICENSE', out), await extras.read('LICENSE'));
  console.log(`${catalog.length} integrations, ${Object.keys(specs).length} configuration specs`);
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) await main();
