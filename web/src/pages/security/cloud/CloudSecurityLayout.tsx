import { NavLink, Outlet, useLocation, useNavigate } from 'react-router-dom';
import { ChevronDown, CloudCog } from 'lucide-react';
import { Menu } from '../../../components/ui/Dropdown';
import '../../monitors/monitors.css';
import '../security.css';
import './cloud.css';

/** Cloud Security's frame: Summary, Inbox, Findings and Inventory. */
export function CloudSecurityLayout() {
  const navigate = useNavigate();
  const { pathname } = useLocation();
  const tab = ({ isActive }: { isActive: boolean }) => `product-tab${isActive ? ' active' : ''}`;
  const active = (prefixes: string[]) => (prefixes.some((p) => pathname.startsWith(p)) ? ' active' : '');
  return (
    <div className="mon-layout sec-layout">
      <header className="product-header sec-header">
        <Menu
          align="left"
          triggerClassName="product-title sec-switch"
          trigger={
            <>
              <CloudCog size={20} /> Cloud Security <ChevronDown size={16} />
            </>
          }
          items={[
            { label: 'Security', onClick: () => navigate('/security') },
            { label: 'Cloud SIEM', onClick: () => navigate('/security/siem') },
            { label: 'Cloud Security', onClick: () => navigate('/security/cloud') },
            { label: 'Code Security', onClick: () => navigate('/code-security') },
          ]}
        />
        <nav className="product-tabs">
          <NavLink to="/security/cloud" end className={tab}>
            Summary
          </NavLink>
          <NavLink to="/security/cloud/inbox" className={tab}>
            Inbox
          </NavLink>
          <Menu
            triggerClassName={`product-tab rum-tab-menu${active(['/security/cloud/findings', '/security/cloud/vulnerabilities'])}`}
            trigger={
              <>
                Findings <ChevronDown size={15} />
              </>
            }
            items={[
              { label: 'Misconfigurations', onClick: () => navigate('/security/cloud/findings') },
              { label: 'Vulnerabilities', onClick: () => navigate('/security/cloud/vulnerabilities') },
            ]}
          />
          <Menu
            triggerClassName={`product-tab rum-tab-menu${active(['/security/cloud/inventory'])}`}
            trigger={
              <>
                Inventory <ChevronDown size={15} />
              </>
            }
            items={[
              { label: 'Resources', onClick: () => navigate('/security/cloud/inventory') },
              { label: 'Infrastructure Packages', onClick: () => navigate('/security/cloud/inventory/packages') },
              { label: 'CVE Explorer', onClick: () => navigate('/security/cloud/inventory/cves') },
            ]}
          />
        </nav>
      </header>
      <div className="mon-section">
        <Outlet />
      </div>
    </div>
  );
}

/** Sub-tabs of Findings and Inventory. */
export function SubTabs({ tabs }: { tabs: [string, string][] }) {
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

export const FINDING_TABS: [string, string][] = [
  ['Misconfigurations', '/security/cloud/findings'],
  ['Vulnerabilities', '/security/cloud/vulnerabilities'],
];
export const INVENTORY_TABS: [string, string][] = [
  ['Resources', '/security/cloud/inventory'],
  ['Infrastructure Packages', '/security/cloud/inventory/packages'],
  ['CVE Explorer', '/security/cloud/inventory/cves'],
];
