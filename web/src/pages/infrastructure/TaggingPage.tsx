import { Fragment, useMemo, useState } from 'react';
import { Link, NavLink, useNavigate, useSearchParams } from 'react-router-dom';
import { Pencil, Plus, Search, Sparkles, Trash2 } from 'lucide-react';
import type { EChartsOption } from 'echarts';
import type { CoverageRow, NonCompliantResource, TaggingPolicy, TagTarget } from '../../api/tagging/types';
import { describeGap } from '../../api/tagging/logic';
import { Chart } from '../../components/charts/Chart';
import { Empty, ErrorState, Loading } from '../../components/ui/AsyncState';
import { Select } from '../../components/ui/Dropdown';
import { NoSourceToggle, SourceCell, VIA_LABELS } from '../../components/provenance/SourceCell';
import { ResourceScope, useAccountNames } from '../../components/provenance/ResourceScope';
import { ResourceType } from '../security/cloud/resourceTypes';
import { AddTagsPanel } from './AddTagsPanel';
import { token } from '../../theme/chartTheme';
import { ScoreBar } from './PoliciesPage';
import { useDeleteTaggingPolicy, useSaveTaggingPolicy, useTagCoverage, useTaggingPolicies } from './taggingQueries';
import './policies.css';
import './tagging.css';

export const TARGET_LABELS: Record<TagTarget, string> = { cloud: 'Cloud resources', kubernetes: 'Kubernetes objects', image: 'Images', service: 'Services' };
const TARGET_SHORT: Record<TagTarget, string> = { cloud: 'Cloud', kubernetes: 'Kubernetes', image: 'Image', service: 'Service' };
const TARGETS = Object.keys(TARGET_LABELS) as TagTarget[];
const MAX_ROWS = 8;

function TaggingTabs() {
  return (
    <nav className="tg-subtabs">
      <NavLink to="/infrastructure/tagging" end className={({ isActive }) => (isActive ? 'active' : '')}>
        Coverage
      </NavLink>
      <NavLink to="/infrastructure/tagging/policies" className={({ isActive }) => (isActive ? 'active' : '')}>
        Policies
      </NavLink>
    </nav>
  );
}

