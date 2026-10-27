import { useCallback, useDeferredValue, useMemo, useState } from 'react';
import { useSearchParams } from 'react-router-dom';
import { Columns3, Download, EllipsisVertical, Hexagon, List, Megaphone, PanelLeft, PanelLeftClose, Search, Server, Sparkles, X } from 'lucide-react';
import type { HostSortField, HostSummary, MetricFilter } from '../../api/infrastructure/types';
import { TimePicker } from '../../components/shell/PageHeader';
import { TimeProvider, useTime } from '../../components/shell/TimeContext';
import { Empty, ErrorState, Loading } from '../../components/ui/AsyncState';
import { Select, usePopover } from '../../components/ui/Dropdown';
import { Pager } from '../../components/ui/Pager';
import { formatQuery, parseQuery, type Term } from '../../lib/searchQuery';
import { snapRange } from '../../lib/time';
import { HostFilters } from './HostFilters';
import { ALL_COLUMNS, HostTable, type HostColumn } from './HostTable';
import { HostPanel } from './panel/HostPanel';
import { HostMap } from './HostMap';
import { useHosts } from './queries';
import './infrastructure.css';

const PAGE_SIZE = 20;
/** Hosts the Host Map draws at once. */
const MAP_LIMIT = 1000;
const BANNER_KEY = 'oc.infra.bannerDismissed';

const GROUP_OPTIONS = [
  { value: '', label: 'Attributes or Tags' },
  { value: 'env', label: 'env' },
  { value: 'cloud', label: 'cloud' },
  { value: 'os', label: 'os' },
  { value: 'team', label: 'team' },
  { value: 'cluster', label: 'kube_cluster_name' },
  { value: 'region', label: 'region' },
  { value: 'collector', label: 'collector' },
];

function readBannerDismissed(): boolean {
  try {
    return localStorage.getItem(BANNER_KEY) === '1';
  } catch {
    return false;
  }
}

function toCsv(hosts: HostSummary[]): string {
  const rows = [['host', 'cloud', 'os', 'env', 'team', 'cpu_pct', 'mem_pct', 'monitors_alert', 'monitors_ok']].concat(
    hosts.map((h) => [h.name, h.cloud, h.os, h.env ?? '', h.team ?? '', String(h.cpuPct), String(h.memPct), String(h.monitors.alert), String(h.monitors.ok)]),
  );
  return rows.map((r) => r.join(',')).join('\n');
}

function ColumnsMenu({ value, onChange }: { value: Set<HostColumn>; onChange: (v: Set<HostColumn>) => void }) {
  const { open, setOpen, ref } = usePopover();
  return (
    <div className="dd" ref={ref}>
      <button className="link-btn ht-tool" onClick={() => setOpen((o) => !o)}>
        <Columns3 size={15} /> Columns
      </button>
      {open && (
        <div className="dd-menu dd-right options-menu-list">
          {ALL_COLUMNS.map((c) => (
            <label key={c.key}>
              <input
                type="checkbox"
                className="checkbox"
                checked={value.has(c.key)}
                onChange={() => {
                  const next = new Set(value);
                  if (next.has(c.key)) next.delete(c.key);
                  else next.add(c.key);
                  onChange(next);
                }}
              />
              {c.label}
            </label>
          ))}
        </div>
      )}
    </div>
  );
}

export function HostsPage() {
  return (
    <TimeProvider initial="15m">
      <HostsPageBody />
    </TimeProvider>
  );
}

