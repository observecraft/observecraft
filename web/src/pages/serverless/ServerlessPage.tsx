import { useEffect, useMemo, useState, type ReactNode } from 'react';
import { keepPreviousData, useQuery } from '@tanstack/react-query';
import type { EChartsOption } from 'echarts';
import { Link, useNavigate, useSearchParams } from 'react-router-dom';
import { AlertTriangle, ArrowUpCircle, Check, ChevronDown, ChevronRight, Clock, ExternalLink, Lightbulb, MoreHorizontal, Settings, X } from 'lucide-react';
import { searchLogPage } from '../../api/client';
import { functionInvocations, searchFunctions, serverlessTimeseries, type LambdaFunction, type LambdaInsightKind, type ServerlessSearchPage, type ServerlessSeries } from '../../api/serverless/client';
import type { Facet } from '../../api/types';
import { BrandLogo } from '../../components/brand/BrandLogo';
import { Chart } from '../../components/charts/Chart';
import { FacetPanel } from '../../components/facets/FacetPanel';
import { QueryBar } from '../../components/query/QueryBar';
import { TimePicker } from '../../components/shell/PageHeader';
import { useTime } from '../../components/shell/TimeContext';
import { Empty, ErrorState, Loading } from '../../components/ui/AsyncState';
import { Menu, Select } from '../../components/ui/Dropdown';
import { Modal } from '../../components/ui/Modal';
import { UsageBar } from '../../components/ui/Pager';
import { formatLongDate, hhmm, timeAgo } from '../../lib/format';
import { addTerm, formatQuery, parseQuery, type Term } from '../../lib/searchQuery';
import { LogTable } from '../logs/LogTable';
import { SetupOptions } from '../setup/SetupOptions';
import {
  INSIGHT_INFO,
  NO_SOURCE_FILTERS,
  applySourceFilters,
  formatCost,
  formatCount,
  formatInsightValue,
  formatMemory,
  formatMs,
  groupByService,
  insightCards,
  type Presence,
  type SourceFilters,
} from './serverlessModel';
import { SERVERLESS_SETUP } from './serverlessSetup';
import './serverless.css';

type View = 'lambda' | 'step_functions' | 'fargate';
type Provider = 'aws' | 'azure' | 'gcp';

const VIEWS: { value: View; label: string }[] = [
  { value: 'lambda', label: 'Lambda' },
  { value: 'step_functions', label: 'Step Functions' },
  { value: 'fargate', label: 'Fargate' },
];

const PROVIDERS: { value: Provider; label: string; logo: string }[] = [
  { value: 'aws', label: 'Amazon Web Services', logo: 'aws' },
  { value: 'azure', label: 'Microsoft Azure', logo: 'azure' },
  { value: 'gcp', label: 'Google Cloud', logo: 'googlecloud' },
];

const FACETS: { key: string; label: string; group: string }[] = [
  { key: 'functionname', label: 'Function Name', group: 'CORE' },
  { key: 'insight', label: 'Insights', group: 'CORE' },
  { key: 'region', label: 'Region', group: 'CORE' },
  { key: 'env', label: 'Environment', group: 'CORE' },
  { key: 'stage', label: 'Stage', group: 'CORE' },
  { key: 'service', label: 'Service', group: 'CORE' },
  { key: 'runtime', label: 'Runtime', group: 'CORE' },
  { key: 'memorysize', label: 'Memory', group: 'CORE' },
  { key: 'aws_account', label: 'AWS Account', group: 'AWS' },
  { key: 'architecture', label: 'Architecture', group: 'AWS' },
];

const CHARTS: { metric: string; title: string; fmt: (v: number) => string }[] = [
  { metric: 'invocations', title: 'Invocations', fmt: formatCount },
  { metric: 'errors', title: 'Errors', fmt: formatCount },
  { metric: 'duration', title: 'Duration (p95)', fmt: formatMs },
  { metric: 'cold_starts', title: 'Cold starts', fmt: formatCount },
];

type Column = 'runtime' | 'invocations' | 'errors' | 'duration' | 'p95' | 'cold' | 'memory' | 'cost' | 'deploy';

const COLUMNS: { key: Column; label: string }[] = [
  { key: 'runtime', label: 'Runtime' },
  { key: 'invocations', label: 'Invocations' },
  { key: 'errors', label: 'Errors' },
  { key: 'duration', label: 'Duration (avg)' },
  { key: 'p95', label: 'Duration (p95)' },
  { key: 'cold', label: 'Cold starts' },
  { key: 'memory', label: 'Memory used / allocated' },
  { key: 'cost', label: 'Estimated cost' },
  { key: 'deploy', label: 'Last deploy' },
];

