import { NavLink, Outlet, useLocation, useNavigate, useSearchParams } from 'react-router-dom';
import {
  Bot,
  ChevronDown,
  CircleDollarSign,
  ClipboardCheck,
  Code2,
  Database,
  FlaskConical,
  Globe,
  LayoutDashboard,
  LayoutGrid,
  Lightbulb,
  ListTree,
  MessageSquare,
  MessageSquareText,
  Play,
  Settings,
  Shapes,
  type LucideIcon,
} from 'lucide-react';
import type { LlmQuery } from '../../api/llm/client';
import { TimePicker } from '../../components/shell/PageHeader';
import { TimeProvider, useTime } from '../../components/shell/TimeContext';
import { Loading } from '../../components/ui/AsyncState';
import { Menu, Select } from '../../components/ui/Dropdown';
import { LlmOnboarding } from './LlmOnboarding';
import { useHasLlmData, useLlmFacets } from './queries';
import './llm.css';

interface NavItem {
  to: string;
  label: string;
  icon: LucideIcon;
  /** Not built yet: listed greyed out. */
  soon?: boolean;
}

const NAV: { section: string; items: NavItem[] }[] = [
  {
    section: 'OVERVIEW',
    items: [
      { to: '/llm', label: 'Applications', icon: Code2 },
      { to: '/llm/cost', label: 'Cost', icon: CircleDollarSign },
      { to: 'patterns', label: 'Patterns', icon: Shapes, soon: true },
    ],
  },
  {
    section: 'INVESTIGATE',
    items: [
      { to: '/llm/traces', label: 'Traces', icon: ListTree },
      { to: '/llm/evaluations', label: 'Evaluations', icon: ClipboardCheck },
      { to: 'annotations', label: 'Annotations', icon: MessageSquare, soon: true },
      { to: 'insights', label: 'Insights', icon: Lightbulb, soon: true },
    ],
  },
  {
    section: 'IMPROVE',
    items: [
      { to: 'playground', label: 'Playground', icon: Play, soon: true },
      { to: 'experiments', label: 'Experiments', icon: FlaskConical, soon: true },
      { to: 'datasets', label: 'Datasets', icon: Database, soon: true },
    ],
  },
  {
    section: 'CONFIGURE',
    items: [
      { to: 'prompts', label: 'Prompts', icon: MessageSquareText, soon: true },
      { to: 'gateway', label: 'AI Gateway', icon: Globe, soon: true },
    ],
  },
];

/** Default time range per section: a week for the overviews, an hour when investigating. */
function section(pathname: string): { id: string; title: string; initial: string } {
  if (pathname.startsWith('/llm/traces')) return { id: 'traces', title: 'Traces', initial: '1h' };
  if (pathname.startsWith('/llm/evaluations')) return { id: 'evaluations', title: 'Evaluations', initial: '1d' };
  if (pathname.startsWith('/llm/cost')) return { id: 'cost', title: 'Cost', initial: '1w' };
  if (pathname.startsWith('/llm/setup')) return { id: 'setup', title: 'Applications', initial: '1w' };
  return { id: 'applications', title: 'Applications', initial: '1w' };
}

/** The selected ML application (`?app=`), shared by every LLM Observability page. */
export function useLlmApp(): [string, (app: string) => void] {
  const [params, setParams] = useSearchParams();
  const app = params.get('app') ?? '';
  const set = (next: string) => {
    const n = new URLSearchParams(params);
    if (next) n.set('app', next);
    else n.delete('app');
    setParams(n, { replace: true });
  };
  return [app, set];
}

/** The page query: time range, search and facet selections, narrowed to the selected app. */
export function useLlmQuery(query = '', filters: Record<string, string[]> = {}): LlmQuery {
  const t = useTime();
  const [app] = useLlmApp();
  return { range: t.range, query, filters: app ? { ...filters, ml_app: [app] } : filters };
}