function HostsPageBody() {
  const { range: live } = useTime();
  const { from, to } = snapRange(live);
  const range = useMemo(() => ({ from, to }), [from, to]);
  const [params, setParams] = useSearchParams();
  const query = params.get('q') ?? '';
  const groupBy = params.get('group');
  const [sortField, sortDir] = (params.get('sort') ?? 'cpu:desc').split(':') as [HostSortField, 'asc' | 'desc'];
  const offset = Number(params.get('offset') ?? 0);
  const openHost = params.get('host');
  const view = params.get('view') === 'map' ? 'map' : 'list';

  const setParam = useCallback(
    (updates: Record<string, string | null>, push = false) =>
      setParams(
        (p) => {
          const next = new URLSearchParams(p);
          for (const [k, v] of Object.entries(updates)) {
            if (v === null || v === '') next.delete(k);
            else next.set(k, v);
          }
          return next;
        },
        { replace: !push },
      ),
    [setParams],
  );

  const [draft, setDraft] = useState(query);
  const [metricFilters, setMetricFilters] = useState<MetricFilter[]>([{ metric: 'system.uptime' }, { metric: 'system.mem.pct_usable' }]);
  const [showFilters, setShowFilters] = useState(true);
  const [columns, setColumns] = useState<Set<HostColumn>>(() => new Set(ALL_COLUMNS.map((c) => c.key)));
  const [bannerHidden, setBannerHidden] = useState(readBannerDismissed);

  const terms = useMemo(() => parseQuery(query), [query]);
  const setTerms = useCallback((t: Term[]) => setParam({ q: formatQuery(t), offset: null }), [setParam]);

  // Only filters with a bound applied reach the backend.
  const activeFilters = useDeferredValue(metricFilters.filter((f) => f.min !== undefined || f.max !== undefined));
  const hostsQ = useHosts({
    range,
    query,
    metricFilters: activeFilters,
    groupBy,
    sort: { field: sortField, dir: sortDir },
    // The map shows every host at once.
    offset: view === 'map' ? 0 : offset,
    limit: view === 'map' ? MAP_LIMIT : PAGE_SIZE,
  });

  const onSort = (field: HostSortField) => {
    const dir = field === sortField ? (sortDir === 'desc' ? 'asc' : 'desc') : field === 'name' || field === 'env' ? 'asc' : 'desc';
    setParam({ sort: `${field}:${dir}`, offset: null });
  };

  const exportCsv = () => {
    const blob = new Blob([toCsv(hostsQ.data?.hosts ?? [])], { type: 'text/csv' });
    const a = document.createElement('a');
    a.href = URL.createObjectURL(blob);
    a.download = 'hosts.csv';
    a.click();
    URL.revokeObjectURL(a.href);
  };

  const page = hostsQ.data;
  const total = page?.total ?? 0;

  return (
    <div className="hosts-page">
      <header className="product-header">
        <div className="product-title">
          <Server size={20} /> Hosts
        </div>
        <div className="hosts-header-right">
          <button className="link-btn">
            <Megaphone size={15} /> Give Feedback
          </button>
        </div>
      </header>

      <div className="section-header et-section-header">
        <div className="section-header-left">
          <button className="page-tab et-views">
            <PanelLeft size={15} /> Views
          </button>
          <div className="page-tab active">
            My View <EllipsisVertical size={16} className="muted" />
          </div>
          <div className="hosts-tabs">
            <button className={`hosts-tab${view === 'list' ? ' active' : ''}`} onClick={() => setParam({ view: null })}>
              <List size={15} /> Hosts
            </button>
            <button className={`hosts-tab${view === 'map' ? ' active' : ''}`} onClick={() => setParam({ view: 'map', offset: null })}>
              <Hexagon size={15} /> Host Map
            </button>
          </div>
        </div>
        <TimePicker compact />
      </div>

      <div className="hosts-scroll">
        <div className="hosts-search">
          <label className="hosts-search-input">
            <Search size={15} />
            <input
              value={draft}
              placeholder="Search by tags or filters (e.g. env:production service:api)"
              onChange={(e) => setDraft(e.target.value)}
              onKeyDown={(e) => e.key === 'Enter' && setParam({ q: draft.trim(), offset: null })}
              onBlur={() => draft.trim() !== query && setParam({ q: draft.trim(), offset: null })}
            />
          </label>
          <span className="hosts-groupby-label">Group by</span>
          <Select value={groupBy ?? ''} options={GROUP_OPTIONS} onChange={(g) => setParam({ group: g || null, offset: null })} triggerClassName="dd-trigger hosts-groupby" align="right" />
        </div>

        {!bannerHidden && (
          <div className="hosts-banner">
            <div>
              <h3>
                <Sparkles size={18} /> Assist Infrastructure Operations Coming Soon!
              </h3>
              <p>Sign up to be part of the AI-assisted troubleshooting & remediation private preview for host and infrastructure issues.</p>
              <button className="btn">Join Preview</button>
            </div>
            <button
              className="hosts-banner-close"
              title="Dismiss"
              onClick={() => {
                setBannerHidden(true);
                try {
                  localStorage.setItem(BANNER_KEY, '1');
                } catch {
                  /* storage unavailable: dismiss for this session only */
                }
              }}
            >
              <X size={20} />
            </button>
          </div>
        )}

        <div className="hosts-body">
          {showFilters && (
            <HostFilters
              terms={terms}
              onTerms={(t) => {
                setDraft(formatQuery(t));
                setTerms(t);
              }}
              metricFilters={metricFilters}
              onMetricFilters={(f) => {
                setMetricFilters(f);
                setParam({ offset: null });
              }}
            />
          )}

          <div className="hosts-main">
            <div className="hosts-toolbar">
              <button className="link-btn" onClick={() => setShowFilters((s) => !s)}>
                {showFilters ? <PanelLeftClose size={15} /> : <PanelLeft size={15} />}
                {showFilters ? 'Hide Filters' : 'Show Filters'}
              </button>
              <span className="hosts-showing">
                Showing <strong>{total ? `${offset + 1}–${Math.min(offset + PAGE_SIZE, total)}` : 0}</strong> of <strong>{total.toLocaleString()}</strong> Hosts
              </span>
              <span className="hosts-toolbar-right">
                <button className="link-btn ht-tool" onClick={exportCsv}>
                  <Download size={15} /> Export
                </button>
                <ColumnsMenu value={columns} onChange={setColumns} />
              </span>
            </div>

            {hostsQ.isPending ? (
              <Loading label="Loading hosts…" />
            ) : hostsQ.isError ? (
              <ErrorState error={hostsQ.error} onRetry={() => hostsQ.refetch()} />
            ) : !page || page.hosts.length === 0 ? (
              <Empty>No hosts match these filters.</Empty>
            ) : (
              <div className={hostsQ.isPlaceholderData ? 'is-stale' : undefined}>
                {view === 'map' ? (
                  <HostMap hosts={page.hosts} selected={openHost} onOpen={(h) => setParam({ host: h }, true)} />
                ) : (
                  <>
                    <HostTable page={page} columns={columns} sort={{ field: sortField, dir: sortDir }} onSort={onSort} selected={openHost} onOpen={(h) => setParam({ host: h }, true)} />
                    <Pager offset={offset} limit={PAGE_SIZE} total={total} onChange={(o) => setParam({ offset: o ? String(o) : null })} />
                  </>
                )}
              </div>
            )}
          </div>
        </div>
      </div>

      {openHost && <HostPanel name={openHost} onClose={() => setParam({ host: null }, true)} />}
    </div>
  );
}
