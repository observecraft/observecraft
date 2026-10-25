import { Fragment, useCallback, useMemo, useState } from 'react';
import { keepPreviousData, useQuery } from '@tanstack/react-query';
import {
  ArrowDownWideNarrow,
  ArrowUpNarrowWide,
  ChevronDown,
  ChevronRight,
  Columns3,
  Cpu,
  Download,
  EllipsisVertical,
  LayoutGrid,
  List,
  Map as MapIcon,
  PanelLeft,
  PanelLeftClose,
  Pin,
  Search,
  Settings,
  Ship,
  Table2,
} from 'lucide-react';
import { useNavigate, useSearchParams } from 'react-router-dom';
import { searchKubernetes, type KubeKind, type KubeResource, type KubeSearchPage } from '../../api/kubernetes/client';
import type { Facet } from '../../api/types';
import { FacetPanel } from '../../components/facets/FacetPanel';
import { Empty, ErrorState, Loading } from '../../components/ui/AsyncState';
import { Menu, Select, usePopover } from '../../components/ui/Dropdown';
import { Pager } from '../../components/ui/Pager';
import { addTerm, formatQuery, parseQuery, type Term } from '../../lib/searchQuery';
import { COLUMNS, KIND_INFO, RESOURCE_TREE, TAG_LABELS, facetGroup, isEcs, logQuery, type CellContext, type ExplorerKind, type KubeColumn, type TreeNode } from './columns';
import { KubePanel } from './KubePanel';
import { LiveClock, LiveProvider, OrchestrationHeader, ProductIcon, useLive } from './OrchestrationHeader';
import { ResourceUtilization } from './ResourceUtilization';
import '../infrastructure/infrastructure.css';
import './kubernetes.css';

const PAGE_SIZE = 50;

type View = 'list' | 'map';
type Sub = 'inventory' | 'utilization' | 'gpu';
type MapFill = 'cpu' | 'memory' | 'status';