const UPGRADE_LABELS = { runtime: 'Runtime', datadog_lambda: 'Datadog Lambda library', extension: 'Datadog extension' } as const;

/** URL state of the page: query, view, provider, upgrade toggle, team and the open function. */
function useServerlessParams() {
  const [params, setParams] = useSearchParams();
  const set = (patch: Record<string, string | null>) =>
    setParams(
      (p) => {
        const n = new URLSearchParams(p);
        for (const [k, v] of Object.entries(patch)) {
          if (v) n.set(k, v);
          else n.delete(k);
        }
        return n;
      },
      { replace: true },
    );
  const query = params.get('q') ?? '';
  const team = params.get('team') ?? '';
  return {
    query,
    team,
    full: team ? addTerm(query, 'team', team) : query,
    view: (params.get('view') as View | null) ?? 'lambda',
    provider: (params.get('provider') as Provider | null) ?? 'aws',
    upgrade: params.get('upgrade') === '1',
    selected: params.get('fn'),
    set,
  };
}

/** Infrastructure → Serverless: the organization's AWS Lambda functions, their insights and their telemetry. */
export function ServerlessPage() {
  const p = useServerlessParams();
  const { range } = useTime();
  const [setup, setSetup] = useState(false);
  const page = useQuery({
    queryKey: ['serverless', 'functions', range, p.full, p.upgrade],
    queryFn: () => searchFunctions({ range, query: p.full, upgradeAvailable: p.upgrade }),
    placeholderData: keepPreviousData,
    enabled: p.view === 'lambda' && p.provider === 'aws',
  });
  const provider = PROVIDERS.find((x) => x.value === p.provider) ?? PROVIDERS[0];
  const selected = page.data?.functions.find((f) => f.id === p.selected) ?? null;
  // Nothing reports yet: only the view list and the two ways to start, no search or facets.
  const noFunctions = page.data?.enabled === false;
  return (
    <div className="sls-page">
      <header className="product-header">
        <div className="product-title">
          <span className="sls-fx" aria-hidden>
            ƒx
          </span>
          Serverless
        </div>
        <div className="sls-provider">
          <Select
            value={p.provider}
            options={PROVIDERS.map((x) => ({ value: x.value, label: x.label, icon: <BrandLogo name={x.logo} size={16} /> }))}
            onChange={(v) => p.set({ provider: v === 'aws' ? null : v })}
            trigger={() => (
              <span className="sls-provider-pill">
                <BrandLogo name={provider.logo} size={16} /> {provider.label} <ChevronDown size={14} />
              </span>
            )}
            triggerClassName="sls-provider-trigger"
            title="Cloud provider"
          />
        </div>
        <div className="sls-header-actions">
          <button className="btn" onClick={() => setSetup(true)}>
            <Settings size={14} /> Settings
          </button>
        </div>
      </header>
      <div className="section-header sls-views">
        <div className="section-header-left">
          <span className="sls-views-label">Views</span>
          <div className="page-tab active">My View</div>
          <Menu
            align="left"
            triggerClassName="icon-btn sls-views-more"
            trigger={<MoreHorizontal size={16} />}
            items={[
              { label: 'Reset view', onClick: () => p.set({ q: null, team: null, upgrade: null, view: null, provider: null, fn: null }) },
              { label: 'Setup and settings', onClick: () => setSetup(true) },
            ]}
          />
        </div>
        <TimePicker compact />
      </div>
      {!noFunctions && (
        <div className="sls-search">
          <QueryBar value={p.query} onChange={(q) => p.set({ q: q || null })} placeholder="Search for any tag on any function" />
        </div>
      )}
      <div className="sls-body">
        {noFunctions ? (
          <div className="sls-side">
            <ViewList params={p} />
          </div>
        ) : (
          <SidePanel page={page.data} params={p} />
        )}
        <main className="sls-main">
          {p.provider !== 'aws' ? (
            <NotAvailable title={`${provider.label} serverless functions are not available yet`}>
              Serverless covers AWS Lambda for now. Functions of other clouds already show up in APM and Logs when their traces and logs reach ObserveCraft.
            </NotAvailable>
          ) : p.view === 'step_functions' ? (
            <NotAvailable title="Step Functions are not available yet">
              State machine executions (<code>aws.states.*</code> metrics and Step Functions traces) are not read yet. The Lambda functions a state machine calls are listed under Lambda.
            </NotAvailable>
          ) : p.view === 'fargate' ? (
            <NotAvailable title="Fargate is not available here yet">
              ECS tasks on Fargate are listed in the <Link to="/orchestration/explorer?resource=ecs_tasks">ECS Explorer</Link>, with their containers and metrics from the Datadog Agent.
            </NotAvailable>
          ) : page.isError ? (
            <ErrorState error={page.error} onRetry={() => page.refetch()} />
          ) : !page.data ? (
            <Loading label="Loading functions…" />
          ) : !page.data.enabled ? (
            <ServerlessEmpty onSetup={() => setSetup(true)} />
          ) : (
            <LambdaView page={page.data} onOpen={(id) => p.set({ fn: id })} onQuery={(q) => p.set({ q: q || null })} query={p.query} />
          )}
        </main>
      </div>
      {selected && <FunctionPanel fn={selected} onClose={() => p.set({ fn: null })} />}
      {setup && (
        <Modal label="Serverless setup" onClose={() => setSetup(false)} width={880} height="auto">
          <div className="sls-setup">
            <header>
              <h2>Send AWS Lambda telemetry to ObserveCraft</h2>
              <button className="icon-btn" onClick={() => setSetup(false)} title="Close">
                <X size={16} />
              </button>
            </header>
            <SetupOptions options={SERVERLESS_SETUP} />
          </div>
        </Modal>
      )}
    </div>
  );
}

