import { Fragment, useCallback, useEffect, useMemo, useState } from 'react';
import { useNavigate, useSearchParams } from 'react-router-dom';
import {
  ArrowDownWideNarrow,
  ArrowUpNarrowWide,
  Box,
  Boxes,
  ChevronDown,
  Columns3,
  Container,
  Download,
  ExternalLink,
  Globe,
  FolderTree,
  Layers,
  Network,
  Plus,
  Search,
  Server,
  Ship,
  X,
} from 'lucide-react';
import type { CatalogCategory, CatalogFacet, CatalogGroup, CatalogResource, CatalogType } from '../../api/catalog/client';
import type { Cloud } from '../../api/infrastructure/types';
import { QueryBar } from '../../components/query/QueryBar';
import { Empty, ErrorState, Loading } from '../../components/ui/AsyncState';
import { Menu, Select, usePopover } from '../../components/ui/Dropdown';
import { Pager } from '../../components/ui/Pager';
import { formatLongDate, timeAgo } from '../../lib/format';
import { addTerm, clearKey, formatQuery, parseQuery, selectedValues, toggleValue, type Term } from '../../lib/searchQuery';
import { StatusPill } from '../kubernetes/columns';
import { CATEGORY_LABELS, catalogLink, facetLabel, TYPE_LABELS } from './catalogModel';
import { CLOUD_LABEL, CloudIcon } from './hostIcons';
import { useCatalog } from './queries';
import './infrastructure.css';
import '../kubernetes/kubernetes.css';

const PAGE_SIZE = 50;
const HOUR = 3600e3;
const CLOUDS = new Set(Object.keys(CLOUD_LABEL));
/** Filter dropdowns under the search bar; "+ Filter" adds the others. */
const FILTERS = ['team', 'resource_type', 'cloud', 'region', 'env', 'account', 'service'];
const MORE_FILTERS = ['resource_category', 'cluster', 'namespace'];
const GROUP_OPTIONS = [
  { value: 'none', label: 'None' },
  { value: 'resource_category', label: 'Resource Category' },
  { value: 'resource_type', label: 'Resource Type' },
  { value: 'cloud', label: 'Cloud Provider' },
  { value: 'account', label: 'Account' },
  { value: 'region', label: 'Region' },
  { value: 'cluster', label: 'Cluster' },
  { value: 'env', label: 'Environment' },
  { value: 'service', label: 'Service' },
  { value: 'team', label: 'Team' },
];
const ATTRIBUTE_LABELS: Record<string, string> = {
  os: 'OS',
  distro: 'Distribution',
  collector: 'Collector',
  cpu_pct: 'CPU %',
  mem_pct: 'Memory %',
  apps: 'Integrations',
  node: 'Node',
  ready: 'Ready',
  owner: 'Owner',
  images: 'Images',
  version: 'Version',
  instance_type: 'Instance type',
  availability_zone: 'Availability zone',
  arch: 'Architecture',
  ip: 'IP',
  service_type: 'Service type',
  task_definition: 'Task definition',
  launch_type: 'Launch type',
  nodes: 'Nodes',
  pods: 'Pods',
  restarts: 'Restarts',
};

type SortField = 'type' | 'name' | 'cluster' | 'region' | 'last_seen';
type Column = 'cluster' | 'region' | 'env' | 'service' | 'team' | 'last_seen';
const COLUMNS: { key: Column; label: string }[] = [
  { key: 'cluster', label: 'Cluster / Account' },
  { key: 'region', label: 'Region' },
  { key: 'env', label: 'Environment' },
  { key: 'service', label: 'Service' },
  { key: 'team', label: 'Team' },
  { key: 'last_seen', label: 'Last seen' },
];

export function TypeIcon({ type, size = 15 }: { type: CatalogType; size?: number }) {
  const Icon =
    type === 'host' || type === 'kube_node'
      ? Server
      : type === 'kube_cluster' || type === 'ecs_cluster'
        ? Ship
        : type === 'kube_namespace'
          ? FolderTree
          : type === 'kube_pod' || type === 'ecs_task'
            ? Box
            : type === 'container'
              ? Container
              : type === 'kube_service' || type === 'kube_ingress' || type === 'ecs_service'
                ? Network
                : type === 'kube_persistent_volume' || type === 'kube_persistent_volume_claim'
                  ? Layers
                  : Boxes;
  return <Icon size={size} className="rc-type-icon" aria-hidden />;
}

/** "1 resource category", "12 regions": the number of groups in the grouped view. */
function groupCount(groupBy: string, n: number): string {
  const one = facetLabel(groupBy).toLowerCase();
  const many = one.endsWith('y') ? `${one.slice(0, -1)}ies` : `${one}s`;
  return `${n.toLocaleString()} ${n === 1 ? one : many}`;
}

