import { useCallback, useMemo, useState } from 'react';
import { Link, useNavigate, useParams, useSearchParams } from 'react-router-dom';
import { ChevronLeft, Pencil, Trash2 } from 'lucide-react';
import type { EChartsOption } from 'echarts';
import type { CatalogResource } from '../../api/catalog/client';
import { Chart } from '../../components/charts/Chart';
import { Empty, ErrorState, Loading } from '../../components/ui/AsyncState';
import { formatLongDate } from '../../lib/format';
import { addTerm } from '../../lib/searchQuery';
import { token } from '../../theme/chartTheme';
import { TYPE_LABELS } from './catalogModel';
import { CatalogPanel, TypeIcon } from './CatalogPanel';
import { CATEGORY_NAMES, ruleSummary } from './policyModel';
import { PriorityBadge, ScoreBar } from './PoliciesPage';
import { ResourceLogo } from './ResourceLogo';
import { useDeletePolicy, usePolicy } from './queries';
import './policies.css';

/** A policy: its score over time, the rule, and the resources that don't follow it (each opens the catalog panel). */
export function PolicyDetailPage() {
  const { id = '' } = useParams();
  const [params, setParams] = useSearchParams();
  const query = params.get('q') ?? '';
  const navigate = useNavigate();
  const q = usePolicy(id, query);
  const del = useDeletePolicy();
  const [selected, setSelected] = useState<CatalogResource | null>(null);
  const [filter, setFilter] = useState('');
  const addFilter = useCallback((k: string, v: string) => setParams({ q: addTerm(query, k, v) }, { replace: true }), [setParams, query]);

  const history = q.data?.history;
  const option = useMemo<EChartsOption>(() => {
    const brand = token('--brand') || '#0b6e69';
    return {
      grid: { left: 40, right: 16, top: 16, bottom: 28 },
      tooltip: { trigger: 'axis', valueFormatter: (v) => (v == null ? '—' : `${v}%`) },
      xAxis: { type: 'category', data: (history ?? []).map((h) => h.day), boundaryGap: false },
      yAxis: { type: 'value', min: 0, max: 100, axisLabel: { formatter: '{value}%' } },
      series: [
        {
          type: 'line',
          name: 'Compliance score',
          data: (history ?? []).map((h) => h.score),
          showSymbol: (history?.length ?? 0) < 3,
          lineStyle: { color: brand, width: 2 },
          itemStyle: { color: brand },
          areaStyle: { color: brand, opacity: 0.12 },
        },
      ],
    };
  }, [history]);

  if (q.isPending) return <Loading label="Loading policy…" />;
  if (q.isError) return <ErrorState error={q.error} onRetry={() => q.refetch()} />;
  const { policy: p, violations } = q.data;
  const e = p.evaluation;
  const shown = violations.filter((v) => `${v.resource.name} ${v.reason} ${v.resource.cluster ?? ''} ${v.resource.namespace ?? ''}`.toLowerCase().includes(filter.toLowerCase()));
  const remove = () => {
    if (!window.confirm(`Delete the policy "${p.name}" and its history?`)) return;
    del.mutate(p.id, { onSuccess: () => navigate('/infrastructure/policies') });
  };

  return (
    <div className="rc-page">
      <div className="hosts-scroll pol-detail">
        <Link to={`/infrastructure/policies${query ? `?q=${encodeURIComponent(query)}` : ''}`} className="pol-back">
          <ChevronLeft size={14} /> Policies
        </Link>
        <header className="pol-detail-head">
          <ResourceLogo type={p.resourceType} size={28} />
          <div>
            <h1 className="pol-title">{p.name}</h1>
            <div className="pol-detail-meta">
              <PriorityBadge priority={p.priority} />
              <span>{CATEGORY_NAMES[p.category]}</span>
              <span className="muted-text">·</span>
              <span>{TYPE_LABELS[p.resourceType]}</span>
              {p.createdBy && (
                <>
                  <span className="muted-text">·</span>
                  <span className="muted-text">
                    Created by {p.createdBy} on {formatLongDate(p.createdAt)}
                  </span>
                </>
              )}
            </div>
          </div>
          <span className="pol-detail-actions">
            <button className="btn" onClick={() => navigate(`/infrastructure/policies/${p.id}/edit`)}>
              <Pencil size={14} /> Edit
            </button>
            <button className="btn" onClick={remove} disabled={del.isPending}>
              <Trash2 size={14} /> Delete
            </button>
          </span>
        </header>
        {p.description && <p className="pol-description">{p.description}</p>}
        {query && (
          <p className="pol-filtered">
            Counting only resources matching <code>{query}</code>.{' '}
            <button className="link-btn" onClick={() => setParams({}, { replace: true })}>
              Show all
            </button>
          </p>
        )}

        <div className="pol-cards">
          <div className="pol-card">
            <span className="pol-card-label">Compliance score</span>
            <span className="pol-card-value">{e.score == null ? '—' : `${e.score}%`}</span>
            <ScoreBar score={e.score} />
          </div>
          <div className="pol-card">
            <span className="pol-card-label">Noncompliant resources</span>
            <span className={`pol-card-value${e.noncompliant ? ' pol-noncompliant' : ''}`}>{e.noncompliant.toLocaleString()}</span>
            <span className="muted-text">of {e.evaluated.toLocaleString()} evaluated</span>
          </div>
          <div className="pol-card pol-card-rule">
            <span className="pol-card-label">Rule</span>
            <span>
              <TypeIcon type={p.resourceType} size={14} /> {TYPE_LABELS[p.resourceType]}
              {p.scope && (
                <>
                  {' '}
                  in <code>{p.scope}</code>
                </>
              )}
            </span>
            <code className="pol-rule">{ruleSummary(p.conditions)}</code>
          </div>
        </div>

        <section className="pol-section">
          <h2>Score over time</h2>
          {history && history.length ? <Chart option={option} height={220} /> : <Empty>The first daily snapshot is taken when the policy is saved.</Empty>}
          <p className="muted-text pol-note">One snapshot a day on all resources of the policy (page filters don't apply), refreshed hourly.</p>
        </section>

        <section className="pol-section">
          <div className="pol-section-head">
            <h2>Noncompliant resources</h2>
            <input className="pol-input pol-section-filter" placeholder="Filter resources" value={filter} onChange={(ev) => setFilter(ev.target.value)} />
          </div>
          {violations.length === 0 ? (
            <Empty>{e.evaluated ? 'Every resource in scope follows this policy.' : 'No resources are in scope yet.'}</Empty>
          ) : (
            <table className="host-table rc-table">
              <thead>
                <tr>
                  <th>RESOURCE</th>
                  <th>CLUSTER / ACCOUNT</th>
                  <th>REGION</th>
                  <th>TEAM</th>
                  <th>WHY</th>
                </tr>
              </thead>
              <tbody>
                {shown.map((v) => (
                  <tr key={v.resource.id} className={selected?.id === v.resource.id ? 'selected' : undefined} onClick={() => setSelected(v.resource)}>
                    <td className="rc-resource">
                      <TypeIcon type={v.resource.resourceType} size={18} />
                      <span>
                        <span className="rc-name" title={v.resource.name}>
                          {v.resource.name}
                        </span>
                        <span className="rc-subtype">
                          {TYPE_LABELS[v.resource.resourceType]}
                          {v.resource.namespace && ` · ${v.resource.namespace}`}
                        </span>
                      </span>
                    </td>
                    <td>{[v.resource.cluster, v.resource.account].filter(Boolean).join(' · ') || <span className="muted-text">—</span>}</td>
                    <td>{v.resource.region ? <span className="tag-chip">{v.resource.region}</span> : <span className="muted-text">—</span>}</td>
                    <td>{v.resource.team ? <span className="tag-chip">{v.resource.team}</span> : <span className="muted-text">—</span>}</td>
                    <td>
                      <code className="pol-reason">{v.reason}</code>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
          {violations.length >= 500 && <p className="muted-text pol-note">Showing the first 500 noncompliant resources.</p>}
        </section>
      </div>
      {selected && <CatalogPanel resource={selected} onClose={() => setSelected(null)} onFilter={addFilter} />}
    </div>
  );
}
