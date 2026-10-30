import { useCallback, useEffect, useRef, useState } from 'react';
import { Link, useNavigate, useParams, useSearchParams } from 'react-router-dom';
import {
  Activity,
  Braces,
  Bug,
  Check,
  ChevronDown,
  ChevronRight,
  Cloud,
  Copy,
  Database,
  ExternalLink,
  GitCommitHorizontal,
  Hexagon,
  Info,
  Link2,
  ListTree,
  Network,
  ScrollText,
  Server,
  Settings,
  Share2,
  Star,
  Trophy,
  type LucideIcon,
} from 'lucide-react';
import type { ServiceScope } from '../../../api/apm/client';
import { TimePicker } from '../../../components/shell/PageHeader';
import { useTime } from '../../../components/shell/TimeContext';
import { ErrorState, Loading } from '../../../components/ui/AsyncState';
import { activeSection } from '../../../components/ui/CollapsibleSection';
import { Menu, Select } from '../../../components/ui/Dropdown';
import { useServiceSummary } from '../queries';
import { TraceView } from '../trace/TraceView';
import { LANGUAGE_LABEL, ServiceIcon, serviceLogo, useStars } from '../visuals';
import { BrandLogo } from '../../../components/brand/BrandLogo';
import { ChecklistSection, CostsSection, InfoSection, ScorecardsSection, useServiceEntity } from './ServiceGovern';
import { DatabasesSection, DependenciesSection, DeploymentsSection, ErrorsSection, InfrastructureSection, LogsSection, ResourcesSection, SummarySection, TracesSection } from './ServiceSections';

type Mode = 'info' | 'health' | 'govern' | 'configuration';

interface Group {
  id: Mode;
  label: string;
  icon: LucideIcon;
  items: { id: string; label: string; icon: LucideIcon }[];
}

/** The page's groups, as on the service page: one group's sections show at a time. */
const GROUPS: Group[] = [
  { id: 'info', label: 'Info', icon: Info, items: [{ id: 'info', label: 'Info', icon: Info }] },
  {
    id: 'health',
    label: 'Health',
    icon: Activity,
    items: [
      { id: 'summary', label: 'Service Summary', icon: Hexagon },
      { id: 'resources', label: 'Resources', icon: Braces },
      { id: 'deployments', label: 'Deployments', icon: GitCommitHorizontal },
      { id: 'dependencies', label: 'Dependencies', icon: Share2 },
      { id: 'traces', label: 'Traces', icon: ListTree },
      { id: 'errors', label: 'Errors', icon: Bug },
      { id: 'infrastructure', label: 'Infrastructure', icon: Server },
      { id: 'databases', label: 'Databases', icon: Database },
      { id: 'logs', label: 'Logs', icon: ScrollText },
    ],
  },
  {
    id: 'govern',
    label: 'Govern',
    icon: Trophy,
    items: [
      { id: 'scorecards', label: 'Scorecards', icon: Trophy },
      { id: 'costs', label: 'Costs', icon: Cloud },
    ],
  },
  { id: 'configuration', label: 'Configuration', icon: Settings, items: [{ id: 'checklist', label: 'Checklist', icon: Check }] },
];

const ALL = '*';
const NEXT: Partial<Record<Mode, Mode>> = { info: 'health', health: 'govern', govern: 'configuration' };

