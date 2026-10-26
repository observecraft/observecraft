import { createContext, useContext, useEffect, useState, type ReactNode } from 'react';
import { ChevronDown, ListTree, Pause, Play, Ship } from 'lucide-react';
import { NavLink, useNavigate } from 'react-router-dom';
import { usePopover } from '../../components/ui/Dropdown';

export type Product = 'kubernetes' | 'ecs' | 'processes';

const TABS: Record<Product, { label: string; to: string; isNew?: boolean; end?: boolean }[]> = {
  kubernetes: [
    { label: 'Overview', to: '/kubernetes', end: true },
    { label: 'Explorer', to: '/orchestration/explorer' },
    { label: 'Autoscaling', to: '/kubernetes/autoscaling', isNew: true },
    { label: 'Remediation', to: '/kubernetes/remediation', isNew: true },
    { label: 'Live Rollouts', to: '/kubernetes/rollouts', isNew: true },
  ],
  ecs: [{ label: 'Explorer', to: '/orchestration/explorer?resource=ecs_tasks' }],
  processes: [
    { label: 'Explorer', to: '/process' },
    { label: 'Manage Metrics', to: '/process/metrics' },
  ],
};

const NAMES: Record<Product, string> = { kubernetes: 'Kubernetes', ecs: 'ECS', processes: 'Processes' };

function AwsIcon({ size = 20 }: { size?: number }) {
  return (
    <span className="ox-aws" style={{ fontSize: size * 0.9 }} aria-hidden>
      a
    </span>
  );
}

export function ProductIcon({ product, size = 20 }: { product: Product; size?: number }) {
  if (product === 'processes') return <ListTree size={size} className="ox-k8s" />;
  return product === 'ecs' ? <AwsIcon size={size} /> : <Ship size={size} className="ox-k8s" />;
}

/** "Kubernetes ▾ | Overview Explorer …" header of the Kubernetes, ECS and Processes pages, with the live clock. */
export function OrchestrationHeader({ product, active, right }: { product: Product; active?: string; right?: ReactNode }) {
  const navigate = useNavigate();
  const { open, setOpen, ref } = usePopover();
  return (
    <header className="product-header ox-header">
      <div className="dd" ref={ref}>
        <button className="ox-product" onClick={() => setOpen((o) => !o)}>
          <ProductIcon product={product} /> {NAMES[product]} <ChevronDown size={16} />
        </button>
        {open && (
          <div className="dd-menu ox-product-menu">
            <button onClick={() => (setOpen(false), navigate('/kubernetes'))}>
              <ProductIcon product="kubernetes" size={16} /> Kubernetes
            </button>
            <button onClick={() => (setOpen(false), navigate('/orchestration/explorer?resource=ecs_tasks'))}>
              <ProductIcon product="ecs" size={16} /> ECS
            </button>
            <button onClick={() => (setOpen(false), navigate('/process'))}>
              <ProductIcon product="processes" size={16} /> Processes
            </button>
          </div>
        )}
      </div>
      <nav className="ox-tabs">
        {TABS[product].map((t) => (
          <NavLink key={t.label} to={t.to} end={t.end} className={({ isActive }) => `ox-tab${(active ? active === t.label : isActive) ? ' active' : ''}`}>
            {t.isNew && <span className="ox-new">NEW</span>}
            {t.label}
          </NavLink>
        ))}
      </nav>
      {right && <div className="ox-header-right">{right}</div>}
    </header>
  );
}

// ---------------------------------------------------------------- live clock

interface Live {
  paused: boolean;
  setPaused: (p: boolean) => void;
  /** Refetch interval for live data: off while paused. */
  interval: number | false;
}

const LiveContext = createContext<Live>({ paused: false, setPaused: () => undefined, interval: 30_000 });

export function LiveProvider({ children }: { children: ReactNode }) {
  const [paused, setPaused] = useState(false);
  return <LiveContext.Provider value={{ paused, setPaused, interval: paused ? false : 30_000 }}>{children}</LiveContext.Provider>;
}

export const useLive = () => useContext(LiveContext);

function clock(t: number): string {
  const d = new Date(t);
  const date = d.toLocaleDateString('en-US', { month: 'short', day: 'numeric' });
  const time = d.toLocaleTimeString('en-US', { hour: 'numeric', minute: '2-digit' }).toLowerCase();
  return `${date}, ${time}`;
}

/** Datadog's "LIVE Oct 25, 9:31 pm ⏸": the time of the data shown; pausing stops refreshing. */
export function LiveClock({ updatedAt }: { updatedAt?: number }) {
  const { paused, setPaused } = useLive();
  const [now, setNow] = useState(Date.now());
  useEffect(() => {
    if (paused) return;
    const t = setInterval(() => setNow(Date.now()), 15_000);
    return () => clearInterval(t);
  }, [paused]);
  return (
    <span className="ox-live">
      <span className={`ox-live-badge${paused ? ' paused' : ''}`}>{paused ? 'PAUSED' : 'LIVE'}</span>
      <span className={paused ? 'muted' : 'ox-live-time'}>{clock(paused ? (updatedAt ?? now) : now)}</span>
      <button className="ox-live-btn" title={paused ? 'Resume live updates' : 'Pause live updates'} onClick={() => setPaused(!paused)}>
        {paused ? <Play size={15} /> : <Pause size={15} />}
      </button>
    </span>
  );
}