/** LLM Observability frame: section navigation, application selector, views and the time range. */
export function LlmLayout() {
  const { pathname } = useLocation();
  const s = section(pathname);
  return (
    <TimeProvider key={s.id} initial={s.initial}>
      <LlmFrame setup={s.id === 'setup'} title={s.title} />
    </TimeProvider>
  );
}

function LlmFrame({ setup, title }: { setup: boolean; title: string }) {
  const navigate = useNavigate();
  const [params] = useSearchParams();
  const has = useHasLlmData();
  /** A link that keeps the selected app, plus `extra` parameters. */
  const keep = (to: string, extra: Record<string, string> = {}) => {
    const n = new URLSearchParams(extra);
    const app = params.get('app');
    if (app) n.set('app', app);
    const qs = n.toString();
    return qs ? `${to}?${qs}` : to;
  };
  return (
    <div className="llm-layout">
      <header className="product-header llm-header">
        <div className="product-title llm-title">
          <Bot size={20} />
          LLM Observability
        </div>
        <h1 className="llm-page-title">{title}</h1>
        <AppSelector />
        <div className="llm-header-right">
          <Menu
            trigger={
              <>
                <LayoutGrid size={14} /> Views <ChevronDown size={14} />
              </>
            }
            align="right"
            items={[
              { label: 'Traces with errors', onClick: () => navigate(keep('/llm/traces', { status: 'error' })) },
              { label: 'Agent runs', onClick: () => navigate(keep('/llm/traces', { kind: 'agent' })) },
              { label: 'Failed evaluations: PII in output', onClick: () => navigate(keep('/llm/evaluations', { check: 'pii_in_output' })) },
              { label: 'Cost by application', onClick: () => navigate(keep('/llm/cost', { by: 'ml_app' })) },
            ]}
          />
          <TimePicker />
        </div>
      </header>
      <div className="llm-body">
        <aside className="llm-nav" aria-label="LLM Observability">
          {NAV.map((g) => (
            <div key={g.section} className="llm-nav-group">
              <div className="llm-nav-section">{g.section}</div>
              {g.items.map((i) =>
                i.soon ? (
                  <span key={i.to} className="llm-nav-link soon" title={`${i.label} isn't built yet`}>
                    <i.icon size={15} />
                    {i.label}
                    <span className="llm-soon">SOON</span>
                  </span>
                ) : (
                  <NavLink key={i.to} to={keep(i.to)} end className={({ isActive }) => `llm-nav-link${isActive ? ' active' : ''}`}>
                    <i.icon size={15} />
                    {i.label}
                  </NavLink>
                ),
              )}
            </div>
          ))}
          <div className="llm-nav-bottom">
            <NavLink to="/dashboards" className="llm-nav-link">
              <LayoutDashboard size={15} />
              Dashboards
            </NavLink>
            <span className="llm-nav-link soon" title="Settings aren't built yet">
              <Settings size={15} />
              Settings
              <span className="llm-soon">SOON</span>
            </span>
          </div>
        </aside>
        <main className="llm-main">{setup ? <LlmOnboarding /> : has.isPending ? <Loading /> : has.data === false ? <LlmOnboarding /> : <Outlet />}</main>
      </div>
    </div>
  );
}

function AppSelector() {
  const [app, setApp] = useLlmApp();
  const t = useTime();
  const facets = useLlmFacets({ range: t.range, query: '', filters: {} }, ['ml_app']);
  const apps = facets.data?.facets.ml_app ?? [];
  const options = [{ value: '', label: 'All Applications' }, ...apps.map((a) => ({ value: a.value, label: a.value, hint: a.count.toLocaleString() }))];
  if (app && !apps.some((a) => a.value === app)) options.push({ value: app, label: app, hint: '0' });
  return <Select value={app} options={options} onChange={setApp} triggerClassName="dd-trigger llm-app-select" title="ML application" />;
}
