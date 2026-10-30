import { useState } from 'react';
import { Link } from 'react-router-dom';
import { useQuery } from '@tanstack/react-query';
import { ChevronDown, ChevronRight, Copy, Download, ExternalLink, Search, X } from 'lucide-react';
import type { ReactNode } from 'react';
import { metricDetail } from '../../api/metricCatalog/client';
import type { MetricRule } from '../../api/metricRules/client';
import { Chart } from '../../components/charts/Chart';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { useTime } from '../../components/shell/TimeContext';
import { timeAgo } from '../../lib/format';
import { token } from '../../theme/chartTheme';
import { KIND_LABEL } from './MetricsLayout';
import { compactCount, downloadCsv, queryableTags } from './catalog';
import type { CatalogRow } from './MetricsCatalog';

/** Values a tag row shows before "+N". */
const VALUES_SHOWN = 8;

type AssetKind = 'dashboards' | 'notebooks' | 'monitors' | 'slos';
const ASSETS: { kind: AssetKind; label: string; href: (id: string) => string }[] = [
  { kind: 'dashboards', label: 'Dashboards', href: (id) => `/dashboards/${id}` },
  { kind: 'notebooks', label: 'Notebooks', href: (id) => `/notebooks/${id}` },
  { kind: 'monitors', label: 'Monitors', href: (id) => `/monitors/${id}` },
  { kind: 'slos', label: 'SLOs', href: (id) => `/slo?slo=${id}` },
];

function Section({ title, children }: { title: string; children: ReactNode }) {
  const [open, setOpen] = useState(true);
  return (
    <section className="mp-section">
      <button className="mp-section-title" onClick={() => setOpen((o) => !o)}>
        {open ? <ChevronDown size={16} /> : <ChevronRight size={16} />} {title}
      </button>
      {open && <div className="mp-section-body">{children}</div>}
    </section>
  );
}

