import { NavLink, Outlet, useLocation, useNavigate } from 'react-router-dom';
import { ChevronDown, Settings, ShieldHalf } from 'lucide-react';
import { Menu } from '../../../components/ui/Dropdown';
import '../../monitors/monitors.css';
import '../security.css';
import '../cloud/cloud.css';
import './appsec.css';

/** App & API Protection's frame: Overview, Signals, Findings, Investigate, Inventory and Policies. */
export function AppsecLayout() {
  const navigate = useNavigate();
  const { pathname } = useLocation();
  const on = (p: string) => (pathname.startsWith(p) ? ' active' : '');
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
              <ShieldHalf size={20} /> App &amp; API Protection <ChevronDown size={16} />
            </>
          }
          items={[
            { label: 'Security', onClick: () => navigate('/security') },
            { label: 'Cloud SIEM', onClick: () => navigate('/security/siem') },
            { label: 'Cloud Security', onClick: () => navigate('/security/cloud') },
            { label: 'App & API Protection', onClick: () => navigate('/security/appsec') },
            { label: 'Code Security', onClick: () => navigate('/code-security') },
          ]}
        />
        <nav className="product-tabs">
          <NavLink to="/security/appsec" end className={({ isActive }) => `product-tab${isActive ? ' active' : ''}`}>
            Overview
          </NavLink>
          <NavLink to="/security/appsec/signals" className={({ isActive }) => `product-tab${isActive ? ' active' : ''}`}>
            Signals
          </NavLink>
          {menu('Findings', '/security/appsec/findings', [
            ['API Findings', '/security/appsec/findings'],
            ['Compliance', '/security/appsec/findings/compliance'],
          ])}
          {menu('Investigate', '/security/appsec/investigate', [
            ['Traces', '/security/appsec/investigate'],
            ['Users', '/security/appsec/investigate/users'],
            ['Attackers', '/security/appsec/investigate/attackers'],
          ])}
          {menu('Inventory', '/security/appsec/inventory', [
            ['API Endpoints', '/security/appsec/inventory'],
            ['Services', '/security/appsec/inventory/services'],
          ])}
          {menu('Policies', '/security/appsec/policies', [
            ['In-App WAF', '/security/appsec/policies'],
            ['Business Logic', '/security/appsec/policies/business-logic'],
            ['Detection Rules', '/security/appsec/policies/rules'],
            ['Denylist', '/security/appsec/policies/denylist'],
            ['Passlist', '/security/appsec/policies/passlist'],
            ['Sensitive Data Detection', '/security/appsec/policies/sensitive-data'],
          ])}
        </nav>
        <div className="mon-header-actions">
          <button className="btn" onClick={() => navigate('/security/appsec/inventory/services')}>
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

/** Sub-tabs (Findings, Investigate, Inventory, Policies). */
export function AapTabs({ tabs }: { tabs: [string, string][] }) {
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

export const FINDINGS_TABS: [string, string][] = [
  ['API Findings', '/security/appsec/findings'],
  ['Compliance', '/security/appsec/findings/compliance'],
];
export const INVESTIGATE_TABS: [string, string][] = [
  ['Traces', '/security/appsec/investigate'],
  ['Users', '/security/appsec/investigate/users'],
  ['Attackers', '/security/appsec/investigate/attackers'],
];
export const INVENTORY_TABS: [string, string][] = [
  ['API Endpoints', '/security/appsec/inventory'],
  ['Services', '/security/appsec/inventory/services'],
];
export const POLICY_TABS: [string, string][] = [
  ['In-App WAF', '/security/appsec/policies'],
  ['Business Logic', '/security/appsec/policies/business-logic'],
  ['Detection Rules', '/security/appsec/policies/rules'],
  ['Denylist', '/security/appsec/policies/denylist'],
  ['Passlist', '/security/appsec/policies/passlist'],
  ['Sensitive Data Detection', '/security/appsec/policies/sensitive-data'],
];

export function MethodBadge({ method }: { method: string }) {
  return <span className={`aap-method m-${method.toLowerCase()}`}>{method}</span>;
}