function CategoryIcon({ category }: { category: string }) {
  const Icon = category === 'kubernetes' ? Ship : category === 'containers' ? Container : category === 'compute' ? Server : Boxes;
  return <Icon size={16} className="rc-type-icon" aria-hidden />;
}

/** A facet value as people read it (type and category labels). */
function valueLabel(key: string, value: string): string {
  if (key === 'resource_type') return TYPE_LABELS[value as CatalogType] ?? value;
  if (key === 'resource_category') return CATEGORY_LABELS[value as CatalogCategory] ?? value;
  if (key === 'cloud') return CLOUD_LABEL[value as Cloud] ?? value;
  return value;
}

const chip = (v: string | null | undefined) => (v ? <span className="tag-chip">{v}</span> : <span className="muted-text">—</span>);
const CHIP_LIMIT = 3;
const chips = (values: string[], icon?: boolean) =>
  values.length ? (
    <span className="rc-chips">
      {values.slice(0, CHIP_LIMIT).map((v) => (
        <span key={v} className={`tag-chip${icon ? ' rc-pill' : ''}`}>
          {icon && <Globe size={13} />}
          {v}
        </span>
      ))}
      {values.length > CHIP_LIMIT && (
        <span className="rc-more" title={values.slice(CHIP_LIMIT).join(', ')}>
          +{values.length - CHIP_LIMIT}
        </span>
      )}
    </span>
  ) : (
    <span className="muted-text">—</span>
  );

