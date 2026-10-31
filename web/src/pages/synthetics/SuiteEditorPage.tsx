import { useMemo, useState } from 'react';
import { Link, useNavigate, useParams, useSearchParams } from 'react-router-dom';
import { ChevronRight, Search, Trash2 } from 'lucide-react';
import type { SyntheticSuite } from '../../api/synthetics/client';
import { fmtUptime, matchesTestQuery } from '../../api/synthetics/logic';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { StatusBadge } from '../monitors/common';
import { TestStatus, TypeBadge } from './common';
import { useDeleteSyntheticSuites, useSaveSyntheticSuite, useSyntheticSuite, useSyntheticTests } from './queries';

const list = (s: string) =>
  s
    .split(',')
    .map((t) => t.trim())
    .filter(Boolean);

/** New Suite / a suite: its name, tags, environment, team and tests. */
export function SuiteEditorPage() {
  const { id } = useParams();
  const q = useSyntheticSuite(id);
  if (id && q.isPending) return <Loading label="Loading suite…" />;
  if (id && q.isError) return <ErrorState error={q.error} onRetry={() => q.refetch()} />;
  return <SuiteForm key={id ?? 'new'} suite={q.data} />;
}

function SuiteForm({ suite }: { suite?: SyntheticSuite }) {
  const navigate = useNavigate();
  // The tests list's Create Suite passes the selected tests (`?tests=a,b`).
  const [params] = useSearchParams();
  const [name, setName] = useState(suite?.name ?? '');
  const [description, setDescription] = useState(suite?.description ?? '');
  const [env, setEnv] = useState(suite?.env ?? '');
  const [team, setTeam] = useState(suite?.team ?? '');
  const [tags, setTags] = useState((suite?.tags ?? []).join(', '));
  const [picked, setPicked] = useState<string[]>(suite?.tests.map((t) => t.id) ?? (params.get('tests') ?? '').split(',').filter(Boolean));
  const [filter, setFilter] = useState('');
  const [range] = useState(() => ({ from: Math.floor(Date.now() / 60_000) * 60_000 - 86_400_000, to: Math.floor(Date.now() / 60_000) * 60_000 }));
  const tests = useSyntheticTests({ query: '', range });
  const save = useSaveSyntheticSuite(suite?.id);
  const remove = useDeleteSyntheticSuites();
  const shown = useMemo(() => (tests.data?.tests ?? []).filter((t) => matchesTestQuery(t, filter)), [tests.data, filter]);
  const toggle = (id: string) => setPicked((p) => (p.includes(id) ? p.filter((x) => x !== id) : [...p, id]));
  const submit = () =>
    save.mutate({ name, description, env, team, tags: list(tags), testIds: picked }, { onSuccess: (s) => (suite ? undefined : navigate(`/synthetics/suites/${s.id}`, { replace: true })) });

  return (
    <div className="syn-page syn-suite-editor">
      <header className="syn-crumbs">
        <Link to="/synthetics/suites">Test Suites</Link>
        <ChevronRight size={16} className="muted" />
        <span>{suite ? suite.name : 'New Suite'}</span>
        {suite && (
          <span className="syn-crumbs-right">
            <StatusBadge status={suite.status} />
            <span className="muted">Uptime (1 day)</span> <b>{fmtUptime(suite.uptime)}</b>
            <button
              className="btn syn-danger"
              disabled={remove.isPending}
              onClick={() => window.confirm(`Delete the suite “${suite.name}”? Its tests stay.`) && remove.mutate([suite.id], { onSuccess: () => navigate('/synthetics/suites') })}
            >
              <Trash2 size={14} /> Delete
            </button>
          </span>
        )}
      </header>
      <section className="syn-card">
        <h3 className="syn-h3">Details</h3>
        <div className="syn-name-grid">
          <label className="form-label syn-span-2">
            Name*
            <input className="syn-input" value={name} onChange={(e) => setName(e.target.value)} placeholder="e.g. Checkout journey" autoFocus={!suite} />
          </label>
          <label className="form-label syn-span-2">
            Description
            <input className="syn-input" value={description} onChange={(e) => setDescription(e.target.value)} placeholder="What the suite covers" />
          </label>
          <label className="form-label">
            Environment
            <input className="syn-input" value={env} onChange={(e) => setEnv(e.target.value)} placeholder="prod" />
          </label>
          <label className="form-label">
            Team
            <input className="syn-input" value={team} onChange={(e) => setTeam(e.target.value)} placeholder="checkout" />
          </label>
          <label className="form-label syn-span-2">
            Tags
            <input className="syn-input" value={tags} onChange={(e) => setTags(e.target.value)} placeholder="service:order-service, tier:critical" />
          </label>
        </div>
      </section>
      <section className="syn-card">
        <h3 className="syn-h3">
          Tests <span className="muted">{picked.length} selected</span>
        </h3>
        <div className="mon-filter">
          <Search size={16} />
          <input placeholder="Filter tests by name, domain or tag (type:http, env:prod, status:alert)" value={filter} onChange={(e) => setFilter(e.target.value)} />
        </div>
        {tests.isPending ? (
          <Loading label="Loading tests…" />
        ) : tests.isError ? (
          <ErrorState error={tests.error} onRetry={() => tests.refetch()} />
        ) : (
          <div className="mon-table-wrap syn-pick">
            <table className="mon-table syn-table">
              <thead>
                <tr>
                  <th className="syn-col-check" />
                  <th className="syn-col-status">STATUS</th>
                  <th className="syn-col-type">TYPE</th>
                  <th>NAME</th>
                  <th>DOMAIN</th>
                  <th className="syn-col-env">ENV</th>
                </tr>
              </thead>
              <tbody>
                {shown.map((t) => (
                  <tr key={t.id} onClick={() => toggle(t.id)} className={picked.includes(t.id) ? 'selected' : undefined}>
                    <td className="syn-col-check">
                      <input type="checkbox" className="checkbox" checked={picked.includes(t.id)} onChange={() => toggle(t.id)} onClick={(e) => e.stopPropagation()} aria-label={`Include ${t.name}`} />
                    </td>
                    <td className="syn-col-status">
                      <TestStatus status={t.status} paused={t.paused} small />
                    </td>
                    <td className="syn-col-type">
                      <TypeBadge type={t.type} />
                    </td>
                    <td className="mon-name">{t.name}</td>
                    <td className="mono syn-domain">{t.domain}</td>
                    <td className="syn-col-env">{t.env || <span className="muted">—</span>}</td>
                  </tr>
                ))}
              </tbody>
            </table>
            {shown.length === 0 && <p className="muted syn-pad">{tests.data.tests.length ? 'No test matches this filter.' : 'Create a test first: suites group existing tests.'}</p>}
          </div>
        )}
      </section>
      {save.isError && <p className="form-error">{(save.error as Error).message}</p>}
      <div className="syn-form-actions">
        <button className="btn" onClick={() => navigate('/synthetics/suites')}>
          Cancel
        </button>
        <button className="btn btn-primary" disabled={!name.trim() || save.isPending} onClick={submit}>
          {save.isPending ? 'Saving…' : suite ? 'Save Suite' : 'Create Suite'}
        </button>
        {save.isSuccess && suite && <span className="muted">Saved.</span>}
      </div>
    </div>
  );
}