function NotAvailable({ title, children }: { title: string; children: ReactNode }) {
  return (
    <div className="sls-unavailable">
      <h3>{title}</h3>
      <p className="muted">{children}</p>
    </div>
  );
}

/** No functions yet: instrument your own, or look around with the demo generator's functions. */
function ServerlessEmpty({ onSetup }: { onSetup: () => void }) {
  return (
    <div className="sls-empty">
      <section>
        <span className="sls-fx sls-fx-large" aria-hidden>
          ƒx
        </span>
        <h2>Monitor my own functions</h2>
        <p className="sls-empty-time">
          <Clock size={14} /> About 15 minutes
        </p>
        <p>Point the Datadog Lambda extension, the AWS Lambda metrics or OpenTelemetry at ObserveCraft and every function shows up here with its invocations, errors, cold starts and cost.</p>
        <button className="btn btn-primary" onClick={onSetup}>
          Set up Serverless
        </button>
      </section>
      <section>
        <span className="sls-fx sls-fx-large" aria-hidden>
          ƒx
        </span>
        <h2>Try it with demo functions</h2>
        <p className="sls-empty-time">
          <Clock size={14} /> About 2 minutes
        </p>
        <p>The demo generator sends five Lambda functions of a fictional shop, traced and with enhanced metrics. Run it next to the server:</p>
        <code className="sls-empty-cmd">make tracegen</code>
      </section>
    </div>
  );
}

// ---------------------------------------------------------------- left column

function ViewList({ params }: { params: ReturnType<typeof useServerlessParams> }) {
  return (
    <>
      <div className="sls-side-title">Select a view</div>
      <ul className="sls-view-list">
        {VIEWS.map((v) => (
          <li key={v.value}>
            <button className={params.view === v.value ? 'active' : ''} onClick={() => params.set({ view: v.value === 'lambda' ? null : v.value, fn: null })}>
              {v.label}
            </button>
          </li>
        ))}
      </ul>
    </>
  );
}