function toCsv(items: CatalogResource[]): string {
  const esc = (v: string | null | undefined) => (v && /[",\n]/.test(v) ? `"${v.replace(/"/g, '""')}"` : (v ?? ''));
  const rows = [['category', 'type', 'name', 'cloud', 'account', 'region', 'cluster', 'namespace', 'env', 'service', 'team', 'last_seen']].concat(
    items.map((r) => [
      r.resourceCategory,
      r.resourceType,
      r.name,
      r.cloud ?? '',
      r.account ?? '',
      r.region ?? '',
      r.cluster ?? '',
      r.namespace ?? '',
      r.env ?? '',
      r.service ?? '',
      r.team ?? '',
      new Date(r.lastSeen).toISOString(),
    ]),
  );
  return rows.map((r) => r.map(esc).join(',')).join('\n');
}

/** One filter of the bar under the search: the facet's values with counts, checked from the query. */
function FilterDropdown({ facetKey, facet, terms, onChange, onRemove }: { facetKey: string; facet?: CatalogFacet; terms: Term[]; onChange: (t: Term[]) => void; onRemove?: () => void }) {
  const { open, setOpen, ref } = usePopover();
  const [filter, setFilter] = useState('');
  const selected = selectedValues(terms)[facetKey] ?? [];
  const values = (facet?.values ?? []).filter((v) => valueLabel(facetKey, v.value).toLowerCase().includes(filter.toLowerCase()));
  const label = facetLabel(facetKey);
  return (
    <div className="dd dd-labelled rc-filter-dd" ref={ref}>
      <span className="dd-floating-label">{label}</span>
      <button className={`dd-trigger rc-filter${selected.length ? ' active' : ''}`} onClick={() => setOpen((o) => !o)}>
        <span className="dd-trigger-label">{selected.length === 1 ? valueLabel(facetKey, selected[0]) : selected.length > 1 ? `${selected.length} selected` : 'All'}</span>
        <ChevronDown size={14} className="dd-caret" />
      </button>
      {open && (
        <div className="dd-menu rc-filter-menu">
          <label className="rc-filter-search">
            <Search size={13} />
            <input autoFocus placeholder={`Filter ${label.toLowerCase()}`} value={filter} onChange={(e) => setFilter(e.target.value)} />
          </label>
          <div className="options-menu-list rc-filter-values">
            {values.length === 0 && <div className="muted-text">No values.</div>}
            {values.map((v) => (
              <label key={v.value}>
                <input type="checkbox" className="checkbox" checked={selected.includes(v.value)} onChange={() => onChange(toggleValue(terms, facetKey, v.value))} />
                {facetKey === 'resource_type' && <TypeIcon type={v.value as CatalogType} size={13} />}
                <span className="rc-filter-value">{valueLabel(facetKey, v.value)}</span>
                <span className="muted-text">{v.count.toLocaleString()}</span>
              </label>
            ))}
          </div>
          <div className="rc-filter-actions">
            <button className="link-btn" disabled={!selected.length} onClick={() => onChange(clearKey(terms, facetKey))}>
              Clear
            </button>
            {onRemove && (
              <button className="link-btn" onClick={onRemove}>
                Remove filter
              </button>
            )}
          </div>
        </div>
      )}
    </div>
  );
}

function GroupTable({ groups, groupBy, onOpen }: { groups: CatalogGroup[]; groupBy: string; onOpen: (g: CatalogGroup) => void }) {
  return (
    <table className="host-table rc-table">
      <thead>
        <tr>
          <th>{facetLabel(groupBy).toUpperCase()}</th>
          <th className="num">RESOURCE COUNT</th>
          <th>REGIONS</th>
          <th>ENVIRONMENTS</th>
          <th>TEAMS</th>
          <th>SERVICES</th>
        </tr>
      </thead>
      <tbody>
        {groups.map((g) => (
          <tr key={g.key} onClick={() => onOpen(g)} title="Show these resources">
            <td className="rc-group-name">
              {groupBy === 'resource_type' && g.key in TYPE_LABELS && <TypeIcon type={g.key as CatalogType} size={16} />}
              {groupBy === 'resource_category' && <CategoryIcon category={g.key} />}
              {valueLabel(groupBy, g.key)}
            </td>
            <td className="num">{g.count.toLocaleString()}</td>
            <td>{chips(g.regions, true)}</td>
            <td>{chips(g.envs)}</td>
            <td>{chips(g.teams)}</td>
            <td>{chips(g.services)}</td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}

function ColumnsMenu({ value, onChange }: { value: Set<Column>; onChange: (v: Set<Column>) => void }) {
  const { open, setOpen, ref } = usePopover();
  return (
    <div className="dd" ref={ref}>
      <button className="link-btn ht-tool" onClick={() => setOpen((o) => !o)}>
        <Columns3 size={15} /> Table Options
      </button>
      {open && (
        <div className="dd-menu dd-right options-menu-list">
          {COLUMNS.map((c) => (
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

function CatalogPanel({ resource: r, onClose, onFilter }: { resource: CatalogResource; onClose: () => void; onFilter: (k: string, v: string) => void }) {
  const navigate = useNavigate();
  const link = catalogLink(r);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === 'Escape' && !(e.target as HTMLElement).closest('input, textarea') && onClose();
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [onClose]);

  const field = (label: string, key: string, value: string | null | undefined) => (
    <>
      <dt>{label}</dt>
      <dd>{value ? <button onClick={() => onFilter(key, value)}>{valueLabel(key, value)}</button> : '—'}</dd>
    </>
  );
  const attributes = Object.entries(r.attributes);
  return (
    <aside className="rc-panel" role="dialog" aria-label={`${TYPE_LABELS[r.resourceType]} ${r.name}`}>
      <header>
        <TypeIcon type={r.resourceType} size={18} />
        <span>
          {CATEGORY_LABELS[r.resourceCategory]} · {TYPE_LABELS[r.resourceType]}
        </span>
        <button className="rc-panel-close" onClick={onClose} title="Close">
          <X size={16} />
        </button>
      </header>
      <h2>{r.name}</h2>
      <div className="rc-panel-actions">
        {r.status && r.health && <StatusPill status={r.status} health={r.health} />}
        {link && (
          <button className="btn" onClick={() => navigate(link)}>
            <ExternalLink size={14} /> {r.resourceType === 'host' ? 'Open host' : r.kubeKind?.startsWith('ecs_') ? 'Open in ECS Explorer' : 'Open in Kubernetes Explorer'}
          </button>
        )}
      </div>
      <h3>Overview</h3>
      <dl>
        {field('Cloud', 'cloud', r.cloud)}
        {field('Account', 'account', r.account)}
        {field('Region', 'region', r.region)}
        {field('Cluster', 'cluster', r.cluster)}
        {r.namespace && field('Namespace', 'namespace', r.namespace)}
        {field('Environment', 'env', r.env)}
        {field('Service', 'service', r.service)}
        {field('Team', 'team', r.team)}
        {r.createdAt && (
          <>
            <dt>Created</dt>
            <dd>{formatLongDate(r.createdAt)}</dd>
          </>
        )}
        <dt>Last seen</dt>
        <dd title={formatLongDate(r.lastSeen)}>{timeAgo(r.lastSeen)}</dd>
      </dl>
      {attributes.length > 0 && (
        <>
          <h3>Attributes</h3>
          <dl>
            {attributes.map(([k, v]) => (
              <Fragment key={k}>
                <dt>{ATTRIBUTE_LABELS[k] ?? k}</dt>
                <dd className="rc-wrap">{v}</dd>
              </Fragment>
            ))}
          </dl>
        </>
      )}
      <h3>Tags</h3>
      {r.tags.length ? (
        <div className="rc-tags">
          {r.tags.map((t) => {
            const i = t.indexOf(':');
            return (
              <button key={t} className="tag-chip" title="Filter by this tag" onClick={() => i > 0 && onFilter(t.slice(0, i), t.slice(i + 1))}>
                {t}
              </button>
            );
          })}
        </div>
      ) : (
        <p className="muted-text">No tags.</p>
      )}
    </aside>
  );
}

/** Infrastructure → Resources → Catalog: every host, Kubernetes and ECS resource in one searchable list. */
export function ResourceCatalogPage() {
  // Hosts that reported in the last hour; Kubernetes and ECS resources are current.
  const range = useMemo(() => {
    const to = Math.floor(Date.now() / 60e3) * 60e3;
    return { from: to - HOUR, to };
  }, []);
  const [params, setParams] = useSearchParams();
  const query = params.get('q') ?? '';
  const groupBy = params.get('group') ?? 'resource_category';
  const grouped = groupBy !== 'none';
  const [sortField, sortDir] = (params.get('sort') ?? 'name:asc').split(':') as [SortField, 'asc' | 'desc'];
  const offset = Number(params.get('offset') ?? 0);
  const [added, setAdded] = useState<string[]>([]);
  const [columns, setColumns] = useState<Set<Column>>(() => new Set(COLUMNS.map((c) => c.key)));
  const [selected, setSelected] = useState<CatalogResource | null>(null);

  const setParam = useCallback(
    (updates: Record<string, string | null>) =>
      setParams(
        (p) => {
          const next = new URLSearchParams(p);
          for (const [k, v] of Object.entries(updates)) {
            if (v === null || v === '') next.delete(k);
            else next.set(k, v);
          }
          return next;
        },
        { replace: true },
      ),
    [setParams],
  );
  const terms = useMemo(() => parseQuery(query), [query]);
  const setTerms = useCallback((t: Term[]) => setParam({ q: formatQuery(t), offset: null }), [setParam]);
  const addFilter = useCallback((k: string, v: string) => setParam({ q: addTerm(query, k, v), offset: null }), [setParam, query]);

  const q = useCatalog({ range, query, groupBy: grouped ? groupBy : null, sort: sortField, desc: sortDir === 'desc', offset, limit: PAGE_SIZE });
  const page = q.data;
  const total = page?.total ?? 0;
  const facet = (key: string) => page?.facets.find((f) => f.key === key);
  // Filters for keys the query uses stay visible.
  const used = Object.keys(selectedValues(terms));
  const extra = [...new Set([...added, ...used.filter((k) => MORE_FILTERS.includes(k))])];
  const category = selectedValues(terms).resource_category;
  const noun = category?.length === 1 ? (CATEGORY_LABELS[category[0] as CatalogCategory] ?? 'Resources') : 'Resources';

  const onSort = (field: SortField) => {
    const dir = field === sortField ? (sortDir === 'desc' ? 'asc' : 'desc') : field === 'last_seen' ? 'desc' : 'asc';
    setParam({ sort: `${field}:${dir}`, offset: null });
  };
  const SortIcon = sortDir === 'desc' ? ArrowDownWideNarrow : ArrowUpNarrowWide;
  const th = (label: string, field?: SortField) => (
    <th className={field ? 'sortable' : undefined} onClick={field ? () => onSort(field) : undefined}>
      {field && sortField === field && <SortIcon size={13} />} {label}
    </th>
  );
  const openGroup = (g: CatalogGroup) => {
    const missing = g.key === `no ${groupBy}`;
    const next = missing ? formatQuery([...clearKey(terms, groupBy), { kind: 'tag', key: groupBy, values: ['*'], negate: true }]) : addTerm(query, groupBy, g.key);
    setParam({ q: next, group: 'none', offset: null });
  };

  const exportCsv = () => {
    const blob = new Blob([toCsv(page?.items ?? [])], { type: 'text/csv' });
    const a = document.createElement('a');
    a.href = URL.createObjectURL(blob);
    a.download = 'resources.csv';
    a.click();
    URL.revokeObjectURL(a.href);
  };

  return (
    <div className="rc-page">
      <div className="hosts-scroll">
        <h2 className="rc-heading">Infrastructure</h2>
        <div className="rc-searchbox">
          <QueryBar value={query} onChange={(v) => setParam({ q: v, offset: null })} placeholder="Search by resource name, tags or filters (e.g. resource_category:kubernetes env:prod)" />

          <div className="rc-filters">
            {FILTERS.map((k) => (
              <FilterDropdown key={k} facetKey={k} facet={facet(k)} terms={terms} onChange={setTerms} />
            ))}
            {extra.map((k) => (
              <FilterDropdown
                key={k}
                facetKey={k}
                facet={facet(k)}
                terms={terms}
                onChange={setTerms}
                onRemove={() => {
                  setAdded((a) => a.filter((x) => x !== k));
                  setTerms(clearKey(terms, k));
                }}
              />
            ))}
            {MORE_FILTERS.some((k) => !extra.includes(k)) && (
              <Menu
                align="left"
                triggerClassName="rc-filter rc-filter-add"
                trigger={
                  <>
                    <Plus size={15} /> Filter
                  </>
                }
                items={MORE_FILTERS.filter((k) => !extra.includes(k)).map((k) => ({ label: facetLabel(k), onClick: () => setAdded((a) => [...a, k]) }))}
              />
            )}
          </div>
        </div>
        <div className="rc-groupby">
          <Select label="Group by" value={groupBy} options={GROUP_OPTIONS} onChange={(g) => setParam({ group: g === 'resource_category' ? null : g, offset: null })} />
        </div>

        <div className="hosts-toolbar">
          <span className="rc-count">{grouped && page?.groups ? groupCount(groupBy, page.groups.length) : `${total.toLocaleString()} ${noun}`}</span>
          <span className="hosts-toolbar-right">
            <button className="link-btn ht-tool" onClick={exportCsv} disabled={grouped}>
              <Download size={15} /> Export as CSV
            </button>
            {!grouped && <ColumnsMenu value={columns} onChange={setColumns} />}
          </span>
        </div>

        {q.isPending ? (
          <Loading label="Loading resources…" />
        ) : q.isError ? (
          <ErrorState error={q.error} onRetry={() => q.refetch()} />
        ) : !page || total === 0 ? (
          <Empty>No resources match this search. Hosts appear once they report host metrics; Kubernetes and ECS resources once a collector sends them.</Empty>
        ) : (
          <div className={q.isPlaceholderData ? 'is-stale' : undefined}>
            {grouped && page.groups ? (
              <GroupTable groups={page.groups} groupBy={groupBy} onOpen={openGroup} />
            ) : (
              <>
                <table className="host-table rc-table">
                  <thead>
                    <tr>
                      {th('RESOURCE', 'name')}
                      {columns.has('cluster') && th('CLUSTER / ACCOUNT', 'cluster')}
                      {columns.has('region') && th('REGION', 'region')}
                      {columns.has('env') && th('ENVIRONMENT')}
                      {columns.has('service') && th('SERVICE')}
                      {columns.has('team') && th('TEAM')}
                      {columns.has('last_seen') && th('LAST SEEN', 'last_seen')}
                    </tr>
                  </thead>
                  <tbody>
                    {page.items.map((r) => (
                      <tr key={r.id} className={selected?.id === r.id ? 'selected' : undefined} onClick={() => setSelected(r)}>
                        <td className="rc-resource">
                          <TypeIcon type={r.resourceType} size={18} />
                          <span>
                            <span className="rc-name" title={r.name}>
                              {r.name}
                            </span>
                            <span className="rc-subtype">
                              {TYPE_LABELS[r.resourceType]}
                              {r.namespace && ` · ${r.namespace}`}
                            </span>
                          </span>
                        </td>
                        {columns.has('cluster') && (
                          <td>
                            {r.cluster || r.account ? (
                              <span className="rc-cloud">
                                {r.cloud && CLOUDS.has(r.cloud) && <CloudIcon cloud={r.cloud as Cloud} size={14} />}
                                {[r.cluster, r.account].filter(Boolean).join(' · ')}
                              </span>
                            ) : (
                              <span className="muted-text">—</span>
                            )}
                          </td>
                        )}
                        {columns.has('region') && <td>{chip(r.region)}</td>}
                        {columns.has('env') && <td>{chip(r.env)}</td>}
                        {columns.has('service') && <td>{chip(r.service)}</td>}
                        {columns.has('team') && <td>{chip(r.team)}</td>}
                        {columns.has('last_seen') && <td title={formatLongDate(r.lastSeen)}>{timeAgo(r.lastSeen)}</td>}
                      </tr>
                    ))}
                  </tbody>
                </table>
                <Pager offset={offset} limit={PAGE_SIZE} total={total} onChange={(o) => setParam({ offset: o ? String(o) : null })} />
              </>
            )}
          </div>
        )}
      </div>

      {selected && <CatalogPanel resource={selected} onClose={() => setSelected(null)} onFilter={addFilter} />}
    </div>
  );
}
