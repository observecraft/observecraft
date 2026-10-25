import { useEffect, useMemo, useState } from 'react';
import { useNavigate, useSearchParams } from 'react-router-dom';
import { ArrowDown, ArrowUp, Bug, ChevronRight, Lightbulb, Plus, Search, Sparkles, Star } from 'lucide-react';
import type { Anomaly, Recommendation, ResourceStats, ServiceStats, SpanErrorGroup } from '../../api/apm/client';
import { TimePicker } from '../../components/shell/PageHeader';
import { useTime } from '../../components/shell/TimeContext';
import { Empty, ErrorState, Loading } from '../../components/ui/AsyncState';
import { Select } from '../../components/ui/Dropdown';
import { compact, timeAgo } from '../../lib/format';
import { InstrumentModal } from './InstrumentModal';
import { useAnomalies, useEnvs, useRecommendations, useResources, useServices, useSpanErrors } from './queries';
import { errorTone, fmtLatency, fmtPct, fmtRate, ServiceIcon, useStars } from './visuals';

type Attention = 'recommendations' | 'anomalies' | 'errors';
type RecType = 'all' | Recommendation['kind'];

const REC_TITLE: Record<Recommendation['kind'], string> = {
  repeated_sequential_calls: 'Repeated Sequential Calls',
  high_error_rate: 'High Error Rate',
  slow_endpoint: 'Slow Endpoint',
};

export const ALL_ENVS = '*';

export function servicePath(service: string, env: string, resource?: string): string {
  const p = new URLSearchParams();
  if (env !== ALL_ENVS) p.set('env', env);
  if (resource) p.set('resource', resource);
  const qs = p.toString();
  return `/apm/services/${encodeURIComponent(service)}${qs ? `?${qs}` : ''}`;
}

export function ApmHome() {
  const { range } = useTime();
  const [env, setEnv] = useState(ALL_ENVS);
  const [instrumenting, setInstrumenting] = useState(false);
  const envs = useEnvs(range);
  // Deep links from the APM menu: ?instrument=1 opens the setup guide, ?view=recommendations scrolls to them.
  const [params, setParams] = useSearchParams();
  const want = params.get('instrument') === '1' ? 'instrument' : params.get('view');
  useEffect(() => {
    if (!want) return;
    if (want === 'instrument') setInstrumenting(true);
    if (want === 'recommendations') document.getElementById('apm-attention')?.scrollIntoView({ behavior: 'smooth' });
    setParams({}, { replace: true });
  }, [want, setParams]);
  const scope = { range, env: env === ALL_ENVS ? null : env };

  return (
    <div className="apm-home">
      <div className="apm-hero">
        <h1>Welcome to APM</h1>
        <div className="apm-hero-actions">
          <Select
            label="Env"
            value={env}
            onChange={setEnv}
            options={[{ value: ALL_ENVS, label: 'All environments' }, ...(envs.data ?? []).map((e) => ({ value: e, label: e }))]}
          />
          <TimePicker compact />
          <button className="btn btn-primary" onClick={() => setInstrumenting(true)}>
            <Plus size={14} /> Instrument a Service
          </button>
        </div>
      </div>
      <AttentionSection scope={scope} env={env} />
      <GlanceSection scope={scope} env={env} />
      {instrumenting && <InstrumentModal onClose={() => setInstrumenting(false)} />}
    </div>
  );
}

type Scope = { range: { from: number; to: number }; env: string | null };

