import { NavLink, Outlet, useNavigate } from 'react-router-dom';
import { AlertTriangle, Check, ChevronDown, Hand, Settings, ShieldHalf } from 'lucide-react';
import { Menu } from '../../../components/ui/Dropdown';
import '../../monitors/monitors.css';
import '../security.css';
import './aiGuard.css';

/** AI Guard's frame: Signals, Discover, Investigate and Playground. */
export function AiGuardLayout() {
  const navigate = useNavigate();
  const tab = (to: string, label: string, badge?: string) => (
    <NavLink to={to} end className={({ isActive }) => `product-tab${isActive ? ' active' : ''}`}>
      {badge && <sup className="aig-tab-badge">{badge}</sup>}
      {label}
    </NavLink>
  );
  return (
    <div className="mon-layout sec-layout">
      <header className="product-header sec-header">
        <Menu
          align="left"
          triggerClassName="product-title sec-switch"
          trigger={
            <>
              <ShieldHalf size={20} /> AI Guard <span className="aig-new">NEW</span> <ChevronDown size={16} />
            </>
          }
          items={[
            { label: 'Security', onClick: () => navigate('/security') },
            { label: 'Cloud SIEM', onClick: () => navigate('/security/siem') },
            { label: 'Cloud Security', onClick: () => navigate('/security/cloud') },
            { label: 'App & API Protection', onClick: () => navigate('/security/appsec') },
            { label: 'AI Guard', onClick: () => navigate('/security/ai-guard') },
          ]}
        />
        <nav className="product-tabs">
          {tab('/security/ai-guard/signals', 'Signals')}
          {tab('/security/ai-guard/discover', 'Discover', 'NEW')}
          {tab('/security/ai-guard', 'Investigate')}
          {tab('/security/ai-guard/playground', 'Playground')}
        </nav>
        <div className="mon-header-actions">
          <button className="btn" onClick={() => navigate('/security/ai-guard/settings')}>
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

/** Attack category chip. */
export function CategoryChip({ category }: { category: string }) {
  return <span className="aig-cat">{category}</span>;
}

/** SAFE / UNSAFE / BLOCKED pill. */
export function Assessment({ action, blocked }: { action: string; blocked?: boolean }) {
  if (blocked)
    return (
      <span className="aig-assess blocked">
        <Hand size={12} /> BLOCKED
      </span>
    );
  return action === 'ALLOW' ? (
    <span className="aig-assess safe">
      <Check size={12} /> SAFE
    </span>
  ) : (
    <span className="aig-assess unsafe">
      <AlertTriangle size={12} /> UNSAFE
    </span>
  );
}