/** A breakdown of the coverage (per policy, account, cluster or team): the worst rows first. */
function Breakdown({ title, rows, empty, onPick }: { title: string; rows: CoverageRow[]; empty: string; onPick?: (key: string) => void }) {
  const [all, setAll] = useState(false);
  const shown = all ? rows : rows.slice(0, MAX_ROWS);
  return (
    <section className="tg-card">
      <h3>{title}</h3>
      {!rows.length ? (
        <p className="muted">{empty}</p>
      ) : (
        <table className="tg-breakdown">
          <tbody>
            {shown.map((r) => (
              <tr key={r.key}>
                <td className="tg-label" title={r.label}>
                  {onPick ? (
                    <button className="link-btn" onClick={() => onPick(r.key)}>
                      {r.label}
                    </button>
                  ) : (
                    r.label
                  )}
                </td>
                <td className="num muted">
                  {r.compliant.toLocaleString()}/{r.evaluated.toLocaleString()}
                </td>
                <td className="tg-bar">
                  <ScoreBar score={r.score} compact />
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
      {rows.length > MAX_ROWS && (
        <button className="link-btn" onClick={() => setAll(!all)}>
          {all ? 'Show fewer' : `Show all ${rows.length}`}
        </button>
      )}
    </section>
  );
}

function Missing({ r }: { r: NonCompliantResource }) {
  return (
    <span className="tg-gaps">
      {r.policies.flatMap((p) =>
        p.gaps.map((g) => (
          <span key={`${p.policyId}-${g.key}`} className={`tag-chip${g.value == null ? '' : ' tg-bad'}`} title={p.policyName}>
            {describeGap(g)}
          </span>
        )),
      )}
    </span>
  );
}

/** Infrastructure → Resources → Tagging → Coverage: compliance with the tagging policies, over time and per scope. */
export function TagCoveragePage() {
  const [params, setParams] = useSearchParams();
  const policy = params.get('policy') ?? undefined;
  const target = (params.get('target') ?? '') as TagTarget | '';
  const noSource = params.get('source') === 'none';
  const [text, setText] = useState('');
  const [tagging, setTagging] = useState<string | null>(null);
  const q = useTagCoverage(policy);
  const policies = useTaggingPolicies();
  const names = useAccountNames();
  const set = (k: string, v: string | null) => {
    const next = new URLSearchParams(params);
    if (v) next.set(k, v);
    else next.delete(k);
    setParams(next, { replace: true });
  };

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
          name: 'Coverage',
          data: (history ?? []).map((h) => h.score),
          showSymbol: (history?.length ?? 0) < 3,
          lineStyle: { color: brand, width: 2 },
          itemStyle: { color: brand },
          areaStyle: { color: brand, opacity: 0.12 },
        },
      ],
    };
  }, [history]);

  if (q.isPending) return <Loading label="Evaluating tagging policies…" />;
  if (q.isError) return <ErrorState error={q.error} onRetry={() => q.refetch()} />;
  const c = q.data;
  const rows = c.noncompliant.filter(
    (r) =>
      (!target || r.resource.target === target) &&
      (!noSource || !r.resource.provenance) &&
      `${r.resource.name} ${r.resource.resourceType} ${r.resource.account ?? ''} ${r.resource.cluster ?? ''} ${r.resource.team ?? ''}`.toLowerCase().includes(text.toLowerCase()),
  );
  const policyOptions = [{ value: '', label: 'All enabled policies' }, ...(policies.data?.policies ?? []).map((p) => ({ value: p.id, label: p.name }))];
  const pv = c.provenance;
  const kubeEvaluated = pv.byTarget.kubernetes?.[0] ?? 0;

  return (
    <div className="rc-page">
      <div className="hosts-scroll tg-page">
        <TaggingTabs />
        <div className="tg-toolbar">
          <Select label="Policy" value={policy ?? ''} options={policyOptions} onChange={(v) => set('policy', v || null)} />
          <Link to="/infrastructure/tagging/policies" className="link-btn">
            Manage policies
          </Link>
        </div>
        {!c.total.evaluated ? (
          <Empty>
            <b>Nothing to evaluate yet.</b>
            <p className="muted">Tagging policies look at the cloud resources the account scans list, the Kubernetes objects, the images’ SBOMs and the services sending traces.</p>
          </Empty>
        ) : (
          <>
            <div className="tg-kpis">
              <div className="tg-kpi">
                <span className="tg-kpi-label">Coverage</span>
                <span className="tg-kpi-value">{c.total.score == null ? '—' : `${c.total.score}%`}</span>
                <ScoreBar score={c.total.score} />
              </div>
              <div className="tg-kpi">
                <span className="tg-kpi-label">Compliant resources</span>
                <span className="tg-kpi-value">
                  {c.total.compliant.toLocaleString()} <span className="muted">/ {c.total.evaluated.toLocaleString()}</span>
                </span>
              </div>
              <div className="tg-kpi">
                <span className="tg-kpi-label">Missing tags</span>
                <span className="tg-kpi-value">{c.noncompliantTotal.toLocaleString()}</span>
              </div>
              <div className="tg-kpi">
                <span className="tg-kpi-label">With a known source</span>
                <span className="tg-kpi-value">
                  {c.withProvenance.toLocaleString()} <span className="muted">/ {c.total.evaluated.toLocaleString()}</span>
                </span>
                <span className="muted tg-kpi-note">
                  {Object.entries(pv.byVia)
                    .map(([via, n]) => `${VIA_LABELS[via as keyof typeof VIA_LABELS] ?? via} ${n}`)
                    .join(' · ') || 'no source found yet'}
                </span>
              </div>
            </div>
            {kubeEvaluated > 0 && c.argocdApplications === 0 && (
              <p className="tg-note">
                No Argo CD Applications are collected: Kubernetes objects resolve to their Application’s name only. Add <code>argoproj.io/v1alpha1/applications</code> to the Cluster Agent’s
                orchestrator custom resources to get their repository, path and revision.
              </p>
            )}
            <section className="tg-card">
              <h3>Coverage over time</h3>
              {history && history.length > 0 ? <Chart option={option} height={180} /> : <p className="muted">The first daily snapshot is taken within the hour.</p>}
            </section>
            <div className="tg-grid">
              <Breakdown title="By policy" rows={[...c.byPolicy].sort((a, b) => (a.score ?? 100) - (b.score ?? 100))} empty="No enabled policy." onPick={(id) => set('policy', id)} />
              <Breakdown title="By team" rows={c.byTeam} empty="No team tags." />
              <Breakdown title="By cloud account" rows={c.byAccount} empty="No cloud resources evaluated." />
              <Breakdown title="By cluster" rows={c.byCluster} empty="No Kubernetes objects evaluated." />
            </div>
            <section className="tg-card">
              <h3>Resources missing required tags</h3>
              <div className="tg-filters">
                <span className="tg-search">
                  <Search size={14} />
                  <input value={text} placeholder="Filter by name, account, cluster or team" onChange={(e) => setText(e.target.value)} />
                </span>
                <Select value={target} options={[{ value: '', label: 'All targets' }, ...TARGETS.map((t) => ({ value: t, label: TARGET_LABELS[t] }))]} onChange={(v) => set('target', v || null)} />
                <NoSourceToggle checked={noSource} onChange={(v) => set('source', v ? 'none' : null)} />
              </div>
              {!rows.length ? (
                <p className="muted">{c.noncompliant.length ? 'No resource matches the filters.' : 'Every evaluated resource carries its required tags.'}</p>
              ) : (
                <table className="mon-table syn-table tg-table prov-table">
                  <colgroup>
                    <col style={{ width: '20%' }} />
                    <col style={{ width: '9%' }} />
                    <col style={{ width: '17%' }} />
                    <col style={{ width: '9%' }} />
                    <col />
                    <col style={{ width: '19%' }} />
                    <col style={{ width: 112 }} />
                  </colgroup>
                  <thead>
                    <tr>
                      <th>RESOURCE</th>
                      <th>TARGET</th>
                      <th>SCOPE</th>
                      <th>TEAM</th>
                      <th>MISSING OR NOT ALLOWED</th>
                      <th>SOURCE</th>
                      <th />
                    </tr>
                  </thead>
                  <tbody>
                    {rows.map((r) => (
                      <Fragment key={r.resource.id}>
                        <tr>
                          <td title={`${r.resource.name}\n${r.resource.id}`}>
                            <b className="tg-name">{r.resource.name}</b>
                            <div className="muted tg-sub">
                              <ResourceType type={r.resource.resourceType} resource={r.resource.name} size={13} />
                            </div>
                          </td>
                          <td>{TARGET_SHORT[r.resource.target]}</td>
                          <td>
                            <ResourceScope resource={r.resource} names={names} />
                          </td>
                          <td title={r.resource.team ?? ''}>{r.resource.team ?? <span className="muted">—</span>}</td>
                          <td className="prov-wrap">
                            <Missing r={r} />
                          </td>
                          <td>
                            <SourceCell provenance={r.resource.provenance} />
                          </td>
                          <td>
                            <button
                              className={`btn btn-sm${tagging === r.resource.id ? ' active' : ''}`}
                              title="Open a merge request that adds the tags naming this resource's source"
                              onClick={() => setTagging(tagging === r.resource.id ? null : r.resource.id)}
                            >
                              <Sparkles size={13} /> Add tags
                            </button>
                          </td>
                        </tr>
                        {tagging === r.resource.id && (
                          <tr className="tg-addtags-row">
                            <td colSpan={7}>
                              <AddTagsPanel resource={r.resource} />
                            </td>
                          </tr>
                        )}
                      </Fragment>
                    ))}
                  </tbody>
                </table>
              )}
              {c.noncompliantTotal > c.noncompliant.length && (
                <p className="muted">
                  Showing the first {c.noncompliant.length.toLocaleString()} of {c.noncompliantTotal.toLocaleString()}.
                </p>
              )}
            </section>
          </>
        )}
      </div>
    </div>
  );
}

