import { useCallback, useMemo, useRef, useState } from 'react';
import { Link, useSearchParams } from 'react-router-dom';
import { Box, ChevronDown, Download, Layers3, Maximize, Minus, MoreVertical, Plus, Search, Server, Square, X } from 'lucide-react';
import type { CatalogResource } from '../../api/catalog/client';
import { OTHER_GROUP, type CloudMapOverlay, type CloudMapProvider, type CloudMapProviderSummary } from '../../api/cloudMap/client';
import type { Cloud } from '../../api/infrastructure/types';
import { BrandLogo } from '../../components/brand/BrandLogo';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { Menu, Select, usePopover } from '../../components/ui/Dropdown';
import { addTerm } from '../../lib/searchQuery';
import { facetLabel } from './catalogModel';
import { CatalogPanel } from './CatalogPanel';
import { CloudDiagram, type DiagramHandle } from './CloudDiagram';
import { boxFaces, layoutDiagram, type PlacedBlock, type ViewMode } from './cloudMapLayout';
import { DEFAULT_PALETTE, faces, groupColor } from './cloudMapStyle';
import { CLOUD_LABEL, CloudIcon } from './hostIcons';
import { useCloudMap } from './queries';
import './cloudcraft.css';

const PROVIDERS: CloudMapProvider[] = ['kubernetes', 'hosts', 'ecs'];
const PROVIDER_LABEL: Record<CloudMapProvider, string> = { kubernetes: 'Kubernetes', hosts: 'Hosts', ecs: 'ECS' };
const DEFAULT_GROUP_BY: Record<CloudMapProvider, string[]> = { kubernetes: ['cluster', 'namespace'], hosts: ['cloud', 'region', 'availability_zone'], ecs: ['region', 'cluster'] };
const GROUP_KEYS = ['cloud', 'account', 'region', 'availability_zone', 'cluster', 'namespace', 'env', 'service', 'team'];
const OVERLAYS: { value: CloudMapOverlay; label: string }[] = [
  { value: 'default', label: 'Default' },
  { value: 'health', label: 'Health' },
  { value: 'dependencies', label: 'Dependencies' },
];
const VIEW_KEY = 'oc.cloudcraft.myView';
const CLOUDS = new Set(Object.keys(CLOUD_LABEL));

const keyLabel = (k: string) => (k === 'availability_zone' ? 'Availability Zone' : facetLabel(k));

/** "No region", "Cluster-wide", "Other (smaller groups)": how empty and merged group values read. */
function groupValueLabel(key: string, value: string): string {
  if (value === OTHER_GROUP) return 'Other groups';
  if (value) return key === 'cloud' ? (CLOUD_LABEL[value as Cloud] ?? value) : value;
  if (key === 'namespace') return 'Cluster-wide';
  if (key === 'cloud') return 'No cloud';
  return `No ${keyLabel(key).toLowerCase()}`;
}

interface SavedView {
  provider: CloudMapProvider;
  groupBy: string[];
  overlay: CloudMapOverlay;
  mode: ViewMode;
  q: string;
}

function readSavedView(): SavedView | null {
  try {
    const raw = localStorage.getItem(VIEW_KEY);
    return raw ? (JSON.parse(raw) as SavedView) : null;
  } catch {
    return null;
  }
}

function ProviderMarks({ p }: { p: CloudMapProviderSummary | undefined }) {
  return (
    <>
      {p?.clouds
        .filter((c) => CLOUDS.has(c))
        .map((c) => (
          <CloudIcon key={c} cloud={c as Cloud} size={16} />
        ))}
    </>
  );
}

