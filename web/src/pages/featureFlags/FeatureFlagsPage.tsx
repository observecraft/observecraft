import { useDeferredValue, useMemo, useState } from 'react';
import { Outlet, useNavigate, useSearchParams } from 'react-router-dom';
import { BookOpen, Flag, Plus, Search, X } from 'lucide-react';
import type { FeatureFlag } from '../../api/featureFlags/client';
import { fmtCount, filterFlags, toolbarFilter, variantColor, type ToolbarFilter } from '../../api/featureFlags/logic';
import { Select } from '../../components/ui/Dropdown';
import { useProfile } from '../settings/queries';
import { useTime } from '../../components/shell/TimeContext';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { Modal } from '../../components/ui/Modal';
import { TagChips } from '../monitors/common';
import { EnvPill, FlagsHeader, HeroArt, SetupSnippets, Switch, TypeBadge } from './common';
import { useFeatureFlags, useFlagEvaluations } from './queries';
import '../monitors/monitors.css';
import './featureFlags.css';

/** Feature Flags frame: header with Overview and Global Flag History. */
export function FeatureFlagsLayout() {
  return (
    <div className="ff-layout">
      <FlagsHeader />
      <div className="ff-section">
        <Outlet />
      </div>
    </div>
  );
}

const ago = (ms: number) => {
  const s = Math.max(0, (Date.now() - ms) / 1000);
  if (s < 3600) return `${Math.max(1, Math.round(s / 60))}m ago`;
  if (s < 86_400) return `${Math.round(s / 3600)}h ago`;
  return `${Math.round(s / 86_400)}d ago`;
};

/** Overview: the first-run hero, or the flags with search and facets. */
export function FlagsOverview() {
  const q = useFeatureFlags();
  if (q.isPending) return <Loading label="Loading feature flags…" />;
  if (q.isError) return <ErrorState error={q.error} onRetry={() => q.refetch()} />;
  return q.data.flags.length ? <FlagList flags={q.data.flags} /> : <FirstRun />;
}

function FirstRun() {
  const navigate = useNavigate();
  const [docs, setDocs] = useState(false);
  return (
    <div className="ff-overview">
      <section className="ff-hero">
        <div>
          <div className="ff-hero-kicker">
            <span className="ff-hero-icon">
              <Flag size={18} />
            </span>
            Feature Flags
          </div>
          <h1>
            Ship new functionality
            <br />
            quickly and reliably
          </h1>
          <p>
            Turn code paths on per environment, target the users who should see them first, and roll out step by step while you watch the evaluations. Applications ask through OpenFeature, so no
            vendor SDK goes into your code.
          </p>
          <div className="ff-hero-actions">
            <button className="btn ff-btn-white" onClick={() => navigate('/feature-flags/create')}>
              Create Flag <Plus size={16} />
            </button>
            <button className="btn ff-btn-ghost" onClick={() => setDocs(true)}>
              View Docs
            </button>
          </div>
        </div>
        <HeroArt />
      </section>
      <div className="ff-hero-steps">
        <div className="ff-hero-step">
          <b>
            <span>1</span> Create a flag
          </b>
          <p>Name it, pick its type and variants. The key is what your code asks for.</p>
        </div>
        <div className="ff-hero-step">
          <b>
            <span>2</span> Connect an OpenFeature SDK
          </b>
          <p>Point the OFREP provider at this install’s intake with an ingest API key.</p>
        </div>
        <div className="ff-hero-step">
          <b>
            <span>3</span> Target and roll out
          </b>
          <p>Turn it on per environment, add rules and percentage rollouts, and follow the change history.</p>
        </div>
      </div>
      {docs && (
        <Modal onClose={() => setDocs(false)} label="Evaluate flags with OpenFeature" width={820} height="auto">
          <div className="small-modal">
            <div className="modal-title-row">
              <h3>
                <BookOpen size={17} /> Evaluate flags with OpenFeature
              </h3>
              <button className="icon-plain" onClick={() => setDocs(false)} title="Close">
                <X size={18} />
              </button>
            </div>
            <p className="muted">
              The intake answers the OpenFeature Remote Evaluation Protocol (OFREP) at <code>/ofrep/v1/evaluate/flags</code>. Any OpenFeature SDK with an OFREP provider can evaluate your flags;
              percentage splits keep each <code>targetingKey</code> on the same variant.
            </p>
            <SetupSnippets flagKey="checkout-new-payment-flow" type="boolean" />
          </div>
        </Modal>
      )}
    </div>
  );
}

