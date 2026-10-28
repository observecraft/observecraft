import { useMemo, useState } from 'react';
import { Link, useNavigate, useParams } from 'react-router-dom';
import type { EChartsOption } from 'echarts';
import { ChevronRight, Flag, Pencil, Trash2 } from 'lucide-react';
import type { FeatureFlag, FlagEnvConfig } from '../../api/featureFlags/client';
import { TYPE_LABEL, configProblem, fmtCount, formatVariantValue, serveLabel, variantColor, variantToken } from '../../api/featureFlags/logic';
import { Chart } from '../../components/charts/Chart';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { token } from '../../theme/chartTheme';
import { TagChips } from '../monitors/common';
import { CopyButton } from '../settings/SecuritySettings';
import { EnvPill, SetupSnippets, Swatch, TypeBadge } from './common';
import { HistoryTable } from './FlagHistoryPage';
import { TargetingEditor } from './TargetingEditor';
import { useDeleteFlag, useFeatureFlag, useFlagEvaluationSeries, useFlagHistory, useSaveFlagEnvironment } from './queries';
import '../monitors/monitors.css';
import './featureFlags.css';

type Tab = 'targeting' | 'variants' | 'history' | 'setup';
const TABS: { id: Tab; label: string }[] = [
  { id: 'targeting', label: 'Targeting' },
  { id: 'variants', label: 'Variants' },
  { id: 'history', label: 'Flag History' },
  { id: 'setup', label: 'Use in Code' },
];

const SDK_LABEL: Record<FeatureFlag['sdkAccess'], string> = { all: 'Client and server SDKs', client: 'Client SDKs only', server: 'Server SDKs only' };

/** A flag: its targeting per environment, variants, history and how to evaluate it. */
export function FlagDetailPage() {
  const { id } = useParams();
  const q = useFeatureFlag(id);
  if (q.isPending) return <Loading label="Loading flag…" />;
  if (q.isError) return <ErrorState error={q.error} onRetry={() => q.refetch()} />;
  return <FlagView flag={q.data} />;
}

function FlagView({ flag }: { flag: FeatureFlag }) {
  const navigate = useNavigate();
  const [tab, setTab] = useState<Tab>('targeting');
  const del = useDeleteFlag();
  const remove = () => {
    if (window.confirm(`Delete the flag ${flag.name}? SDKs asking for ${flag.key} get FLAG_NOT_FOUND and use their code default.`))
      del.mutate(flag.id, { onSuccess: () => navigate('/feature-flags') });
  };
  return (
    <div className="ff-layout">
      <header className="mon-crumbs">
        <Link to="/feature-flags" className="mon-crumb-root">
          <Flag size={22} /> Feature Flags
        </Link>
        <ChevronRight size={18} className="muted" />
        <span>{flag.name}</span>
      </header>
      <div className="ff-section">
        <div className="ff-page-head">
          <div>
            <h1>{flag.name}</h1>
            <div className="ff-meta">
              <span className="ff-key">{flag.key}</span>
              <CopyButton text={flag.key} />
              <TypeBadge type={flag.flagType} />
              <span>{flag.lifecycle === 'permanent' ? 'Permanent' : 'Temporary'}</span>
              <span>{SDK_LABEL[flag.sdkAccess]}</span>
              {flag.environments.map((e) => (
                <EnvPill key={e.environmentId} name={e.environmentName} enabled={!!e.config.enabled} />
              ))}
            </div>
            {flag.description && <p className="ff-desc">{flag.description}</p>}
          </div>
          <div className="ff-head-actions">
            <button className="btn" onClick={() => navigate(`/feature-flags/${flag.id}/edit`)}>
              <Pencil size={14} /> Edit
            </button>
            <button className="btn btn-danger" onClick={remove} disabled={del.isPending}>
              <Trash2 size={14} /> Delete
            </button>
          </div>
        </div>
        <nav className="ff-tabs" role="tablist">
          {TABS.map((t) => (
            <button key={t.id} role="tab" aria-selected={tab === t.id} className={tab === t.id ? 'active' : ''} onClick={() => setTab(t.id)}>
              {t.label}
            </button>
          ))}
        </nav>
        {tab === 'targeting' && <TargetingTab flag={flag} />}
        {tab === 'variants' && <VariantsTab flag={flag} />}
        {tab === 'history' && <FlagHistory id={flag.id} />}
        {tab === 'setup' && (
          <div className="ff-page-body single">
            <div className="ff-card">
              <div className="ff-card-head">
                <h3>Evaluate {flag.key} with OpenFeature</h3>
              </div>
              <div className="ff-card-body">
                <SetupSnippets flagKey={flag.key} type={flag.flagType} />
              </div>
            </div>
          </div>
        )}
      </div>
    </div>
  );
}

