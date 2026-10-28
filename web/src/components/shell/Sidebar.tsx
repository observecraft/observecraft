import { useCallback, useEffect, useLayoutEffect, useRef, useState } from 'react';
import { Link, NavLink, useLocation } from 'react-router-dom';
import { CircleHelp, LifeBuoy, Menu as MenuIcon, Minus, PanelLeftOpen, Plus, X } from 'lucide-react';
import { NAV_FOOTER, NAV_SECTIONS, QUICK_CREATE, type NavItem } from './navMenus';
import './shell.css';

/** expanded: icons + labels · rail: icons only · hidden: no sidebar, a menu button in the top bar brings it back. */
export type NavMode = 'expanded' | 'rail' | 'hidden';

const MODE_KEY = 'oc.nav.mode';

export function useNavMode(): [NavMode, (m: NavMode) => void] {
  const [mode, setMode] = useState<NavMode>(() => {
    try {
      const v = localStorage.getItem(MODE_KEY);
      return v === 'rail' || v === 'hidden' ? v : 'expanded';
    } catch {
      return 'expanded';
    }
  });
  const set = useCallback((m: NavMode) => {
    setMode(m);
    try {
      localStorage.setItem(MODE_KEY, m);
    } catch {
      /* per-browser convenience only */
    }
  }, []);
  return [mode, set];
}

type Open = { key: string; top: number; item?: NavItem };

/** Delay before a hover menu closes, so the pointer can travel from the item into the menu. */
const CLOSE_DELAY = 180;

export function Sidebar({ mode, setMode }: { mode: NavMode; setMode: (m: NavMode) => void }) {
  const rail = mode === 'rail';
  const [open, setOpen] = useState<Open | null>(null);
  const timer = useRef<number>();
  const { pathname, search } = useLocation();

  const cancelClose = () => window.clearTimeout(timer.current);
  const scheduleClose = () => {
    cancelClose();
    timer.current = window.setTimeout(() => setOpen(null), CLOSE_DELAY);
  };
  const show = (key: string, el: HTMLElement, item?: NavItem) => {
    cancelClose();
    setOpen({ key, top: el.getBoundingClientRect().top, item });
  };

  // Any navigation closes the menu.
  useEffect(() => setOpen(null), [pathname, search]);
  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => e.key === 'Escape' && setOpen(null);
    document.addEventListener('keydown', onKey);
    return () => document.removeEventListener('keydown', onKey);
  }, [open]);
  useEffect(() => () => cancelClose(), []);

  const item = (it: NavItem) => {
    const hasFlyout = !!it.menu || rail;
    return (
      <NavLink
        key={it.to}
        to={it.to}
        className={({ isActive }) => `nav-item${isActive ? ' active' : ''}${open?.key === it.to ? ' hovered' : ''}`}
        aria-label={rail ? it.label : undefined}
        aria-haspopup={hasFlyout ? 'menu' : undefined}
        aria-expanded={hasFlyout ? open?.key === it.to : undefined}
        onMouseEnter={(e) => (hasFlyout ? show(it.to, e.currentTarget, it) : setOpen(null))}
        onMouseLeave={scheduleClose}
        onFocus={(e) => hasFlyout && show(it.to, e.currentTarget, it)}
      >
        <it.icon size={rail ? 18 : 16} strokeWidth={1.75} />
        {!rail && <span>{it.label}</span>}
      </NavLink>
    );
  };

  return (
    <nav className={`sidebar${rail ? ' rail' : ''}`} aria-label="Main navigation">
      <div className="sidebar-head">
        {rail ? (
          <>
            <NavLink to="/" className="sidebar-logo small" aria-label="ObserveCraft home">
              <img src="/logo.svg" alt="" width={30} height={30} />
            </NavLink>
            <button className="sidebar-ctl" title="Expand navigation" onClick={() => setMode('expanded')}>
              <PanelLeftOpen size={18} />
            </button>
            <button
              className={`sidebar-ctl${open?.key === 'create' ? ' hovered' : ''}`}
              title="Create"
              aria-haspopup="menu"
              onMouseEnter={(e) => show('create', e.currentTarget)}
              onMouseLeave={scheduleClose}
              onClick={(e) => (open?.key === 'create' ? setOpen(null) : show('create', e.currentTarget))}
            >
              <Plus size={18} />
            </button>
          </>
        ) : (
          <>
            <button className="sidebar-ctl corner left" title="Hide navigation" onClick={() => setMode('hidden')}>
              <X size={18} />
            </button>
            <button className="sidebar-ctl corner right" title="Collapse navigation" onClick={() => setMode('rail')}>
              <Minus size={18} />
            </button>
            <NavLink to="/" className="sidebar-logo">
              <img src="/logo.svg" alt="" width={52} height={52} />
              <span className="wordmark">OBSERVECRAFT</span>
            </NavLink>
          </>
        )}
      </div>
      <div className="sidebar-scroll" onScroll={() => setOpen(null)}>
        {NAV_SECTIONS.map((section, i) => (
          <div className="nav-section" key={i}>
            {section.map(item)}
          </div>
        ))}
      </div>
      <div className="nav-section nav-footer">{NAV_FOOTER.map(item)}</div>
      <div className="sidebar-bottom">
        {!rail && (
          <button>
            <LifeBuoy size={16} strokeWidth={1.75} />
            Support
          </button>
        )}
        <button>
          <CircleHelp size={16} strokeWidth={1.75} />
          Help
        </button>
      </div>
      {open && <Flyout open={open} rail={rail} onEnter={cancelClose} onLeave={scheduleClose} onPick={() => setOpen(null)} />}
    </nav>
  );
}

