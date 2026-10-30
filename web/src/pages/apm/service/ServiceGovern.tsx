import { useMemo, useState } from 'react';
import { useQuery } from '@tanstack/react-query';
import type { EChartsOption } from 'echarts';
import { Link } from 'react-router-dom';
import { ArrowRight, BookOpen, Check, ChevronDown, ChevronRight, Cloud, ExternalLink, Info, ListChecks, Mail, MessageSquare, Trophy } from 'lucide-react';
import type { ServiceSummary } from '../../../api/apm/client';
import { queryCosts } from '../../../api/cloudCost/client';
import type { PortalEntity } from '../../../api/portalCatalog/types';
import { profileOverview } from '../../../api/profiles/client';
import { getScorecards } from '../../../api/scorecards/client';
import { serviceOutcomes } from '../../../api/scorecards/logic';
import { Chart } from '../../../components/charts/Chart';
import { useTime } from '../../../components/shell/TimeContext';
import { Loading } from '../../../components/ui/AsyncState';
import { CollapsibleSection } from '../../../components/ui/CollapsibleSection';
import { token } from '../../../theme/chartTheme';
import { usePortalCatalog } from '../../portal/catalog/queries';
import { money, moneyAxis } from '../../cloudCost/costModel';

/** The service's Software Catalog entity: owner, contacts, links and reliability counts. */
export function useServiceEntity(service: string, env: string | null) {
  const { range } = useTime();
  const q = usePortalCatalog(range, env);
  return { ...q, entity: q.data?.entities.find((e) => e.kind === 'service' && e.name === service) ?? null };
}

/* ---------- Info ---------- */