function TargetingTab({ flag }: { flag: FeatureFlag }) {
  const [envId, setEnvId] = useState(() => (flag.environments.find((e) => e.isProduction) ?? flag.environments[0])?.environmentId);
  const env = flag.environments.find((e) => e.environmentId === envId) ?? flag.environments[0];
  // Unsaved edits per environment.
  const [drafts, setDrafts] = useState<Record<string, FlagEnvConfig>>({});
  const save = useSaveFlagEnvironment(flag.id);
  if (!env) {
    return (
      <div className="ff-page-body single">
        <div className="ff-empty">
          <h3>No environments yet</h3>
          <p>
            Create one in <Link to="/feature-flags/environments">Settings → Environments</Link> to turn this flag on.
          </p>
        </div>
      </div>
    );
  }
  const draft = drafts[env.environmentId] ?? env.config;
  const dirty = JSON.stringify(draft) !== JSON.stringify(env.config);
  const problem = configProblem(draft, flag.variants);
  const discard = () => setDrafts(({ [env.environmentId]: _, ...rest }) => rest);
  const submit = () =>
    save.mutate(
      { environmentId: env.environmentId, update: { version: flag.version, config: draft } },
      {
        onSuccess: discard,
      },
    );
  return (
    <div className="ff-page-body">
      <div>
        <div className="ff-env-tabs" role="tablist" aria-label="Environment">
          {flag.environments.map((e) => (
            <button
              key={e.environmentId}
              role="tab"
              aria-selected={e.environmentId === env.environmentId}
              className={`ff-env-tab${e.environmentId === env.environmentId ? ' active' : ''}`}
              onClick={() => setEnvId(e.environmentId)}
            >
              <span className={`ff-dot${e.config.enabled ? ' on' : ''}`} />
              {e.environmentName}
              {e.isProduction && <span className="ff-prod-tag">PROD</span>}
              {drafts[e.environmentId] && JSON.stringify(drafts[e.environmentId]) !== JSON.stringify(e.config) && <span title="Unsaved changes">•</span>}
            </button>
          ))}
        </div>
        <div className="ff-card">
          <div className="ff-card-head">
            <h3>Targeting in {env.environmentName}</h3>
            <span className="ff-card-tools muted">{env.updatedAt ? `Updated ${new Date(env.updatedAt).toLocaleString()}${env.updatedBy ? ` by ${env.updatedBy}` : ''}` : 'Never configured'}</span>
          </div>
          <div className="ff-card-body">
            <TargetingEditor key={env.environmentId} envName={env.environmentName} config={draft} variants={flag.variants} onChange={(c) => setDrafts((d) => ({ ...d, [env.environmentId]: c }))} />
          </div>
          <div className="ff-save-bar">
            {problem ? (
              <span className="ff-error-text">{problem}</span>
            ) : save.isError ? (
              <span className="ff-error-text">{(save.error as Error).message}</span>
            ) : (
              <span className="muted">{dirty ? 'Unsaved changes' : 'All changes saved'}</span>
            )}
            <button className="btn" disabled={!dirty || save.isPending} onClick={discard}>
              Discard
            </button>
            <button className="btn btn-primary" style={{ marginLeft: 0 }} disabled={!dirty || !!problem || save.isPending} onClick={submit}>
              Save Changes
            </button>
          </div>
        </div>
      </div>
      <aside className="ff-side">
        <Evaluations flag={flag} />
        <div className="ff-card">
          <div className="ff-card-head">
            <h3>Details</h3>
          </div>
          <div className="ff-card-body">
            <dl>
              <dt>Default</dt>
              <dd>{serveLabel(env.config.defaultServe ?? {})}</dd>
              <dt>Rules</dt>
              <dd>{(env.config.rules ?? []).length}</dd>
              <dt>Teams</dt>
              <dd>{flag.teams.join(', ') || '—'}</dd>
              <dt>Tags</dt>
              <dd>{flag.tags.length ? <TagChips tags={flag.tags} max={6} /> : '—'}</dd>
              <dt>Created</dt>
              <dd>
                {new Date(flag.createdAt).toLocaleDateString()}
                {flag.createdBy ? ` by ${flag.createdBy}` : ''}
              </dd>
              <dt>Version</dt>
              <dd>v{flag.version}</dd>
            </dl>
          </div>
        </div>
      </aside>
    </div>
  );
}