function ProviderTab({ provider, summary, active, onClick }: { provider: CloudMapProvider; summary?: CloudMapProviderSummary; active: boolean; onClick: () => void }) {
  return (
    <button className={`cc-provider${active ? ' active' : ''}`} onClick={onClick} role="tab" aria-selected={active}>
      {provider === 'kubernetes' ? (
        <BrandLogo name="kubernetes" size={18} />
      ) : provider === 'ecs' ? (
        <BrandLogo name="aws" size={18} />
      ) : summary?.clouds.length ? (
        <ProviderMarks p={summary} />
      ) : (
        <Server size={16} />
      )}
      <span>{PROVIDER_LABEL[provider]}</span>
      {summary && <span className="cc-provider-count">{summary.resources.toLocaleString()}</span>}
    </button>
  );
}

/** Group-by levels as removable chips, outermost first, plus a picker for one more level. */
function GroupByControl({ value, onChange }: { value: string[]; onChange: (v: string[]) => void }) {
  const left = GROUP_KEYS.filter((k) => !value.includes(k));
  return (
    <div className="cc-groupby">
      <span className="cc-control-label">Group by</span>
      {value.map((k, i) => (
        <span key={k} className="cc-chip">
          {i > 0 && <span className="cc-chip-sep">›</span>}
          {keyLabel(k)}
          <button title={`Stop grouping by ${keyLabel(k)}`} onClick={() => onChange(value.filter((x) => x !== k))}>
            <X size={11} />
          </button>
        </span>
      ))}
      {value.length < 4 && left.length > 0 && (
        <Menu
          align="left"
          triggerClassName="cc-chip cc-chip-add"
          trigger={<Plus size={13} aria-label="Add a level" />}
          items={left.map((k) => ({ label: keyLabel(k), onClick: () => onChange([...value, k]) }))}
        />
      )}
    </div>
  );
}

function ExportMenu({ diagram }: { diagram: React.RefObject<DiagramHandle | null> }) {
  const save = (blob: Blob, name: string) => {
    const a = document.createElement('a');
    a.href = URL.createObjectURL(blob);
    a.download = name;
    a.click();
    setTimeout(() => URL.revokeObjectURL(a.href), 1000);
  };
  const svg = () => diagram.current?.toSvg() ?? '';
  const png = () => {
    const text = svg();
    if (!text) return;
    const img = new Image();
    const url = URL.createObjectURL(new Blob([text], { type: 'image/svg+xml' }));
    img.onload = () => {
      const canvas = document.createElement('canvas');
      const scale = Math.min(2, 8000 / Math.max(img.width, img.height, 1));
      canvas.width = Math.round(img.width * scale);
      canvas.height = Math.round(img.height * scale);
      const ctx = canvas.getContext('2d');
      if (!ctx) return;
      ctx.scale(scale, scale);
      ctx.drawImage(img, 0, 0);
      URL.revokeObjectURL(url);
      canvas.toBlob((b) => b && save(b, 'architecture.png'), 'image/png');
    };
    img.src = url;
  };
  return (
    <Menu
      triggerClassName="btn"
      trigger={
        <>
          <Download size={14} /> Export <ChevronDown size={13} />
        </>
      }
      items={[
        { label: 'PNG image', onClick: png },
        { label: 'SVG image', onClick: () => save(new Blob([svg()], { type: 'image/svg+xml' }), 'architecture.svg') },
      ]}
    />
  );
}

