import { useState } from 'react';
import { Link } from 'react-router-dom';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { ChevronDown, ChevronRight, Gauge, Pencil, Plus, Trash2, X } from 'lucide-react';
import {
  createMetricRule,
  deleteMetricRule,
  listMetricRules,
  metricRulesOverview,
  updateMetricRule,
  type MetricCardinality,
  type MetricRule,
  type MetricRuleAction,
  type MetricRuleInput,
  type MetricRuleKind,
} from '../../../api/metricRules/client';
import { metricVolume } from '../../../api/dashboards/client';
import { useMetricCatalog } from '../../dashboards/queries';
import { ConfigLayout, type ConfigNavGroup } from '../../../components/shell/ConfigLayout';
import { ErrorState, Loading } from '../../../components/ui/AsyncState';
import { Modal } from '../../../components/ui/Modal';
import { compact, timeAgo } from '../../../lib/format';
import { globMatch, parseTags, ruleSummary } from '../../../lib/metricRules';
import { useAccess } from '../../settings/queries';
import '../metrics.css';

const KEY = ['metrics', 'rules'];
const HOUR_MS = 3_600_000;
const RANGES: { label: string; ms: number }[] = [
  { label: 'Past 1 Hour', ms: HOUR_MS },
  { label: 'Past 4 Hours', ms: 4 * HOUR_MS },
  { label: 'Past 1 Day', ms: 24 * HOUR_MS },
  { label: 'Past 1 Week', ms: 7 * 24 * HOUR_MS },
];

const NAV: ConfigNavGroup[] = [
  {
    section: 'CONFIGURATION',
    items: [
      { to: '/metrics/settings/rules', label: 'Rules' },
      { to: '/metrics/settings/filtered', label: 'Filtered Metrics' },
    ],
  },
];

/** Metrics › Settings: rules over what metrics are stored and how they may be queried. */
export function MetricsSettingsLayout() {
  return <ConfigLayout root={{ to: '/metrics', label: 'Metrics', icon: <Gauge size={22} /> }} title="Settings" nav={NAV} label="Metrics settings" />;
}

type Measure = 'indexed' | 'ingested';

function useOverview(ms: number) {
  const [to] = useState(() => Date.now());
  return useQuery({ queryKey: [...KEY, 'overview', ms, to], queryFn: ({ signal }) => metricRulesOverview({ from: to - ms, to }, signal) });
}

function MeasureToggle({ value, onChange }: { value: Measure; onChange: (m: Measure) => void }) {
  return (
    <div className="btn-group" role="group" aria-label="Cardinality">
      {(['indexed', 'ingested'] as Measure[]).map((m) => (
        <button key={m} type="button" className={`btn btn-sm${value === m ? ' btn-active' : ''}`} onClick={() => onChange(m)}>
          {m === 'indexed' ? 'Indexed' : 'Ingested'}
        </button>
      ))}
    </div>
  );
}

