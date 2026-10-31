import { NavLink, Outlet, useLocation, useNavigate } from 'react-router-dom';
import { ChevronDown, Settings, ShieldCheck } from 'lucide-react';
import { Menu } from '../../../components/ui/Dropdown';
import '../../monitors/monitors.css';
import '../security.css';
import '../cloud/cloud.css';
import './workload.css';

const BASE = '/security/workload-protection';

/** Workload Protection's frame: Overview, Signals, Investigate, Findings, Inventory and Policies. */
export function WorkloadLayout() {
  const navigate = useNavigate();
  const { pathname } = useLocation();
  const on = (p: string) => (pathname.startsWith(p) ? ' active' : '');
  const tab = ({ isActive }: { isActive: boolean }) => `product-tab${isActive ? ' active' : ''}`;
  const menu = (label: string, prefix: string, items: [string, string][]) => (
    <Menu
      triggerClassName={`product-tab rum-tab-menu${on(prefix)}`}
      trigger={
        <>
          {label} <ChevronDown size={15} />
        </>
      }
      items={items.map(([l, to]) => ({ label: l, onClick: () => navigate(to) }))}
    />
  );
  return (
    <div className="mon-layout sec-layout">
      <header className="product-header sec-header">
        <Menu
          align="left"
          triggerClassName="product-title sec-switch"
          trigger={
            <>
              <ShieldCheck size={20} /> Workload Protection <ChevronDown size={16} />
            </>
          }
          items={[
            { label: 'Security', onClick: () => navigate('/security') },
            { label: 'Cloud SIEM', onClick: () => navigate('/security/siem') },
            { label: 'Cloud Security', onClick: () => navigate('/security/cloud') },
            { label: 'Workload Protection', onClick: () => navigate(BASE) },
            { label: 'App & API Protection', onClick: () => navigate('/security/appsec') },
            { label: 'Code Security', onClick: () => navigate('/code-security') },
          ]}
        />
        <nav className="product-tabs">
          <NavLink to={BASE} end className={tab}>
            Overview
          </NavLink>
          <NavLink to={`${BASE}/signals`} className={tab}>
            Signals
          </NavLink>
          <NavLink to={`${BASE}/investigate`} className={tab}>
            Investigate
          </NavLink>
          <NavLink to={`${BASE}/findings`} className={tab}>
            Findings
          </NavLink>
          {menu('Inventory', `${BASE}/inventory`, INVENTORY_TABS)}
          {menu('Policies', `${BASE}/policies`, POLICY_TABS)}
        </nav>
        <div className="mon-header-actions">
          <button className="btn" onClick={() => navigate(`${BASE}/policies`)}>
            <Settings size={15} /> Settings
          </button>
        </div>
      </header>
      <div className="mon-section">
        <Outlet />
      </div>
    </div>
  );
}

/** Sub-tabs (Inventory, Policies). */
export function WpTabs({ tabs }: { tabs: [string, string][] }) {
  return (
    <nav className="sec-subtabs csm-subtabs">
      {tabs.map(([label, to]) => (
        <NavLink key={to} to={to} end className={({ isActive }) => (isActive ? 'active' : '')}>
          {label}
        </NavLink>
      ))}
    </nav>
  );
}

export const INVENTORY_TABS: [string, string][] = [
  ['Hosts & Containers', `${BASE}/inventory`],
  ['Serverless', `${BASE}/inventory/serverless`],
  ['Coverage', `${BASE}/inventory/coverage`],
];
export const POLICY_TABS: [string, string][] = [
  ['Agent configuration', `${BASE}/policies`],
  ['Detection rules', `${BASE}/policies/detection-rules`],
  ['Finding rules', `${BASE}/policies/finding-rules`],
];
