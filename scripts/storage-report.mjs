#!/usr/bin/env node
// Prints the storage report (scripts/storage-report.sql) for a window: rows and bytes written per
// table, merges, CPU by query kind and the heaviest SELECT shapes (docs/adr/0002).
//
//   node scripts/storage-report.mjs [--window 1h] [--cluster oc]      (make storage-report WINDOW=1h CLUSTER=oc)
//
// Env: OC_CLICKHOUSE_URL (http://localhost:8123), OC_CLICKHOUSE_USER / OC_CLICKHOUSE_PASSWORD
// (observecraft), OC_CLICKHOUSE_CLUSTER (the cluster name; empty: single node).
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

const UNITS = { s: 1, m: 60, h: 3600, d: 86400 };

/** `1h`, `20m`, `90s`, `2d` or plain seconds → seconds. */
export function parseWindow(text) {
  const m = /^(\d+)([smhd]?)$/.exec(String(text).trim());
  const seconds = m ? Number(m[1]) * UNITS[m[2] || 's'] : NaN;
  if (!Number.isSafeInteger(seconds) || seconds <= 0) throw new Error(`invalid window: ${text} (use e.g. 1h, 20m, 3600)`);
  return seconds;
}

/** The report's statements, each titled by the comment line right above it. */
export function statements(sql) {
  return sql
    .split(/;\s*(?:\n|$)/)
    .map((chunk) => {
      const lines = chunk.split('\n');
      const first = lines.findIndex((l) => l.trim() !== '' && !l.trim().startsWith('--'));
      if (first < 0) return null;
      let top = first;
      while (top > 0 && lines[top - 1].trim().startsWith('--')) top--;
      const title = top < first ? lines[top].trim().replace(/^--\s*/, '') : '';
      return { title, sql: lines.slice(first).join('\n').trim() };
    })
    .filter(Boolean);
}

/** A cluster name as it may appear in SQL (it is interpolated, not bound). */
function clusterName(cluster) {
  if (!/^[A-Za-z0-9_.-]+$/.test(cluster)) throw new Error(`invalid cluster name: ${cluster}`);
  return cluster;
}

/** The statement reading `system.part_log` / `system.query_log` of every replica of `cluster`. */
export function forCluster(sql, cluster) {
  if (!cluster) return sql;
  return sql.replace(/\bsystem\.(part_log|query_log)\b/g, `clusterAllReplicas('${clusterName(cluster)}', system.$1)`);
}

async function run(url, auth, sql, params) {
  const query = new URLSearchParams({ log_comment: 'storage-report', ...params });
  const res = await fetch(`${url.replace(/\/$/, '')}/?${query}`, { method: 'POST', body: sql, headers: { Authorization: auth } });
  const text = await res.text();
  if (!res.ok) throw new Error(`ClickHouse ${res.status}: ${text.trim()}`);
  return text;
}

async function main(argv) {
  const arg = (name) => {
    const i = argv.indexOf(`--${name}`);
    return i >= 0 ? argv[i + 1] : undefined;
  };
  const window = parseWindow(arg('window') ?? '1h');
  const cluster = arg('cluster') ?? process.env.OC_CLICKHOUSE_CLUSTER ?? '';
  const url = process.env.OC_CLICKHOUSE_URL || 'http://localhost:8123';
  const user = process.env.OC_CLICKHOUSE_USER || 'observecraft';
  const password = process.env.OC_CLICKHOUSE_PASSWORD ?? 'observecraft';
  const auth = `Basic ${Buffer.from(`${user}:${password}`).toString('base64')}`;
  // The logs are flushed every few seconds; flush them so the window's last seconds count.
  await run(url, auth, cluster ? `SYSTEM FLUSH LOGS ON CLUSTER \`${clusterName(cluster)}\`` : 'SYSTEM FLUSH LOGS', {}).catch((e) => console.error(`warning: logs not flushed (${e.message})`));
  console.log(`Storage report: last ${window} s${cluster ? `, cluster ${cluster}` : ''}, ${url}\n`);
  const sql = readFileSync(new URL('./storage-report.sql', import.meta.url), 'utf8');
  for (const s of statements(sql)) {
    console.log(`== ${s.title}`);
    console.log(await run(url, auth, forCluster(s.sql, cluster), { param_window: String(window), default_format: 'PrettyCompactMonoBlock', output_format_pretty_row_numbers: '0' }));
  }
}

if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) {
  main(process.argv.slice(2)).catch((e) => {
    console.error(e.message);
    process.exit(1);
  });
}