/** Metrics › Settings › Rules: the overview cards and the ingestion and indexing controls. */
export function MetricRulesPage() {
  const qc = useQueryClient();
  const { isAdmin } = useAccess();
  const [rangeMs, setRangeMs] = useState(HOUR_MS);
  const overview = useOverview(rangeMs);
  const rules = useQuery({ queryKey: KEY, queryFn: listMetricRules });
  const [filteredMeasure, setFilteredMeasure] = useState<Measure>('indexed');
  const [topMeasure, setTopMeasure] = useState<Measure>('indexed');
  const [editing, setEditing] = useState<MetricRule | MetricRuleKind | null>(null);
  const refresh = () => qc.invalidateQueries({ queryKey: KEY });
  const remove = useMutation({ mutationFn: deleteMetricRule, onSuccess: refresh });
  const toggle = useMutation({ mutationFn: (r: MetricRule) => updateMetricRule(r.id, { ...r, enabled: !r.enabled }), onSuccess: refresh });
  const filtered = overview.data?.filtered ?? [];
  const total = filtered.reduce((n, m) => n + m[filteredMeasure], 0);
  const top = [...(overview.data?.top ?? [])].sort((a, b) => b[topMeasure] - a[topMeasure]);
  const biggest = Math.max(1, ...top.map((m) => m[topMeasure]));
  return (
    <div className="settings-pad">
      <div className="settings-title-row">
        <h1 className="settings-title">Rules Overview</h1>
        <select className="mr-range" value={rangeMs} onChange={(e) => setRangeMs(Number(e.target.value))} aria-label="Time range">
          {RANGES.map((r) => (
            <option key={r.ms} value={r.ms}>
              {r.label}
            </option>
          ))}
        </select>
      </div>
      {overview.isError ? (
        <ErrorState error={overview.error} onRetry={() => overview.refetch()} />
      ) : (
        <div className="mr-cards">
          <section className="settings-card">
            <div className="mr-card-head">
              <h2>Estimated cardinality of filtered metrics</h2>
              <MeasureToggle value={filteredMeasure} onChange={setFilteredMeasure} />
            </div>
            {overview.isPending ? (
              <Loading />
            ) : (
              <>
                <div className="mr-big">{compact(total)}</div>
                <p className="muted">
                  {filteredMeasure === 'indexed' ? 'Queryable' : 'Stored'} series of the {filtered.length.toLocaleString()} metric{filtered.length === 1 ? '' : 's'} that rules match.{' '}
                  <Link to="/metrics/settings/filtered">See filtered metrics</Link>
                </p>
              </>
            )}
          </section>
          <section className="settings-card">
            <div className="mr-card-head">
              <h2>Highest-cardinality metrics</h2>
              <MeasureToggle value={topMeasure} onChange={setTopMeasure} />
            </div>
            {overview.isPending ? (
              <Loading />
            ) : top.length === 0 ? (
              <p className="muted">No metrics reported in this time range.</p>
            ) : (
              <ul className="mr-bars">
                {top.slice(0, 6).map((m) => (
                  <li key={m.name}>
                    <Link className="mono" to={`/metrics?metric=${encodeURIComponent(m.name)}`} title={m.name}>
                      {m.name}
                    </Link>
                    <span className="num">{compact(m[topMeasure])}</span>
                    <span className="mr-bar">
                      <i style={{ width: `${Math.max(1, (m[topMeasure] / biggest) * 100)}%` }} />
                    </span>
                  </li>
                ))}
              </ul>
            )}
          </section>
        </div>
      )}
      <div className="settings-title-row">
        <h1 className="settings-title">Active Rules</h1>
        {isAdmin && (
          <button className="btn btn-primary" onClick={() => setEditing('ingestion')}>
            <Plus size={14} /> Create Rule
          </button>
        )}
      </div>
      {(remove.error || toggle.error) && <p className="form-error">{((remove.error ?? toggle.error) as Error).message}</p>}
      {rules.isPending ? (
        <Loading />
      ) : rules.isError ? (
        <ErrorState error={rules.error} onRetry={() => rules.refetch()} />
      ) : (
        (['ingestion', 'indexing'] as MetricRuleKind[]).map((kind) => (
          <RuleSection
            key={kind}
            kind={kind}
            rules={rules.data.filter((r) => r.kind === kind)}
            canEdit={isAdmin}
            onEdit={setEditing}
            onToggle={(r) => toggle.mutate(r)}
            onDelete={(r) => window.confirm(`Delete rule "${r.name}"?`) && remove.mutate(r.id)}
          />
        ))
      )}
      {editing && (
        <RuleModal
          rule={typeof editing === 'string' ? null : editing}
          kind={typeof editing === 'string' ? editing : editing.kind}
          onClose={() => setEditing(null)}
          onSaved={() => (setEditing(null), void refresh())}
        />
      )}
    </div>
  );
}

const SECTION: Record<MetricRuleKind, { title: string; about: string }> = {
  ingestion: { title: 'INGESTION CONTROLS', about: 'Applied at the intake to Datadog Agent and API metrics, before they are stored.' },
  indexing: { title: 'INDEXING CONTROLS', about: 'The tags queries may filter and group the metrics by; the series are still stored.' },
};