function SidePanel({ page, params }: { page: ServerlessSearchPage | undefined; params: ReturnType<typeof useServerlessParams> }) {
  const [teamsOpen, setTeamsOpen] = useState(true);
  const terms = useMemo(() => parseQuery(params.query), [params.query]);
  const teams = page?.facets.find((f) => f.key === 'team')?.values ?? [];
  const facets: Facet[] = FACETS.flatMap((d) => {
    const f = page?.facets.find((x) => x.key === d.key);
    return f ? [{ key: d.key, label: d.label, group: d.group, values: f.values }] : [];
  });
  const header = (
    <>
      <ViewList params={params} />
      <label className="sls-side-card">
        <span>
          <ArrowUpCircle size={14} /> Upgrade Available
        </span>
        <button role="switch" aria-checked={params.upgrade} className={`switch${params.upgrade ? ' on' : ''}`} onClick={() => params.set({ upgrade: params.upgrade ? null : '1' })} />
      </label>
      <div className="sls-side-card">
        <button className="sls-side-toggle" onClick={() => setTeamsOpen((o) => !o)}>
          {teamsOpen ? <ChevronDown size={14} /> : <ChevronRight size={14} />} Teams
        </button>
      </div>
      {teamsOpen && (
        <ul className="sls-team-list">
          {teams.length === 0 && <li className="muted">No team tags on these functions.</li>}
          {teams.map((t) => (
            <li key={t.value}>
              <label>
                <input type="radio" name="sls-team" checked={params.team === t.value} onChange={() => params.set({ team: t.value })} /> {t.value}
                <span className="facet-count">{t.count}</span>
              </label>
            </li>
          ))}
          {params.team && (
            <li>
              <button className="link-btn" onClick={() => params.set({ team: null })}>
                All teams
              </button>
            </li>
          )}
        </ul>
      )}
      <div className="sls-side-title">Filter facets</div>
    </>
  );
  return (
    <div className="sls-side">
      <FacetPanel
        facets={facets}
        terms={terms}
        onChange={(next: Term[]) => params.set({ q: formatQuery(next) || null })}
        valueSearch
        defaultOpen={['functionname', 'insight', 'region', 'env', 'runtime']}
        verified={[]}
        header={header}
        valueLabel={(key, value) => (key === 'insight' ? (INSIGHT_INFO[value as LambdaInsightKind]?.title ?? value) : key === 'memorysize' ? formatMemory(Number(value)) : value)}
      />
    </div>
  );
}

// ---------------------------------------------------------------- Lambda view

function Collapsible({ title, count, children }: { title: string; count?: number; children: ReactNode }) {
  const [open, setOpen] = useState(true);
  return (
    <section className="sls-section">
      <button className="sls-section-toggle" onClick={() => setOpen((o) => !o)}>
        {open ? <ChevronDown size={16} /> : <ChevronRight size={16} />} {title}
        {count != null && <span className="sls-count">{count}</span>}
      </button>
      {open && <div className="sls-section-body">{children}</div>}
    </section>
  );
}

function LambdaView({ page, query, onQuery, onOpen }: { page: ServerlessSearchPage; query: string; onQuery: (q: string) => void; onOpen: (id: string) => void }) {
  const [filters, setFilters] = useState<SourceFilters>(NO_SOURCE_FILTERS);
  const [hidden, setHidden] = useState<Set<Column>>(new Set());
  const functions = useMemo(() => applySourceFilters(page.functions, filters), [page.functions, filters]);
  const cards = useMemo(() => insightCards(functions), [functions]);
  const charted = useMemo(
    () =>
      [...functions]
        .sort((a, b) => (b.invocations ?? 0) - (a.invocations ?? 0))
        .slice(0, 10)
        .map((f) => f.id),
    [functions],
  );
  const presence = (label: string, key: keyof SourceFilters) => (
    <Select<Presence>
      label={label}
      value={filters[key]}
      options={[
        { value: 'any', label: 'Any' },
        { value: 'with', label: `With ${label.toLowerCase()}` },
        { value: 'without', label: `Without ${label.toLowerCase()}` },
      ]}
      onChange={(v) => setFilters((f) => ({ ...f, [key]: v }))}
      triggerClassName="dd-trigger sls-filter"
    />
  );
  return (
    <>
      <div className="sls-totals">
        <Total label="Functions" value={page.summary.functions.toLocaleString()} />
        <Total label="Invocations" value={formatCount(page.summary.invocations)} />
        <Total label="Errors" value={formatCount(page.summary.errors)} tone={page.summary.errors ? 'error' : undefined} />
        <Total label="Cold starts" value={formatCount(page.summary.coldStarts)} />
        <Total label="Estimated cost" value={formatCost(page.summary.costUsd)} />
        <Total label="Upgrades available" value={page.summary.upgradable.toLocaleString()} />
      </div>
      <Collapsible title="Issues & Insights" count={cards.length}>
        {cards.length === 0 ? (
          <p className="muted">No issues: no function has high errors, cold starts, timeouts, out-of-memory errors, throttles or memory to adjust in this range.</p>
        ) : (
          <div className="sls-cards">
            {cards.map((c) => {
              const info = INSIGHT_INFO[c.kind];
              return (
                <button key={`${c.functionId}-${c.kind}`} className={`sls-card sls-card-${info.severity}`} onClick={() => onOpen(c.functionId)}>
                  <span className="sls-card-title">
                    {info.severity === 'info' ? <Lightbulb size={14} /> : <AlertTriangle size={14} />} {info.title}
                  </span>
                  <strong className="sls-card-fn">{c.functionName}</strong>
                  <span className="sls-card-metric">
                    {info.metric} <b>{formatInsightValue(c.kind, c.value)}</b>
                  </span>
                </button>
              );
            })}
          </div>
        )}
      </Collapsible>
      <Collapsible title="Summary Graphs">
        <SummaryGraphs ids={charted} names={Object.fromEntries(functions.map((f) => [f.id, f.name]))} />
      </Collapsible>
      <div className="sls-toolbar">
        <span className="muted">Filter groups with:</span>
        {presence('Enhanced Metrics', 'enhanced')}
        {presence('Traces', 'traces')}
        {presence('Logs', 'logs')}
        <span className="sls-spacer" />
        <Menu
          triggerClassName="btn"
          trigger={<>Table Options</>}
          items={COLUMNS.map((c) => ({
            label: (
              <span className="sls-col-option">
                {hidden.has(c.key) ? <span className="sls-col-check" /> : <Check size={14} />} {c.label}
              </span>
            ),
            onClick: () =>
              setHidden((h) => {
                const n = new Set(h);
                if (n.has(c.key)) n.delete(c.key);
                else n.add(c.key);
                return n;
              }),
          }))}
        />
      </div>
      <FunctionTable functions={functions} hidden={hidden} onOpen={onOpen} onTag={(k, v) => onQuery(addTerm(query, k, v))} />
    </>
  );
}

