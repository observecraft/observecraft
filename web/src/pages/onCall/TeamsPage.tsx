import { useDeferredValue, useState } from 'react';
import { Link, useNavigate } from 'react-router-dom';
import { ChevronDown, PhoneCall, Plus, Search, Users, X } from 'lucide-react';
import type { OnCallTeam } from '../../api/onCall/client';
import { onboardedTeams } from '../../api/onCall/logic';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { Menu, Select } from '../../components/ui/Dropdown';
import { Modal } from '../../components/ui/Modal';
import { NewPageModal } from './PagesPage';
import { People, Segmented, errorText } from './common';
import { useOnCallPolicies, useOnCallTeams, useSetOnCallRouting } from './queries';

const NEW_POLICY = '__new';

/** On-Call → Teams: the teams set up for On-Call, with their escalation policy, schedules and who's on call now. */
export function OnCallTeamsPage() {
  const q = useOnCallTeams();
  const policies = useOnCallPolicies();
  const route = useSetOnCallRouting();
  const [query, setQuery] = useState('');
  const deferred = useDeferredValue(query);
  const [scope, setScope] = useState<'mine' | 'all'>('all');
  const [paging, setPaging] = useState<string | null>(null);
  const [adding, setAdding] = useState(false);
  const text = deferred.trim().toLowerCase();
  const onboarded = onboardedTeams(q.data ?? []);
  const rows = onboarded.filter((t) => (scope === 'all' || t.mine) && (!text || t.name.toLowerCase().includes(text) || t.handle.includes(text)));
  const policyOptions = [{ value: '', label: 'None: page every member' }, ...(policies.data ?? []).map((p) => ({ value: p.id, label: p.name }))];
  const add = <AddTeamMenu onExisting={() => setAdding(true)} />;
  if (q.isPending) return <Loading label="Loading teams…" />;
  if (q.isError) return <ErrorState error={q.error} onRetry={() => q.refetch()} />;
  return (
    <div className="onc-list-page">
      {!onboarded.length ? (
        <div className="onc-empty onc-empty-page">
          <h2>No On-Call teams</h2>
          <p className="muted">None of your teams is set up for On-Call yet. Route a team to an escalation policy so its pages reach whoever is on call.</p>
          {add}
        </div>
      ) : (
        <>
          <div className="onc-search-row">
            <div className="mon-filter onc-search">
              <Search size={16} />
              <input placeholder="Search teams" value={query} onChange={(e) => setQuery(e.target.value)} />
            </div>
            {add}
          </div>
          <div className="onc-main-bar">
            <Segmented
              value={scope}
              onChange={setScope}
              options={[
                ['mine', 'My Teams'],
                ['all', 'All Teams'],
              ]}
            />
            <span className="muted onc-showing">
              Pages sent to a team follow its escalation policy. Manage members under <Link to="/teams">Teams</Link>.
            </span>
          </div>
          {route.error && <p className="form-error">{errorText(route.error)}</p>}
          <div className="onc-table-wrap">
            <table className="onc-table onc-teams-table">
              <thead>
                <tr>
                  <th>TEAM</th>
                  <th className="onc-col-routing">ESCALATION POLICY</th>
                  <th>SCHEDULES</th>
                  <th className="onc-col-resp">ON CALL NOW</th>
                  <th className="onc-col-num">OPEN PAGES</th>
                  <th className="onc-col-action" />
                </tr>
              </thead>
              <tbody>
                {rows.map((t) => (
                  <tr key={t.handle} className="static">
                    <td>
                      <b>{t.name}</b>
                      <span className="onc-tags">
                        @{t.handle} · {t.memberCount} member{t.memberCount === 1 ? '' : 's'}
                      </span>
                    </td>
                    <td className="onc-col-routing">
                      <Select value={t.policy?.id ?? ''} onChange={(v) => route.mutate({ handle: t.handle, policyId: v || null })} options={policyOptions} />
                    </td>
                    <td>
                      {t.schedules.length ? (
                        t.schedules.map((s, i) => (
                          <span key={s.id}>
                            {i > 0 && ', '}
                            <Link to={`/on-call/schedules/${s.id}`}>{s.name}</Link>
                          </span>
                        ))
                      ) : (
                        <span className="muted">—</span>
                      )}
                    </td>
                    <td className="onc-col-resp">
                      <People users={t.onCallNow} empty={t.schedules.length ? 'Nobody' : '—'} />
                    </td>
                    <td className="onc-col-num">{t.openPages ? <Link to={`/on-call/pages?team=${encodeURIComponent(t.handle)}`}>{t.openPages}</Link> : <span className="muted">0</span>}</td>
                    <td className="onc-col-action">
                      <button className="btn btn-sm" onClick={() => setPaging(t.handle)}>
                        <PhoneCall size={13} /> Page
                      </button>
                    </td>
                  </tr>
                ))}
                {!rows.length && (
                  <tr className="onc-empty-row">
                    <td colSpan={6}>
                      <p className="muted onc-pad">No teams match.</p>
                    </td>
                  </tr>
                )}
              </tbody>
            </table>
          </div>
        </>
      )}
      {paging && <NewPageModal team={paging} onClose={() => setPaging(null)} />}
      {adding && <AddTeamModal teams={(q.data ?? []).filter((t) => !onboarded.includes(t))} onClose={() => setAdding(false)} />}
    </div>
  );
}

