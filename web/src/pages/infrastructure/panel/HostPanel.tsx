import { useEffect, useRef, useState } from 'react';
import {
  Activity,
  BellOff,
  Box,
  Check,
  ChevronDown,
  Copy,
  Flame,
  Gauge,
  GitCommitHorizontal,
  Hexagon,
  ListTree,
  Network,
  ScrollText,
  Share2,
  ShieldCheck,
  Ship,
  Waypoints,
  Wrench,
  X,
} from 'lucide-react';
import { useNavigate } from 'react-router-dom';
import type { HostDetail } from '../../../api/infrastructure/types';
import { ErrorState, Loading } from '../../../components/ui/AsyncState';
import { activeSection, SectionNav, type NavItem } from '../../../components/ui/CollapsibleSection';
import { Menu } from '../../../components/ui/Dropdown';
import { CLOUD_LABEL, CloudIcon, CollectorIcon } from '../hostIcons';
import { useHost } from '../queries';
import { MetricsSection } from './MetricsSection';
import { AgentSection, CollectorSection, EmptySection, LogsSection } from './OtherSections';
import { SummarySection, TagsCard } from './SummarySection';
import { ContainersSection, KubernetesSection, ProcessesSection } from './WorkloadSections';

export const SECTIONS: NavItem[] = [
  { id: 'summary', label: 'Host Summary', icon: Hexagon },
  { id: 'metrics', label: 'Metrics', icon: Gauge },
  { id: 'kubernetes', label: 'Kubernetes', icon: Ship },
  { id: 'containers', label: 'Containers', icon: Box },
  { id: 'processes', label: 'Processes', icon: Activity },
  { id: 'relationships', label: 'Relationships', icon: Share2 },
  { id: 'logs', label: 'Logs', icon: ScrollText },
  { id: 'traces', label: 'Traces', icon: ListTree },
  { id: 'profiles', label: 'Profiles', icon: Flame },
  { id: 'network', label: 'Network', icon: Network },
  { id: 'changes', label: 'Changes', icon: GitCommitHorizontal },
  { id: 'security', label: 'Security', icon: ShieldCheck },
  { id: 'agent', label: 'Collector', icon: Wrench },
  { id: 'otel', label: 'OTel Pipelines', icon: Waypoints },
];

export function HostPanel({ name, onClose }: { name: string; onClose: () => void }) {
  const q = useHost(name);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === 'Escape' && !(e.target as HTMLElement).closest('input, textarea') && onClose();
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [onClose]);

  return (
    <aside className="host-panel" role="dialog" aria-label={`Host ${name}`}>
      {q.isPending ? <Loading label="Loading host…" /> : q.isError ? <ErrorState error={q.error} onRetry={() => q.refetch()} /> : <Body host={q.data} onClose={onClose} />}
    </aside>
  );
}

