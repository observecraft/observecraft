import { NavLink, Outlet, useLocation, useNavigate } from 'react-router-dom';
import { ChevronDown, Code2, KeyRound, Layers, Package, ScanEye, Settings, ShieldCheck } from 'lucide-react';
import { Menu } from '../../components/ui/Dropdown';
import './codeSecurity.css';

export const VULN_TYPES = [
  { id: 'libraries', label: 'Libraries (SCA)', icon: ShieldCheck },
  { id: 'static-code', label: 'Static Code (SAST)', icon: Code2 },
  { id: 'runtime-code', label: 'Runtime Code (IAST)', icon: ScanEye },
  { id: 'secrets', label: 'Secret Scanning (Secrets)', icon: KeyRound },
  { id: 'iac', label: 'Infrastructure as Code (IaC)', icon: Layers },
] as const;

export const COVERAGE_TABS = [
  { id: 'cves', label: 'CVE Explorer' },
  { id: 'sast-rules', label: 'SAST Rules' },
  { id: 'iac-rules', label: 'IaC Rules' },
] as const;

/** Security → Code Security: the product header with its tabs; Vulnerabilities and Detection Coverage open menus. */
export function CodeSecurityLayout() {
  const { pathname } = useLocation();
  const navigate = useNavigate();
  const tab = (to: string, label: string, end = false) => (
    <NavLink to={to} end={end} className={({ isActive }) => `product-tab${isActive ? ' active' : ''}`}>
      {label}
    </NavLink>
  );
  const menuTab = (prefix: string, label: string, items: readonly { id: string; label: string; icon?: typeof ShieldCheck }[], isNew = false) => (
    <Menu
      align="left"
      triggerClassName={`product-tab cs-menu-tab${pathname.startsWith(prefix) ? ' active' : ''}`}
      trigger={
        <>
          {isNew && <span className="cs-new">NEW</span>}
          {label} <ChevronDown size={14} />
        </>
      }
      items={items.map((i) => ({ label: i.label, icon: i.icon ? <i.icon size={15} /> : undefined, onClick: () => navigate(`${prefix}/${i.id}`) }))}
    />
  );
  return (
    <div className="cs-layout">
      <header className="product-header">
        <div className="product-title">
          <ShieldCheck size={20} /> Code Security
        </div>
        <nav className="product-tabs">
          {tab('/code-security', 'Summary', true)}
          {menuTab('/code-security/vulnerabilities', 'Vulnerabilities', VULN_TYPES)}
          {tab('/code-security/repositories', 'Repositories')}
          {menuTab('/code-security/detection-coverage', 'Detection Coverage', COVERAGE_TABS, true)}
          {tab('/code-security/inventory', 'Inventory')}
          <NavLink to="/code-security/pr-security" className={({ isActive }) => `product-tab${isActive ? ' active' : ''}`}>
            <span className="cs-new">NEW</span>
            PR Security
          </NavLink>
        </nav>
        <NavLink to="/code-security/settings" className="btn product-config">
          <Settings size={14} /> Settings
        </NavLink>
      </header>
      <div className="cs-body">
        <Outlet />
      </div>
    </div>
  );
}

export function SubTabs({ items, active, onPick }: { items: readonly { id: string; label: string; icon?: typeof Package }[]; active: string; onPick: (id: string) => void }) {
  return (
    <div className="cs-subtabs">
      {items.map((i) => (
        <button key={i.id} className={i.id === active ? 'active' : ''} onClick={() => onPick(i.id)}>
          {i.icon && <i.icon size={15} />} {i.label}
        </button>
      ))}
    </div>
  );
}