function AddTeamMenu({ onExisting }: { onExisting: () => void }) {
  const navigate = useNavigate();
  return (
    <Menu
      align="right"
      triggerClassName="btn btn-primary"
      trigger={
        <>
          Add Team to On-Call <ChevronDown size={14} />
        </>
      }
      items={[
        { label: 'Add an existing team', icon: <Users size={14} />, onClick: onExisting },
        { label: 'Create a new team', icon: <Plus size={14} />, onClick: () => navigate('/teams') },
      ]}
    />
  );
}

/** Route a team that isn't on call yet to an existing escalation policy, or start a new policy for it. */
function AddTeamModal({ teams, onClose }: { teams: OnCallTeam[]; onClose: () => void }) {
  const navigate = useNavigate();
  const policies = useOnCallPolicies();
  const route = useSetOnCallRouting();
  const [team, setTeam] = useState('');
  const [policy, setPolicy] = useState(NEW_POLICY);
  const submit = () => {
    if (policy === NEW_POLICY) navigate(`/on-call/escalation-policies/new?team=${encodeURIComponent(team)}`);
    else route.mutate({ handle: team, policyId: policy }, { onSuccess: onClose });
  };
  return (
    <Modal onClose={onClose} label="Add Team to On-Call" width={560} height="auto">
      <form
        className="small-modal onc-modal"
        onSubmit={(e) => {
          e.preventDefault();
          submit();
        }}
      >
        <div className="modal-title-row">
          <h3>Add Team to On-Call</h3>
          <button type="button" className="icon-plain" onClick={onClose} title="Close">
            <X size={18} />
          </button>
        </div>
        {!teams.length ? (
          <p className="muted">
            Every team is already on call. Create another under <Link to="/teams">Teams</Link>.
          </p>
        ) : (
          <>
            <Select
              label="Team"
              searchable
              value={team}
              onChange={setTeam}
              options={[{ value: '', label: 'Select a team…' }, ...teams.map((t) => ({ value: t.handle, label: t.name, hint: `@${t.handle}` }))]}
            />
            <Select
              label="Escalation policy"
              value={policy}
              onChange={setPolicy}
              options={[{ value: NEW_POLICY, label: 'Create a new escalation policy' }, ...(policies.data ?? []).map((p) => ({ value: p.id, label: p.name }))]}
            />
            <p className="muted onc-hint">Pages sent to the team follow this policy: who is notified first, and who comes next when nobody acknowledges.</p>
          </>
        )}
        {route.error && <p className="form-error">{errorText(route.error)}</p>}
        <div className="small-modal-actions">
          <button type="button" className="btn" onClick={onClose}>
            Cancel
          </button>
          <button className="btn btn-primary" disabled={!team || route.isPending}>
            {policy === NEW_POLICY ? 'Continue' : 'Add Team'}
          </button>
        </div>
      </form>
    </Modal>
  );
}