/** Our own isometric illustration for the onboarding card: a region with two clusters of blocks. */
function Illustration() {
  const unit = 13;
  const p = DEFAULT_PALETTE;
  const platform = (x: number, y: number, w: number, d: number, z: number, key: string, level: number) => {
    const f = boxFaces(x, y, w, d, z, 0.25, 'iso', unit);
    const c = groupColor(p, key, level);
    return (
      <g key={`${x}-${y}-${key}`}>
        <polygon points={f.left!} fill={c.side} />
        <polygon points={f.right!} fill={c.side} />
        <polygon points={f.top} fill={c.top} stroke={c.stroke} />
      </g>
    );
  };
  const block = (x: number, y: number, z: number, color: string, h = 1) => {
    const f = boxFaces(x, y, 1, 1, z, h, 'iso', unit);
    const c = faces(color);
    return (
      <g key={`${x}-${y}-${z}`}>
        <polygon points={f.left!} fill={c.left} />
        <polygon points={f.right!} fill={c.right} />
        <polygon points={f.top} fill={c.top} stroke={c.edge} strokeWidth={0.5} />
      </g>
    );
  };
  const blocks: [number, number, string, number?][] = [
    [1.2, 2, p.brand],
    [2.8, 2, p.brand],
    [4.4, 2, p.brand, 1.6],
    [1.2, 3.6, p.purple],
    [2.8, 3.6, p.brand],
    [8.6, 2, p.brand],
    [10.2, 2, p.orange],
    [8.6, 3.6, p.brand, 1.4],
    [10.2, 3.6, p.purple],
    [2.2, 9.4, p.gray],
    [3.8, 9.4, p.gray],
    [5.4, 9.4, p.gray],
    [9.2, 9.4, p.orange],
  ];
  return (
    <svg viewBox="-170 -40 340 250" className="cc-illustration" aria-hidden>
      {platform(0, 0, 13, 12, 0, 'region', 0)}
      {platform(0.6, 0.6, 5.6, 5, 0.25, 'cluster', 1)}
      {platform(7.4, 0.6, 5, 5, 0.25, 'cluster', 1)}
      {platform(0.6, 7.4, 11.8, 4, 0.25, 'availability_zone', 1)}
      {[...blocks].sort((a, b) => a[0] + a[1] - (b[0] + b[1])).map(([x, y, c, h]) => block(x, y, 0.5, c, h ?? 1))}
    </svg>
  );
}

function Onboarding() {
  return (
    <div className="cc-onboarding">
      <section className="cc-hero">
        <span className="cc-kicker">CLOUDCRAFT</span>
        <h2>See your architecture drawn for you</h2>
        <p>Every cluster, namespace, workload and host your collectors report becomes a live diagram, grouped by account, region and cluster, with health and service calls on top.</p>
        <Illustration />
      </section>
      <section className="cc-setup">
        <h3>Turn on collection to draw your first diagram</h3>
        <ol>
          <li>
            <BrandLogo name="kubernetes" size={18} />
            <div>
              <strong>Kubernetes</strong>
              <p>
                Run the Datadog Agent with the Cluster Agent and the orchestrator explorer enabled (Helm: <code>datadog.orchestratorExplorer.enabled=true</code>). Clusters, namespaces, workloads and
                nodes appear within minutes.
              </p>
            </div>
          </li>
          <li>
            <Server size={18} />
            <div>
              <strong>Hosts</strong>
              <p>The Datadog Agent sends host metadata (cloud, region and zone) on its own; with OpenTelemetry, add the host and cloud resource detectors to the Collector.</p>
            </div>
          </li>
          <li>
            <BrandLogo name="aws" size={18} />
            <div>
              <strong>Amazon ECS</strong>
              <p>Run the Datadog Agent as a daemon service in each ECS cluster; services and tasks are drawn per region and cluster.</p>
            </div>
          </li>
        </ol>
        <Link to="/infrastructure/add" className="btn btn-primary">
          Add Resources
        </Link>
      </section>
    </div>
  );
}

function Legend({ overlay }: { overlay: CloudMapOverlay }) {
  const p = DEFAULT_PALETTE;
  const items =
    overlay === 'health'
      ? [
          ['OK', p.ok],
          ['Warning', p.warn],
          ['Alert / errors', p.error],
          ['No data', p.nodata],
        ]
      : overlay === 'dependencies'
        ? [
            ['Calls (last hour)', p.blue],
            ['Calls with ≥ 5% errors', p.error],
          ]
        : [
            ['Workload', p.brand],
            ['Service', p.purple],
            ['Node / host', p.gray],
            ['AWS host', p.orange],
          ];
  return (
    <div className="cc-legend">
      {items.map(([label, color]) => (
        <span key={label}>
          <i style={{ background: color }} />
          {label}
        </span>
      ))}
    </div>
  );
}