/** A metric's details beside the catalog: volume, metadata, tags, related assets and cardinality. */
export function MetricPanel({ name, row, explorerLink, rules, onClose }: { name: string; row?: CatalogRow; explorerLink: string; rules: MetricRule[]; onClose: () => void }) {
  const { range } = useTime();
  const detail = useQuery({ queryKey: ['metrics', 'detail', name, range], queryFn: ({ signal }) => metricDetail(name, range, signal) });
  const [tagSearch, setTagSearch] = useState('');
  const [tagView, setTagView] = useState<'queryable' | 'ingested'>('queryable');
  const [asset, setAsset] = useState<AssetKind>('dashboards');
  const [assetSearch, setAssetSearch] = useState('');
  const d = detail.data;
  const allKeys = d?.tags.map((t) => t.key) ?? [];
  const queryable = new Set(queryableTags(name, allKeys, rules));
  const tags = (d?.tags ?? []).filter((t) => (tagView === 'ingested' || queryable.has(t.key)) && (!tagSearch || `${t.key} ${t.values.join(' ')}`.toLowerCase().includes(tagSearch.toLowerCase())));
  const assets = (d?.related[asset] ?? []).filter((a) => !assetSearch || a.title.toLowerCase().includes(assetSearch.toLowerCase()));
  const assetInfo = ASSETS.find((a) => a.kind === asset)!;
  return (
    <aside className="mp-panel" aria-label={`Metric ${name}`}>
      <header className="mp-head">
        <h2 className="mono" title={name}>
          {name}
        </h2>
        <button className="icon-plain" title="Copy name" onClick={() => navigator.clipboard?.writeText(name)}>
          <Copy size={15} />
        </button>
        <Link className="btn btn-sm" to={explorerLink}>
          Open in Metrics Explorer <ExternalLink size={13} />
        </Link>
        <button className="icon-plain" title="Close" onClick={onClose}>
          <X size={18} />
        </button>
      </header>
      {detail.isPending ? (
        <Loading />
      ) : detail.isError ? (
        <ErrorState error={detail.error} onRetry={() => detail.refetch()} />
      ) : (
        d && (
          <div className="mp-scroll">
            <div className="mp-kpis">
              <div>
                <span>INGESTED SERIES</span>
                <strong>{compactCount(row?.ingested ?? d.series)}</strong>
              </div>
              <div>
                <span>INDEXED SERIES</span>
                <strong>{compactCount(row?.indexed ?? d.series)}</strong>
              </div>
              <div>
                <span>HOSTS</span>
                <strong>{compactCount(d.hosts)}</strong>
              </div>
              <div>
                <span>TAG VALUES</span>
                <strong>{compactCount(d.tagValues)}</strong>
              </div>
            </div>
            <Section title="Metadata">
              <dl className="mp-meta">
                <dt>Metric Type:</dt>
                <dd>
                  <span className="mx-kind">{KIND_LABEL[row?.kind ?? 'gauge'] ?? row?.kind}</span>
                </dd>
                <dt>Unit:</dt>
                <dd>{row?.unit || '—'}</dd>
                <dt>Description:</dt>
                <dd>{row?.description || 'No description.'}</dd>
                <dt>Origin:</dt>
                <dd>
                  <span className="mx-kind">{row?.origin ?? '—'}</span>
                </dd>
                <dt>First submitted:</dt>
                <dd>{d.firstSeen ? `${timeAgo(d.firstSeen)} (${new Date(d.firstSeen).toLocaleDateString()})` : '—'}</dd>
              </dl>
            </Section>
            <Section title="Tags">
              <label className="tv-filter mp-search">
                <Search size={15} />
                <input placeholder="Search tags" value={tagSearch} onChange={(e) => setTagSearch(e.target.value)} />
              </label>
              <div className="mp-toolbar">
                <div className="btn-group">
                  <button className={`btn btn-sm${tagView === 'queryable' ? ' btn-active' : ''}`} onClick={() => setTagView('queryable')}>
                    Queryable Tags
                  </button>
                  <button className={`btn btn-sm${tagView === 'ingested' ? ' btn-active' : ''}`} onClick={() => setTagView('ingested')}>
                    Ingested Tags
                  </button>
                </div>
                <span className="muted">
                  Showing {tags.length} of {allKeys.length} tag keys
                </span>
                <Link className="btn btn-sm mp-push" to="/metrics/settings/rules">
                  Manage Tags <ExternalLink size={13} />
                </Link>
                <button
                  className="btn btn-sm"
                  title="Download as CSV"
                  onClick={() =>
                    downloadCsv(
                      `${name}-tags.csv`,
                      ['tag_key', 'count', 'values'],
                      tags.map((t) => [t.key, t.count, t.values.join(' ')]),
                    )
                  }
                >
                  <Download size={14} />
                </button>
              </div>
              <table className="sx-table mp-tags">
                <thead>
                  <tr>
                    <th>TAG KEY</th>
                    <th className="num">COUNT</th>
                    <th>TAG VALUES</th>
                  </tr>
                </thead>
                <tbody>
                  {tags.map((t) => (
                    <tr key={t.key}>
                      <td>{t.key}</td>
                      <td className="num">{t.count.toLocaleString()}</td>
                      <td>
                        <span className="mp-values">
                          {t.values.slice(0, VALUES_SHOWN).map((v) => (
                            <span key={v} className="iw-ev-tag" title={`${t.key}:${v}`}>
                              {t.key}:<b>{v}</b>
                            </span>
                          ))}
                          {t.count > VALUES_SHOWN && <span className="iw-ev-tag iw-ev-more">+{t.count - VALUES_SHOWN}</span>}
                        </span>
                      </td>
                    </tr>
                  ))}
                  {!tags.length && (
                    <tr>
                      <td colSpan={3} className="muted">
                        No tags.
                      </td>
                    </tr>
                  )}
                </tbody>
              </table>
            </Section>
            <Section title="Related Assets">
              <p>This metric is used by these dashboards, notebooks, monitors and SLOs.</p>
              <div className="mp-toolbar">
                <select className="input mp-asset-kind" value={asset} onChange={(e) => setAsset(e.target.value as AssetKind)}>
                  {ASSETS.map((a) => (
                    <option key={a.kind} value={a.kind}>
                      {a.label} ({d.related[a.kind].length})
                    </option>
                  ))}
                </select>
                <label className="tv-filter mp-search mp-push">
                  <Search size={15} />
                  <input placeholder={`Search ${assetInfo.label.toLowerCase()}`} value={assetSearch} onChange={(e) => setAssetSearch(e.target.value)} />
                </label>
                <button
                  className="btn btn-sm"
                  onClick={() =>
                    downloadCsv(
                      `${name}-${asset}.csv`,
                      ['id', 'title'],
                      assets.map((a) => [a.id, a.title]),
                    )
                  }
                >
                  <Download size={14} /> Export as CSV
                </button>
              </div>
              <table className="sx-table">
                <thead>
                  <tr>
                    <th>{assetInfo.label.toUpperCase()}</th>
                  </tr>
                </thead>
                <tbody>
                  {assets.map((a) => (
                    <tr key={a.id}>
                      <td>
                        <Link to={assetInfo.href(a.id)}>{a.title}</Link>
                      </td>
                    </tr>
                  ))}
                  {!assets.length && (
                    <tr>
                      <td className="muted mp-empty">No matching results found</td>
                    </tr>
                  )}
                </tbody>
              </table>
            </Section>
            <Section title="Tags Cardinality Explorer">
              <h4>Series volume</h4>
              <div className="mp-chart">
                <Chart
                  height="100%"
                  option={{
                    grid: { left: 8, right: 12, top: 12, bottom: 22, containLabel: true },
                    tooltip: { trigger: 'axis', confine: true },
                    xAxis: { type: 'time', axisLabel: { hideOverlap: true } },
                    yAxis: { type: 'value', minInterval: 1 },
                    series: [{ type: 'line', step: 'end', showSymbol: false, color: token('--chart-1'), data: d.timeline.map((p) => [p.t, p.series]) }],
                  }}
                />
              </div>
              <h4>Top tags by unique values</h4>
              <table className="sx-table">
                <thead>
                  <tr>
                    <th>TAG KEY</th>
                    <th className="num">TOTAL TAG VALUES</th>
                  </tr>
                </thead>
                <tbody>
                  {[...d.tags]
                    .sort((a, b) => b.count - a.count)
                    .slice(0, 10)
                    .map((t) => (
                      <tr key={t.key}>
                        <td>{t.key}</td>
                        <td className="num">{t.count.toLocaleString()}</td>
                      </tr>
                    ))}
                </tbody>
              </table>
            </Section>
          </div>
        )
      )}
    </aside>
  );
}
