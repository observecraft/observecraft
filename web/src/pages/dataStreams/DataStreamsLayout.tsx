import { NavLink, Outlet, useLocation } from 'react-router-dom';
import { Waypoints } from 'lucide-react';
import { TimePicker } from '../../components/shell/PageHeader';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { useStreamMap } from './queries';
import { StreamsIntro } from './StreamsIntro';
import './dataStreams.css';

const TABS: { to: string; label: string; end?: boolean; isNew?: boolean }[] = [
  { to: '/data-streams', label: 'Map', end: true },
  { to: '/data-streams/pipelines', label: 'Pipelines', isNew: true },
  { to: '/data-streams/measure', label: 'Measure' },
  { to: '/data-streams/kafka', label: 'Kafka Console' },
  { to: '/data-streams/setup', label: 'Setup' },
];

/** The tabs built on messaging spans: they show the onboarding until spans arrive. */
const SPAN_TABS = ['/data-streams', '/data-streams/pipelines', '/data-streams/measure'];

/** Data Streams Monitoring: Map, Pipelines, Measure, Kafka Console and Setup, with the time range. */
export function DataStreamsLayout() {
  const { pathname } = useLocation();
  const onSpans = SPAN_TABS.includes(pathname.replace(/\/$/, ''));
  const map = useStreamMap();
  let body = <Outlet />;
  if (onSpans) {
    if (map.isError) body = <ErrorState error={map.error} onRetry={() => map.refetch()} />;
    else if (!map.data) body = <Loading label="Loading data streams…" />;
    else if (!map.data.enabled) body = <StreamsIntro />;
  }
  return (
    <div className="dsm-layout">
      <header className="product-header">
        <div className="product-title">
          <Waypoints size={20} />
          <span className="dsm-title-text">Data Streams Monitoring</span>
        </div>
        <nav className="product-tabs">
          {TABS.map((t) => (
            <NavLink key={t.to} to={t.to} end={t.end} className={({ isActive }) => `product-tab${isActive ? ' active' : ''}`}>
              {t.label}
              {t.isNew && <span className="badge-new product-tab-new">NEW</span>}
            </NavLink>
          ))}
        </nav>
        <div className="dsm-header-right">
          <TimePicker />
        </div>
      </header>
      <div className="dsm-section">{body}</div>
    </div>
  );
}
