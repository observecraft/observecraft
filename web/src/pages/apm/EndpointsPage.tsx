import { useState } from 'react';
import { Link } from 'react-router-dom';
import { Search } from 'lucide-react';
import { TimePicker } from '../../components/shell/PageHeader';
import { useTime } from '../../components/shell/TimeContext';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { Select } from '../../components/ui/Dropdown';
import { ALL_ENVS } from './ApmHome';
import { useEnvs, useResources } from './queries';
import { fmtLatency, fmtPct, fmtRate } from './visuals';
import { ServiceGlyph } from './explorer/SpanBits';
import './explorer/explorer.css';

type SortKey = 'requests' | 'errorRate' | 'p95Ms' | 'timeShare';

/** Every endpoint (service entry resource) across services. */
export function EndpointsPage() {
  const { range } = useTime();
  const [env, setEnv] = useState(ALL_ENVS);
  const [text, setText] = useState('');
  const [sort, setSort] = useState<SortKey>('requests');
  const envs = useEnvs(range);
  const q = useResources({ range, env: env === ALL_ENVS ? null : env, service: null });
  const f = text.trim().toLowerCase();
  const rows = (q.data ?? []).filter((r) => !f || `${r.service} ${r.resource}`.toLowerCase().includes(f)).sort((a, b) => b[sort] - a[sort]);
  return (
    <div className="endpoints-page">
      <div className="section-header">
        <div className="section-header-left">
          <Select label="Env" value={env} onChange={setEnv} options={[{ value: ALL_ENVS, label: 'All environments' }, ...(envs.data ?? []).map((e) => ({ value: e, label: e }))]} />
          <label className="tv-filter">
            <Search size={15} />
            <input placeholder="Filter endpoints or services" value={text} onChange={(e) => setText(e.target.value)} />
          </label>
        </div>
        <TimePicker />
      </div>
      {q.isPending ? (
        <Loading label="Loading endpoints…" />
      ) : q.isError ? (
        <ErrorState error={q.error} onRetry={() => q.refetch()} />
      ) : (
        <div className="sx-results endpoints-table">
          <table className="sx-table">
            <thead>
              <tr>
                <th>ENDPOINT</th>
                <th>SERVICE</th>
                {(
                  [
                    ['requests', 'REQUESTS'],
                    ['errorRate', 'ERROR RATE'],
                    ['p95Ms', 'P95 LATENCY'],
                    ['timeShare', '% TIME SPENT'],
                  ] as [SortKey, string][]
                ).map(([k, l]) => (
                  <th key={k} className="num">
                    <button className={`link-muted${sort === k ? ' strong' : ''}`} onClick={() => setSort(k)}>
                      {l}
                      {sort === k ? ' ↓' : ''}
                    </button>
                  </th>
                ))}
                <th className="num">P50</th>
                <th className="num">P99</th>
              </tr>
            </thead>
            <tbody>
              {rows.map((r) => (
                <tr key={`${r.service}|${r.resource}`}>
                  <td className="sx-resource">
                    <Link to={`/apm/traces?query=${encodeURIComponent(`service:${r.service} resource:"${r.resource}"`)}`}>{r.resource}</Link>
                  </td>
                  <td>
                    <Link to={`/apm/services/${encodeURIComponent(r.service)}`} className="sx-svc">
                      <ServiceGlyph service={r.service} size={16} /> {r.service}
                    </Link>
                  </td>
                  <td className="num">
                    {r.requests.toLocaleString()} <span className="muted">({fmtRate(r.reqPerSec)})</span>
                  </td>
                  <td className={`num${r.errorRate > 0.05 ? ' sx-err' : ''}`}>{fmtPct(r.errorRate)}</td>
                  <td className="num">{fmtLatency(r.p95Ms)}</td>
                  <td className="num">{fmtPct(r.timeShare)}</td>
                  <td className="num">{fmtLatency(r.p50Ms)}</td>
                  <td className="num">{fmtLatency(r.p99Ms)}</td>
                </tr>
              ))}
            </tbody>
          </table>
          {rows.length === 0 && <p className="mon-empty">No endpoints received requests in this timeframe.</p>}
        </div>
      )}
    </div>
  );
}
