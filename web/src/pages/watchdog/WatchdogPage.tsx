import { useCallback, useMemo, useState } from 'react';
import { useNavigate, useSearchParams } from 'react-router-dom';
import { useQuery } from '@tanstack/react-query';
import { Activity, Binoculars, ChevronRight, FileText, LayoutDashboard, PanelLeft, PanelLeftClose, Pin, Plus, Server } from 'lucide-react';
import { watchdogAlerts, type WatchdogAlert } from '../../api/watchdog/client';
import { FacetPanel } from '../../components/facets/FacetPanel';
import { QueryBar } from '../../components/query/QueryBar';
import { Empty, ErrorState, Loading } from '../../components/ui/AsyncState';
import { Menu } from '../../components/ui/Dropdown';
import { formatQuery, parseQuery, selectedValues, toggleValue, type Term } from '../../lib/searchQuery';
import { alertFacets, filterAlerts, formatDuration, formatValue, sparkline, teamCounts, valueLabel } from '../../lib/watchdog';
import { AlertPanel } from './AlertPanel';

/** Saved searches offered under "Views". */
const VIEWS: { label: string; query: string }[] = [
  { label: 'All alerts', query: '' },
  { label: 'Ongoing', query: 'status:ongoing' },
  { label: 'APM', query: 'category:apm' },
  { label: 'Infrastructure', query: 'category:infrastructure' },
  { label: 'Logs', query: 'category:logs' },
];

export const CATEGORY_ICON = { apm: Activity, infrastructure: Server, logs: FileText } as const;

export function useWatchdogAlerts() {
  // Detection runs on each request over the past day; refresh with the 5-minute buckets.
  return useQuery({ queryKey: ['watchdog', 'alerts'], queryFn: ({ signal }) => watchdogAlerts(signal), refetchInterval: 60_000 });
}

/** Status colour bar next to Alert Status values (and on each card). */
function StatusBar({ status }: { status: string }) {
  return <span className={`wd-status-bar wd-status-${status}`} />;
}

const valueIcon = (key: string, value: string) => (key === 'status' ? <StatusBar status={value} /> : null);

/** When an alert started and how long it lasted (or has lasted so far). */
function timing(a: WatchdogAlert, now: number): string {
  const ago = `Started ${formatDuration(now - a.start)} ago`;
  return a.status === 'ongoing' ? `${ago} · still ongoing` : `${ago} · lasted ${formatDuration(a.end - a.start)}`;
}

function Sparkline({ alert }: { alert: WatchdogAlert }) {
  const w = 160;
  const h = 40;
  const s = sparkline(alert.points, alert.start, alert.end, w, h);
  return (
    <svg className="wd-spark" width={w} height={h} viewBox={`0 0 ${w} ${h}`} aria-hidden>
      <rect className={`wd-spark-window wd-window-${alert.status}`} x={s.windowX} y={0} width={Math.max(s.windowWidth, 2)} height={h} />
      <path className="wd-spark-line" d={s.path} />
    </svg>
  );
}

function AlertCard({ alert, now, selected, onOpen }: { alert: WatchdogAlert; now: number; selected: boolean; onOpen: () => void }) {
  const Icon = CATEGORY_ICON[alert.category];
  return (
    <button className={`wd-card${selected ? ' selected' : ''}`} onClick={onOpen}>
      <StatusBar status={alert.status} />
      <span className="wd-card-icon" title={valueLabel('category', alert.category)}>
        <Icon size={18} />
      </span>
      <span className="wd-card-body">
        <span className="wd-card-title">{alert.title}</span>
        <span className="wd-card-tags">
          {alert.scope.map((t) => (
            <span key={t} className="wd-tag">
              {t}
            </span>
          ))}
          {alert.team && <span className="wd-tag">team:{alert.team}</span>}
          <span className="wd-tag wd-tag-muted">{valueLabel('category', alert.category)}</span>
        </span>
        <span className="wd-card-meta">
          <span className={`wd-status-text wd-status-text-${alert.status}`}>{valueLabel('status', alert.status)}</span>
          <span>{timing(alert, now)}</span>
          <span>
            Peak <b>{formatValue(alert.peak, alert.unit)}</b> vs. {formatValue(alert.baseline, alert.unit)} usual
          </span>
        </span>
      </span>
      <Sparkline alert={alert} />
    </button>
  );
}

