import { useMemo, useState } from 'react';
import { Link } from 'react-router-dom';
import { CheckCircle2, Globe, MonitorSmartphone, Plus, Search, Waypoints } from 'lucide-react';
import type { SyntheticEndpoint } from '../../api/synthetics/client';
import { TimePicker } from '../../components/shell/PageHeader';
import { useTime } from '../../components/shell/TimeContext';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { compact } from '../../lib/format';
import { useSyntheticCoverage } from './queries';

const minute = 60_000;
const pct = (covered: number, total: number) => (total ? Math.round((covered * 100) / total) : 0);

/**
 * Synthetic Monitoring & Testing → Test Coverage: the HTTP endpoints APM saw serving traffic and the
 * API tests that call them. Browser coverage needs Real User Monitoring, which ObserveCraft hasn't yet.
 */
export function TestCoveragePage() {
  const { range, relative } = useTime();
  const r = useMemo(() => ({ from: Math.floor(range.from / minute) * minute, to: Math.ceil(range.to / minute) * minute }), [range.from, range.to]);
  const q = useSyntheticCoverage({ range: r, env: null });
  const [filter, setFilter] = useState('');
  const [untested, setUntested] = useState(false);
  const endpoints = q.data?.endpoints ?? [];
  const covered = endpoints.filter((e) => e.tests.length > 0).length;
  const services = useMemo(() => {
    const m = new Map<string, { total: number; covered: number }>();
    for (const e of endpoints) {
      const s = m.get(e.service) ?? { total: 0, covered: 0 };
      s.total++;
      if (e.tests.length) s.covered++;
      m.set(e.service, s);
    }
    return [...m.entries()].sort((a, b) => b[1].total - a[1].total);
  }, [endpoints]);
  const shown = endpoints.filter((e) => (!untested || e.tests.length === 0) && `${e.service} ${e.method} ${e.route}`.toLowerCase().includes(filter.toLowerCase()));

  return (
    <div className="syn-page syn-coverage">
      <section className="syn-hero">
        <div className="syn-hero-text">
          <h2>Know what your tests cover</h2>
          <p>Coverage compares the traffic your applications serve with what your synthetic tests exercise, so you can see which endpoints and user journeys would break without a test noticing.</p>
          <ul className="syn-hero-list">
            <li>
              <CheckCircle2 size={16} className="syn-ok" />
              <span>
                <b>API endpoints</b>: endpoints seen in APM traces, matched to the method and URL path of your HTTP tests.
              </span>
            </li>
            <li>
              <MonitorSmartphone size={16} className="muted" />
              <span>
                <b>Browser views and actions</b>: needs Real User Monitoring and browser tests, which aren't available in ObserveCraft yet.
              </span>
            </li>
          </ul>
        </div>
        <div className="syn-hero-preview" aria-label="Preview of the coverage summary">
          <div className="syn-preview-head">
            <Waypoints size={15} /> API coverage
            <span className="syn-grow" />
            <b>{q.data ? `${pct(covered, endpoints.length)}%` : '—'}</b>
          </div>
          <div className="syn-meter">
            <i style={{ width: `${pct(covered, endpoints.length)}%` }} />
          </div>
          <div className="syn-preview-rows">
            {services.slice(0, 4).map(([name, s]) => (
              <div key={name}>
                <span className="syn-grow">{name}</span>
                <span className="muted">
                  {s.covered}/{s.total}
                </span>
              </div>
            ))}
            {services.length === 0 && <span className="muted">Endpoints appear here once APM receives traces.</span>}
          </div>
          <div className="syn-preview-head syn-preview-disabled">
            <Globe size={15} /> Browser coverage
            <span className="syn-grow" />
            <span className="syn-soon">needs RUM</span>
          </div>
        </div>
      </section>

      <section className="syn-card">
        <div className="syn-coverage-head">
          <h3 className="syn-h3">API endpoint coverage</h3>
          <span className="muted">Endpoints with traffic over the {relative.label.toLowerCase()}</span>
          <span className="syn-grow" />
          <TimePicker compact />
        </div>
        {q.isPending ? (
          <Loading label="Comparing APM endpoints with your tests…" />
        ) : q.isError ? (
          <ErrorState error={q.error} onRetry={() => q.refetch()} />
        ) : endpoints.length === 0 ? (
          <div className="syn-empty">
            <Waypoints size={36} strokeWidth={1.4} />
            <h3>No HTTP endpoints in APM for this time range</h3>
            <p className="muted">
              Coverage lists the endpoints of instrumented services (server spans with an <code>http.route</code>). <Link to="/apm">Instrument a service</Link> to see which of its endpoints your tests
              call.
            </p>
          </div>
        ) : (
          <>
            <div className="syn-coverage-stats">
              <div>
                <span className="muted">Endpoints with traffic</span>
                <b>{endpoints.length}</b>
              </div>
              <div>
                <span className="muted">Covered by a test</span>
                <b>
                  {covered} <span className="muted">({pct(covered, endpoints.length)}%)</span>
                </b>
              </div>
              <div>
                <span className="muted">HTTP tests</span>
                <b>{q.data.httpTests}</b>
              </div>
              <div className="syn-grow">
                <span className="muted">By service</span>
                <div className="syn-service-bars">
                  {services.map(([name, s]) => (
                    <span key={name} title={`${name}: ${s.covered} of ${s.total} endpoints covered`}>
                      {name}
                      <span className="syn-meter small">
                        <i style={{ width: `${pct(s.covered, s.total)}%` }} />
                      </span>
                    </span>
                  ))}
                </div>
              </div>
            </div>
            <div className="syn-list-top">
              <div className="mon-filter syn-grow">
                <Search size={16} />
                <input placeholder="Filter by service, method or route" value={filter} onChange={(e) => setFilter(e.target.value)} />
              </div>
              <label className="syn-check">
                <input type="checkbox" className="checkbox" checked={untested} onChange={(e) => setUntested(e.target.checked)} /> Untested endpoints only
              </label>
            </div>
            <EndpointsTable endpoints={shown} />
          </>
        )}
      </section>
    </div>
  );
}