function FlagList({ flags }: { flags: FeatureFlag[] }) {
  const navigate = useNavigate();
  const [params, setParams] = useSearchParams();
  const query = params.get('q') ?? '';
  const deferred = useDeferredValue(query);
  const [filter, setFilter] = useState<ToolbarFilter>({});
  const me = useProfile();
  const { range } = useTime();
  const hour = useMemo(() => ({ from: range.to - 3_600_000, to: range.to }), [range.to]);
  const evals = useFlagEvaluations(hour);
  const counts = new Map((evals.data ?? []).map((e) => [e.flag, e.total]));
  const teams = [...new Set(flags.flatMap((f) => f.teams))].sort();
  const shown = toolbarFilter(filterFlags(flags, deferred, {}), filter, me.data?.name, Date.now());
  const set = (patch: ToolbarFilter) => setFilter((f) => ({ ...f, ...patch }));
  const setQuery = (v: string) => {
    const next = new URLSearchParams(params);
    if (v) next.set('q', v);
    else next.delete('q');
    setParams(next, { replace: true });
  };

  return (
    <div className="ff-list">
      <div className="mon-filter ff-search">
        <Search size={16} />
        <input placeholder="Search by name, key, or tags" value={query} onChange={(e) => setQuery(e.target.value)} />
      </div>
      <div className="ff-toolbar">
        <Switch label="My Feature Flags" checked={!!filter.mine} onChange={(v) => set({ mine: v })} />
        <Select label="Teams" value={filter.team ?? ''} options={[{ value: '', label: 'All teams' }, ...teams.map((t) => ({ value: t, label: t }))]} onChange={(v) => set({ team: v || undefined })} />
        <Switch label="Stale Flags" checked={!!filter.stale} onChange={(v) => set({ stale: v })} />
        <Select
          label="Production Status"
          value={filter.prod ?? ''}
          options={[
            { value: '', label: 'All' },
            { value: 'on', label: 'On' },
            { value: 'off', label: 'Off' },
          ]}
          onChange={(v) => set({ prod: (v || undefined) as ToolbarFilter['prod'] })}
        />
      </div>
      <div className="ff-showing">
        Showing <b>{shown.length ? `1–${shown.length}` : '0–0'}</b> of <b>{shown.length}</b> feature flags
      </div>
      <div className="mon-table-wrap">
        <table className="mon-table ff-table">
          <thead>
            <tr>
              <th>DETAILS</th>
              <th className="ff-num">EVALUATIONS (LAST HOUR)</th>
              <th>VARIANTS</th>
              <th>PROD STATUS</th>
              <th>TEAM</th>
              <th className="col-tags">TAGS</th>
              <th>MODIFIED</th>
            </tr>
          </thead>
          <tbody>
            {shown.map((f) => {
              const prod = f.environments.filter((e) => e.isProduction);
              const on = prod.some((e) => e.config.enabled);
              return (
                <tr key={f.id} onClick={() => navigate(`/feature-flags/${f.id}`)}>
                  <td>
                    <div className="ff-name">
                      <b>{f.name}</b>
                      <span className="ff-key">
                        {f.key} <TypeBadge type={f.flagType} />
                      </span>
                    </div>
                  </td>
                  <td className="ff-num">{evals.data ? fmtCount(counts.get(f.key) ?? 0) : evals.isError ? '—' : '…'}</td>
                  <td>
                    <div className="ff-variants">
                      {f.variants.slice(0, 3).map((v, i) => (
                        <span key={v.key} className="ff-variant">
                          <i style={{ background: variantColor(i) }} />
                          {v.key}
                        </span>
                      ))}
                      {f.variants.length > 3 && <span className="muted">+{f.variants.length - 3}</span>}
                    </div>
                  </td>
                  <td>{prod.length ? <EnvPill name={on ? 'On' : 'Off'} enabled={on} /> : <span className="muted">—</span>}</td>
                  <td>{f.teams.length ? f.teams.join(', ') : <span className="muted">—</span>}</td>
                  <td className="col-tags">
                    <TagChips tags={f.tags} max={2} />
                  </td>
                  <td className="muted ff-when" title={`${new Date(f.updatedAt).toLocaleString()}${f.updatedBy ? ` by ${f.updatedBy}` : ''}`}>
                    {ago(f.updatedAt)}
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
        {shown.length === 0 && (
          <div className="ff-no-results">
            <Flag size={56} strokeWidth={1.4} />
            <span>No feature flags found</span>
            <small className="muted">Create a flag to control rollouts and follow their impact.</small>
            <button className="btn btn-primary" onClick={() => navigate('/feature-flags/create')}>
              <Plus size={14} /> New Flag
            </button>
          </div>
        )}
      </div>
    </div>
  );
}