/** Monitoring → Watchdog: anomalies found in the past day of APM, log and host data, without monitors. */
export function WatchdogPage() {
  const navigate = useNavigate();
  const [params, setParams] = useSearchParams();
  const query = params.get('q') ?? '';
  const openId = params.get('alert');
  const setParam = useCallback(
    (key: string, value: string | null) =>
      setParams(
        (p) => {
          const next = new URLSearchParams(p);
          if (value === null || value === '') next.delete(key);
          else next.set(key, value);
          return next;
        },
        { replace: key !== 'alert' },
      ),
    [setParams],
  );
  const [showControls, setShowControls] = useState(true);
  const [viewsOpen, setViewsOpen] = useState(false);
  const [teamsOpen, setTeamsOpen] = useState(true);
  const q = useWatchdogAlerts();
  const alerts = useMemo(() => q.data?.alerts ?? [], [q.data]);
  const shown = useMemo(() => filterAlerts(alerts, query), [alerts, query]);
  const facets = useMemo(() => alertFacets(alerts, query), [alerts, query]);
  const teams = useMemo(() => teamCounts(alerts, query), [alerts, query]);
  const terms = useMemo(() => parseQuery(query), [query]);
  const selected = useMemo(() => selectedValues(terms), [terms]);
  const setTerms = useCallback((next: Term[]) => setParam('q', formatQuery(next)), [setParam]);
  const now = q.dataUpdatedAt || Date.now();
  const opened = alerts.find((a) => a.id === openId);

  const facetHeader = (
    <>
      <div className="wd-side-card">
        <button className="wd-side-toggle" onClick={() => setTeamsOpen((o) => !o)}>
          <ChevronRight size={14} className={teamsOpen ? 'rot90' : ''} /> Teams
        </button>
        <Pin size={14} className="muted" aria-label="Pinned" />
      </div>
      {teamsOpen && (
        <ul className="wd-team-list">
          {teams.length === 0 && <li className="muted-text">No team owns the affected services. Teams come from service metadata in the Developer Portal.</li>}
          {teams.map((t) => (
            <li key={t.value}>
              <label>
                <input type="checkbox" className="checkbox" checked={(selected.team ?? []).includes(t.value)} onChange={() => setTerms(toggleValue(terms, 'team', t.value))} />
                <span className="wd-team-name">{t.value}</span>
                <span className="facet-count">{t.count}</span>
              </label>
            </li>
          ))}
        </ul>
      )}
    </>
  );

  return (
    <div className="wd-page">
      <div className="section-header">
        <div className="section-header-left">
          <button className={`page-tab${viewsOpen ? ' active' : ''}`} onClick={() => setViewsOpen((o) => !o)} aria-expanded={viewsOpen}>
            Views
          </button>
          <div className="page-tab active">{VIEWS.find((v) => v.query === query && v.query)?.label ?? 'My View'}</div>
        </div>
      </div>
      {viewsOpen && (
        <div className="wd-views">
          {VIEWS.map((v) => (
            <button key={v.label} className={`wd-view${v.query === query ? ' active' : ''}`} onClick={() => setParam('q', v.query)}>
              {v.label}
            </button>
          ))}
        </div>
      )}

      <div className="wd-query">
        <QueryBar value={query} onChange={(v) => setParam('q', v)} placeholder="Filter alerts" />
      </div>

      <div className="wd-body">
        {showControls && (
          <FacetPanel
            facets={facets}
            terms={terms}
            onChange={setTerms}
            defaultOpen={['category', 'type', 'status']}
            verified={[]}
            noun="facets"
            valueLabel={valueLabel}
            renderValueIcon={valueIcon}
            header={facetHeader}
          />
        )}
        <main className="wd-main">
          <div className="wd-toolbar">
            <button className="link-btn" onClick={() => setShowControls((s) => !s)}>
              {showControls ? <PanelLeftClose size={15} /> : <PanelLeft size={15} />}
              {showControls ? 'Hide Controls' : 'Show Controls'}
            </button>
            <span className="wd-sep" />
            <span className="wd-count">{q.data ? `${shown.length} recent alert${shown.length === 1 ? '' : 's'} found` : ' '}</span>
            <span className="wd-toolbar-right">
              <button className="btn" disabled title="Not available yet">
                <LayoutDashboard size={14} /> Save to Dashboard
              </button>
              <Menu
                trigger="More…"
                items={[
                  { label: 'Copy link to view', onClick: () => navigator.clipboard?.writeText(window.location.href) },
                  { label: 'Refresh now', onClick: () => q.refetch() },
                ]}
              />
              <button className="btn btn-primary" onClick={() => navigate('/monitors/create')}>
                <Plus size={15} /> New Monitor
              </button>
            </span>
          </div>
          {q.isPending ? (
            <Loading label="Looking for anomalies…" />
          ) : q.isError ? (
            <ErrorState error={q.error} onRetry={() => q.refetch()} />
          ) : alerts.length === 0 ? (
            <div className="wd-empty">
              <span className="wd-empty-icon">
                <Binoculars size={56} strokeWidth={1.2} />
              </span>
              <h2>Nothing unusual right now</h2>
              <p>
                Watchdog compares the past day of request errors and latency, error logs and host CPU, memory and disk with their recent baseline. When something drifts out of its usual range, it
                shows up here.
              </p>
              <p className="muted-text">
                Watching {q.data.services} service{q.data.services === 1 ? '' : 's'} and {q.data.hosts} host{q.data.hosts === 1 ? '' : 's'}.
              </p>
            </div>
          ) : shown.length === 0 ? (
            <Empty>No alerts match these filters.</Empty>
          ) : (
            <div className="wd-list">
              {shown.map((a) => (
                <AlertCard key={a.id} alert={a} now={now} selected={a.id === openId} onOpen={() => setParam('alert', a.id)} />
              ))}
            </div>
          )}
        </main>
      </div>
      {opened && <AlertPanel alert={opened} alerts={alerts} now={now} onOpen={(id) => setParam('alert', id)} onClose={() => setParam('alert', null)} />}
    </div>
  );
}