function EndpointsTable({ endpoints }: { endpoints: SyntheticEndpoint[] }) {
  return (
    <div className="mon-table-wrap">
      <table className="mon-table syn-table syn-static">
        <thead>
          <tr>
            <th className="syn-col-wide">SERVICE</th>
            <th className="syn-col-type">METHOD</th>
            <th>ROUTE</th>
            <th className="syn-col-num syn-col-short">REQUESTS</th>
            <th className="syn-col-num syn-col-short">ERRORS</th>
            <th>COVERED BY</th>
          </tr>
        </thead>
        <tbody>
          {endpoints.map((e) => (
            <tr key={`${e.service} ${e.method} ${e.route}`}>
              <td className="syn-col-wide">{e.service}</td>
              <td className="syn-col-type mono">{e.method || '—'}</td>
              <td className="mono">{e.route}</td>
              <td className="syn-col-num syn-col-short">{compact(e.requests)}</td>
              <td className={`syn-col-num syn-col-short${e.errors > 0 ? ' syn-bad' : ''}`}>{compact(e.errors)}</td>
              <td>
                {e.tests.length ? (
                  <span className="syn-covered">
                    {e.tests.map((t) => (
                      <Link key={t.id} to={`/synthetics/tests/${t.id}`} className="syn-chip">
                        <CheckCircle2 size={12} className="syn-ok" /> {t.name}
                      </Link>
                    ))}
                  </span>
                ) : (
                  <span className="syn-untested">
                    Not tested
                    <Link to="/synthetics/create" className="link-btn">
                      <Plus size={13} /> New Test
                    </Link>
                  </span>
                )}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
      {endpoints.length === 0 && <p className="muted syn-pad">No endpoint matches this filter.</p>}
    </div>
  );
}