function Flyout({ open, rail, onEnter, onLeave, onPick }: { open: Open; rail: boolean; onEnter: () => void; onLeave: () => void; onPick: () => void }) {
  const ref = useRef<HTMLDivElement>(null);
  const [top, setTop] = useState(open.top);

  // Align with the hovered item, but keep the whole menu on screen.
  useLayoutEffect(() => {
    const h = ref.current?.offsetHeight ?? 0;
    setTop(Math.max(8, Math.min(open.top - (rail ? 0 : 8), window.innerHeight - h - 8)));
  }, [open.top, open.key, rail]);

  const it = open.item;
  return (
    <div className="nav-flyout" ref={ref} style={{ top }} role="menu" onMouseEnter={onEnter} onMouseLeave={onLeave}>
      {open.key === 'create' ? (
        <section className="flyout-group">
          <h4>Create</h4>
          <div className="flyout-cols">
            <ul>
              {QUICK_CREATE.map((l) => (
                <li key={l.label}>
                  <Link to={l.to} role="menuitem" onClick={onPick}>
                    {l.label}
                  </Link>
                </li>
              ))}
            </ul>
          </div>
        </section>
      ) : (
        it && (
          <>
            {rail && (
              <Link to={it.to} className="flyout-product" onClick={onPick}>
                {it.label}
              </Link>
            )}
            {it.menu?.map((g) => (
              <section className="flyout-group" key={g.title}>
                <h4>
                  {g.to ? (
                    <Link to={g.to} className="flyout-title-link" role="menuitem" onClick={onPick}>
                      {g.title}
                    </Link>
                  ) : (
                    g.title
                  )}
                  {g.badge && <span className={`flyout-new${g.badge === 'PREVIEW' ? ' preview' : ''}`}>{g.badge}</span>}
                </h4>
                {g.columns.length > 0 && (
                  <div className="flyout-cols">
                    {g.columns.map((col, i) => (
                      <ul key={i}>
                        {col.map((l) => (
                          <li key={l.label}>
                            <Link to={l.to} role="menuitem" onClick={onPick} className={l.plus ? 'flyout-create' : undefined}>
                              {l.plus && <span className="flyout-plus">+</span>}
                              {l.label}
                              {l.isNew && <span className="flyout-new">NEW</span>}
                              {l.isPreview && <span className="flyout-new preview">PREVIEW</span>}
                            </Link>
                          </li>
                        ))}
                      </ul>
                    ))}
                  </div>
                )}
              </section>
            ))}
          </>
        )
      )}
    </div>
  );
}

/** Shown in the top bar while the sidebar is hidden. */
export function ShowNavButton({ onClick }: { onClick: () => void }) {
  return (
    <button className="show-nav-btn" title="Show navigation" aria-label="Show navigation" onClick={onClick}>
      <MenuIcon size={18} />
    </button>
  );
}