function Total({ label, value, tone }: { label: string; value: string; tone?: 'error' }) {
  return (
    <div className={`sls-total${tone ? ` sls-total-${tone}` : ''}`}>
      <span className="muted">{label}</span>
      <strong>{value}</strong>
    </div>
  );
}

function SeriesChart({ title, series, fmt, names, height = 170 }: { title: string; series: ServerlessSeries[]; fmt: (v: number) => string; names: Record<string, string>; height?: number }) {
  const option = useMemo<EChartsOption>(
    () => ({
      grid: { left: 56, right: 12, top: 12, bottom: series.length > 1 ? 44 : 24 },
      tooltip: { trigger: 'axis', valueFormatter: (v) => (v == null ? '—' : fmt(Number(v))) },
      legend: series.length > 1 ? { type: 'scroll', bottom: 0, textStyle: { fontSize: 11 } } : undefined,
      xAxis: { type: 'time', splitNumber: 4, axisLabel: { formatter: (v: number) => hhmm(v), hideOverlap: true } },
      // Counts have whole ticks.
      yAxis: { type: 'value', splitNumber: 3, minInterval: fmt === formatMs ? undefined : 1, axisLabel: { formatter: (v: number) => fmt(v) } },
      series: series.map((s) => ({
        name: names[s.function] ?? s.function,
        type: 'line',
        showSymbol: false,
        lineStyle: { width: 1.4 },
        data: s.points,
        areaStyle: series.length === 1 ? { opacity: 0.12 } : undefined,
      })),
    }),
    [series, fmt, names],
  );
  return (
    <div className="sls-chart">
      <h4>{title}</h4>
      {series.every((s) => s.points.length === 0) ? <div className="sls-chart-empty muted">No data in this range</div> : <Chart option={option} height={height} />}
    </div>
  );
}

function useSeries(ids: string[]) {
  const { range } = useTime();
  return useQuery({ queryKey: ['serverless', 'series', range, ids], queryFn: () => serverlessTimeseries({ range, functions: ids }), placeholderData: keepPreviousData, enabled: ids.length > 0 });
}

function SummaryGraphs({ ids, names }: { ids: string[]; names: Record<string, string> }) {
  const q = useSeries(ids);
  if (ids.length === 0) return <p className="muted">No functions to chart.</p>;
  if (q.isError) return <ErrorState error={q.error} onRetry={() => q.refetch()} />;
  if (!q.data) return <Loading label="Loading graphs…" />;
  return (
    <>
      <p className="muted small">The {ids.length === 1 ? 'function' : `${ids.length} busiest functions`} of the list.</p>
      <div className="sls-charts">
        {CHARTS.map((c) => (
          <SeriesChart key={c.metric} title={c.title} fmt={c.fmt} names={names} series={q.data.filter((s) => s.metric === c.metric)} />
        ))}
      </div>
    </>
  );
}

