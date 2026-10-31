import { useEffect, useRef, useState, type ReactNode } from 'react';
import { Link, NavLink, Outlet, useNavigate } from 'react-router-dom';
import { ArrowRight, ChevronDown, ListFilter, Monitor, Search, Settings } from 'lucide-react';
import { Menu } from '../../components/ui/Dropdown';
import { AppTypeIcon, useSelectedApplication } from './common';
import { useRumApplications } from './queries';
import '../monitors/monitors.css';
import './rum.css';

/** RUM's frame: product switcher, application selector, tabs and settings. */
export function RumLayout() {
  const navigate = useNavigate();
  const [selected] = useSelectedApplication();
  const keep = (to: string) => (selected ? `${to}?application=${selected}` : to);
  return (
    <div className="mon-layout rum-layout">
      <header className="product-header rum-header">
        <Menu
          align="left"
          triggerClassName="product-title rum-switch"
          trigger={
            <>
              <Monitor size={20} /> RUM <ChevronDown size={16} />
            </>
          }
          items={[
            { label: 'Real User Monitoring', onClick: () => navigate('/rum') },
            { label: 'Synthetic Monitoring & Testing', onClick: () => navigate('/synthetics') },
            { label: 'Product Analytics', onClick: () => navigate('/product-analytics') },
          ]}
        />
        <ApplicationSelector />
        <nav className="product-tabs">
          <NavLink to={keep('/rum')} end className={({ isActive }) => `product-tab${isActive ? ' active' : ''}`}>
            Summary
          </NavLink>
          <NavLink to={keep('/rum/operations')} className={({ isActive }) => `product-tab${isActive ? ' active' : ''}`}>
            Operations <sup className="rum-new">NEW</sup>
          </NavLink>
          <Menu
            triggerClassName="product-tab rum-tab-menu"
            trigger={
              <>
                Investigate <ChevronDown size={15} />
              </>
            }
            items={[
              { label: 'Sessions', onClick: () => navigate(keep('/rum/explorer') + `${selected ? '&' : '?'}type=session`) },
              { label: 'Errors', onClick: () => navigate(keep('/rum/explorer') + `${selected ? '&' : '?'}type=error`) },
              { label: 'Error Tracking', onClick: () => navigate('/errors') },
            ]}
          />
          <NavLink to={keep('/rum/explorer')} className={({ isActive }) => `product-tab${isActive ? ' active' : ''}`}>
            Explore
          </NavLink>
        </nav>
        <div className="mon-header-actions">
          <Menu
            triggerClassName="btn rum-settings-btn"
            trigger={
              <>
                <Settings size={15} /> Settings <ChevronDown size={14} />
              </>
            }
            items={[
              { label: 'Manage Applications', onClick: () => navigate('/rum/applications') },
              { label: 'Generate Metrics', onClick: () => navigate('/rum/generate-metrics') },
            ]}
          />
        </div>
      </header>
      <div className="mon-section">
        <Outlet />
      </div>
    </div>
  );
}

const OPEN_SELECTOR = 'oc:rum-open-selector';

