import { useQuery } from '@tanstack/react-query';
import { Loader2, Play, Table2 } from 'lucide-react';
import type { AppComponent, AppSpec, AppValues } from '../../../api/apps/types';
import { queryScalar } from '../../../api/dashboards/client';
import type { TimeAggregator } from '../../../api/dashboards/types';
import { useWorkflowRun } from '../queries';
import { cellText, layoutRows, tableRows } from './appLogic';

/** The latest run each button started: button id → its workflow and run. */
export type ButtonRuns = Record<string, { workflowId: string; runId: number }>;

interface Props {
  spec: AppSpec;
  values: AppValues;
  onValue: (id: string, v: string | number) => void;
  onPress: (button: AppComponent) => void;
  runs: ButtonRuns;
  /** Buttons waiting for the server to accept a run. */
  pressing?: string | null;
  /** Builder: the selected component, and clicks select instead of acting. */
  selected?: string | null;
  onSelect?: (id: string) => void;
}

/** An app's components on its 12-column grid (the builder's preview and the running app). */
export function AppRenderer({ spec, values, onValue, onPress, runs, pressing, selected, onSelect }: Props) {
  const rows = layoutRows(spec.components);
  if (rows.length === 0) return <div className="ab-canvas-empty">Add components from the left panel to start building.</div>;
  return (
    <div className="ab-grid">
      {rows.map((row, i) => (
        <div className="ab-row" key={i}>
          {row.map((c) => (
            <div
              key={c.id}
              className={`ab-cell${onSelect ? ' ab-cell-edit' : ''}${selected === c.id ? ' ab-cell-selected' : ''}`}
              style={{ gridColumn: `span ${c.width}` }}
              onClickCapture={onSelect ? () => onSelect(c.id) : undefined}
            >
              {onSelect && <span className="ab-cell-tag">{c.id}</span>}
              <ComponentView c={c} values={values} onValue={onValue} onPress={onPress} runs={runs} pressing={pressing} editing={!!onSelect} />
            </div>
          ))}
        </div>
      ))}
    </div>
  );
}

function ComponentView({
  c,
  values,
  onValue,
  onPress,
  runs,
  pressing,
  editing,
}: {
  c: AppComponent;
  values: AppValues;
  onValue: Props['onValue'];
  onPress: Props['onPress'];
  runs: ButtonRuns;
  pressing?: string | null;
  editing: boolean;
}) {
  switch (c.kind) {
    case 'heading':
      return <h2 className="ab-heading">{c.text}</h2>;
    case 'text':
      return <p className="ab-text">{c.text}</p>;
    case 'text_input':
      return (
        <label className="ab-field">
          <span className="ab-label">{c.label || c.id}</span>
          <input className="ab-input" value={String(values[c.id] ?? '')} placeholder={c.placeholder} onChange={(e) => onValue(c.id, e.target.value)} />
        </label>
      );
    case 'number_input':
      return (
        <label className="ab-field">
          <span className="ab-label">{c.label || c.id}</span>
          <input
            className="ab-input"
            type="number"
            value={String(values[c.id] ?? '')}
            placeholder={c.placeholder}
            onChange={(e) => onValue(c.id, e.target.value === '' ? '' : Number(e.target.value))}
          />
        </label>
      );
    case 'select':
      return (
        <label className="ab-field">
          <span className="ab-label">{c.label || c.id}</span>
          <select className="ab-input" value={String(values[c.id] ?? '')} onChange={(e) => onValue(c.id, e.target.value)}>
            {c.options.map((o) => (
              <option key={o}>{o}</option>
            ))}
          </select>
        </label>
      );
    case 'button':
      return <ButtonView c={c} onPress={onPress} run={runs[c.id]} pressing={pressing === c.id} editing={editing} />;
    case 'table':
      return <TableView c={c} run={runs[c.source]} />;
    case 'metric':
      return <MetricView c={c} />;
  }
}

function RunChip({ run }: { run: ButtonRuns[string] | undefined }) {
  const r = useWorkflowRun(run?.workflowId, run?.runId);
  if (!run) return null;
  const status = r.data?.status ?? 'queued';
  const busy = status === 'queued' || status === 'running' || status === 'waiting';
  return (
    <span className={`ab-run-chip ab-run-${status}`} title={r.data?.error ?? undefined}>
      {busy && <Loader2 size={12} className="ab-spin" />}
      Run #{run.runId} · {status}
    </span>
  );
}

function ButtonView({ c, onPress, run, pressing, editing }: { c: AppComponent; onPress: Props['onPress']; run: ButtonRuns[string] | undefined; pressing: boolean; editing: boolean }) {
  const r = useWorkflowRun(run?.workflowId, run?.runId);
  const busy = pressing || (!!r.data && ['queued', 'running', 'waiting'].includes(r.data.status));
  return (
    <div className="ab-button-wrap">
      <button className="ab-button" disabled={busy && !editing} onClick={() => onPress(c)}>
        {busy ? <Loader2 size={14} className="ab-spin" /> : <Play size={13} />}
        {c.label || 'Run'}
      </button>
      <RunChip run={run} />
      {r.data?.status === 'failed' && r.data.error && <span className="ab-run-error">{r.data.error}</span>}
    </div>
  );
}

function TableView({ c, run }: { c: AppComponent; run: ButtonRuns[string] | undefined }) {
  const r = useWorkflowRun(run?.workflowId, run?.runId);
  const { columns, rows } = tableRows(r.data, c.path, c.columns);
  return (
    <div className="ab-table-card">
      <div className="ab-card-title">{c.label || c.id}</div>
      {rows.length === 0 ? (
        <div className="ab-table-empty">
          <Table2 size={18} />
          {!run
            ? `Press “${c.source || 'a button'}” to fill this table.`
            : r.data && ['queued', 'running', 'waiting'].includes(r.data.status)
              ? 'Waiting for the run…'
              : 'The run returned no rows at this path.'}
        </div>
      ) : (
        <div className="ab-table-scroll">
          <table className="ab-table">
            <thead>
              <tr>
                {columns.map((col) => (
                  <th key={col}>{col}</th>
                ))}
              </tr>
            </thead>
            <tbody>
              {rows.map((row, i) => (
                <tr key={i}>
                  {columns.map((col) => (
                    <td key={col}>{cellText(row[col])}</td>
                  ))}
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
    </div>
  );
}

const fmt = (v: number) => (Math.abs(v) >= 1000 ? v.toLocaleString(undefined, { maximumFractionDigits: 0 }) : v.toLocaleString(undefined, { maximumFractionDigits: 2 }));

function MetricView({ c }: { c: AppComponent }) {
  const aggregator = (c.aggregation || 'avg') as TimeAggregator;
  const q = useQuery({
    queryKey: ['automation-app-metric', c.query, aggregator],
    queryFn: () => {
      const to = Date.now();
      return queryScalar({ query: c.query, aggregator, range: { from: to - 15 * 60_000, to } });
    },
    enabled: !!c.query.trim(),
    refetchInterval: 60_000,
    retry: 1,
  });
  return (
    <div className="ab-metric">
      <div className="ab-card-title">{c.label || c.query}</div>
      <div className="ab-metric-value" title={q.isError && q.error instanceof Error ? q.error.message : undefined}>
        {q.isLoading ? '…' : q.isError ? '—' : q.data === undefined || q.data === null || Number.isNaN(q.data) ? 'No data' : fmt(q.data)}
        {c.unit && q.data !== undefined && !q.isError && <span className="ab-metric-unit">{c.unit}</span>}
      </div>
      <div className="ab-metric-sub">{aggregator} · past 15 minutes</div>
    </div>
  );
}
