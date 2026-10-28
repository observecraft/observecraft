import { useCallback, useEffect, useRef, useState } from 'react';
import { Link, useParams, useSearchParams } from 'react-router-dom';
import { Bug, ChevronRight, Database, GitCommitHorizontal, Hexagon, LayoutGrid, ListTree, ScrollText, Server, Share2 } from 'lucide-react';
import type { ServiceScope } from '../../../api/apm/client';
import { TimePicker } from '../../../components/shell/PageHeader';
import { useTime } from '../../../components/shell/TimeContext';
import { ErrorState, Loading } from '../../../components/ui/AsyncState';
import { activeSection, SectionNav, type NavItem } from '../../../components/ui/CollapsibleSection';
import { Select } from '../../../components/ui/Dropdown';
import { useServiceSummary } from '../queries';
import { TraceView } from '../trace/TraceView';
import { HealthBadge, KIND_LABEL, LANGUAGE_LABEL, ServiceIcon } from '../visuals';
import { DatabasesSection, DependenciesSection, DeploymentsSection, ErrorsSection, InfrastructureSection, LogsSection, ResourcesSection, SummarySection, TracesSection } from './ServiceSections';

const SECTIONS: NavItem[] = [
  { id: 'summary', label: 'Service Summary', icon: Hexagon },
  { id: 'resources', label: 'Resources', icon: LayoutGrid },
  { id: 'deployments', label: 'Deployments', icon: GitCommitHorizontal },
  { id: 'dependencies', label: 'Dependencies', icon: Share2 },
  { id: 'traces', label: 'Traces', icon: ListTree },
  { id: 'errors', label: 'Errors', icon: Bug },
  { id: 'infrastructure', label: 'Infrastructure', icon: Server },
  { id: 'databases', label: 'Databases', icon: Database },
  { id: 'logs', label: 'Logs', icon: ScrollText },
];

const ALL = '*';

export function ServicePage() {
  const service = decodeURIComponent(useParams().service ?? '');
  const [params, setParams] = useSearchParams();
  const { range } = useTime();
  const env = params.get('env');
  const resource = params.get('resource');
  const version = params.get('version');
  const scope: ServiceScope = { range, service, env, resource, version };
  const summary = useServiceSummary(scope);
  // The selectors list every env/resource/version, so they come from an unfiltered summary.
  const options = useServiceSummary({ range, service, env });
  const [traceId, setTraceId] = useState<string | null>(null);
  const [navCollapsed, setNavCollapsed] = useState(false);
  const [active, setActive] = useState('summary');
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
    const onScroll = () => setActive(activeSection(root, 'summary'));
    root.addEventListener('scroll', onScroll, { passive: true });
    return () => root.removeEventListener('scroll', onScroll);
  }, []);

  const jump = (id: string) => {
    const root = scroller.current;
    const el = root?.querySelector<HTMLElement>(`[data-section="${id}"]`);
    if (root && el) root.scrollTo({ top: el.offsetTop - 12, behavior: 'smooth' });
  };

  const opt = options.data;
  const s = summary.data;

  return (
    <div className="svc-page" ref={scroller}>
      <div className="svc-toolbar">
        <div className="svc-crumbs">
          <Link to="/apm">Services</Link>
          <ChevronRight size={14} />
          <span>{service}</span>
        </div>
        <div className="svc-filters">
          <Select
            label="Operation"
            value={resource ?? ALL}
            onChange={(v) => setParam('resource', v)}
            options={[{ value: ALL, label: 'All resources' }, ...(opt?.resources ?? []).map((r) => ({ value: r, label: r }))]}
          />
          <Select
            label="Env"
            value={env ?? ALL}
            onChange={(v) => setParam('env', v)}
            options={[{ value: ALL, label: 'All environments' }, ...(opt?.envs ?? []).map((e) => ({ value: e, label: e }))]}
          />
          <Select
            label="Version"
            value={version ?? ALL}
            onChange={(v) => setParam('version', v)}
            options={[{ value: ALL, label: 'All versions' }, ...(opt?.versions ?? []).map((v) => ({ value: v, label: v }))]}
          />
          <TimePicker />
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
              <ServiceIcon name={s!.service} kind={s!.kind} language={s!.language} size={22} />
              <h1>{s!.service}</h1>
              <HealthBadge health={s!.health} />
              <span className="svc-sub">
                {KIND_LABEL[s!.kind]}
                {s!.language && ` · ${LANGUAGE_LABEL[s!.language] ?? s!.language}`}
                {!s!.instrumented && ' · inferred from client spans'}
              </span>
            </div>
          </header>
          <div className="svc-monbar">No Monitors</div>

          <div className={`hp-layout${navCollapsed ? ' nav-collapsed' : ''}`}>
            <SectionNav items={SECTIONS} active={active} onJump={jump} collapsed={navCollapsed} onToggle={() => setNavCollapsed((c) => !c)} />
            <div className="hp-sections">
              <SummarySection summary={s!} />
              <ResourcesSection scope={scope} onPick={(r) => setParam('resource', r)} />
              <DeploymentsSection scope={scope} onPick={(v) => setParam('version', v)} />
              <DependenciesSection scope={scope} />
              <TracesSection scope={scope} onOpen={setTraceId} />
              <ErrorsSection scope={scope} onOpen={setTraceId} />
              <InfrastructureSection hosts={s!.hosts} />
              <DatabasesSection scope={scope} />
              <LogsSection service={service} />
            </div>
          </div>
        </>
      )}
      {traceId && <TraceView traceId={traceId} focusSpanId={null} onClose={() => setTraceId(null)} onSelectSpan={() => {}} />}
    </div>
  );
}
