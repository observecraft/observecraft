// Pure report/config helpers shared by the isolated benchmark and its tests.
export function percentile(values, fraction) {
  if (!values.length) return null;
  const sorted = [...values].sort((a, b) => a - b);
  return sorted[Math.max(0, Math.ceil(fraction * sorted.length) - 1)];
}
export function distribution(values) {
  return { samples: values.length, p50: percentile(values, 0.5), p95: percentile(values, 0.95), p99: percentile(values, 0.99), max: values.length ? values.reduce((a, b) => Math.max(a, b)) : null };
}
export function memoryBytes(value) {
  const match = /^([\d.]+)\s*(B|kB|KB|MB|GB|KiB|MiB|GiB)$/.exec(value.trim());
  if (!match) throw new Error(`Unknown Docker memory unit: ${value}`);
  return Number(match[1]) * { B: 1, kB: 1e3, KB: 1e3, MB: 1e6, GB: 1e9, KiB: 1024, MiB: 1024 ** 2, GiB: 1024 ** 3 }[match[2]];
}
export function kafkaOffsets(output, topic = 'oc.logs.json', freshTopic = false) {
  return output
    .split('\n')
    .flatMap((line) => {
      const f = line.trim().split(/\s+/);
      if (f[0] !== 'oc-vector' || f[1] !== topic || !/^\d+$/.test(f[2]) || !/^\d+$/.test(f[4])) return [];
      const committed = /^\d+$/.test(f[3]) ? Number(f[3]) : null;
      const end = Number(f[4]);
      return [{ partition: Number(f[2]), committed, end, lag: committed === null ? (freshTopic || end === 0 ? end : null) : end - committed }];
    })
    .sort((a, b) => a.partition - b.partition);
}
export function totalLag(offsets) {
  return !offsets.length || offsets.some((p) => p.lag === null) ? null : offsets.reduce((n, p) => n + p.lag, 0);
}
export function configFrom(env) {
  const number = (key, fallback, min, max) => {
    const value = Number(env[key] ?? fallback);
    if (!Number.isInteger(value) || value < min || value > max) throw new Error(`${key} must be an integer between ${min} and ${max}`);
    return value;
  };
  const rates = (env.OC_BENCH_RATES ?? '1000,5000').split(',').map(Number);
  if (!rates.length || rates.length > 4 || rates.some((n) => !Number.isInteger(n) || n < 100 || n > 100000) || new Set(rates).size !== rates.length)
    throw new Error('OC_BENCH_RATES: choose 1–4 distinct integer rates between 100 and 100000 logs/s');
  const cfg = {
    seconds: number('OC_BENCH_SECONDS', 15, 5, 300),
    rates,
    orgs: number('OC_BENCH_ORGS', 4, 1, 16),
    replicas: number('OC_BENCH_API_REPLICAS', 2, 1, 4),
    batch: number('OC_BENCH_BATCH', 500, 10, 2000),
    concurrency: number('OC_BENCH_CONCURRENCY', 16, 1, 128),
    messageBytes: number('OC_BENCH_MESSAGE_BYTES', 128, 32, 4096),
    delayMs: number('OC_BENCH_STORAGE_DELAY_MS', 250, 1, 2000),
    image: env.OC_BENCH_IMAGE ?? 'observecraft:local',
    output: env.OC_BENCH_OUTPUT,
    mode: env.OC_BENCH_MODE ?? 'all',
  };
  if (!['all', 'throughput', 'faults'].includes(cfg.mode)) throw new Error('OC_BENCH_MODE must be all, throughput or faults');
  if ((Math.max(...rates) * cfg.seconds) / cfg.batch > 100000) throw new Error('Reduce duration/rate or increase batch size: at most 100000 requests per phase');
  return cfg;
}
export function phases(cfg) {
  const highest = Math.max(...cfg.rates);
  const throughput = [
    ...cfg.rates.flatMap((rate) => [
      { name: `unlimited-${rate}`, rate, quota: false },
      { name: `quota-${rate}`, rate, quota: true },
    ]),
    { name: `metrics-${highest}`, rate: highest, quota: true, metrics: true },
  ];
  const faults = [
    { name: `slow-${highest}`, rate: highest, quota: true, delayMs: cfg.delayMs },
    { name: `paused-${highest}`, rate: highest, quota: true, holdSeconds: Math.min(5, cfg.seconds / 2) },
  ];
  return cfg.mode === 'throughput' ? throughput : cfg.mode === 'faults' ? faults : [...throughput, ...faults];
}
