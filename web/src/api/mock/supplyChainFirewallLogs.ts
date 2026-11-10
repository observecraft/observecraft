// Mock logs of Supply Chain Firewall (Datadog's open-source `scfw`, its integration's dashboard and Logs →
// source:scfw): one per package manager command it inspected, as the Datadog Agent forwards its JSON lines and the
// integration's pipeline parses them (`created` as the date, `username` → @usr.name, `action` → @evt.outcome,
// `msg` as the message). The JSON's `hostname` is the log's host and stays `@hostname`, as the log normalizer makes it.
import type { JsonValue, LogEvent } from '../types';
import type { Rand } from './generator';

const HOUR = 3600_000;

const pick = <T>(r: Rand, a: readonly T[]): T => a[Math.floor(r() * a.length)];

/** Developer workstations and CI runners running scfw, with the people using them. */
const MACHINES: [host: string, users: string[], env: string][] = [
  ['dev-laptop-ana', ['ana.silva'], 'dev'],
  ['dev-laptop-jordan', ['jordan.patel'], 'dev'],
  ['dev-laptop-sam', ['sam.okafor'], 'dev'],
  ['dev-workstation-07', ['lee.chen', 'maria.garcia'], 'dev'],
  ['ci-runner-01', ['runner'], 'ci'],
  ['ci-runner-02', ['runner'], 'ci'],
];

/** Packages by ecosystem: name, version, and whether the firewall's verifiers find it malicious (blocked) or suspicious (warned). */
const PACKAGES: Record<'PyPI' | 'npm', [name: string, version: string, verdict: 'ok' | 'warn' | 'block'][]> = {
  PyPI: [
    ['requests', '2.32.3', 'ok'],
    ['urllib3', '2.2.3', 'ok'],
    ['fastapi', '0.115.4', 'ok'],
    ['pydantic', '2.9.2', 'ok'],
    ['boto3', '1.35.54', 'ok'],
    ['pytest', '8.3.3', 'ok'],
    ['numpy', '2.1.3', 'ok'],
    ['sqlalchemy', '2.0.36', 'ok'],
    ['urllib3', '1.24.1', 'warn'],
    ['jinja2', '2.10', 'warn'],
    ['requestss', '2.32.0', 'block'],
    ['colorama-utils', '0.4.6', 'block'],
  ],
  npm: [
    ['react', '18.3.1', 'ok'],
    ['express', '4.21.1', 'ok'],
    ['lodash', '4.17.21', 'ok'],
    ['typescript', '5.6.3', 'ok'],
    ['vite', '5.4.10', 'ok'],
    ['axios', '1.7.7', 'ok'],
    ['event-stream', '3.3.6', 'warn'],
    ['minimist', '0.0.8', 'warn'],
    ['lodahs', '4.17.21', 'block'],
    ['crossenv', '1.0.0', 'block'],
  ],
};

const MANAGERS: [manager: string, ecosystem: 'PyPI' | 'npm', weight: number][] = [
  ['pip', 'PyPI', 5],
  ['npm', 'npm', 4],
  ['poetry', 'PyPI', 2],
];
const TOTAL = MANAGERS.reduce((n, m) => n + m[2], 0);

const command = (manager: string, targets: string[]) =>
  manager === 'pip' ? `pip install ${targets.join(' ')}` : manager === 'npm' ? `npm install ${targets.join(' ')}` : `poetry add ${targets.join(' ')}`;

/** A week of firewall runs, denser in the last hours so short ranges look alive. */
export function supplyChainFirewallLogs(r: Rand, now: number): LogEvent[] {
  const out: LogEvent[] = [];
  for (let i = 0; i < 420; i++) {
    const ts = Math.round(now - r() * (i < 150 ? 3 * HOUR : 7 * 24 * HOUR));
    let w = r() * TOTAL;
    const [manager, ecosystem] = MANAGERS.find((m) => (w -= m[2]) < 0) ?? MANAGERS[0];
    const [host, users, env] = pick(r, MACHINES);
    const user = pick(r, users);
    const count = r() < 0.75 ? 1 : 2;
    const chosen = Array.from({ length: count }, () => pick(r, PACKAGES[ecosystem]));
    const blocked = chosen.some((p) => p[2] === 'block');
    const warning = !blocked && chosen.some((p) => p[2] === 'warn');
    // A warned command goes ahead when the developer confirms it (most do), or is stopped.
    const action = blocked || (warning && r() < 0.3) ? 'BLOCK' : 'ALLOW';
    const spec = chosen.map(([name, version]) => (manager === 'pip' ? `${name}==${version}` : `${name}@${version}`));
    const cwd = `/home/${user}/src/${pick(r, ['storefront-api', 'order-service', 'billing', 'catalog-service', 'inventory-service'])}`;
    const attributes: Record<string, JsonValue> = {
      created: ts,
      cwd,
      ecosystem,
      env,
      evt: { outcome: action },
      executable: manager === 'npm' ? '/usr/local/bin/npm' : manager === 'poetry' ? `/home/${user}/.local/bin/poetry` : `${cwd}/.venv/bin/python`,
      hostname: host,
      install_targets: chosen.map(([name, version]) => ({ ecosystem, name, version })),
      log_version: 1,
      package_manager: manager,
      usr: { name: user },
      verified: true,
      version: '3.1.0',
      warning,
    };
    out.push({
      id: `scfw-${ts}-${i}`,
      timestamp: ts,
      status: 'info',
      service: 'scfw',
      host,
      source: 'scfw',
      message: `Command '${command(manager, spec)}' was ${action === 'ALLOW' ? 'allowed' : 'blocked'}`,
      tags: {},
      attributes,
    });
  }
  return out;
}