export function ServicePage() {
  const service = decodeURIComponent(useParams().service ?? '');
  const navigate = useNavigate();
  const [params, setParams] = useSearchParams();
  const { range } = useTime();
  const env = params.get('env');
  const resource = params.get('resource');
  const version = params.get('version');
  const mode = (GROUPS.find((g) => g.id === params.get('mode'))?.id ?? 'health') as Mode;
  const scope: ServiceScope = { range, service, env, resource, version };
  const summary = useServiceSummary(scope);
  // The selectors list every env/resource/version, so they come from an unfiltered summary.
  const options = useServiceSummary({ range, service, env });
  const entity = useServiceEntity(service, env);
  const { stars, toggle } = useStars();
  const [traceId, setTraceId] = useState<string | null>(null);
  const group = GROUPS.find((g) => g.id === mode)!;
  const [active, setActive] = useState(group.items[0].id);
  const [copied, setCopied] = useState<'' | 'name' | 'link'>('');
  const scroller = useRef<HTMLDivElement>(null);

  const setParam = useCallback(
    (key: string, value: string) =>
      setParams(
        (p) => {
          const next = new URLSearchParams(p);
          if (value === ALL) next.delete(key);
          else next.set(key, value);
          // Version and resource lists depend on env.
          if (key === 'env') {
            next.delete('version');
            next.delete('resource');
          }
          return next;
        },
        { replace: true },
      ),
    [setParams],
  );

  useEffect(() => {
    const root = scroller.current;
    if (!root) return;
    const onScroll = () => setActive(activeSection(root, group.items[0].id));
    root.addEventListener('scroll', onScroll, { passive: true });
    return () => root.removeEventListener('scroll', onScroll);
  }, [group]);

  const jump = (m: Mode, id: string) => {
    if (m !== mode) {
      setParam('mode', m === 'health' ? ALL : m);
      setActive(id);
      // The group's sections render on the next pass; scroll once they are there.
      requestAnimationFrame(() => scrollTo(id));
      return;
    }
    scrollTo(id);
  };
  const scrollTo = (id: string) => {
    const root = scroller.current;
    const el = root?.querySelector<HTMLElement>(`[data-section="${id}"]`);
    if (root && el) root.scrollTo({ top: el.offsetTop - 12, behavior: 'smooth' });
    else root?.scrollTo({ top: 0 });
  };
  const copy = (what: 'name' | 'link') => {
    void navigator.clipboard?.writeText(what === 'name' ? service : window.location.href);
    setCopied(what);
    setTimeout(() => setCopied(''), 1500);
  };

  const opt = options.data;
  const s = summary.data;
  const svcQuery = encodeURIComponent(`service:${service}`);
  const logo = s && serviceLogo(s.service, s.language, null);
  const health = s?.health ?? 'ok';
  const next = NEXT[mode];

  return (
    <div className="svc-page" ref={scroller}>
      <div className="svc-topbar">
        <div className="svc-crumbs">
          <Link to="/apm">
            <Network size={14} /> APM
          </Link>
          <ChevronRight size={14} />
          <span className="muted">service:</span> <b>{service}</b>
        </div>
        <div className="svc-quick">
          <Menu
            triggerClassName="link-btn"
            trigger={
              <>
                <ExternalLink size={14} /> Quick Links <ChevronDown size={13} />
              </>
            }
            items={[
              { label: 'Traces', onClick: () => navigate(`/apm/traces?query=${svcQuery}`) },
              { label: 'Logs', onClick: () => navigate(`/logs?query=${svcQuery}`) },
              { label: 'Profiles', onClick: () => navigate(`/apm/profiles?q=${svcQuery}`) },
              { label: 'Error Tracking', onClick: () => navigate(`/errors?query=${svcQuery}`) },
              { label: 'Monitors', onClick: () => navigate(`/monitors?query=${svcQuery}`) },
              { label: 'Software Catalog', onClick: () => navigate(`/portal/catalog?section=service&q=${encodeURIComponent(service)}`) },
            ]}
          />
          <button className="icon-plain" title={copied === 'link' ? 'Copied' : 'Copy link to this page'} onClick={() => copy('link')}>
            {copied === 'link' ? <Check size={15} /> : <Link2 size={15} />}
          </button>
        </div>
      </div>

      {summary.isPending ? (
        <Loading label="Loading service…" />
      ) : summary.isError ? (
        <ErrorState error={summary.error} onRetry={() => summary.refetch()} />
      ) : (
        <>
          <header className="svc-header">
            <div className="svc-title">
              <button className={`icon-plain svc-star${stars.has(service) ? ' starred' : ''}`} onClick={() => toggle(service)} aria-label={stars.has(service) ? 'Unstar' : 'Star'}>
                <Star size={18} />
              </button>
              <ServiceIcon name={s!.service} kind={s!.kind} size={24} />
              <h1>{s!.service}</h1>
              {logo && (
                <span title={LANGUAGE_LABEL[s!.language ?? ''] ?? s!.language ?? ''}>
                  <BrandLogo name={logo} size={18} />
                </span>
              )}
              <button className="icon-plain" title={copied === 'name' ? 'Copied' : 'Copy service name'} onClick={() => copy('name')}>
                {copied === 'name' ? <Check size={15} /> : <Copy size={15} />}
              </button>
              <span className="svc-title-sep" />
              <span className="muted">Health</span>
              <span className={`svc-health h-${health}`}>
                <Activity size={14} /> {health === 'ok' ? 'OK' : health === 'warn' ? 'WARN' : 'ALERT'}
              </span>
              {!s!.instrumented && <span className="svc-inferred">inferred from client spans</span>}
            </div>
            <Menu
              triggerClassName="link-btn"
              trigger={
                <>
                  Actions <ChevronDown size={13} />
                </>
              }
              items={[
                { label: 'Create monitor', onClick: () => navigate('/monitors/create/apm') },
                { label: 'Create SLO', onClick: () => navigate('/slo/create') },
                { label: 'Edit service definition', onClick: () => navigate(`/portal/catalog?section=service&entity=${encodeURIComponent(`service:${service}`)}`) },
              ]}
            />
          </header>
          {entity.entity && entity.entity.monitors > 0 ? (
            <Link className={`svc-monbar${entity.entity.monitorsAlerting ? ' alert' : ''}`} to={`/monitors?query=${svcQuery}`}>
              {entity.entity.monitorsAlerting ? `${entity.entity.monitorsAlerting} of ${entity.entity.monitors} monitors alerting` : `${entity.entity.monitors} monitors OK`}
            </Link>
          ) : (
            <div className="svc-monbar">No Monitors</div>
          )}

          <div className="hp-layout">
            <nav className="hp-nav svc-groups">
              {GROUPS.map((g) => (
                <div key={g.id} className={`svc-group${g.id === mode ? ' open' : ''}`}>
                  <button className="svc-group-head" onClick={() => jump(g.id, g.items[0].id)}>
                    <g.icon size={16} />
                    <span>{g.label}</span>
                    {g.id === mode ? <ChevronDown size={16} /> : <ChevronRight size={16} />}
                  </button>
                  {g.id === mode && g.items.length > 1 && (
                    <div className="svc-group-items">
                      {g.items.map((it) => (
                        <button key={it.id} className={active === it.id ? 'active' : ''} onClick={() => jump(g.id, it.id)}>
                          <it.icon size={15} />
                          <span>{it.label}</span>
                        </button>
                      ))}
                    </div>
                  )}
                </div>
              ))}
            </nav>
            <div className="hp-sections">
              {mode !== 'info' && mode !== 'configuration' && (
                <div className="svc-filters">
                  {mode === 'health' && (
                    <Select
                      label="Operation"
                      value={resource ?? ALL}
                      onChange={(v) => setParam('resource', v)}
                      options={[{ value: ALL, label: 'All resources' }, ...(opt?.resources ?? []).map((r) => ({ value: r, label: r }))]}
                    />
                  )}
                  <Select label="Env" value={env ?? ALL} onChange={(v) => setParam('env', v)} options={[{ value: ALL, label: '*' }, ...(opt?.envs ?? []).map((e) => ({ value: e, label: e }))]} />
                  {mode === 'health' && (
                    <Select
                      label="Version"
                      value={version ?? ALL}
                      onChange={(v) => setParam('version', v)}
                      options={[{ value: ALL, label: 'All' }, ...(opt?.versions ?? []).map((v) => ({ value: v, label: v }))]}
                    />
                  )}
                  <span className="svc-filters-gap" />
                  <TimePicker />
                </div>
              )}
              {mode === 'info' && <InfoSection entity={entity.entity} summary={s!} />}
              {mode === 'health' && (
                <>
                  <SummarySection summary={s!} reliability={entity.entity} />
                  <ResourcesSection scope={scope} onPick={(r) => setParam('resource', r)} />
                  <DeploymentsSection scope={scope} onPick={(v) => setParam('version', v)} />
                  <DependenciesSection scope={scope} />
                  <TracesSection scope={scope} onOpen={setTraceId} />
                  <ErrorsSection scope={scope} onOpen={setTraceId} />
                  <InfrastructureSection hosts={s!.hosts} />
                  <DatabasesSection scope={scope} />
                  <LogsSection service={service} />
                </>
              )}
              {mode === 'govern' && (
                <>
                  <ScorecardsSection service={service} />
                  <CostsSection service={service} env={env} />
                </>
              )}
              {mode === 'configuration' && <ChecklistSection summary={s!} entity={entity.entity} />}
              {next && (
                <button className="svc-next" onClick={() => jump(next, GROUPS.find((g) => g.id === next)!.items[0].id)}>
                  View {GROUPS.find((g) => g.id === next)!.label} →
                </button>
              )}
            </div>
          </div>
        </>
      )}
      {traceId && <TraceView traceId={traceId} focusSpanId={null} onClose={() => setTraceId(null)} onSelectSpan={() => {}} />}
    </div>
  );
}
