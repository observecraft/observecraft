import { useQueries } from '@tanstack/react-query';
import { Link } from 'react-router-dom';
import { listWorkflowRuns } from '../../../api/workflows/client';
import { TRIGGER_LABEL } from '../../../api/workflows/logic';
import type { WorkflowRun } from '../../../api/workflows/types';
import { useWorkflows, workflowKeys } from '../queries';
import { fmtAgo, fmtDuration, RunStatusChip } from './common';
import './workflows.css';

/** At most this many workflows have their runs read for the table. */
const DETAILED = 50;

function stats(runs: WorkflowRun[] | undefined) {
  const done = (runs ?? []).filter((r) => r.status === 'succeeded' || r.status === 'failed');
  const ok = done.filter((r) => r.status === 'succeeded').length;
  const durations = done.filter((r) => r.startedAt && r.finishedAt).map((r) => r.finishedAt! - r.startedAt!);
  return {
    runs: runs?.length ?? 0,
    succeeded: ok,
    failed: done.length - ok,
    rate: done.length ? Math.round((ok / done.length) * 100) : null,
    avg: durations.length ? durations.reduce((a, b) => a + b, 0) / durations.length : null,
    byTrigger: (runs ?? []).reduce<Record<string, number>>((m, r) => ({ ...m, [r.triggerKind]: (m[r.triggerKind] ?? 0) + 1 }), {}),
  };
}

/** Workflow Automation → Analytics: runs, success rate and duration per workflow (last 100 runs each). */
export function WorkflowsAnalytics() {
  const { data } = useWorkflows();
  const list = (data ?? []).slice(0, DETAILED);
  const runs = useQueries({ queries: list.map((w) => ({ queryKey: workflowKeys.runs(w.id), queryFn: ({ signal }: { signal: AbortSignal }) => listWorkflowRuns(w.id, signal), staleTime: 30_000 })) });
  const per = list.map((w, i) => ({ w, s: stats(runs[i]?.data) }));
  const total = per.reduce((a, x) => ({ runs: a.runs + x.s.runs, ok: a.ok + x.s.succeeded, failed: a.failed + x.s.failed }), { runs: 0, ok: 0, failed: 0 });
  const triggers = per.reduce<Record<string, number>>((m, x) => {
    for (const [k, n] of Object.entries(x.s.byTrigger)) m[k] = (m[k] ?? 0) + n;
    return m;
  }, {});
  const kpis: [string, string][] = [
    ['Workflows', String(data?.length ?? 0)],
    ['Published', String((data ?? []).filter((w) => w.published).length)],
    ['Runs', String(total.runs)],
    ['Success rate', total.ok + total.failed ? `${Math.round((total.ok / (total.ok + total.failed)) * 100)}%` : '—'],
    ['Failed runs', String(total.failed)],
  ];
  return (
    <div className="wfa-page">
      <div className="wfa-kpis">
        {kpis.map(([label, value]) => (
          <div key={label} className="wfa-kpi">
            <div className="wfa-kpi-label">{label}</div>
            <div className="wfa-kpi-value">{value}</div>
          </div>
        ))}
        <div className="wfa-kpi wfa-kpi-wide">
          <div className="wfa-kpi-label">Runs by trigger</div>
          <div className="wfa-bars">
            {Object.entries(triggers)
              .sort((a, b) => b[1] - a[1])
              .map(([k, n]) => (
                <div key={k} className="wfa-bar-row">
                  <span>{TRIGGER_LABEL[k as keyof typeof TRIGGER_LABEL] ?? k}</span>
                  <span className="wfa-bar">
                    <span style={{ width: `${(n / Math.max(1, total.runs)) * 100}%` }} />
                  </span>
                  <span className="wf-muted">{n}</span>
                </div>
              ))}
            {total.runs === 0 && <span className="wf-muted">No runs yet</span>}
          </div>
        </div>
      </div>
      <table className="wfl-table wfa-table">
        <thead>
          <tr>
            <th>Workflow</th>
            <th>Runs</th>
            <th>Succeeded</th>
            <th>Failed</th>
            <th>Success rate</th>
            <th>Avg duration</th>
            <th>Last execution</th>
          </tr>
        </thead>
        <tbody>
          {per.map(({ w, s }) => (
            <tr key={w.id}>
              <td>
                <Link to={`/automation/workflows/${w.id}`}>{w.name}</Link>
              </td>
              <td>{s.runs}</td>
              <td>{s.succeeded}</td>
              <td>{s.failed}</td>
              <td>
                {s.rate === null ? (
                  <span className="wf-muted">—</span>
                ) : (
                  <span className="wfa-rate">
                    <span className="wfa-bar">
                      <span style={{ width: `${s.rate}%` }} />
                    </span>
                    {s.rate}%
                  </span>
                )}
              </td>
              <td>{fmtDuration(s.avg)}</td>
              <td>
                {w.lastRun ? (
                  <span className="wfl-last">
                    <RunStatusChip status={w.lastRun.status} /> <span className="wf-muted">{fmtAgo(w.lastRun.createdAt)}</span>
                  </span>
                ) : (
                  <span className="wf-muted">Never run</span>
                )}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}