function Evaluations({ flag }: { flag: FeatureFlag }) {
  const range = useMemo(() => {
    const to = Date.now();
    return { from: to - 86_400_000, to };
  }, []);
  const q = useFlagEvaluationSeries(flag.key, range);
  const series = q.data ?? [];
  const total = series.reduce((s, v) => s + v.points.reduce((a, [, n]) => a + n, 0), 0);
  const indexOf = (variant: string) => flag.variants.findIndex((v) => v.key === variant);
  const option = useMemo<EChartsOption>(
    () => ({
      grid: { left: 46, right: 8, top: 10, bottom: 22 },
      tooltip: { trigger: 'axis' },
      xAxis: { type: 'time', min: range.from, max: range.to },
      yAxis: { type: 'value', minInterval: 1 },
      series: series.map((s) => {
        const i = indexOf(s.variant);
        return { name: s.variant, type: 'bar', stack: 'evaluations', barMaxWidth: 12, data: s.points, itemStyle: { color: i >= 0 ? token(variantToken(i)) : token('--status-nodata') } };
      }),
    }),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [series],
  );
  return (
    <div className="ff-card">
      <div className="ff-card-head">
        <h3>Evaluations</h3>
        <span className="ff-card-tools muted">{fmtCount(total)} in the past day</span>
      </div>
      <div className="ff-card-body">
        {q.isPending ? (
          <Loading label="Loading evaluations…" />
        ) : series.length === 0 ? (
          <p className="ff-hint">No evaluations in the past day. Connect an OpenFeature SDK (Use in Code) to see them here.</p>
        ) : (
          <>
            <Chart option={option} height={150} />
            <div className="ff-legend">
              {series.map((s) => (
                <span key={s.variant}>
                  <Swatch color={indexOf(s.variant) >= 0 ? variantColor(indexOf(s.variant)) : 'var(--status-nodata)'} />
                  {s.variant === 'N/A' ? 'code default (off)' : s.variant}
                </span>
              ))}
            </div>
          </>
        )}
      </div>
    </div>
  );
}

function VariantsTab({ flag }: { flag: FeatureFlag }) {
  return (
    <div className="ff-page-body single">
      <div className="ff-card">
        <div className="ff-card-head">
          <h3>
            {flag.variants.length} {TYPE_LABEL[flag.flagType]} variant{flag.variants.length === 1 ? '' : 's'}
          </h3>
        </div>
        <div className="ff-card-body">
          {flag.variants.map((v, i) => (
            <div key={v.key} className="ff-variant">
              <div className="ff-variant-head">
                <Swatch color={variantColor(i)} />
                <span className="ff-variant-name">{v.key}</span>
                <span className="ff-variant-value">{flag.flagType === 'json' ? '' : formatVariantValue(flag.flagType, v.value)}</span>
                <span className="ff-variant-tools">{flag.defaultVariant === v.key && <span className="ff-default-label">Default</span>}</span>
              </div>
              {flag.flagType === 'json' && (
                <div className="ff-variant-body" style={{ gridTemplateColumns: '1fr' }}>
                  <pre className="ff-key" style={{ margin: 0 }}>
                    {formatVariantValue('json', v.value)}
                  </pre>
                </div>
              )}
            </div>
          ))}
        </div>
      </div>
    </div>
  );
}

function FlagHistory({ id }: { id: string }) {
  const q = useFlagHistory(id);
  return (
    <div className="ff-history">{q.isPending ? <Loading label="Loading history…" /> : q.isError ? <ErrorState error={q.error} onRetry={() => q.refetch()} /> : <HistoryTable changes={q.data} />}</div>
  );
}
