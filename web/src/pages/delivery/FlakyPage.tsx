import { useDeferredValue, useState } from 'react';
import { Search } from 'lucide-react';
import type { FlakyState } from '../../api/ci/client';
import { TimePicker } from '../../components/shell/PageHeader';
import { useTime } from '../../components/shell/TimeContext';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { fmtAgo, fmtDur } from './common';
import { PipelineDrawer } from './PipelineDrawer';
import { useFlaky, useSetFlakyState } from './queries';

const STATES: { id: FlakyState; label: string; hint: string }[] = [
  { id: 'active', label: 'Active', hint: 'Flaky and still running normally' },
  {
    id: 'quarantined',
    label: 'Quarantined',
    hint: 'Still runs, but failures should not block the pipeline',
  },
  { id: 'disabled', label: 'Disabled', hint: 'Skipped until fixed' },
  { id: 'fixed', label: 'Fixed', hint: 'A fix landed; watch for new flakes' },
];

/** Tests that passed and failed on the same commit, with a state the team manages. */
export function FlakyPage() {
  const t = useTime();
  const [state, setState] = useState<FlakyState>('active');
  const [text, setText] = useState('');
  const query = useDeferredValue(text);
  const q = useFlaky(t.range, query, [state]);
  const update = useSetFlakyState();
  const [open, setOpen] = useState<string | null>(null);

  return (
    <div className="ci-flaky">
      <div className="ci-overview-head">
        <div>
          <h2>Flaky Management</h2>
          <p className="muted">A test is flaky when it both passed and failed on the same commit — usually a retry of the same pipeline.</p>
        </div>
        <TimePicker />
      </div>
      <div className="ci-state-tabs">
        {STATES.map((s) => (
          <button key={s.id} className={state === s.id ? 'active' : ''} onClick={() => setState(s.id)} title={s.hint}>
            {s.label} <span>{q.data?.counts[s.id] ?? 0}</span>
          </button>
        ))}
      </div>
      <div className="ci-search">
        <Search size={16} />
        <input placeholder="Filter by test, suite or repository" value={text} onChange={(e) => setText(e.target.value)} />
      </div>
      {q.isPending ? (
        <Loading label="Finding flaky tests…" />
      ) : q.isError ? (
        <ErrorState error={q.error} onRetry={() => q.refetch()} />
      ) : (
        <div className="ci-table-wrap">
          <table className="ci-table">
            <thead>
              <tr>
                <th>TEST</th>
                <th>REPOSITORY</th>
                <th>TEAM</th>
                <th className="num">FLAKY COMMITS</th>
                <th className="num">FAILURE RATE</th>
                <th className="num">RUNS</th>
                <th className="num">AVG DURATION</th>
                <th>FIRST FLAKED</th>
                <th>LAST FLAKED</th>
                <th>STATE</th>
              </tr>
            </thead>
            <tbody>
              {q.data.tests.map((f) => (
                <tr key={`${f.repository}|${f.suite}|${f.name}`} onClick={() => f.lastFailedTraceId && setOpen(f.lastFailedTraceId)} title="Open the last failed run">
                  <td>
                    <b>{f.name}</b>
                    <div className="muted">{f.suite}</div>
                  </td>
                  <td>{f.repository}</td>
                  <td>{f.team}</td>
                  <td className="num">{f.flakyCommits}</td>
                  <td className="num">
                    <span className={f.failureRate >= 15 ? 'ci-bad' : 'ci-warn'}>{f.failureRate.toFixed(1)}%</span>
                  </td>
                  <td className="num">{f.runs}</td>
                  <td className="num">{fmtDur(f.avgMs)}</td>
                  <td>{fmtAgo(f.firstFlake)}</td>
                  <td>{fmtAgo(f.lastFlake)}</td>
                  <td onClick={(e) => e.stopPropagation()}>
                    <select
                      value={f.state}
                      disabled={update.isPending}
                      onChange={(e) =>
                        update.mutate({
                          repository: f.repository,
                          suite: f.suite,
                          name: f.name,
                          state: e.target.value as FlakyState,
                        })
                      }
                      title={f.stateUpdatedBy ? `Last changed by ${f.stateUpdatedBy}` : undefined}
                      aria-label="State"
                    >
                      {STATES.map((s) => (
                        <option key={s.id} value={s.id}>
                          {s.label}
                        </option>
                      ))}
                    </select>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
          {q.data.tests.length === 0 && <p className="mon-empty">No {STATES.find((s) => s.id === state)?.label.toLowerCase()} flaky tests in this timeframe.</p>}
          {update.isError && <p className="form-error">{(update.error as Error).message}</p>}
        </div>
      )}
      {open && <PipelineDrawer traceId={open} onClose={() => setOpen(null)} />}
    </div>
  );
}