function InsightBadges({ fn }: { fn: LambdaFunction }) {
  return (
    <span className="sls-badges">
      {fn.insights.map((i) => (
        <span key={i.kind} className={`sls-badge sls-badge-${INSIGHT_INFO[i.kind].severity}`} title={`${INSIGHT_INFO[i.kind].metric}: ${formatInsightValue(i.kind, i.value)}`}>
          {INSIGHT_INFO[i.kind].title}
        </span>
      ))}
      {fn.upgrades.length > 0 && (
        <span className="sls-badge sls-badge-upgrade" title={fn.upgrades.map((u) => `${UPGRADE_LABELS[u.component]} ${u.current} → ${u.latest}`).join('\n')}>
          <ArrowUpCircle size={11} /> Upgrade
        </span>
      )}
    </span>
  );
}

function FunctionTable({ functions, hidden, onOpen, onTag }: { functions: LambdaFunction[]; hidden: Set<Column>; onOpen: (id: string) => void; onTag: (key: string, value: string) => void }) {
  const groups = useMemo(() => groupByService(functions), [functions]);
  const [closed, setClosed] = useState<Set<string>>(new Set());
  const show = (c: Column) => !hidden.has(c);
  const cols = 1 + COLUMNS.filter((c) => show(c.key)).length;
  return (
    <div className="sls-table-wrap">
      <table className="sls-table">
        <thead>
          <tr>
            <th>FUNCTION</th>
            {show('runtime') && <th>RUNTIME</th>}
            {show('invocations') && <th className="num">INVOCATIONS</th>}
            {show('errors') && <th className="num">ERRORS</th>}
            {show('duration') && <th className="num">DURATION AVG</th>}
            {show('p95') && <th className="num">DURATION P95</th>}
            {show('cold') && <th className="num">COLD STARTS</th>}
            {show('memory') && <th className="num">MEMORY USED / ALLOC.</th>}
            {show('cost') && <th className="num">EST. COST</th>}
            {show('deploy') && <th>LAST DEPLOY</th>}
          </tr>
        </thead>
        {groups.map((g) => {
          const open = !closed.has(g.service);
          return (
            <tbody key={g.service}>
              <tr className="sls-group">
                <td colSpan={cols}>
                  <button
                    className="sls-group-toggle"
                    onClick={() =>
                      setClosed((s) => {
                        const n = new Set(s);
                        if (n.has(g.service)) n.delete(g.service);
                        else n.add(g.service);
                        return n;
                      })
                    }
                  >
                    {open ? <ChevronDown size={14} /> : <ChevronRight size={14} />}
                    <span className="muted">service:</span> <strong>{g.service}</strong>
                  </button>
                  <span className="muted sls-group-stats">
                    {g.functions.length} {g.functions.length === 1 ? 'function' : 'functions'} · {formatCount(g.invocations)} invocations · {formatCount(g.errors)} errors · {formatCost(g.costUsd)}
                  </span>
                </td>
              </tr>
              {open &&
                g.functions.map((f) => (
                  <tr key={f.id} className="sls-row" onClick={() => onOpen(f.id)}>
                    <td>
                      <div className="sls-fn">
                        <BrandLogo name="awslambda" size={16} />
                        <span className="sls-fn-name">{f.name}</span>
                        <span className="muted sls-fn-region">{f.region}</span>
                        <InsightBadges fn={f} />
                      </div>
                    </td>
                    {show('runtime') && (
                      <td>
                        <button
                          className="link-btn"
                          onClick={(e) => {
                            e.stopPropagation();
                            if (f.runtime) onTag('runtime', f.runtime);
                          }}
                        >
                          {f.runtime ?? '—'}
                        </button>
                      </td>
                    )}
                    {show('invocations') && <td className="num">{formatCount(f.invocations)}</td>}
                    {show('errors') && (
                      <td className={`num${(f.errorRatePct ?? 0) >= 5 ? ' sls-bad' : ''}`}>
                        {formatCount(f.errors)}
                        {f.errorRatePct != null && f.errors ? <span className="muted"> ({f.errorRatePct.toFixed(1)} %)</span> : null}
                      </td>
                    )}
                    {show('duration') && <td className="num">{formatMs(f.durationAvgMs)}</td>}
                    {show('p95') && <td className="num">{formatMs(f.durationP95Ms)}</td>}
                    {show('cold') && (
                      <td className="num">
                        {formatCount(f.coldStarts)}
                        {f.coldStartPct != null && f.coldStarts ? <span className="muted"> ({f.coldStartPct.toFixed(1)} %)</span> : null}
                      </td>
                    )}
                    {show('memory') && (
                      <td className="num">
                        {f.memoryUsedPct != null && <UsageBar pct={Math.min(100, f.memoryUsedPct)} tone={f.memoryUsedPct >= 90 ? 'primary' : 'ok'} />} {formatMemory(f.memoryUsedMb)}
                        <span className="muted"> / {formatMemory(f.memoryMb)}</span>
                      </td>
                    )}
                    {show('cost') && <td className="num">{formatCost(f.costUsd)}</td>}
                    {show('deploy') && (
                      <td title={f.lastDeploy ? formatLongDate(f.lastDeploy) : 'No new version in this range'}>
                        {f.lastDeploy ? timeAgo(f.lastDeploy) : <span className="muted">{f.version ? `v${f.version}` : '—'}</span>}
                      </td>
                    )}
                  </tr>
                ))}
            </tbody>
          );
        })}
        {groups.length === 0 && (
          <tbody>
            <tr>
              <td colSpan={cols} className="sls-table-empty">
                No functions match these filters.
              </td>
            </tr>
          </tbody>
        )}
      </table>
    </div>
  );
}