function AttentionSection({ scope, env }: { scope: Scope; env: string }) {
  const navigate = useNavigate();
  const [tab, setTab] = useState<Attention>('recommendations');
  const [type, setType] = useState<RecType>('all');
  const recs = useRecommendations(scope);
  const anomalies = useAnomalies(scope);
  const errors = useSpanErrors(scope);

  const shownRecs = (recs.data ?? []).filter((r) => type === 'all' || r.kind === type);
  const pills: { id: Attention; label: string; icon: typeof Lightbulb; count?: number }[] = [
    { id: 'recommendations', label: 'Recommendations', icon: Lightbulb, count: recs.data?.length },
    { id: 'anomalies', label: 'Anomaly Insights', icon: Sparkles, count: anomalies.data?.length },
    { id: 'errors', label: 'Error Tracking', icon: Bug, count: errors.data?.length },
  ];
  const active = tab === 'recommendations' ? recs : tab === 'anomalies' ? anomalies : errors;

  return (
    <section className="apm-block" id="apm-attention">
      <h2>What needs your attention</h2>
      <div className="apm-attn-bar">
        <div className="apm-pills">
          {pills.map((p) => (
            <button key={p.id} className={`apm-pill${tab === p.id ? ' active' : ''}`} onClick={() => setTab(p.id)}>
              <p.icon size={14} /> {p.label}
              {p.count !== undefined && <span className="apm-pill-count">{p.count}</span>}
            </button>
          ))}
        </div>
        {tab === 'recommendations' && (
          <Select<RecType>
            label="Type"
            value={type}
            onChange={setType}
            options={[{ value: 'all', label: 'All types' }, ...(Object.keys(REC_TITLE) as Recommendation['kind'][]).map((k) => ({ value: k, label: REC_TITLE[k] }))]}
          />
        )}
      </div>
      {active.isPending ? (
        <Loading />
      ) : active.isError ? (
        <ErrorState error={active.error} onRetry={() => active.refetch()} />
      ) : tab === 'recommendations' ? (
        shownRecs.length === 0 ? (
          <Empty>No recommendations for this time range. Services look healthy.</Empty>
        ) : (
          <div className="apm-cards">
            {shownRecs.slice(0, 3).map((r) => (
              <RecommendationCard key={r.id} rec={r} onOpen={() => navigate(servicePath(r.service, env))} />
            ))}
            <ReviewCard recs={shownRecs} onOpen={(r) => navigate(servicePath(r.service, env))} />
          </div>
        )
      ) : tab === 'anomalies' ? (
        (anomalies.data ?? []).length === 0 ? (
          <Empty>No anomalies: latency and error rates match the previous period.</Empty>
        ) : (
          <div className="apm-cards">
            {anomalies.data!.slice(0, 4).map((a) => (
              <AnomalyCard key={`${a.service}:${a.resource}:${a.metric}`} anomaly={a} onOpen={() => navigate(servicePath(a.service, env, a.resource))} />
            ))}
          </div>
        )
      ) : (errors.data ?? []).length === 0 ? (
        <Empty>No span errors in this time range.</Empty>
      ) : (
        <div className="apm-cards">
          {errors.data!.slice(0, 4).map((e) => (
            <ErrorCard key={`${e.service}:${e.resource}:${e.errorType}:${e.message}`} group={e} onOpen={() => navigate(servicePath(e.service, env))} />
          ))}
        </div>
      )}
    </section>
  );
}

function recDetail(r: Recommendation) {
  switch (r.kind) {
    case 'repeated_sequential_calls':
      return (
        <>
          <b>{r.service}</b> makes {Math.round(r.value)} identical <code>{r.resource}</code> calls in one request. Batch them into a single call.
        </>
      );
    case 'high_error_rate':
      return (
        <>
          {fmtPct(r.value)} of {compact(r.requests)} requests to <code>{r.resource}</code> on <b>{r.service}</b> failed.
        </>
      );
    case 'slow_endpoint':
      return (
        <>
          <code>{r.resource}</code> on <b>{r.service}</b> has a p95 latency of {fmtLatency(r.value)}.
        </>
      );
  }
}

function RecommendationCard({ rec, onOpen }: { rec: Recommendation; onOpen: () => void }) {
  return (
    <button className="apm-card" onClick={onOpen}>
      <span className={`prio prio-${rec.priority}`}>{rec.priority.toUpperCase()}</span>
      <h3>
        {REC_TITLE[rec.kind]} on <span className="mono">{rec.resource}</span>
      </h3>
      <p>{recDetail(rec)}</p>
      <footer>
        Detected {compact(rec.occurrences)} times <ChevronRight size={14} />
      </footer>
    </button>
  );
}

