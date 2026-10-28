import { useDeferredValue } from 'react';
import { useNavigate, useSearchParams } from 'react-router-dom';
import { Plus, Radar, Search } from 'lucide-react';
import type { SyntheticTest } from '../../api/synthetics/client';
import { fmtFrequency, fmtUptime } from '../../api/synthetics/logic';
import { TimePicker } from '../../components/shell/PageHeader';
import { useTime } from '../../components/shell/TimeContext';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { TagChips } from '../monitors/common';
import { TestStatus, TypeBadge } from './common';
import { useSyntheticTests } from './queries';
import '../monitors/monitors.css';
import './synthetics.css';

/** Digital Experience → Synthetic Tests: every API test of the org with its state and uptime over the selected time. */
export function SyntheticTestsPage() {
  const navigate = useNavigate();
  const [params, setParams] = useSearchParams();
  const query = params.get('q') ?? '';
  const deferred = useDeferredValue(query);
  const { range, relative } = useTime();
  // Uptime is computed per minute: keep the request stable while live mode slides the window.
  const minute = 60_000;
  const q = useSyntheticTests({ query: deferred, range: { from: Math.floor(range.from / minute) * minute, to: Math.floor(range.to / minute) * minute } });
  const setQuery = (v: string) => {
    const next = new URLSearchParams(params);
    if (v) next.set('q', v);
    else next.delete('q');
    setParams(next, { replace: true });
  };
  const tests = q.data?.tests ?? [];
  const count = (pred: (t: SyntheticTest) => boolean) => tests.filter(pred).length;

  return (
    <div className="mon-layout">
      <header className="product-header">
        <div className="product-title">
          <Radar size={20} />
          Synthetic Tests
        </div>
        <div className="mon-header-actions">
          <TimePicker />
          <button className="btn btn-primary" onClick={() => navigate('/synthetics/create')}>
            <Plus size={15} /> New Test
          </button>
        </div>
      </header>
      <div className="mon-section syn-list">
        <div className="mon-filter">
          <Search size={16} />
          <input placeholder="Search tests by name, domain or tag (type:http, env:prod, status:alert, service:storefront-api)" value={query} onChange={(e) => setQuery(e.target.value)} />
        </div>
        {q.data && (
          <div className="syn-counts">
            <span>
              <b>{tests.length}</b> test{tests.length === 1 ? '' : 's'}
            </span>
            <span className="c-alert">
              <b>{count((t) => !t.paused && t.status === 'alert')}</b> alerting
            </span>
            <span className="c-ok">
              <b>{count((t) => !t.paused && t.status === 'ok')}</b> OK
            </span>
            <span className="c-nodata">
              <b>{count((t) => !t.paused && t.status === 'nodata')}</b> no data
            </span>
            <span className="muted">
              <b>{count((t) => t.paused)}</b> paused
            </span>
            <span className="syn-counts-right muted">
              Running from {q.data.location.name} · uptime over the {relative.label.toLowerCase()}
            </span>
          </div>
        )}
        {q.isPending ? (
          <Loading label="Loading synthetic tests…" />
        ) : q.isError ? (
          <ErrorState error={q.error} onRetry={() => q.refetch()} />
        ) : tests.length === 0 ? (
          <div className="syn-empty">
            {deferred ? (
              <p>No tests match this search.</p>
            ) : (
              <>
                <Radar size={36} strokeWidth={1.4} />
                <h3>Check your endpoints from the ObserveCraft server</h3>
                <p className="muted">HTTP, SSL, DNS and TCP tests run on a schedule, alert through your notification channels and write synthetics.* metrics for dashboards and SLOs.</p>
                <button className="btn btn-primary" onClick={() => navigate('/synthetics/create')}>
                  <Plus size={15} /> New Test
                </button>
              </>
            )}
          </div>
        ) : (
          <div className="mon-table-wrap">
            <table className="mon-table syn-table">
              <thead>
                <tr>
                  <th className="syn-col-status">STATUS</th>
                  <th className="syn-col-type">TYPE</th>
                  <th>NAME</th>
                  <th>DOMAIN</th>
                  <th className="col-tags">TAGS</th>
                  <th className="syn-col-env">ENVS</th>
                  <th className="syn-col-freq">FREQUENCY</th>
                  <th className="syn-col-uptime">UPTIME</th>
                </tr>
              </thead>
              <tbody>
                {tests.map((t) => (
                  <tr key={t.id} onClick={() => navigate(`/synthetics/tests/${t.id}`)}>
                    <td className="syn-col-status">
                      <TestStatus status={t.status} paused={t.paused} small />
                    </td>
                    <td className="syn-col-type">
                      <TypeBadge type={t.type} />
                    </td>
                    <td className="mon-name">
                      <button className="link-btn syn-name" onClick={() => navigate(`/synthetics/tests/${t.id}`)}>
                        {t.name}
                      </button>
                      {t.muted && <span className="syn-muted-tag">muted</span>}
                    </td>
                    <td className="mono syn-domain">{t.domain}</td>
                    <td className="col-tags">
                      <TagChips tags={t.tags} max={2} />
                    </td>
                    <td className="syn-col-env">{t.env ? <span className="syn-chip">{t.env}</span> : <span className="muted">—</span>}</td>
                    <td className="syn-col-freq muted">{fmtFrequency(t.frequencySeconds)}</td>
                    <td className={`syn-col-uptime${t.uptime != null && t.uptime < 99 ? ' low' : ''}`}>{fmtUptime(t.uptime)}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </div>
    </div>
  );
}
