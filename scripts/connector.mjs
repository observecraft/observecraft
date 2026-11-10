// Connector packages (docs/connectors.md): build, check and scaffold them.
//
//   node scripts/connector.mjs new <id> [--dir <parent>]   a package skeleton to fill in (default parent: integrations/connectors)
//   node scripts/connector.mjs build [<package dir>...]     converts assets/dashboards/*.json (Datadog's format) into
//                                                           dist/dashboards.json; without a directory, every bundled
//                                                           package, then web/src/api/connectors/packages.generated.json
//   node scripts/connector.mjs validate <package dir>...    the server's own validation (oc-connector validate)
//   node scripts/connector.mjs test <package dir>...        validation, then the recorded answers replayed through the
//                                                           engine (OC_CONNECTOR_UPDATE=1 rewrites golden files)
//   node scripts/connector.mjs schema                       regenerates integrations/connectors/connector.schema.json
//
// validate, test and schema run the Rust CLI (server/crates/oc-connectors, `cargo run --bin oc-connector`), so a package
// is checked by the code that runs it. Set OC_CONNECTOR_BIN to a built oc-connector to skip cargo.
import { execFileSync, spawnSync } from 'node:child_process';
import { existsSync, mkdirSync, readdirSync, readFileSync, statSync, writeFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { join, relative, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { convertDashboard } from './import-integration-dashboards.mjs';

const ROOT = resolve(fileURLToPath(new URL('..', import.meta.url)));
const BUNDLED = join(ROOT, 'integrations/connectors');
const yaml = createRequire(join(ROOT, 'web/package.json'))('js-yaml');

/** The bundled package directories. */
export function bundledPackages() {
  return readdirSync(BUNDLED)
    .map((d) => join(BUNDLED, d))
    .filter((d) => statSync(d).isDirectory() && existsSync(join(d, 'connector.yaml')))
    .sort();
}

export function readManifest(dir) {
  return yaml.load(readFileSync(join(dir, 'connector.yaml'), 'utf8'));
}

/** Image widgets (banners) become the package's logo tile: packages ship no images. */
function withLogo(widgets, name, logo) {
  return widgets.map((w) => {
    if (w.definition.type === 'group') return { ...w, definition: { ...w.definition, widgets: withLogo(w.definition.widgets, name, logo) } };
    if (w.definition.type === 'image') return { ...w, definition: { type: 'image', title: name, logo, background: '#ffffff' } };
    return w;
  });
}

function renamed(widgets, id) {
  return widgets.map((w) => ({
    ...w,
    id: w.id.replace(/^upstream-/, `${id}-`),
    definition: w.definition.type === 'group' ? { ...w.definition, widgets: renamed(w.definition.widgets, id) } : w.definition,
  }));
}

/** A package's dashboards in ObserveCraft's format, as `dist/dashboards.json` holds them. */
export function buildDashboards(dir) {
  const m = readManifest(dir);
  const logo = m.logo ?? m.tiles?.[0] ?? m.id;
  return (m.assets?.dashboards ?? []).map((path) => {
    const doc = JSON.parse(readFileSync(join(dir, path), 'utf8'));
    const converted = convertDashboard(doc, m.id, path, m.version ?? '1.0.0', m.publisher);
    return {
      ...converted,
      logo,
      description: doc.description ?? `${m.name} dashboard of the ${m.name} connector.`,
      widgets: renamed(withLogo(converted.widgets, m.name, logo), m.id),
      source: { repository: `connector:${m.id}`, path: relative(dir, join(dir, path)), license: m.publisher === 'ObserveCraft' ? 'MIT' : 'see the package' },
    };
  });
}

function cli(args, { capture = false } = {}) {
  const bin = process.env.OC_CONNECTOR_BIN;
  const [cmd, cmdArgs] = bin ? [bin, args] : ['cargo', ['+stable', 'run', '-q', '--manifest-path', join(ROOT, 'server/Cargo.toml'), '-p', 'oc-connectors', '--bin', 'oc-connector', '--', ...args]];
  if (capture) return execFileSync(cmd, cmdArgs, { encoding: 'utf8', stdio: ['ignore', 'pipe', 'inherit'], maxBuffer: 64 << 20 });
  const r = spawnSync(cmd, cmdArgs, { stdio: 'inherit' });
  return r.status ?? 1;
}

function build(dirs) {
  const all = dirs.length === 0;
  for (const dir of all ? bundledPackages() : dirs.map((d) => resolve(d))) {
    const dashboards = buildDashboards(dir);
    if (dashboards.length === 0) continue;
    mkdirSync(join(dir, 'dist'), { recursive: true });
    writeFileSync(join(dir, 'dist/dashboards.json'), JSON.stringify(dashboards, null, 2) + '\n');
    console.log(`${relative(ROOT, dir)}: wrote dist/dashboards.json (${dashboards.length})`);
  }
  if (all) {
    // The web's mock mode: every bundled package's catalog entry, its mock connection and dashboards.
    const entries = JSON.parse(cli(['catalog', ...bundledPackages()], { capture: true }));
    const out = entries.map((e) => {
      const dir = join(BUNDLED, e.entry.id);
      const dist = join(dir, 'dist/dashboards.json');
      // Read from their directories, but bundled: not third-party.
      return { ...e, entry: { ...e.entry, thirdParty: false }, dashboards: existsSync(dist) ? JSON.parse(readFileSync(dist, 'utf8')) : [] };
    });
    const target = join(ROOT, 'web/src/api/connectors/packages.generated.json');
    mkdirSync(join(ROOT, 'web/src/api/connectors'), { recursive: true });
    writeFileSync(target, JSON.stringify(out, null, 2) + '\n');
    console.log(`wrote ${relative(ROOT, target)} (${out.length} packages)`);
  }
}

function scaffold(id, parent) {
  if (!/^[a-z0-9][a-z0-9_.-]{0,99}$/.test(id ?? '')) throw new Error('usage: node scripts/connector.mjs new <id> (lowercase letters, digits, -, _ and .)');
  const dir = join(resolve(parent), id);
  if (existsSync(dir)) throw new Error(`${dir} exists`);
  mkdirSync(join(dir, 'fixtures'), { recursive: true });
  writeFileSync(
    join(dir, 'connector.yaml'),
    `# yaml-language-server: $schema=${relative(dir, join(BUNDLED, 'connector.schema.json'))}
schema_version: 1
id: ${id}
name: ${id}
publisher: Your name or company
vendor: The vendor
description: What is collected and how.
tiles: [${id}]
docs_url: https://docs.example.com/api
setup: |
  1. Where to create the key, which role it needs.
instance: "{{ config.base_url | host }}"
config:
  - { key: base_url, label: API URL, type: url, required: true, default: "https://api.example.com" }
  - { key: api_key, label: API key, type: string, secret: true, required: true }
auth:
  kind: bearer
  token: "{{ secrets.api_key }}"
streams:
  - id: events
    name: Audit events
    description: Every audit event as a log.
    request:
      url: "{{ config.base_url }}/v1/audit"
      query: { since: "{{ window.start | rfc3339 }}", cursor: "{{ page.cursor }}" }
    records: "{{ response.data }}"
    pagination: { kind: cursor, next: "{{ response.next_cursor }}" }
    record_id: "{{ record.id }}"
    incremental: { kind: time, record_time: "{{ record.created_at }}", lookback: 1d, overlap: 5m }
    output:
      logs:
        message: "{{ record.action }}"
        attributes:
          evt: { name: "{{ record.action }}" }
          usr: { email: "{{ record.actor.email }}" }
test:
  request: { url: "{{ config.base_url }}/v1/me" }
  message: "Connected as {{ response.email }}"
mock:
  config: { base_url: "https://api.example.com" }
  secrets: { api_key: test-key }
  now: "2026-11-10T12:00:00Z"
  responses:
    - { url: "https://api.example.com/v1/audit", file: fixtures/audit.json }
    - { url: "https://api.example.com/v1/me", body: { email: "admin@example.com" } }
  expect:
    events: { records: 1, logs: 1, rerun: 0 }
    test: { ok: true, message: "admin@example.com" }
`,
  );
  writeFileSync(
    join(dir, 'fixtures/audit.json'),
    JSON.stringify({ data: [{ id: 'evt-1', action: 'user.login', created_at: '2026-11-10T11:30:00Z', actor: { email: 'jane.doe@example.com' } }], next_cursor: null }, null, 2) + '\n',
  );
  console.log(`wrote ${relative(ROOT, dir)}: edit connector.yaml, record real answers (fictional data) in fixtures/, then node scripts/connector.mjs test ${relative(process.cwd(), dir)}`);
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const [cmd, ...rest] = process.argv.slice(2);
  try {
    switch (cmd) {
      case 'new': {
        const i = rest.indexOf('--dir');
        scaffold(rest[0], i >= 0 ? rest[i + 1] : BUNDLED);
        break;
      }
      case 'build':
        build(rest);
        break;
      case 'validate':
      case 'test':
        if (rest.length === 0) rest.push(...bundledPackages());
        process.exitCode = cli([cmd, ...rest]);
        break;
      case 'schema':
        writeFileSync(join(BUNDLED, 'connector.schema.json'), cli(['schema'], { capture: true }));
        console.log('wrote integrations/connectors/connector.schema.json');
        break;
      default:
        console.error('usage: node scripts/connector.mjs new <id> | build [dir...] | validate <dir>... | test <dir>... | schema');
        process.exitCode = 2;
    }
  } catch (e) {
    console.error(e.message);
    process.exitCode = 1;
  }
}
