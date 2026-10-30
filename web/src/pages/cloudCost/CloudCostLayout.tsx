import { Link, NavLink, Outlet, useLocation, useNavigate } from 'react-router-dom';
import {
  BellRing,
  Calculator,
  ChevronDown,
  CircleDollarSign,
  Cpu,
  Database,
  FileBarChart,
  GitFork,
  History,
  Home,
  Lightbulb,
  ListChecks,
  Map as MapIcon,
  Network,
  Package,
  Plus,
  Puzzle,
  Settings,
  Sparkles,
  Split,
  Table2,
  Tag,
  Workflow,
  type LucideIcon,
} from 'lucide-react';
import { Menu } from '../../components/ui/Dropdown';
import './cloudCost.css';

interface Tab {
  to: string;
  label: string;
  icon: LucideIcon;
  isNew?: boolean;
}

interface Section {
  id: string;
  label: string;
  tabs: Tab[];
  /** Pages of other products the section's menu opens. */
  links?: Tab[];
  /** Shown under another section's header tab (the menu lists these pages under Summarize). */
  under?: string;
}

/** Cost monitors are metric monitors on the cost metrics: the Monitor menu opens them in Monitors. */
const MONITOR_LINKS: Tab[] = [
  { label: 'Cost Monitors', icon: BellRing, to: '/monitors?q=cost' },
  { label: 'New Cost Monitor', icon: Plus, to: `/monitors/new?type=metric&query=${encodeURIComponent('sum:all.cost{*}')}` },
];

const SECTIONS: Section[] = [
  {
    id: 'summarize',
    label: 'Summarize',
    tabs: [
      { to: '/cloud-cost', label: 'Overview', icon: Home },
      { to: '/cloud-cost/containers', label: 'Containers', icon: Package },
      { to: '/cloud-cost/gpu', label: 'GPU', icon: Cpu },
      { to: '/cloud-cost/networking', label: 'Networking', icon: Network },
      { to: '/cloud-cost/storage', label: 'Storage', icon: Database },
      { to: '/cloud-cost/saas', label: 'SaaS Costs', icon: Puzzle },
      { to: '/cloud-cost/ai', label: 'AI Costs', icon: Sparkles, isNew: true },
      { to: '/cloud-cost/map', label: 'Map', icon: MapIcon },
    ],
  },
  {
    id: 'analyze',
    label: 'Analyze',
    tabs: [
      { to: '/cloud-cost/explorer', label: 'Explorer', icon: Table2 },
      { to: '/cloud-cost/reports', label: 'Reports', icon: FileBarChart },
      { to: '/cloud-cost/tags', label: 'Tags', icon: Tag },
    ],
  },
  { id: 'monitor', label: 'Monitor', tabs: [], links: MONITOR_LINKS },
  {
    id: 'optimize',
    label: 'Optimize',
    tabs: [
      { to: '/cloud-cost/recommendations', label: 'Recommendations', icon: Lightbulb },
      { to: '/cloud-cost/automations', label: 'Automations', icon: Workflow, isNew: true },
    ],
  },
  {
    id: 'allocate',
    label: 'Allocate',
    tabs: [
      { to: '/cloud-cost/tag-pipelines', label: 'Tag Pipelines', icon: GitFork },
      { to: '/cloud-cost/allocation-rules', label: 'Custom Allocation Rules', icon: Split },
    ],
  },
  {
    id: 'plan',
    label: 'Plan',
    tabs: [
      { to: '/cloud-cost/budgets', label: 'Budgets', icon: Calculator },
      { to: '/cloud-cost/commitments', label: 'Commitment Programs', icon: Tag },
      { to: '/cloud-cost/unit-costs', label: 'Unit Costs', icon: CircleDollarSign },
    ],
  },
  {
    id: 'setup',
    label: 'Setup',
    under: 'summarize',
    tabs: [
      { to: '/cloud-cost/settings', label: 'Settings', icon: Settings },
      { to: '/cloud-cost/data-history', label: 'Data History', icon: History },
      { to: '/cloud-cost/onboarding', label: 'Onboarding Checklist', icon: ListChecks },
    ],
  },
];

/** Pages of the menu that aren't built yet. */
export function CloudCostComingSoon() {
  const { pathname } = useLocation();
  const tab = SECTIONS.flatMap((s) => s.tabs).find((t) => t.to === pathname);
  return <div className="placeholder-page">{tab?.label ?? 'This page'} hasn't been built yet.</div>;
}

function activeSection(pathname: string): Section {
  return SECTIONS.find((s) => s.id !== 'summarize' && s.tabs.some((t) => pathname.startsWith(t.to))) ?? SECTIONS[0];
}

/** Cloud Cost frame: the sections of the sidebar menu and the tabs of the open one. */
export function CloudCostLayout() {
  const { pathname } = useLocation();
  const navigate = useNavigate();
  const section = activeSection(pathname);
  return (
    <div className="ccm-layout">
      <header className="product-header">
        <div className="product-title">
          <CircleDollarSign size={20} /> Cloud Cost
        </div>
        <nav className="product-tabs">
          {SECTIONS.filter((s) => !s.under).map((s) =>
            s.id === (section.under ?? section.id) ? (
              <span key={s.id} className="product-tab active">
                {s.label}
              </span>
            ) : (
              <Menu
                key={s.id}
                align="left"
                triggerClassName="product-tab ccm-section-menu"
                trigger={
                  <>
                    {s.label} <ChevronDown size={14} />
                  </>
                }
                items={(s.links ?? s.tabs).map((t) => ({ label: t.label, icon: <t.icon size={14} />, onClick: () => navigate(t.to) }))}
              />
            ),
          )}
        </nav>
      </header>
      <nav className="ccm-tabs">
        {section.tabs.map((t) => (
          <NavLink key={t.to} to={t.to} end={t.to === '/cloud-cost'} className={({ isActive }) => `ccm-tab${isActive ? ' active' : ''}`}>
            <t.icon size={15} /> {t.label}
            {t.isNew && <span className="ccm-new">NEW</span>}
          </NavLink>
        ))}
        {pathname === '/cloud-cost/budgets' && (
          <span className="ccm-tabs-right">
            <Link className="btn btn-primary btn-sm" to="/cloud-cost/budgets/new">
              <Plus size={14} /> New Budget
            </Link>
          </span>
        )}
      </nav>
      <div className="ccm-body">
        <Outlet />
      </div>
    </div>
  );
}