/** "All Applications ▾": pick the application the pages show. */
export function ApplicationSelector({ open: openInitially = false, onClose }: { open?: boolean; onClose?: () => void }) {
  const navigate = useNavigate();
  const apps = useRumApplications();
  const [selected, setSelected] = useSelectedApplication();
  const [open, setOpen] = useState(openInitially);
  const [search, setSearch] = useState('');
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => setOpen(openInitially), [openInitially]);
  // "Open Application Selector" on the pages opens this one.
  useEffect(() => {
    const show = () => setOpen(true);
    globalThis.addEventListener(OPEN_SELECTOR, show);
    return () => globalThis.removeEventListener(OPEN_SELECTOR, show);
  }, []);
  useEffect(() => {
    if (!open) return;
    const close = (e: MouseEvent) => {
      if (!ref.current?.contains(e.target as Node)) {
        setOpen(false);
        onClose?.();
      }
    };
    document.addEventListener('mousedown', close);
    return () => document.removeEventListener('mousedown', close);
  }, [open, onClose]);
  const list = apps.data ?? [];
  const current = list.find((a) => a.id === selected);
  const shown = list.filter((a) => a.name.toLowerCase().includes(search.trim().toLowerCase()));
  const choose = (id: string | null) => {
    setSelected(id);
    setOpen(false);
    onClose?.();
  };
  return (
    <div className="rum-app-select" ref={ref}>
      <button className="rum-app-trigger" onClick={() => setOpen((o) => !o)} aria-expanded={open}>
        {current && <AppTypeIcon type={current.type} size={18} />}
        <span>{current?.name ?? 'All Applications'}</span>
        <ChevronDown size={18} />
      </button>
      {open && (
        <div className="rum-app-menu" role="listbox">
          <label className="rum-app-search">
            <Search size={14} />
            <input autoFocus placeholder="Search for an application" value={search} onChange={(e) => setSearch(e.target.value)} />
          </label>
          <button role="option" aria-selected={!current} className={`rum-app-option${!current ? ' active' : ''}`} onClick={() => choose(null)}>
            All Applications
          </button>
          {shown.map((a) => (
            <button key={a.id} role="option" aria-selected={a.id === selected} className={`rum-app-option${a.id === selected ? ' active' : ''}`} onClick={() => choose(a.id)}>
              <AppTypeIcon type={a.type} size={16} /> {a.name}
            </button>
          ))}
          {apps.isSuccess && shown.length === 0 && <p className="muted rum-app-none">No application matches.</p>}
          <button className="rum-app-manage" onClick={() => navigate('/rum/applications')}>
            Add &amp; Manage Applications <ArrowRight size={15} />
          </button>
        </div>
      )}
    </div>
  );
}

/** The state of RUM pages before an application is selected: what RUM shows and how to start. */
export function SelectApplicationHero({ children }: { children?: ReactNode }) {
  return (
    <section className="rum-hero">
      <div className="rum-hero-head">
        <span className="rum-hero-icon">
          <Monitor size={22} />
        </span>
        <h2>See your applications&rsquo; health at a glance</h2>
        <Link className="btn rum-hero-new" to="/rum/applications?new=1">
          + New Application
        </Link>
      </div>
      <div className="rum-hero-body">
        <div className="rum-hero-text">
          <p className="rum-hero-lead">Find the slow pages, errors and failing API calls your users run into in your web and mobile applications.</p>
          {children ?? <p>Select an application to get started.</p>}
          <button className="btn rum-hero-select" onClick={(e) => (e.stopPropagation(), globalThis.dispatchEvent(new Event(OPEN_SELECTOR)))}>
            Open Application Selector <ListFilter size={15} />
          </button>
        </div>
        <div className="rum-hero-preview" aria-hidden>
          <div className="rum-preview-card">
            <span>Loading time p75</span>
            <b>1.2s</b>
            <i className="good" />
          </div>
          <div className="rum-preview-card">
            <span>Interaction to next paint</span>
            <b>180ms</b>
            <i className="good" />
          </div>
          <div className="rum-preview-card wide">
            <span>Views with the most errors</span>
            {[
              ['/checkout', 82],
              ['/cart', 46],
              ['/products/?', 21],
              ['/', 8],
            ].map(([name, pct]) => (
              <div key={name} className="rum-preview-bar">
                <em style={{ width: `${pct}%` }} />
                <span>{name}</span>
              </div>
            ))}
          </div>
          <div className="rum-preview-card">
            <span>Sessions</span>
            <b>12.4k</b>
          </div>
          <div className="rum-preview-card">
            <span>Crash-free sessions</span>
            <b>99.2%</b>
            <i className="good" />
          </div>
        </div>
      </div>
    </section>
  );
}