export function InfoSection({ entity, summary }: { entity: PortalEntity | null; summary: ServiceSummary }) {
  const row = (label: string, value: React.ReactNode) => (
    <div className="svc-info-row">
      <span className="muted">{label}</span>
      <span>{value || <span className="muted">—</span>}</span>
    </div>
  );
  return (
    <CollapsibleSection
      id="info"
      title="Info"
      icon={Info}
      extra={
        <Link className="btn" to={`/portal/catalog?section=service&entity=${encodeURIComponent(`service:${summary.service}`)}`}>
          View in Software Catalog <ExternalLink size={12} />
        </Link>
      }
    >
      <div className="svc-info">
        {row('Team', entity?.owner && <span className="tag-chip">{entity.owner}</span>)}
        {row('Description', entity?.description)}
        {row('Tier', entity?.tier)}
        {row('Lifecycle', entity?.lifecycle)}
        {row('Language', summary.language)}
        {row(
          'Contacts',
          entity?.contacts.length ? (
            <span className="svc-info-list">
              {entity.contacts.map((c, k) => (
                <span key={k}>
                  {c.type === 'email' ? <Mail size={13} /> : <MessageSquare size={13} />} {c.contact}
                </span>
              ))}
            </span>
          ) : null,
        )}
        {row(
          'Repository',
          entity?.repoUrl && (
            <a href={entity.repoUrl} target="_blank" rel="noreferrer">
              {entity.repoUrl.replace(/^https?:\/\//, '')}
            </a>
          ),
        )}
        {row(
          'Docs',
          entity?.docsUrl && (
            <a href={entity.docsUrl} target="_blank" rel="noreferrer">
              <BookOpen size={13} /> {entity.docsUrl.replace(/^https?:\/\//, '')}
            </a>
          ),
        )}
        {row('Environments', summary.envs.join(', '))}
        {row('Hosts', summary.hosts.length ? `${summary.hosts.length}` : null)}
      </div>
    </CollapsibleSection>
  );
}

/* ---------- Govern: Scorecards ---------- */

const LEVELS = [
  { level: 1, label: 'Basic Rules' },
  { level: 2, label: 'Intermediate Rules' },
  { level: 3, label: 'Advanced Rules' },
];

export function ScorecardsSection({ service }: { service: string }) {
  const q = useQuery({ queryKey: ['scorecards'], queryFn: ({ signal }) => getScorecards(signal) });
  const [open, setOpen] = useState<number | null>(null);
  const svc = q.data?.services.find((s) => s.service === service);
  const outcomes = useMemo(() => (q.data ? serviceOutcomes(service, q.data.rules, q.data.outcomes) : []), [q.data, service]);
  const active = outcomes.filter((o) => o.outcome.state !== 'skip');
  return (
    <CollapsibleSection
      id="scorecards"
      title="Scorecards"
      icon={Trophy}
      extra={
        svc && (
          <span className="svc-score">
            Entity Level: <b>LEVEL</b> <span className="svc-level">{svc.level}</span>
            <span className="svc-score-sep" />
            Score: <b>{svc.score == null ? '—' : `${Math.round(svc.score * 100)}%`}</b>
            <span className="svc-score-bar">
              <i style={{ width: `${(svc.score ?? 0) * 100}%` }} />
            </span>
            <span className="svc-score-sep" />
            Active Rules: <span className="tag-chip">{active.length}</span>
          </span>
        )
      }
    >
      {q.isPending ? (
        <Loading />
      ) : (
        <>
          {active.length === 0 && (
            <div className="svc-note">
              <Info size={16} />
              <span>
                <b>There are no applicable rules for this entity.</b>
                <br />
                Change the scope of existing rules in Scorecards, or create a rule, to start scoring this service.
              </span>
              <Link className="btn btn-primary" to="/portal/scorecards">
                View Scorecards <ExternalLink size={12} />
              </Link>
            </div>
          )}
          <div className="svc-levels">
            {LEVELS.map((l) => {
              const rules = outcomes.filter((o) => o.rule.level === l.level);
              const pass = rules.filter((o) => o.outcome.state === 'pass').length;
              return (
                <div key={l.level} className="svc-level-row">
                  <button onClick={() => setOpen((o) => (o === l.level ? null : l.level))}>
                    <i className="svc-level-dot" />
                    {open === l.level ? <ChevronDown size={16} /> : <ChevronRight size={16} />}
                    <b>LEVEL</b> <span className={`svc-level l${l.level}`}>{l.level}</span>
                    <span className="muted">{l.label}</span>
                    {rules.length > 0 && (
                      <span className="muted svc-level-count">
                        {pass} / {rules.length} passing
                      </span>
                    )}
                  </button>
                  {open === l.level && (
                    <ul className="svc-rules">
                      {rules.length === 0 && <li className="muted">No rules at this level.</li>}
                      {rules.map((o) => (
                        <li key={o.rule.id}>
                          <span className={`svc-rule-state s-${o.outcome.state}`}>{o.outcome.state.toUpperCase()}</span>
                          <b>{o.rule.name}</b>
                          {o.outcome.reason && <span className="muted"> · {o.outcome.reason}</span>}
                        </li>
                      ))}
                    </ul>
                  )}
                </div>
              );
            })}
          </div>
        </>
      )}
    </CollapsibleSection>
  );
}

/* ---------- Govern: Costs ---------- */

export function CostsSection({ service, env }: { service: string; env: string | null }) {
  const to = useMemo(() => Date.now(), []);
  const range = { from: to - 30 * 86400e3, to };
  const query = [`service:${service}`, env && `env:${env}`].filter(Boolean).join(' ');
  const q = useQuery({
    queryKey: ['cloud-cost', 'service', query],
    queryFn: ({ signal }) => queryCosts({ range, query, groupBy: ['providername'], interval: 'day', costType: 'amortized', usageOnly: false, limit: 10 }, signal),
  });
  const option = useMemo<EChartsOption | null>(() => {
    const d = q.data;
    if (!d || !d.total) return null;
    return {
      grid: { left: 60, right: 8, top: 10, bottom: 40 },
      tooltip: { trigger: 'axis', valueFormatter: (v) => money(Number(v), { precise: true }) },
      legend: { bottom: 0, left: 0, textStyle: { fontSize: 11 } },
      xAxis: { type: 'category', data: d.buckets.map((b) => new Date(b).toLocaleDateString('en-US', { month: 'short', day: 'numeric' })) },
      yAxis: { type: 'value', axisLabel: { formatter: (v: number) => moneyAxis(v) } },
      color: [token('--chart-1'), token('--chart-2'), token('--chart-3'), token('--chart-4')],
      series: d.series.map((s) => ({ name: Object.values(s.tags).join(' / ') || 'Cost', type: 'bar', stack: 'c', barMaxWidth: 14, data: s.values })),
    };
  }, [q.data]);
  return (
    <CollapsibleSection
      id="costs"
      title="Costs"
      icon={Cloud}
      extra={
        <Link className="btn" to={`/cloud-cost/explorer?q=${encodeURIComponent(query)}`}>
          Cloud Cost Explorer <ExternalLink size={12} />
        </Link>
      }
    >
      {q.isPending ? (
        <Loading />
      ) : option ? (
        <>
          <p className="svc-count">
            <b>{money(q.data!.total)}</b> in the past 30 days (amortized), tagged <code>{query}</code>
          </p>
          <Chart option={option} height={220} />
        </>
      ) : (
        <div className="svc-empty">
          <Cloud size={44} />
          <h3>Cloud Cost Management</h3>
          <p>
            No cloud cost data found for <code>{query}</code>. Tag the service's cloud resources with <code>service</code>, or change the environment to view costs.
          </p>
        </div>
      )}
    </CollapsibleSection>
  );
}

/* ---------- Configuration: Checklist ---------- */

interface CheckRow {
  label: string;
  detected: boolean | null;
  description: string;
  action: { label: string; to: string };
}

export function ChecklistSection({ summary, entity }: { summary: ServiceSummary; entity: PortalEntity | null }) {
  const { range } = useTime();
  const profiles = useQuery({ queryKey: ['profiles', 'overview', range, ''], queryFn: () => profileOverview({ range, query: '' }) });
  const svc = encodeURIComponent(`service:${summary.service}`);
  const profiled = profiles.data ? profiles.data.services.some((s) => s.service === summary.service) : null;
  const logs = entity ? entity.sources.includes('logs') : null;
  const setup: CheckRow[] = [
    { label: 'Monitors', detected: entity ? entity.monitors > 0 : null, description: 'Get alerted on anomalous behavior', action: { label: 'New Monitor', to: '/monitors/create/apm' } },
    { label: 'SLOs', detected: entity ? entity.slos > 0 : null, description: 'Define targets for application performance', action: { label: 'New SLO', to: '/slo/create' } },
    {
      label: 'Error Tracking',
      detected: entity ? entity.errorIssues != null : null,
      description: 'Group errors into a manageable number of issues',
      action: { label: 'View Issues', to: `/errors?query=${svc}` },
    },
    { label: 'Logs Correlation', detected: logs, description: 'Investigate traces with the relevant logs in context', action: { label: 'View Logs', to: `/logs?query=${svc}` } },
  ];
  const telemetry: CheckRow[] = [
    { label: 'Distributed Tracing', detected: summary.instrumented, description: 'Track application requests, errors, and latency', action: { label: 'View Traces', to: `/apm/traces?query=${svc}` } },
    { label: 'Continuous Profiler', detected: profiled, description: 'Analyze code performance in production', action: { label: 'View Profiles', to: `/apm/profiles?q=${svc}` } },
    { label: 'Infrastructure Monitoring', detected: summary.hosts.length > 0, description: 'View host and container metrics', action: { label: 'View Infra Metrics', to: '/infrastructure' } },
    { label: 'Log Management', detected: logs, description: 'Collect, process, archive, explore, and monitor your logs', action: { label: 'View Setup', to: '/logs/onboarding' } },
  ];
  const rows = (title: string, items: CheckRow[], window: string) => (
    <>
      <div className="svc-check-head">
        <b>{title}</b>
        <span className="muted">
          <span className="tag-chip">{window}</span>
        </span>
      </div>
      {items.map((r) => (
        <div key={r.label} className="svc-check-row">
          <span className="svc-check-label">
            <ListChecks size={15} /> {r.label}
          </span>
          <span className={`svc-check-state${r.detected ? ' ok' : r.detected === false ? ' no' : ''}`}>{r.detected == null ? 'CHECKING' : r.detected ? 'DETECTED' : 'NOT DETECTED'}</span>
          <span className="muted">{r.description}</span>
          <Link to={r.action.to}>
            {r.action.label} <ArrowRight size={13} />
          </Link>
        </div>
      ))}
    </>
  );
  return (
    <CollapsibleSection id="checklist" title="Checklist" icon={Check}>
      <h4 className="svc-sub-title">Finish setting up your service</h4>
      {rows('Recommended setup for this service', setup, 'Selected time range')}
      {rows('Recommended telemetry', telemetry, 'Selected time range')}
    </CollapsibleSection>
  );
}