// ---------------------------------------------------------------- side panel

type PanelTab = 'overview' | 'invocations' | 'logs' | 'configuration';

function FunctionPanel({ fn, onClose }: { fn: LambdaFunction; onClose: () => void }) {
  const [tab, setTab] = useState<PanelTab>('overview');
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === 'Escape' && onClose();
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [onClose]);
  const tabs: { id: PanelTab; label: string }[] = [
    { id: 'overview', label: 'Overview' },
    { id: 'invocations', label: 'Invocations' },
    { id: 'logs', label: 'Logs' },
    { id: 'configuration', label: 'Configuration' },
  ];
  return (
    <div className="sls-drawer-backdrop" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <aside className="sls-drawer" role="dialog" aria-label={`Function ${fn.name}`}>
        <header className="sls-drawer-head">
          <BrandLogo name="awslambda" size={22} />
          <div>
            <h2>{fn.name}</h2>
            <span className="muted">{[fn.region, fn.runtime, fn.memoryMb ? formatMemory(fn.memoryMb) : null, fn.service ? `service:${fn.service}` : null].filter(Boolean).join(' · ')}</span>
          </div>
          <button className="icon-btn" onClick={onClose} title="Close">
            <X size={16} />
          </button>
        </header>
        <div className="sls-drawer-stats">
          <Total label="Invocations" value={formatCount(fn.invocations)} />
          <Total label="Errors" value={formatCount(fn.errors)} tone={fn.errors ? 'error' : undefined} />
          <Total label="Duration p95" value={formatMs(fn.durationP95Ms ?? fn.durationAvgMs)} />
          <Total label="Cold starts" value={formatCount(fn.coldStarts)} />
          <Total label="Estimated cost" value={formatCost(fn.costUsd)} />
        </div>
        <nav className="sls-drawer-tabs">
          {tabs.map((t) => (
            <button key={t.id} className={tab === t.id ? 'active' : ''} onClick={() => setTab(t.id)}>
              {t.label}
            </button>
          ))}
        </nav>
        <div className="sls-drawer-body">
          {tab === 'overview' && <PanelOverview fn={fn} />}
          {tab === 'invocations' && <PanelInvocations fn={fn} />}
          {tab === 'logs' && <PanelLogs fn={fn} />}
          {tab === 'configuration' && <PanelConfiguration fn={fn} />}
        </div>
      </aside>
    </div>
  );
}

function PanelOverview({ fn }: { fn: LambdaFunction }) {
  const q = useSeries([fn.id]);
  const names = { [fn.id]: fn.name };
  return (
    <>
      {(fn.insights.length > 0 || fn.upgrades.length > 0) && (
        <div className="sls-panel-insights">
          {fn.insights.map((i) => (
            <div key={i.kind} className={`sls-card sls-card-${INSIGHT_INFO[i.kind].severity}`}>
              <span className="sls-card-title">{INSIGHT_INFO[i.kind].title}</span>
              <span className="sls-card-metric">
                {INSIGHT_INFO[i.kind].metric} <b>{formatInsightValue(i.kind, i.value)}</b>
              </span>
            </div>
          ))}
          {fn.upgrades.map((u) => (
            <div key={u.component} className="sls-card sls-card-upgrade">
              <span className="sls-card-title">
                <ArrowUpCircle size={14} /> {UPGRADE_LABELS[u.component]} upgrade
              </span>
              <span className="sls-card-metric">
                {u.current} → <b>{u.latest}</b>
              </span>
            </div>
          ))}
        </div>
      )}
      {q.isError ? (
        <ErrorState error={q.error} onRetry={() => q.refetch()} />
      ) : !q.data ? (
        <Loading label="Loading graphs…" />
      ) : (
        <div className="sls-charts sls-charts-panel">
          {CHARTS.map((c) => (
            <SeriesChart key={c.metric} title={c.title} fmt={c.fmt} names={names} height={150} series={q.data.filter((s) => s.metric === c.metric)} />
          ))}
        </div>
      )}
    </>
  );
}