function ReviewCard({ recs, onOpen }: { recs: Recommendation[]; onOpen: (r: Recommendation) => void }) {
  return (
    <div className="apm-card apm-review">
      <h3>Review {recs.length} recommendation{recs.length === 1 ? '' : 's'}</h3>
      <table className="apm-mini">
        <tbody>
          {recs.slice(0, 5).map((r) => (
            <tr key={r.id} onClick={() => onOpen(r)}>
              <td>
                <span className={`prio prio-${r.priority}`}>{r.priority.toUpperCase()}</span>
              </td>
              <td className="ellipsis" title={`${REC_TITLE[r.kind]} · ${r.resource}`}>
                {REC_TITLE[r.kind]}
              </td>
              <td className="ellipsis muted">{r.service}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

function AnomalyCard({ anomaly: a, onOpen }: { anomaly: Anomaly; onOpen: () => void }) {
  const latency = a.metric === 'p95_latency_ms';
  const fmt = latency ? fmtLatency : fmtPct;
  return (
    <button className="apm-card" onClick={onOpen}>
      <span className="prio prio-medium">{latency ? 'LATENCY' : 'ERRORS'}</span>
      <h3>
        {latency ? 'p95 latency' : 'Error rate'} rose on <span className="mono">{a.resource}</span>
      </h3>
      <p>
        <b>{a.service}</b>: {fmt(a.previous)} → <b className="tone-error">{fmt(a.current)}</b> versus the previous period, over {compact(a.requests)} requests.
      </p>
      <footer>
        Investigate <ChevronRight size={14} />
      </footer>
    </button>
  );
}

function ErrorCard({ group: e, onOpen }: { group: SpanErrorGroup; onOpen: () => void }) {
  return (
    <button className="apm-card" onClick={onOpen}>
      <span className="prio prio-high">{compact(e.count)} ERRORS</span>
      <h3 className="mono">{e.errorType || 'Error'}</h3>
      <p className="ellipsis-2">{e.message || '(no message)'}</p>
      <footer>
        {e.service} · {e.resource} · last seen {timeAgo(e.lastSeen)}
      </footer>
    </button>
  );
}

type SortKey = 'service' | 'lastDeploy' | 'reqPerSec' | 'errorRate' | 'p95Ms';

function GlanceSection({ scope, env }: { scope: Scope; env: string }) {
  const [tab, setTab] = useState<'services' | 'endpoints'>('services');
  const [filter, setFilter] = useState('');
  return (
    <section className="apm-block">
      <h2>Your system at a glance</h2>
      <div className="apm-glance-bar">
        <div className="apm-seg">
          <button className={tab === 'services' ? 'active' : ''} onClick={() => setTab('services')}>
            Services
          </button>
          <button className={tab === 'endpoints' ? 'active' : ''} onClick={() => setTab('endpoints')}>
            Endpoints
          </button>
        </div>
        <label className="apm-search">
          <Search size={14} />
          <input placeholder={tab === 'services' ? 'Filter services' : 'Filter endpoints'} value={filter} onChange={(e) => setFilter(e.target.value)} />
        </label>
      </div>
      {tab === 'services' ? <ServiceList scope={scope} env={env} filter={filter} /> : <EndpointList scope={scope} env={env} filter={filter} />}
    </section>
  );
}

function SortTh({ k, sort, setSort, children, className }: { k: SortKey; sort: [SortKey, 1 | -1]; setSort: (s: [SortKey, 1 | -1]) => void; children: React.ReactNode; className?: string }) {
  const on = sort[0] === k;
  return (
    <th className={`sortable ${className ?? ''}`} onClick={() => setSort([k, on ? (-sort[1] as 1 | -1) : k === 'service' ? 1 : -1])}>
      {children} {on && (sort[1] === 1 ? <ArrowUp size={12} /> : <ArrowDown size={12} />)}
    </th>
  );
}

function ServiceList({ scope, env, filter }: { scope: Scope; env: string; filter: string }) {
  const navigate = useNavigate();
  const q = useServices(scope);
  const { stars, toggle } = useStars();
  const [sort, setSort] = useState<[SortKey, 1 | -1]>(['reqPerSec', -1]);

  const rows = useMemo(() => {
    const f = filter.trim().toLowerCase();
    const list = (q.data ?? []).filter((s) => !f || s.service.toLowerCase().includes(f));
    const val = (s: ServiceStats) => (sort[0] === 'service' ? s.service : (s[sort[0]] ?? -1));
    // Starred services pin to the top, then the chosen sort.
    return list.sort((a, b) => {
      const st = Number(stars.has(b.service)) - Number(stars.has(a.service));
      if (st) return st;
      const va = val(a);
      const vb = val(b);
      return (va < vb ? -1 : va > vb ? 1 : 0) * sort[1];
    });
  }, [q.data, filter, sort, stars]);

  if (q.isPending) return <Loading label="Loading services…" />;
  if (q.isError) return <ErrorState error={q.error} onRetry={() => q.refetch()} />;
  if (rows.length === 0) return <Empty>{filter ? 'No services match this filter.' : 'No traces received in this time range. Instrument a service to get started.'}</Empty>;

  return (
    <table className="apm-table">
      <thead>
        <tr>
          <th className="col-star" />
          <SortTh k="service" sort={sort} setSort={setSort}>
            SERVICE
          </SortTh>
          <SortTh k="lastDeploy" sort={sort} setSort={setSort}>
            LAST DEPLOY
          </SortTh>
          <SortTh k="reqPerSec" sort={sort} setSort={setSort} className="num">
            REQUESTS
          </SortTh>
          <SortTh k="errorRate" sort={sort} setSort={setSort} className="num">
            ERROR RATE
          </SortTh>
          <SortTh k="p95Ms" sort={sort} setSort={setSort} className="num">
            P95 LATENCY
          </SortTh>
        </tr>
      </thead>
      <tbody>
        {rows.map((s) => (
          <tr key={s.service} onClick={() => navigate(servicePath(s.service, env))}>
            <td className="col-star">
              <button
                className={`star${stars.has(s.service) ? ' on' : ''}`}
                title={stars.has(s.service) ? 'Unstar' : 'Star'}
                onClick={(e) => {
                  e.stopPropagation();
                  toggle(s.service);
                }}
              >
                <Star size={14} />
              </button>
            </td>
            <td>
              <span className="svc-name">
                <ServiceIcon name={s.service} kind={s.kind} language={s.language} />
                {s.service}
                {!s.instrumented && <span className="svc-inferred">inferred</span>}
              </span>
            </td>
            <td className="muted">{s.lastDeploy ? `${timeAgo(s.lastDeploy)}${s.version ? ` · ${s.version}` : ''}` : '—'}</td>
            <td className="num">{fmtRate(s.reqPerSec)}</td>
            <td className={`num ${errorTone(s.errorRate)}`}>{fmtPct(s.errorRate)}</td>
            <td className="num">{fmtLatency(s.p95Ms)}</td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}

function EndpointList({ scope, env, filter }: { scope: Scope; env: string; filter: string }) {
  const navigate = useNavigate();
  const q = useResources(scope);
  const rows = useMemo(() => {
    const f = filter.trim().toLowerCase();
    return (q.data ?? []).filter((r: ResourceStats) => !f || r.resource.toLowerCase().includes(f) || r.service.toLowerCase().includes(f)).slice(0, 200);
  }, [q.data, filter]);

  if (q.isPending) return <Loading label="Loading endpoints…" />;
  if (q.isError) return <ErrorState error={q.error} onRetry={() => q.refetch()} />;
  if (rows.length === 0) return <Empty>No endpoints match.</Empty>;

  return (
    <table className="apm-table">
      <thead>
        <tr>
          <th>ENDPOINT</th>
          <th>SERVICE</th>
          <th className="num">REQUESTS</th>
          <th className="num">ERROR RATE</th>
          <th className="num">P95 LATENCY</th>
        </tr>
      </thead>
      <tbody>
        {rows.map((r) => (
          <tr key={`${r.service}:${r.resource}`} onClick={() => navigate(servicePath(r.service, env, r.resource))}>
            <td className="mono">{r.resource}</td>
            <td>{r.service}</td>
            <td className="num">{fmtRate(r.reqPerSec)}</td>
            <td className={`num ${errorTone(r.errorRate)}`}>{fmtPct(r.errorRate)}</td>
            <td className="num">{fmtLatency(r.p95Ms)}</td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}
