import { useMemo } from 'react';
import { Link, useNavigate } from 'react-router-dom';
import { ArrowRight, Plus, Radar } from 'lucide-react';
import { fmtUptime } from '../../api/synthetics/logic';
import { TimePicker } from '../../components/shell/PageHeader';
import { useTime } from '../../components/shell/TimeContext';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { timeAgo } from '../../lib/format';
import { StatusBadge } from '../monitors/common';
import { PassFail, TestStatus, TypeBadge } from './common';
import { useSyntheticResultSearch, useSyntheticSuites, useSyntheticTests } from './queries';

const minute = 60_000;

/** Synthetic Monitoring & Testing → Overview: the state of the org's tests and suites, and what failed lately. */
export function SyntheticsOverviewPage() {
  const navigate = useNavigate();
  const { range, relative } = useTime();
  const r = useMemo(() => ({ from: Math.floor(range.from / minute) * minute, to: Math.floor(range.to / minute) * minute }), [range.from, range.to]);
  const tests = useSyntheticTests({ query: '', range: r });
  const suites = useSyntheticSuites({ query: '', range: r });
  const failures = useSyntheticResultSearch({ query: 'status:failed', range: r, offset: 0, limit: 8 });

  if (tests.isPending) return <Loading label="Loading synthetic tests…" />;
  if (tests.isError) return <ErrorState error={tests.error} onRetry={() => tests.refetch()} />;
  const all = tests.data.tests;
  if (all.length === 0) {
    return (
      <div className="syn-page">
        <div className="syn-empty">
          <Radar size={36} strokeWidth={1.4} />
          <h3>Check your endpoints from the ObserveCraft server</h3>
          <p className="muted">HTTP, SSL, DNS and TCP tests run on a schedule, alert through your notification channels and write synthetics.* metrics for dashboards and SLOs.</p>
          <button className="btn btn-primary" onClick={() => navigate('/synthetics/create')}>
            <Plus size={15} /> New Test
          </button>
        </div>
      </div>
    );
  }
  const active = all.filter((t) => !t.paused);
  const by = (s: string) => active.filter((t) => t.status === s).length;
  const uptimes = active.map((t) => t.uptime).filter((u): u is number => u != null);
  const avg = uptimes.length ? uptimes.reduce((a, b) => a + b, 0) / uptimes.length : null;
  const failing = active.filter((t) => t.status === 'alert');

  return (
    <div className="syn-page">
      <div className="syn-list-top">
        <span className="muted syn-grow">State now · uptime over the {relative.label.toLowerCase()}</span>
        <TimePicker compact />
      </div>
      <div className="syn-overview-tiles">
        <Link to="/synthetics/tests?q=status:alert" className="syn-tile alert">
          <span>ALERT</span>
          <b>{by('alert')}</b> tests
        </Link>
        <Link to="/synthetics/tests?q=status:ok" className="syn-tile ok">
          <span>OK</span>
          <b>{by('ok')}</b> tests
        </Link>
        <Link to="/synthetics/tests?q=status:nodata" className="syn-tile nodata">
          <span>NO DATA</span>
          <b>{by('nodata')}</b> tests
        </Link>
        <Link to="/synthetics/tests?q=status:paused" className="syn-tile">
          <span>PAUSED</span>
          <b>{all.length - active.length}</b> tests
        </Link>
        <div className="syn-tile">
          <span>AVERAGE UPTIME</span>
          <b>{fmtUptime(avg)}</b>
        </div>
        <Link to="/synthetics/suites" className="syn-tile">
          <span>SUITES</span>
          <b>{suites.data?.suites.length ?? '—'}</b> · {suites.data?.suites.filter((s) => s.status === 'alert').length ?? 0} alerting
        </Link>
      </div>
      <div className="syn-overview-grid">
        <section className="syn-card">
          <h3 className="syn-h3">
            Failing tests
            <Link to="/synthetics/tests?q=status:alert" className="link-btn syn-h3-link">
              All tests <ArrowRight size={13} />
            </Link>
          </h3>
          {failing.length === 0 ? (
            <p className="muted">Every running test passes.</p>
          ) : (
            <table className="mon-table syn-table">
              <tbody>
                {failing.map((t) => (
                  <tr key={t.id} onClick={() => navigate(`/synthetics/tests/${t.id}`)}>
                    <td className="syn-col-status">
                      <TestStatus status={t.status} paused={t.paused} small />
                    </td>
                    <td className="syn-col-type">
                      <TypeBadge type={t.type} />
                    </td>
                    <td className="mon-name">{t.name}</td>
                    <td className="muted">{t.failingSince ? `failing since ${timeAgo(t.failingSince)}` : ''}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
        </section>
        <section className="syn-card">
          <h3 className="syn-h3">
            Recent failed runs
            <Link to="/synthetics/explorer?q=status:failed" className="link-btn syn-h3-link">
              Results Explorer <ArrowRight size={13} />
            </Link>
          </h3>
          {failures.isPending ? (
            <Loading label="Loading runs…" />
          ) : failures.isError ? (
            <ErrorState error={failures.error} />
          ) : failures.data.results.length === 0 ? (
            <p className="muted">No failed run in this time range.</p>
          ) : (
            <table className="mon-table syn-table">
              <tbody>
                {failures.data.results.map((x) => (
                  <tr key={`${x.testId}-${x.id}`} onClick={() => navigate(`/synthetics/tests/${x.testId}`)}>
                    <td className="syn-col-icon">
                      <PassFail passed={x.passed} />
                    </td>
                    <td className="mon-name">
                      <span className="syn-name">{x.testName}</span>
                      <span className="muted syn-sub">{x.error ?? (x.failedAssertions ? `${x.failedAssertions} assertion${x.failedAssertions === 1 ? '' : 's'} failed` : 'Failed')}</span>
                    </td>
                    <td className="muted syn-col-date">{timeAgo(x.startedAt)}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
        </section>
        <section className="syn-card">
          <h3 className="syn-h3">
            Suites
            <Link to="/synthetics/suites" className="link-btn syn-h3-link">
              Test Suites <ArrowRight size={13} />
            </Link>
          </h3>
          {(suites.data?.suites ?? []).length === 0 ? (
            <p className="muted">
              No suite yet. <Link to="/synthetics/suites/new">Group tests into a suite</Link> to follow a journey together.
            </p>
          ) : (
            <table className="mon-table syn-table">
              <tbody>
                {suites.data!.suites.slice(0, 6).map((s) => (
                  <tr key={s.id} onClick={() => navigate(`/synthetics/suites/${s.id}`)}>
                    <td className="syn-col-status">
                      <StatusBadge status={s.status} small />
                    </td>
                    <td className="mon-name">{s.name}</td>
                    <td className="muted">{s.tests.length} tests</td>
                    <td className="syn-col-uptime">{fmtUptime(s.uptime)}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
        </section>
      </div>
    </div>
  );
}