function Body({ host, onClose }: { host: HostDetail; onClose: () => void }) {
  const navigate = useNavigate();
  const scroller = useRef<HTMLDivElement>(null);
  const [active, setActive] = useState('summary');
  const [navCollapsed, setNavCollapsed] = useState(false);
  const [copied, setCopied] = useState(false);
  const [muted, setMuted] = useState(false);

  useEffect(() => {
    const root = scroller.current;
    if (!root) return;
    const onScroll = () => setActive(activeSection(root, 'summary'));
    root.addEventListener('scroll', onScroll, { passive: true });
    return () => root.removeEventListener('scroll', onScroll);
  }, []);

  const jump = (id: string) => {
    const el = scroller.current?.querySelector<HTMLElement>(`[data-section="${id}"]`);
    if (el && scroller.current) scroller.current.scrollTo({ top: el.offsetTop - 12, behavior: 'smooth' });
  };

  const m = host.monitors;
  const totalMon = m.alert + m.warn + m.ok;

  return (
    <div className="hp-scroll" ref={scroller}>
      <header className="hp-header">
        <div className="hp-title">
          <Hexagon size={18} />
          <h2>{host.name}</h2>
          <button
            className="icon-plain"
            title="Copy host name"
            onClick={() => {
              navigator.clipboard?.writeText(host.name);
              setCopied(true);
              setTimeout(() => setCopied(false), 1200);
            }}
          >
            {copied ? <Check size={16} /> : <Copy size={16} />}
          </button>
        </div>
        <div className="hp-actions">
          <Menu
            trigger={
              <>
                Open Resource <ChevronDown size={14} />
              </>
            }
            items={[
              { label: 'Logs from this host', onClick: () => navigate(`/logs?query=${encodeURIComponent(`host:${host.name}`)}`) },
              { label: 'Host dashboard', onClick: () => navigate('/dashboards') },
            ]}
          />
          <button className={`icon-plain${muted ? ' hp-muted' : ''}`} title={muted ? 'Unmute host' : 'Mute host'} onClick={() => setMuted((x) => !x)}>
            <BellOff size={18} />
          </button>
          <button className="icon-plain" title="Close (Esc)" onClick={onClose}>
            <X size={22} />
          </button>
        </div>
      </header>
      <div className="hp-chips">
        {host.cloud !== 'none' && (
          <span className="hp-chip">
            <CloudIcon cloud={host.cloud} size={13} /> {CLOUD_LABEL[host.cloud]}
          </span>
        )}
        <span className="hp-chip">
          <Box size={13} /> {host.hostType}
        </span>
        <span className="hp-chip">
          <CollectorIcon kind={host.collectors[0]} size={13} /> {host.collectorVersion}
        </span>
      </div>
      <dl className="hp-facts">
        {(
          [
            ['ACCOUNT', host.account],
            ['REGION', host.region],
            ['ENVIRONMENT', host.env],
            ['TEAM', host.team],
            ['SERVICE', host.service],
          ] as const
        ).map(([k, v]) => (
          <div key={k}>
            <dt>{k}</dt>
            <dd>{v ? <span className="tag-chip">{v}</span> : '-'}</dd>
          </div>
        ))}
      </dl>

      {totalMon > 0 ? (
        <div className="hp-monbar">
          {m.alert > 0 && (
            <span className="mb-alert" style={{ flex: Math.max(m.alert, totalMon * 0.27) }}>
              ALERT: {m.alert} Monitor{m.alert > 1 ? 's' : ''}
            </span>
          )}
          {m.warn > 0 && (
            <span className="mb-warn" style={{ flex: Math.max(m.warn, totalMon * 0.2) }}>
              WARN: {m.warn} Monitor{m.warn > 1 ? 's' : ''}
            </span>
          )}
          {m.ok > 0 && (
            <span className="mb-ok" style={{ flex: m.ok }}>
              OK: {m.ok} Monitor{m.ok > 1 ? 's' : ''}
            </span>
          )}
        </div>
      ) : (
        <div className="hp-monbar">
          <span className="mb-none">No monitors cover this host</span>
        </div>
      )}

      <div className={`hp-layout${navCollapsed ? ' nav-collapsed' : ''}`}>
        <SectionNav items={SECTIONS} active={active} onJump={jump} collapsed={navCollapsed} onToggle={() => setNavCollapsed((c) => !c)} />

        <div className="hp-sections">
          <TagsCard host={host} />
          <SummarySection host={host} onJump={jump} />
          <MetricsSection host={host} />
          <KubernetesSection host={host} />
          <ContainersSection host={host} />
          <ProcessesSection host={host} />
          <EmptySection id="relationships" title="Relationships" icon={Share2} message="No context data available" />
          <LogsSection host={host} />
          <EmptySection id="traces" title="Traces" icon={ListTree} message={host.products.includes('apm') ? 'Traces from services on this host appear here once the APM views are built.' : 'No services on this host send traces. Instrument them with the OpenTelemetry SDK.'} />
          <EmptySection id="profiles" title="Profiles" icon={Flame} message="Continuous profiling is not enabled on this host." />
          <EmptySection id="network" title="Network" icon={Network} message="Network performance monitoring is not enabled on this host." />
          <EmptySection id="changes" title="Changes" icon={GitCommitHorizontal} message="No deployments or config changes recorded for this host in the selected time range." />
          <EmptySection id="security" title="Security" icon={ShieldCheck} message="No security signals for this host." />
          <AgentSection host={host} />
          <CollectorSection host={host} />
        </div>
      </div>
    </div>
  );
}