function SearchBox({ value, onChange }: { value: string; onChange: (v: string) => void }) {
  const [text, setText] = useState(value);
  return (
    <label className="cc-search">
      <Search size={14} />
      <input
        value={text}
        placeholder="Filter resources: env:prod team:checkout order-*"
        onChange={(e) => setText(e.target.value)}
        onKeyDown={(e) => e.key === 'Enter' && onChange(text.trim())}
        onBlur={() => text.trim() !== value && onChange(text.trim())}
      />
      {text && (
        <button
          title="Clear"
          onClick={() => {
            setText('');
            onChange('');
          }}
        >
          <X size={13} />
        </button>
      )}
    </label>
  );
}

function ViewsMenu({ onApply, onSave, onReset, hasSaved }: { onApply: (v: 'default' | 'mine') => void; onSave: () => void; onReset: () => void; hasSaved: boolean }) {
  const { open, setOpen, ref } = usePopover();
  return (
    <div className="cc-views">
      <div className="dd" ref={ref}>
        <button className="cc-views-trigger" onClick={() => setOpen((o) => !o)}>
          Views <ChevronDown size={13} />
        </button>
        {open && (
          <ul className="dd-menu dd-left">
            <li>
              <button
                onClick={() => {
                  onApply('default');
                  setOpen(false);
                }}
              >
                Default view
              </button>
            </li>
            <li>
              <button
                disabled={!hasSaved}
                onClick={() => {
                  onApply('mine');
                  setOpen(false);
                }}
              >
                My View
              </button>
            </li>
          </ul>
        )}
      </div>
      <span className="cc-views-sep">/</span>
      <span className="cc-views-current">My View</span>
      <Menu
        triggerClassName="icon-btn"
        trigger={<MoreVertical size={15} aria-label="View options" />}
        items={[
          { label: 'Save as My View', onClick: onSave },
          { label: 'Reset to default', onClick: onReset },
        ]}
      />
    </div>
  );
}