function useParamState() {
  const [params, setParams] = useSearchParams();
  const set = useCallback(
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
  return [params, set] as const;
}

/** Infrastructure → Kubernetes Explorer / ECS Explorer (Datadog's orchestrator explorer). */
export function OrchestrationExplorer() {
  return (
    <LiveProvider>
      <ExplorerBody />
    </LiveProvider>
  );
}

function ExplorerBody() {
  const [params, setParam] = useParamState();
  const kind = (params.get('resource') as ExplorerKind | null) ?? 'pods';
  const product = isEcs(kind) ? 'ecs' : 'kubernetes';
  const sub: Sub = isEcs(kind) ? 'inventory' : ((params.get('sub') as Sub | null) ?? 'inventory');
  const panel = params.get('panel');
  const [updatedAt, setUpdatedAt] = useState<number | undefined>();
  const filter = (k: string, v: string) => setParam({ q: addTerm(params.get('q') ?? '', k, v), panel: null, offset: null }, true);

  return (
    <div className="hosts-page ox-page">
      <OrchestrationHeader product={product} active="Explorer" />
      <div className="ox-subbar">
        <button className="link-btn ox-views">
          <PanelLeft size={15} /> Views
        </button>
        <span className="ox-myview">
          My View <EllipsisVertical size={15} className="muted" />
        </span>
        {product === 'kubernetes' && (
          <span className="ox-subtabs">
            <button className={`ox-subtab${sub === 'inventory' ? ' active' : ''}`} onClick={() => setParam({ sub: null })}>
              <Table2 size={15} /> Inventory
            </button>
            <button className={`ox-subtab${sub === 'utilization' ? ' active' : ''}`} onClick={() => setParam({ sub: 'utilization', panel: null })}>
              <LayoutGrid size={15} /> Resource Utilization
            </button>
            <button className={`ox-subtab${sub === 'gpu' ? ' active' : ''}`} onClick={() => setParam({ sub: 'gpu', panel: null })}>
              <Cpu size={15} /> GPU Workloads
            </button>
          </span>
        )}
        <span className="ox-header-right">
          <LiveClock updatedAt={updatedAt} />
        </span>
      </div>
      {sub === 'utilization' ? (
        <ResourceUtilization
          query={params.get('q') ?? ''}
          setQuery={(q) => setParam({ q })}
          res={params.get('ur') === 'memory' ? 'memory' : 'cpu'}
          setRes={(r) => setParam({ ur: r === 'cpu' ? null : r })}
        />
      ) : sub === 'gpu' ? (
        <div className="hosts-scroll">
          <Empty>No GPU workloads: none of the reported pods request GPUs (nvidia.com/gpu).</Empty>
        </div>
      ) : (
        <Inventory kind={kind} onUpdated={setUpdatedAt} filter={filter} />
      )}
      {panel && <KubePanel uid={panel} onClose={() => setParam({ panel: null }, true)} onFilter={filter} onOpen={(uid) => setParam({ panel: uid }, true)} />}
    </div>
  );
}

function TreeItem({ node, kind, depth, onSelect }: { node: TreeNode; kind: ExplorerKind; depth: number; onSelect: (k: ExplorerKind) => void }) {
  const contains = (n: TreeNode): boolean => n.kind === kind || (n.children ?? []).some(contains);
  const [open, setOpen] = useState(() => contains(node));
  const icon = node.label === 'ECS' ? <ProductIcon product="ecs" size={15} /> : depth > 0 || node.label === 'Kubernetes' ? <Ship size={14} /> : <Table2 size={14} />;
  if (node.children) {
    // Top-level products (Kubernetes, ECS) frame their resources; nested groups expand to the right in Datadog, inline here.
    const top = depth === 0;
    return (
      <div className={top && open ? 'ox-tree-product open' : 'ox-tree-group'}>
        <button className={`ox-tree-item${top && open ? ' product' : ''}`} style={{ paddingLeft: 10 + depth * 8 }} onClick={() => setOpen((o) => !o)}>
          {icon} <b>{node.label}</b>
          <span className="ox-tree-caret">{open ? <ChevronDown size={14} /> : <ChevronRight size={14} />}</span>
        </button>
        {open && node.children.map((c) => <TreeItem key={c.label} node={c} kind={kind} depth={depth + 1} onSelect={onSelect} />)}
      </div>
    );
  }
  return (
    <button className={`ox-tree-item${node.kind === kind ? ' active' : ''}`} style={{ paddingLeft: 10 + depth * 8 }} onClick={() => node.kind && onSelect(node.kind)}>
      {icon} {node.label}
    </button>
  );
}

function ResourceTree({ kind, onSelect }: { kind: ExplorerKind; onSelect: (k: ExplorerKind) => void }) {
  const [open, setOpen] = useState(true);
  return (
    <div className="ox-tree">
      <button className="ox-tree-head" onClick={() => setOpen((o) => !o)}>
        {open ? <ChevronDown size={14} /> : <ChevronRight size={14} />} Select Resources
      </button>
      {open && RESOURCE_TREE.map((n) => <TreeItem key={n.label} node={n} kind={kind} depth={0} onSelect={onSelect} />)}
    </div>
  );
}

function CustomizeMenu({ cols, hidden, onChange, trigger }: { cols: KubeColumn[]; hidden: Set<string>; onChange: (h: Set<string>) => void; trigger: React.ReactNode }) {
  const { open, setOpen, ref } = usePopover();
  return (
    <div className="dd" ref={ref}>
      <button className="btn ox-tool" onClick={() => setOpen((o) => !o)}>
        {trigger}
      </button>
      {open && (
        <div className="dd-menu dd-right options-menu-list">
          {cols.slice(1).map((c) => (
            <label key={c.key}>
              <input
                type="checkbox"
                className="checkbox"
                checked={!hidden.has(c.key)}
                onChange={() => {
                  const next = new Set(hidden);
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

function toCsv(cols: KubeColumn[], rows: KubeResource[]): string {
  const cell = (r: KubeResource, c: KubeColumn) => {
    const v =
      c.key === 'cpu'
        ? r.cpu?.usage
        : c.key === 'memory'
          ? r.memory?.usage
          : c.key === 'cluster'
            ? r.cluster
            : c.key === 'age'
              ? r.createdAt && new Date(r.createdAt).toISOString()
              : (r as unknown as Record<string, unknown>)[c.key];
    const s = v == null ? '' : String(v);
    return /[",\n]/.test(s) ? `"${s.replace(/"/g, '""')}"` : s;
  };
  return [cols.map((c) => c.label.toLowerCase()).join(','), ...rows.map((r) => cols.map((c) => cell(r, c)).join(','))].join('\n');
}

function Inventory({ kind, onUpdated, filter }: { kind: ExplorerKind; onUpdated: (t: number) => void; filter: (k: string, v: string) => void }) {
  const navigate = useNavigate();
  const [params, setParam] = useParamState();
  const { interval } = useLive();
  const query = params.get('q') ?? '';
  const groupBy = params.get('group') ?? '';
  const [sortField, sortDir] = (params.get('sort') ?? 'name:asc').split(':');
  const offset = Number(params.get('offset') ?? 0);
  const view: View = params.get('view') === 'map' ? 'map' : 'list';
  const panel = params.get('panel');
  const [draft, setDraft] = useState(query);
  const [showControls, setShowControls] = useState(true);
  const [teamsOpen, setTeamsOpen] = useState(false);
  const [fill, setFill] = useState<MapFill>('cpu');
  const [hidden, setHidden] = useState<Set<string>>(new Set());
  const apiKind = kind === 'processes' ? null : (kind as KubeKind);
  const info = KIND_INFO[kind];

  const q = useQuery({
    queryKey: ['kubernetes', 'search', kind, query, groupBy, sortField, sortDir, view === 'map' ? 0 : offset, view],
    queryFn: async () => {
      const page = await searchKubernetes({
        kind: apiKind!,
        query,
        groupBy: groupBy || null,
        sort: sortField,
        desc: sortDir === 'desc',
        offset: view === 'map' ? 0 : offset,
        limit: view === 'map' ? 1000 : PAGE_SIZE,
      });
      onUpdated(Date.now());
      return page;
    },
    enabled: !!apiKind,
    placeholderData: keepPreviousData,
    refetchInterval: interval,
  });

  const terms = useMemo(() => parseQuery(query), [query]);
  const setTerms = (t: Term[]) => {
    const next = formatQuery(t);
    setDraft(next);
    setParam({ q: next, offset: null });
  };
  const facets: Facet[] = useMemo(
    () =>
      (q.data?.facets ?? [])
        .map((f) => ({ key: f.key, label: TAG_LABELS[f.key] ?? f.key, group: facetGroup(f.key), values: f.values }))
        .sort((a, b) => (a.group === b.group ? 0 : a.group === 'CONDITIONS' ? -1 : 1)),
    [q.data],
  );
  const groupOptions = useMemo(
    () => [
      { value: '', label: `Group your ${info.label}` },
      ...[...new Set([...(groupBy ? [groupBy] : []), 'status', ...(q.data?.tagKeys ?? []).filter((k) => !k.includes('/') || k.startsWith('app.kubernetes.io/'))])].map((k) => ({
        value: k,
        label: TAG_LABELS[k] ? `${TAG_LABELS[k]} (${k})` : k,
      })),
    ],
    [q.data, groupBy, info.label],
  );
  const allCols = apiKind ? COLUMNS[apiKind] : [];
  const cols = allCols.filter((c) => !hidden.has(c.key));
  const onSort = (c: KubeColumn) => {
    if (!c.sort) return;
    const dir = c.sort === sortField ? (sortDir === 'desc' ? 'asc' : 'desc') : ['cpu', 'memory', 'restarts', 'pods'].includes(c.sort) ? 'desc' : 'asc';
    setParam({ sort: `${c.sort}:${dir}`, offset: null });
  };
  const select = (k: ExplorerKind) => (setDraft(''), setParam({ resource: k, q: null, group: null, sort: null, offset: null, panel: null }, true));
  const total = q.data?.total ?? 0;
  const ctx: CellContext = { filter };
  const exportCsv = () => {
    const blob = new Blob([toCsv(cols, q.data?.items ?? [])], { type: 'text/csv' });
    const a = document.createElement('a');
    a.href = URL.createObjectURL(blob);
    a.download = `${kind}.csv`;
    a.click();
    URL.revokeObjectURL(a.href);
  };

  return (
    <div className="hosts-scroll ox-scroll">
      <div className="hosts-search ox-search">
        <span className="hosts-groupby-label ox-search-label">Filter by</span>
        <label className="hosts-search-input">
          <Search size={15} />
          <input
            value={draft}
            placeholder={`Filter your ${info.label}`}
            onChange={(e) => setDraft(e.target.value)}
            onKeyDown={(e) => e.key === 'Enter' && setParam({ q: draft.trim(), offset: null })}
            onBlur={() => draft.trim() !== query && setParam({ q: draft.trim(), offset: null })}
          />
        </label>
        <span className="hosts-groupby-label">Group by</span>
        <Select value={groupBy} options={groupOptions} onChange={(g) => setParam({ group: g || null, offset: null })} triggerClassName="dd-trigger hosts-groupby ox-groupby" align="right" />
        <CustomizeMenu cols={allCols} hidden={hidden} onChange={setHidden} trigger={<Settings size={16} />} />
      </div>

      <div className="hosts-body">
        {showControls && (
          <aside className="ox-controls">
            <div className="ox-viewtoggle">
              <button className={view === 'list' ? 'active' : ''} onClick={() => setParam({ view: null })}>
                <List size={15} /> List
              </button>
              <button className={view === 'map' ? 'active' : ''} onClick={() => setParam({ view: 'map', offset: null })}>
                <MapIcon size={15} /> Map
              </button>
            </div>
            <div className="ox-teams">
              <button onClick={() => setTeamsOpen((o) => !o)}>{teamsOpen ? <ChevronDown size={14} /> : <ChevronRight size={14} />} Teams</button>
              <Pin size={14} className="muted" />
            </div>
            {teamsOpen && (
              <p className="muted small ox-teams-body">
                No team tags on these resources. Add a <code>team</code> label to filter by team.
              </p>
            )}
            <ResourceTree kind={kind} onSelect={select} />
            {apiKind && (
              <FacetPanel
                facets={facets}
                terms={terms}
                onChange={setTerms}
                defaultOpen={['status', 'kube_cluster_name', 'kube_namespace', 'task_launch_type', 'ecs_cluster']}
                verified={[]}
                valueSearch
              />
            )}
          </aside>
        )}
        <div className="hosts-main">
          <div className="hosts-toolbar">
            <button className="link-btn" onClick={() => setShowControls((s) => !s)}>
              {showControls ? <PanelLeftClose size={15} /> : <PanelLeft size={15} />}
              {showControls ? 'Hide Controls' : 'Show Controls'}
            </button>
            <span className="hosts-showing">
              {view === 'list' ? (
                <>
                  Showing <strong>{total ? `${offset + 1}–${Math.min(offset + PAGE_SIZE, total)}` : 0}</strong> of <strong>{total.toLocaleString()}</strong> {info.label}
                </>
              ) : (
                <>
                  <strong>{total.toLocaleString()}</strong> {info.label}
                </>
              )}
            </span>
            <span className="hosts-toolbar-right">
              {view === 'map' && (
                <Select
                  value={fill}
                  options={[
                    { value: 'cpu', label: 'Fill by: CPU usage/limits' },
                    { value: 'memory', label: 'Fill by: Memory usage/limits' },
                    { value: 'status', label: 'Fill by: Status' },
                  ]}
                  onChange={(v) => setFill(v as MapFill)}
                  triggerClassName="dd-trigger"
                  align="right"
                />
              )}
              <button className="btn ox-tool" onClick={exportCsv} disabled={!q.data?.items.length}>
                <Download size={15} /> Export
              </button>
              <CustomizeMenu
                cols={allCols}
                hidden={hidden}
                onChange={setHidden}
                trigger={
                  <>
                    <Columns3 size={15} /> Customize
                  </>
                }
              />
            </span>
          </div>

          {!apiKind ? (
            <Empty>Process collection is not enabled: the ObserveCraft collectors don't report processes running in containers yet.</Empty>
          ) : q.isPending ? (
            <Loading label={`Loading ${info.label.toLowerCase()}…`} />
          ) : q.isError ? (
            <ErrorState error={q.error} onRetry={() => q.refetch()} />
          ) : q.data.items.length === 0 ? (
            <Empty>
              {query
                ? `No ${info.label.toLowerCase()} match these filters.`
                : isEcs(kind)
                  ? 'No ECS data yet. Add the ADOT collector sidecar (deploy/examples/ecs) to your task definitions.'
                  : `No ${info.label.toLowerCase()} reported. Install the ObserveCraft collectors (deploy/examples/kubernetes) to see your clusters here.`}
            </Empty>
          ) : view === 'map' ? (
            <ClusterMap page={q.data} fill={fill} groupBy={groupBy} onOpen={(r) => setParam({ panel: r.uid }, true)} selected={panel} />
          ) : (
            <div className={q.isPlaceholderData ? 'is-stale' : undefined}>
              <div className="kx-table-wrap">
                <table className="host-table ox-table">
                  <thead>
                    <tr>
                      {cols.map((c) => (
                        <th key={c.key} className={`${c.sort ? 'sortable' : ''}${c.num ? ' num' : ''}`} onClick={() => onSort(c)}>
                          {c.sort === sortField && (sortDir === 'desc' ? <ArrowDownWideNarrow size={13} /> : <ArrowUpNarrowWide size={13} />)} {c.label}
                        </th>
                      ))}
                      <th />
                    </tr>
                  </thead>
                  <tbody>
                    <Rows page={q.data} cols={cols} groupBy={groupBy} kind={apiKind} selected={panel} ctx={ctx} onOpen={(r) => setParam({ panel: r.uid }, true)} navigate={navigate} />
                  </tbody>
                </table>
              </div>
              <Pager offset={offset} limit={PAGE_SIZE} total={total} onChange={(o) => setParam({ offset: o ? String(o) : null })} />
            </div>
          )}
        </div>
      </div>
    </div>
  );
}

function Rows({
  page,
  cols,
  groupBy,
  kind,
  selected,
  ctx,
  onOpen,
  navigate,
}: {
  page: KubeSearchPage;
  cols: KubeColumn[];
  groupBy: string;
  kind: KubeKind;
  selected: string | null;
  ctx: CellContext;
  onOpen: (r: KubeResource) => void;
  navigate: (to: string) => void;
}) {
  const groupOf = (r: KubeResource) => {
    if (!groupBy) return '';
    if (groupBy === 'status') return r.status;
    const t = r.tags.find((x) => x.startsWith(`${groupBy}:`));
    return t ? t.slice(groupBy.length + 1) : `no ${groupBy}`;
  };
  const counts = new Map((page.groups ?? []).map((g) => [g.key, g.count]));
  let last: string | null = null;
  return (
    <>
      {page.items.map((r) => {
        const g = groupOf(r);
        const header = groupBy && g !== last;
        last = g;
        const logs = logQuery(r);
        return (
          <Fragment key={r.uid}>
            {header && (
              <tr className="ht-group">
                <td colSpan={cols.length + 1}>
                  {TAG_LABELS[groupBy] ?? groupBy}: {g} <span className="muted">({counts.get(g) ?? ''})</span>
                </td>
              </tr>
            )}
            <tr className={selected === r.uid ? 'selected' : undefined} onClick={() => onOpen(r)}>
              {cols.map((c) => (
                <td key={c.key} className={c.num ? 'num' : undefined}>
                  {c.render(r, ctx)}
                </td>
              ))}
              <td className="ox-row-menu" onClick={(e) => e.stopPropagation()}>
                <Menu
                  trigger={<EllipsisVertical size={15} />}
                  triggerClassName="icon-plain"
                  items={[
                    { label: `View ${KIND_INFO[kind].singular} details`, onClick: () => onOpen(r) },
                    ...(logs ? [{ label: 'View related logs', onClick: () => navigate(`/logs?query=${encodeURIComponent(logs)}`) }] : []),
                    { label: 'Copy name', onClick: () => void navigator.clipboard.writeText(r.name) },
                  ]}
                />
              </td>
            </tr>
          </Fragment>
        );
      })}
    </>
  );
}

function cellClass(r: KubeResource, fill: MapFill): { cls: string; pct: number | null } {
  if (fill === 'status') return { cls: `kx-cell-${r.health}`, pct: null };
  const u = fill === 'cpu' ? r.cpu : r.memory;
  const ref = u?.limits ?? u?.capacity ?? u?.requests;
  if (u?.usage == null || !ref) return { cls: 'kx-cell-none', pct: null };
  const pct = (u.usage / ref) * 100;
  return { cls: pct >= 100 ? 'kx-cell-hot' : pct >= 80 ? 'kx-cell-warm' : pct >= 50 ? 'kx-cell-mid' : pct >= 20 ? 'kx-cell-low' : 'kx-cell-idle', pct };
}

function ClusterMap({ page, fill, groupBy, onOpen, selected }: { page: KubeSearchPage; fill: MapFill; groupBy: string; onOpen: (r: KubeResource) => void; selected: string | null }) {
  const groups = useMemo(() => {
    const m = new Map<string, KubeResource[]>();
    for (const r of page.items) {
      const t = groupBy ? (groupBy === 'status' ? r.status : r.tags.find((x) => x.startsWith(`${groupBy}:`))?.slice(groupBy.length + 1)) : '';
      const k = t ?? `no ${groupBy}`;
      m.set(k, [...(m.get(k) ?? []), r]);
    }
    return [...m];
  }, [page.items, groupBy]);
  return (
    <div className="kx-map">
      <div className="kx-map-legend">
        {fill === 'status' ? (
          (['ok', 'warning', 'error', 'info'] as const).map((h) => (
            <span key={h}>
              <i className={`kx-cell-${h}`} /> {h === 'ok' ? 'OK' : h === 'info' ? 'Completed / other' : h.charAt(0).toUpperCase() + h.slice(1)}
            </span>
          ))
        ) : (
          <>
            {[
              ['idle', '< 20%'],
              ['low', '20–50%'],
              ['mid', '50–80%'],
              ['warm', '80–100%'],
              ['hot', '≥ 100%'],
              ['none', 'no data'],
            ].map(([c, l]) => (
              <span key={c}>
                <i className={`kx-cell-${c}`} /> {l}
              </span>
            ))}
            <span className="muted">of limits (allocatable for nodes)</span>
          </>
        )}
      </div>
      {groups.map(([key, rows]) => (
        <section key={key} className="kx-map-group">
          {groupBy && (
            <h4>
              {TAG_LABELS[groupBy] ?? groupBy}: {key} <span className="muted">({rows.length})</span>
            </h4>
          )}
          <div className="kx-hexes">
            {rows.map((r) => {
              const { cls, pct } = cellClass(r, fill);
              return (
                <button
                  key={r.uid}
                  className={`kx-hex ${cls}${selected === r.uid ? ' selected' : ''}`}
                  title={`${r.name}\n${r.status}${pct != null ? `\n${Math.round(pct)}% of ${fill === 'cpu' ? 'CPU' : 'memory'} limits` : ''}`}
                  onClick={() => onOpen(r)}
                />
              );
            })}
          </div>
        </section>
      ))}
    </div>
  );
}