function scopeText(p: TaggingPolicy): string {
  const s = p.scope;
  const parts = [
    s.provider && `provider ${s.provider}`,
    s.account && `account ${s.account}`,
    s.cluster && `cluster ${s.cluster}`,
    s.resourceType && `type ${s.resourceType}`,
    s.names && `names ${s.names}`,
  ].filter(Boolean);
  return parts.length ? parts.join(' · ') : 'All';
}

/** Infrastructure → Resources → Tagging → Policies: the org's tagging policies and the recommended set. */
export function TaggingPoliciesPage() {
  const q = useTaggingPolicies();
  const save = useSaveTaggingPolicy();
  const del = useDeleteTaggingPolicy();
  const navigate = useNavigate();
  if (q.isPending) return <Loading label="Loading tagging policies…" />;
  if (q.isError) return <ErrorState error={q.error} onRetry={() => q.refetch()} />;
  const { policies, recommended } = q.data;
  const missing = recommended.filter((r) => !r.installed);
  const toggle = (p: TaggingPolicy) => save.mutate({ id: p.id, input: { name: p.name, description: p.description, target: p.target, scope: p.scope, required: p.required, enabled: !p.enabled } });
  const remove = (p: TaggingPolicy) => {
    if (window.confirm(`Delete the tagging policy "${p.name}"?`)) del.mutate(p.id);
  };
  return (
    <div className="rc-page">
      <div className="hosts-scroll tg-page">
        <TaggingTabs />
        <div className="tg-toolbar">
          <p className="muted">Which tags the resources of a scope must carry. Resources missing them show on the coverage page and as low-severity Cloud Security misconfigurations.</p>
          <button className="btn btn-primary" onClick={() => navigate('/infrastructure/tagging/policies/new')}>
            <Plus size={14} /> New Policy
          </button>
        </div>
        {(save.isError || del.isError) && <p className="pol-error">{((save.error ?? del.error) as Error).message}</p>}
        <table className="mon-table syn-table tg-table tg-policies">
          <colgroup>
            <col style={{ width: 80 }} />
            <col />
            <col style={{ width: 150 }} />
            <col style={{ width: 120 }} />
            <col style={{ width: '30%' }} />
            <col style={{ width: 150 }} />
            <col style={{ width: 80 }} />
          </colgroup>
          <thead>
            <tr>
              <th>ENABLED</th>
              <th>POLICY</th>
              <th>TARGET</th>
              <th>SCOPE</th>
              <th>REQUIRED TAGS</th>
              <th>COVERAGE</th>
              <th />
            </tr>
          </thead>
          <tbody>
            {policies.map((p) => (
              <tr key={p.id} className={p.enabled ? undefined : 'tg-disabled'}>
                <td>
                  <button
                    type="button"
                    role="switch"
                    aria-checked={p.enabled}
                    aria-label={`${p.enabled ? 'Disable' : 'Enable'} ${p.name}`}
                    className={`switch${p.enabled ? ' on' : ''}`}
                    onClick={() => toggle(p)}
                  />
                </td>
                <td>
                  <Link to={`/infrastructure/tagging?policy=${p.id}`}>
                    <b>{p.name}</b>
                  </Link>
                  {p.recommended && <span className="tg-badge">Recommended</span>}
                  {p.description && <div className="muted tg-sub">{p.description}</div>}
                </td>
                <td>{TARGET_LABELS[p.target]}</td>
                <td className="muted">{scopeText(p)}</td>
                <td>
                  <span className="tg-gaps">
                    {p.required.map((t) => (
                      <span key={t.key} className="tag-chip mono" title={t.allowedValues?.length ? `Allowed: ${t.allowedValues.join(', ')}` : 'Any value'}>
                        {t.key.split('|').join(' | ')}
                        {t.allowedValues?.length ? ` ∈ {${t.allowedValues.join(', ')}}` : ''}
                      </span>
                    ))}
                  </span>
                </td>
                <td className="tg-bar">
                  <ScoreBar score={p.coverage.score} compact />
                  <div className="muted tg-sub">
                    {p.coverage.compliant.toLocaleString()}/{p.coverage.evaluated.toLocaleString()}
                  </div>
                </td>
                <td className="tg-actions">
                  <button className="icon-btn" title="Edit" onClick={() => navigate(`/infrastructure/tagging/policies/${p.id}`)}>
                    <Pencil size={14} />
                  </button>
                  <button className="icon-btn" title="Delete" onClick={() => remove(p)}>
                    <Trash2 size={14} />
                  </button>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
        {missing.length > 0 && (
          <section className="tg-card">
            <h3>Recommended policies not in use</h3>
            <ul className="tg-recommended">
              {missing.map((r) => (
                <li key={r.key}>
                  <div>
                    <b>{r.name}</b>
                    <div className="muted tg-sub">{r.description}</div>
                  </div>
                  <button className="btn" onClick={() => navigate(`/infrastructure/tagging/policies/new?from=${r.key}`)}>
                    <Plus size={14} /> Add
                  </button>
                </li>
              ))}
            </ul>
          </section>
        )}
      </div>
    </div>
  );
}