function RuleSection({
  kind,
  rules,
  canEdit,
  onEdit,
  onToggle,
  onDelete,
}: {
  kind: MetricRuleKind;
  rules: MetricRule[];
  canEdit: boolean;
  onEdit: (r: MetricRule) => void;
  onToggle: (r: MetricRule) => void;
  onDelete: (r: MetricRule) => void;
}) {
  const [open, setOpen] = useState(true);
  return (
    <section className="mr-section">
      <button className="mr-section-toggle" onClick={() => setOpen((o) => !o)} aria-expanded={open}>
        {open ? <ChevronDown size={16} /> : <ChevronRight size={16} />}
        <span className="mr-section-title">
          {SECTION[kind].title} <span className="muted">({rules.length})</span>
        </span>
        <span className="muted">{SECTION[kind].about}</span>
      </button>
      {open &&
        (rules.length === 0 ? (
          <div className="mr-none">No rules defined</div>
        ) : (
          <table className="settings-table ix-table">
            <thead>
              <tr>
                <th>RULE NAME</th>
                <th>LAST UPDATED</th>
                <th>MODIFIED BY</th>
                <th className="num">METRICS IMPACTED</th>
                <th>ENABLED</th>
                <th />
              </tr>
            </thead>
            <tbody>
              {rules.map((r) => (
                <tr key={r.id}>
                  <td>
                    {r.name}
                    <div className="muted">
                      <code>{r.metricPattern}</code> · {ruleSummary(r)}
                    </div>
                  </td>
                  <td className="muted">{timeAgo(r.updatedAt)}</td>
                  <td>{r.updatedBy || '—'}</td>
                  <td className="num">{r.metricsImpacted.toLocaleString()}</td>
                  <td>
                    <button
                      role="switch"
                      aria-checked={r.enabled}
                      aria-label={r.enabled ? `Disable ${r.name}` : `Enable ${r.name}`}
                      className={`switch${r.enabled ? ' on' : ''}`}
                      disabled={!canEdit}
                      onClick={() => onToggle(r)}
                    />
                  </td>
                  <td className="ix-actions">
                    {canEdit && (
                      <>
                        <button className="icon-plain" title="Edit" onClick={() => onEdit(r)}>
                          <Pencil size={15} />
                        </button>
                        <button className="icon-plain" title="Delete" onClick={() => onDelete(r)}>
                          <Trash2 size={15} />
                        </button>
                      </>
                    )}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        ))}
    </section>
  );
}

function RuleModal({ rule, kind: initialKind, onClose, onSaved }: { rule: MetricRule | null; kind: MetricRuleKind; onClose: () => void; onSaved: () => void }) {
  const [name, setName] = useState(rule?.name ?? '');
  const [kind, setKind] = useState<MetricRuleKind>(initialKind);
  const [action, setAction] = useState<MetricRuleAction>(rule?.action ?? (initialKind === 'indexing' ? 'allow_tags' : 'drop'));
  const [pattern, setPattern] = useState(rule?.metricPattern ?? '');
  const [tags, setTags] = useState(rule?.tags.join(', ') ?? '');
  const [enabled, setEnabled] = useState(rule?.enabled ?? true);
  const volume = useQuery({
    queryKey: ['metrics', 'volume', 'rules'],
    queryFn: ({ signal }) => {
      const to = Date.now();
      return metricVolume({ from: to - 24 * HOUR_MS, to }, signal);
    },
    staleTime: 60_000,
  });
  const catalog = useMetricCatalog();
  const matched = pattern.trim() ? (volume.data?.metrics ?? []).filter((m) => globMatch(pattern.trim(), m.name)) : [];
  const knownTags = [...new Set((catalog.data ?? []).filter((m) => matched.some((x) => x.name === m.name)).flatMap((m) => m.tags))].sort();
  const setKindAndAction = (k: MetricRuleKind) => {
    setKind(k);
    setAction(k === 'indexing' ? 'allow_tags' : 'drop');
  };
  const input: MetricRuleInput = { name, kind, metricPattern: pattern, action, tags: parseTags(tags), enabled };
  const save = useMutation({ mutationFn: () => (rule ? updateMetricRule(rule.id, input) : createMetricRule(input)), onSuccess: onSaved });
  return (
    <Modal onClose={onClose} label={rule ? 'Edit rule' : 'Create rule'} width={680} height="auto">
      <form
        className="small-modal ar-form"
        onSubmit={(e) => {
          e.preventDefault();
          save.mutate();
        }}
      >
        <div className="modal-title-row">
          <h3>{rule ? `Edit ${rule.name}` : 'Create Rule'}</h3>
          <button type="button" className="icon-plain" onClick={onClose} title="Close">
            <X size={18} />
          </button>
        </div>
        <label className="form-label">
          Rule name
          <input value={name} onChange={(e) => setName(e.target.value)} placeholder="Drop debug metrics" maxLength={200} required autoFocus />
        </label>
        <div className="ar-grid">
          <label className="form-label">
            Control
            <select value={kind} onChange={(e) => setKindAndAction(e.target.value as MetricRuleKind)}>
              <option value="ingestion">Ingestion — before metrics are stored</option>
              <option value="indexing">Indexing — what queries may use</option>
            </select>
          </label>
          <label className="form-label">
            Action
            {kind === 'ingestion' ? (
              <select value={action} onChange={(e) => setAction(e.target.value as MetricRuleAction)}>
                <option value="drop">Drop the metrics</option>
                <option value="strip_tags">Strip tags</option>
              </select>
            ) : (
              <select value="allow_tags" disabled>
                <option value="allow_tags">Allow only these tags</option>
              </select>
            )}
          </label>
        </div>
        <label className="form-label">
          Metrics — a name, with * for any characters
          <input className="mono" value={pattern} onChange={(e) => setPattern(e.target.value)} placeholder="kubernetes.*" required />
        </label>
        <p className="muted mr-preview">
          {pattern.trim()
            ? `${matched.length.toLocaleString()} metric${matched.length === 1 ? '' : 's'} reported in the past day match${matched.length === 1 ? 'es' : ''}`
            : 'Every metric whose name matches.'}
          {matched.length > 0 &&
            `: ${matched
              .slice(0, 4)
              .map((m) => m.name)
              .join(', ')}${matched.length > 4 ? ', …' : ''}`}
        </p>
        {action !== 'drop' && (
          <label className="form-label">
            {action === 'strip_tags' ? 'Tags to strip' : 'Queryable tags'} (comma separated)
            <input
              className="mono"
              list="mr-tags"
              value={tags}
              onChange={(e) => setTags(e.target.value)}
              placeholder={action === 'strip_tags' ? 'pod_name, container_id' : 'env, service, kube_namespace'}
              required
            />
            <datalist id="mr-tags">
              {knownTags.map((t) => (
                <option key={t} value={t} />
              ))}
            </datalist>
          </label>
        )}
        <label className="ar-check">
          <input type="checkbox" className="checkbox" checked={enabled} onChange={(e) => setEnabled(e.target.checked)} /> Enabled
        </label>
        {save.error && <p className="form-error">{(save.error as Error).message}</p>}
        <div className="small-modal-actions">
          <button type="button" className="btn" onClick={onClose}>
            Cancel
          </button>
          <button className="btn btn-primary" disabled={save.isPending}>
            {rule ? 'Save' : 'Create Rule'}
          </button>
        </div>
      </form>
    </Modal>
  );
}

/** Metrics › Settings › Filtered Metrics: every metric an enabled rule matches, with its cardinality. */
export function FilteredMetricsPage() {
  const [rangeMs, setRangeMs] = useState(HOUR_MS);
  const overview = useOverview(rangeMs);
  return (
    <div className="settings-pad">
      <div className="settings-title-row">
        <h1 className="settings-title">Filtered Metrics</h1>
        <select className="mr-range" value={rangeMs} onChange={(e) => setRangeMs(Number(e.target.value))} aria-label="Time range">
          {RANGES.map((r) => (
            <option key={r.ms} value={r.ms}>
              {r.label}
            </option>
          ))}
        </select>
      </div>
      <p className="muted">
        Metrics matched by an enabled <Link to="/metrics/settings/rules">rule</Link>: ingested series are stored ones, indexed series the distinct combinations of the tags queries may use.
      </p>
      {overview.isPending ? (
        <Loading />
      ) : overview.isError ? (
        <ErrorState error={overview.error} onRetry={() => overview.refetch()} />
      ) : overview.data.filtered.length === 0 ? (
        <section className="settings-card settings-empty">
          <p className="muted">No metric is filtered by a rule.</p>
        </section>
      ) : (
        <table className="settings-table ix-table">
          <thead>
            <tr>
              <th>METRIC</th>
              <th>RULES</th>
              <th className="num">INGESTED SERIES</th>
              <th className="num">INDEXED SERIES</th>
            </tr>
          </thead>
          <tbody>
            {overview.data.filtered.map((m: MetricCardinality) => (
              <tr key={m.name}>
                <td className="mono">
                  <Link to={`/metrics?metric=${encodeURIComponent(m.name)}`}>{m.name}</Link>
                </td>
                <td>{m.rules.join(', ')}</td>
                <td className="num">{m.ingested.toLocaleString()}</td>
                <td className="num">{m.indexed.toLocaleString()}</td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </div>
  );
}