/** Infrastructure → Resources → Cloudcraft: an architecture diagram drawn from the resource catalog. */
export function CloudcraftPage() {
  const [params, setParams] = useSearchParams();
  const provider = (PROVIDERS.includes(params.get('provider') as CloudMapProvider) ? params.get('provider') : 'kubernetes') as CloudMapProvider;
  const groupParam = params.get('group');
  const groupBy = groupParam === null ? DEFAULT_GROUP_BY[provider] : groupParam.split(',').filter((k) => GROUP_KEYS.includes(k));
  const overlay = (params.get('overlay') ?? 'default') as CloudMapOverlay;
  const mode: ViewMode = params.get('view') === '2d' ? 'flat' : 'iso';
  const query = params.get('q') ?? '';
  const [selected, setSelected] = useState<CatalogResource | null>(null);
  const diagram = useRef<DiagramHandle>(null);

  const setParam = useCallback(
    (updates: Record<string, string | null>) =>
      setParams(
        (p) => {
          const next = new URLSearchParams(p);
          for (const [k, v] of Object.entries(updates)) {
            if (v === null) next.delete(k);
            else next.set(k, v);
          }
          return next;
        },
        { replace: true },
      ),
    [setParams],
  );

  const q = useCloudMap({ provider, query, groupBy, overlay });
  const data = q.data;
  const layout = useMemo(() => (data ? layoutDiagram(data.root) : null), [data]);
  const anything = data?.providers.some((p) => p.resources > 0);
  const summary = (p: CloudMapProvider) => data?.providers.find((x) => x.provider === p);
  const onSelect = useCallback((b: PlacedBlock | null) => setSelected(b?.node.resource ?? null), []);
  const applyView = (v: SavedView | null) =>
    setParams(v ? { provider: v.provider, group: v.groupBy.join(','), overlay: v.overlay, view: v.mode === 'flat' ? '2d' : '3d', ...(v.q ? { q: v.q } : {}) } : { provider }, { replace: true });
  const saveView = () => {
    try {
      localStorage.setItem(VIEW_KEY, JSON.stringify({ provider, groupBy, overlay, mode, q: query } satisfies SavedView));
    } catch {
      // Storage refused (private window): the view stays in the URL only.
    }
  };

  return (
    <div className="rc-page cc-page">
      <div className="cc-header">
        <ViewsMenu hasSaved={!!readSavedView()} onApply={(v) => applyView(v === 'mine' ? readSavedView() : null)} onSave={saveView} onReset={() => applyView(null)} />
        <h2 className="cc-title">Cloudcraft</h2>
        <nav className="cc-providers" role="tablist" aria-label="Providers">
          {PROVIDERS.map((p) => (
            <ProviderTab key={p} provider={p} summary={summary(p)} active={p === provider} onClick={() => setParams({ provider: p }, { replace: true })} />
          ))}
        </nav>
      </div>

      {data && !anything ? (
        <div className="hosts-scroll">
          <Onboarding />
        </div>
      ) : (
        <>
          <div className="cc-toolbar">
            <SearchBox key={`${provider}:${query}`} value={query} onChange={(v) => setParam({ q: v || null })} />
            <GroupByControl value={groupBy} onChange={(g) => setParam({ group: g.join(',') })} />
            <Select label="Overlay" value={overlay} options={OVERLAYS} onChange={(o) => setParam({ overlay: o === 'default' ? null : o })} />
            <span className="btn-group cc-mode" role="group" aria-label="Projection">
              <button className={`btn${mode === 'iso' ? ' btn-active' : ''}`} onClick={() => setParam({ view: null })} title="Isometric">
                <Box size={14} /> 3D
              </button>
              <button className={`btn${mode === 'flat' ? ' btn-active' : ''}`} onClick={() => setParam({ view: '2d' })} title="Top-down">
                <Square size={14} /> 2D
              </button>
            </span>
            <span className="cc-toolbar-right">
              <ExportMenu diagram={diagram} />
            </span>
          </div>
          <div className="cc-stage">
            {q.isPending ? (
              <Loading label="Drawing your architecture…" />
            ) : q.isError ? (
              <ErrorState error={q.error} onRetry={() => q.refetch()} />
            ) : !layout || !layout.blocks.length ? (
              <div className="cc-none">
                <Layers3 size={40} />
                <p>{query ? 'No resources match this filter.' : `No ${PROVIDER_LABEL[provider]} resources reported yet.`}</p>
                {!query && (
                  <Link to="/infrastructure/add" className="link-btn">
                    Add resources
                  </Link>
                )}
              </div>
            ) : (
              <>
                <CloudDiagram ref={diagram} layout={layout} mode={mode} overlay={overlay} edges={data!.edges} selectedId={selected?.id ?? null} onSelect={onSelect} groupLabel={groupValueLabel} />
                <div className="cc-zoom">
                  <button className="icon-btn" title="Zoom in" onClick={() => diagram.current?.zoomIn()}>
                    <Plus size={15} />
                  </button>
                  <button className="icon-btn" title="Zoom out" onClick={() => diagram.current?.zoomOut()}>
                    <Minus size={15} />
                  </button>
                  <button className="icon-btn" title="Fit to screen" onClick={() => diagram.current?.fit()}>
                    <Maximize size={14} />
                  </button>
                </div>
                <Legend overlay={overlay} />
                <div className="cc-stats">
                  {data!.total.toLocaleString()} resources · {data!.groupBy.map(keyLabel).join(' › ') || 'ungrouped'}
                  {overlay === 'dependencies' && ` · ${data!.edges.length} service calls`}
                </div>
              </>
            )}
            {selected && <CatalogPanel resource={selected} onClose={() => setSelected(null)} onFilter={(k, v) => setParam({ q: addTerm(query, k, v) })} />}
          </div>
        </>
      )}
    </div>
  );
}
