// Number and label formatting for LLM Observability.

export function fmtTokens(n: number): string {
  if (n >= 1e9) return `${(n / 1e9).toFixed(1)}B`;
  if (n >= 1e6) return `${(n / 1e6).toFixed(1)}M`;
  if (n >= 1e4) return `${(n / 1e3).toFixed(1)}K`;
  return Math.round(n).toLocaleString('en-US');
}

/** Estimated USD; "<$0.01" below a cent, "—" without a price. */
export function fmtCost(usd: number | null | undefined): string {
  if (usd === null || usd === undefined) return '—';
  if (usd === 0) return '$0.00';
  if (usd < 0.01) return '<$0.01';
  return `$${usd.toLocaleString('en-US', { minimumFractionDigits: 2, maximumFractionDigits: 2 })}`;
}

/** A small per-unit cost (per request) with three significant digits. */
export function fmtUnitCost(usd: number): string {
  return usd === 0 ? '$0' : `$${Number(usd.toPrecision(3))}`;
}

export function fmtPercent(ratio: number): string {
  return `${(ratio * 100).toFixed(2)}%`;
}

const KIND_LABELS: Record<string, string> = { llm: 'LLM', embedding: 'Embedding', retrieval: 'Retrieval', tool: 'Tool', agent: 'Agent', workflow: 'Workflow', task: 'Task' };

export function kindLabel(kind: string): string {
  return KIND_LABELS[kind] ?? (kind ? kind[0].toUpperCase() + kind.slice(1) : 'Other');
}
