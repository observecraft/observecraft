import { useDeferredValue, useState } from 'react';
import { useNavigate, useParams, useSearchParams } from 'react-router-dom';
import { Plus, Search } from 'lucide-react';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { useProfile } from '../settings/queries';
import { Segmented, Showing, TeamFacet } from './common';
import { PolicyModal } from './PolicyEditor';
import { useOnCallPolicies, useOnCallTeams } from './queries';

/** On-Call → Escalation Policies. */
export function PoliciesPage() {
  const navigate = useNavigate();
  const { id } = useParams();
  const [params] = useSearchParams();
  const q = useOnCallPolicies();
  const teams = useOnCallTeams();
  const me = useProfile();
  const [query, setQuery] = useState('');
  const deferred = useDeferredValue(query);
  const [scope, setScope] = useState<'all' | 'mine'>('all');
  const [selected, setSelected] = useState<string[]>([]);
  const [facetOpen, setFacetOpen] = useState(false);
  const myTeams = new Set((teams.data ?? []).filter((t) => t.mine).map((t) => t.handle));
  const text = deferred.trim().toLowerCase();
  const teamName = (h: string) => (teams.data ?? []).find((t) => t.handle === h)?.name ?? h;
  const rows = (q.data ?? []).filter(
    (p) =>
      (!text || p.name.toLowerCase().includes(text) || (p.team ?? '').includes(text) || p.tags.some((t) => t.includes(text))) &&
      (!selected.length || (p.team != null && selected.includes(p.team))) &&
      (scope === 'all' || (p.team != null && myTeams.has(p.team)) || p.steps.some((s) => s.targets.some((t) => t.kind === 'user' && t.id === me.data?.id))),
  );
  const counts: Record<string, number> = {};
  for (const p of q.data ?? []) if (p.team) counts[p.team] = (counts[p.team] ?? 0) + 1;
  const facet = (
    <TeamFacet teams={(teams.data ?? []).map((t) => ({ handle: t.handle, name: t.name }))} selected={selected} onChange={setSelected} counts={counts} open={facetOpen} onOpenChange={setFacetOpen} />
  );
  return (
    <div className="onc-list-page">
      <div className="onc-search-row">
        <div className="mon-filter onc-search">
          <Search size={16} />
          <input placeholder="Search escalation policies by name, team or tag" value={query} onChange={(e) => setQuery(e.target.value)} />
        </div>
        <button className="btn btn-primary" onClick={() => navigate('/on-call/escalation-policies/new')}>
          <Plus size={15} /> New Escalation Policy
        </button>
      </div>
      <div className={`onc-split${facetOpen ? '' : ' no-controls'}`}>
        {facetOpen && <aside className="onc-side">{facet}</aside>}
        <div className="onc-main">
          <div className="onc-main-bar">
            {!facetOpen && facet}
            <Segmented
              value={scope}
              onChange={setScope}
              options={[
                ['all', 'All Escalation Policies'],
                ['mine', 'My Escalation Policies'],
              ]}
            />
            <span className="onc-bar-sep" />
            <Showing count={rows.length} noun="Policies" />
          </div>
          {q.isPending ? (
            <Loading label="Loading escalation policies…" />
          ) : q.isError ? (
            <ErrorState error={q.error} onRetry={() => q.refetch()} />
          ) : (
            <div className="onc-table-wrap">
              <table className="onc-table">
                <thead>
                  <tr>
                    <th>ESCALATION POLICY</th>
                    <th className="onc-col-team">OWNED BY</th>
                    <th>TAGS</th>
                    <th>USED BY</th>
                  </tr>
                </thead>
                <tbody>
                  {rows.length === 0 ? (
                    <tr className="onc-empty-row">
                      <td colSpan={4}>
                        <div className="onc-empty">
                          <Search className="onc-empty-icon" size={64} strokeWidth={1.5} />
                          <h3>No matching results found</h3>
                          {!q.data.length && <p className="muted">An escalation policy says who is notified first and who comes next when nobody acknowledges.</p>}
                        </div>
                      </td>
                    </tr>
                  ) : (
                    rows.map((p) => (
                      <tr key={p.id} onClick={() => navigate(`/on-call/escalation-policies/${p.id}`)}>
                        <td>
                          <span className="onc-link">{p.name}</span>
                          <span className="onc-tags">
                            {p.steps.length} step{p.steps.length === 1 ? '' : 's'}
                            {p.repeat > 0 ? ` · repeats ${p.repeat}×` : ''}
                          </span>
                        </td>
                        <td className="onc-col-team">{p.team ? teamName(p.team) : <span className="muted">—</span>}</td>
                        <td>
                          {p.tags.length ? (
                            p.tags.slice(0, 3).map((t) => (
                              <span key={t} className="onc-chip">
                                {t}
                              </span>
                            ))
                          ) : (
                            <span className="muted">—</span>
                          )}
                        </td>
                        <td>{p.usedBy.length ? p.usedBy.map((t) => `@${t}`).join(', ') : <span className="muted">Not routed</span>}</td>
                      </tr>
                    ))
                  )}
                </tbody>
              </table>
            </div>
          )}
        </div>
      </div>
      {id && <PolicyModal id={id === 'new' ? undefined : id} team={params.get('team') ?? undefined} onClose={() => navigate('/on-call/escalation-policies')} />}
    </div>
  );
}