function PanelInvocations({ fn }: { fn: LambdaFunction }) {
  const navigate = useNavigate();
  const { range } = useTime();
  const q = useQuery({ queryKey: ['serverless', 'invocations', range, fn.name], queryFn: () => functionInvocations({ range, function: fn.name }) });
  if (q.isPending) return <Loading label="Loading invocations…" />;
  if (q.isError) return <ErrorState error={q.error} onRetry={() => q.refetch()} />;
  if (q.data.length === 0) return <Empty>No traced invocations in this range. Invocations are listed when the Datadog Lambda library or OpenTelemetry traces the function.</Empty>;
  return (
    <table className="sls-mini">
      <thead>
        <tr>
          <th>DATE</th>
          <th>REQUEST ID</th>
          <th>TRIGGER</th>
          <th className="num">DURATION</th>
          <th>STATUS</th>
        </tr>
      </thead>
      <tbody>
        {q.data.map((i) => (
          <tr key={i.spanId} className="sls-row" onClick={() => navigate(`/apm/traces?trace=${i.traceId}`)} title={i.message || undefined}>
            <td>{timeAgo(i.timestamp)}</td>
            <td className="mono sls-ellipsis">{i.requestId || '—'}</td>
            <td>{i.trigger || '—'}</td>
            <td className="num">
              {formatMs(i.durationMs)} {i.coldStart && <span className="sls-badge sls-badge-warn">cold start</span>}
            </td>
            <td>
              <span className={`sls-status sls-status-${i.error ? 'error' : 'ok'}`}>{i.error ? 'error' : 'ok'}</span>
            </td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}

function PanelLogs({ fn }: { fn: LambdaFunction }) {
  const navigate = useNavigate();
  const { range } = useTime();
  const query = `functionname:${fn.name}`;
  const q = useQuery({ queryKey: ['logs', 'serverless', query, range], queryFn: () => searchLogPage({ range, query }, 100) });
  return (
    <div>
      <div className="sls-logs-head">
        <code>{query}</code>
        <button className="btn btn-sm" onClick={() => navigate(`/logs?query=${encodeURIComponent(query)}`)}>
          View in Log Explorer <ExternalLink size={13} />
        </button>
      </div>
      {q.isPending ? (
        <Loading label="Loading logs…" />
      ) : q.isError ? (
        <ErrorState error={q.error} onRetry={() => q.refetch()} />
      ) : q.data.total === 0 ? (
        <Empty>No logs in this range. The Datadog extension sends the function's logs when DD_SERVERLESS_LOGS_ENABLED is on; they are tagged with functionname.</Empty>
      ) : (
        <LogTable
          logs={q.data.logs.slice(0, 50)}
          selectedId={null}
          onSelect={() => navigate(`/logs?query=${encodeURIComponent(query)}`)}
          options={{ wrap: false, showHost: false, showService: true }}
        />
      )}
    </div>
  );
}

const SOURCE_LABELS = { enhanced_metrics: 'Enhanced metrics', integration_metrics: 'AWS integration metrics', otel_metrics: 'OpenTelemetry metrics', traces: 'Traces' } as const;

function PanelConfiguration({ fn }: { fn: LambdaFunction }) {
  const tags = Object.entries(fn.tags).sort(([a], [b]) => a.localeCompare(b));
  return (
    <>
      <h4 className="sls-panel-h">Telemetry</h4>
      <p>
        {fn.sources.map((s) => SOURCE_LABELS[s]).join(', ') || '—'}
        {fn.hasLogs ? ', Logs' : ''}
      </p>
      <h4 className="sls-panel-h">Tags</h4>
      <table className="sls-mini sls-tags">
        <tbody>
          {tags.map(([k, v]) => (
            <tr key={k}>
              <td className="mono muted">{k}</td>
              <td className="mono">{v}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </>
  );
}
