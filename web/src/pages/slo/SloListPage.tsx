import { useDeferredValue, useEffect, useState } from 'react';
import { useSearchParams } from 'react-router-dom';
import { PanelLeftClose, PanelLeftOpen, Search } from 'lucide-react';
import type { BurnLevel, SloItem, SloSearchRequest, SloState, SloType, SloWindow } from '../../api/slos/client';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { Select } from '../../components/ui/Dropdown';
import { Pager } from '../../components/ui/Pager';
import { Facet, TagChips, ValuesFacet, fmtCount } from '../monitors/common';
import { BURN_LABEL, STATE_COLOR, STATE_LABEL, StateBadge, TYPE_LABEL, TypeIcon, budgetLeft, fmtPct } from './common';
import { useSlos } from './queries';
import { SloPanel } from './SloPanel';

const STATES: SloState[] = ['breached', 'warning', 'ok', 'no_data'];
const TOGGLE: { value: SloWindow | null; label: string }[] = [
  { value: null, label: 'Primary' },
  { value: '1d', label: 'Daily' },
  { value: '7d', label: 'Weekly' },
  { value: '30d', label: 'Monthly' },
];

/** The SLO list: facets, state summary, time window toggle and the table; `?slo=<id>` opens the side panel. */
export function SloListPage() {
  const [params, setParams] = useSearchParams();
  const query = params.get('q') ?? '';
  const open = params.get('slo');
  const [states, setStates] = useState<string[]>([]);
  const [types, setTypes] = useState<string[]>([]);
  const [burns, setBurns] = useState<string[]>([]);
  const [creators, setCreators] = useState<string[]>([]);
  const [teams, setTeams] = useState<string[]>([]);
  const [services, setServices] = useState<string[]>([]);
  const [envs, setEnvs] = useState<string[]>([]);
  const [tags, setTags] = useState<string[]>([]);
  const [timeWindow, setTimeWindow] = useState<SloWindow | null>(null);
  const [sort, setSort] = useState('state');
  const [limit, setLimit] = useState(50);
  const [offset, setOffset] = useState(0);
  const [controls, setControls] = useState(true);
  const deferred = useDeferredValue(query);

  const req: SloSearchRequest = {
    query: deferred,
    types: types as SloType[],
    states: states as SloState[],
    burnRates: burns as BurnLevel[],
    creators: creators.map((c) => c.split('|')[0]),
    teams,
    services,
    envs,
    tags,
    window: timeWindow,
    sort,
    offset,
    limit,
  };
  const q = useSlos(req);
  useEffect(() => setOffset(0), [deferred, states, types, burns, creators, teams, services, envs, tags, timeWindow, sort, limit]);

  const setParam = (key: string, value: string | null) => {
    const next = new URLSearchParams(params);
    if (value) next.set(key, value);
    else next.delete(key);
    setParams(next, { replace: key === 'q' });
  };
  const f = q.data?.facets;
  const summary = q.data?.summary;
  const pickState = (s: SloState | null) => setStates(s === null || (states.length === 1 && states[0] === s) ? [] : [s]);

  return (
    <div className="mon-page">
      <div className="mon-filter">
        <Search size={16} />
        <input placeholder="Search SLOs (name, description, tag or key:value)" value={query} onChange={(e) => setParam('q', e.target.value)} />
      </div>
      <div className="mon-body">
        {controls && (
          <aside className="mon-controls">
            <Facet title="Teams">
              <ValuesFacet values={f?.teams ?? []} selected={teams} onChange={setTeams} searchable />
            </Facet>
            <Facet title="Creator" defaultOpen={false}>
              <ValuesFacet values={f?.creators ?? []} selected={creators} onChange={setCreators} label={(v) => v.split('|')[1] ?? v} searchable />
            </Facet>
            <Facet title="Type">
              <ValuesFacet values={f?.type ?? []} selected={types} onChange={setTypes} label={(v) => TYPE_LABEL[v as SloType] ?? v} />
            </Facet>
            <Facet title="State">
              <ValuesFacet
                values={[...(f?.state ?? [])].sort((a, b) => STATES.indexOf(a.value as SloState) - STATES.indexOf(b.value as SloState))}
                selected={states}
                onChange={setStates}
                label={(v) => (
                  <>
                    <i className="slo-dot" style={{ background: STATE_COLOR[v as SloState] }} />
                    {STATE_LABEL[v as SloState] ?? v}
                  </>
                )}
              />
            </Facet>
            <Facet title="Burn Rate (1h)">
              <ValuesFacet values={f?.burnRate ?? []} selected={burns} onChange={setBurns} label={(v) => BURN_LABEL[v] ?? v} />
            </Facet>
            <Facet title="Env">
              <ValuesFacet values={f?.envs ?? []} selected={envs} onChange={setEnvs} />
            </Facet>
            <Facet title="Service">
              <ValuesFacet values={f?.services ?? []} selected={services} onChange={setServices} searchable />
            </Facet>
            <Facet title="Tags" defaultOpen={false}>
              <ValuesFacet values={f?.tags ?? []} selected={tags} onChange={setTags} searchable />
            </Facet>
          </aside>
        )}
        <main className="mon-main">
          <div className="slo-summary">
            {(
              [
                ['breached', summary?.breached],
                ['warning', summary?.warning],
                ['ok', summary?.ok],
                ['no_data', summary?.noData],
              ] as [SloState, number | undefined][]
            ).map(([s, n]) => (
              <button key={s} className={`slo-card${states.length === 1 && states[0] === s ? ' active' : ''}`} onClick={() => pickState(s)} style={{ borderTopColor: STATE_COLOR[s] }}>
                <span className="slo-card-label">{STATE_LABEL[s]}</span>
                <span className="slo-card-value">{n ?? '–'}</span>
              </button>
            ))}
            <button className={`slo-card${states.length === 0 ? ' active' : ''}`} onClick={() => pickState(null)}>
              <span className="slo-card-label">Total</span>
              <span className="slo-card-value">{summary?.total ?? '–'}</span>
            </button>
          </div>
          <div className="mon-toolbar">
            <button className="link-btn" onClick={() => setControls((c) => !c)}>
              {controls ? <PanelLeftClose size={16} /> : <PanelLeftOpen size={16} />} {controls ? 'Hide Controls' : 'Show Controls'}
            </button>
            <span className="mon-sep" />
            {q.data && (
              <span>
                Showing{' '}
                <b>
                  {q.data.total === 0 ? 0 : offset + 1}–{offset + q.data.slos.length}
                </b>{' '}
                of <b>{fmtCount(q.data.total)}</b>
              </span>
            )}
            <span className="mon-toolbar-right">
              <span className="btn-group" role="group" aria-label="Time window">
                {TOGGLE.map((t) => (
                  <button
                    key={t.label}
                    className={`btn${timeWindow === t.value ? ' btn-active' : ''}`}
                    onClick={() => setTimeWindow(t.value)}
                    title={t.value ? `Rolling ${t.value}` : "Each SLO's primary time window"}
                  >
                    {t.label}
                  </button>
                ))}
              </span>
              <Select
                label="Sort by"
                value={sort}
                onChange={setSort}
                options={[
                  { value: 'state', label: 'State' },
                  { value: 'name', label: 'Name' },
                  { value: 'sli', label: 'Status (lowest first)' },
                  { value: 'budget', label: 'Error budget (lowest first)' },
                  { value: 'modified', label: 'Last modified' },
                ]}
              />
            </span>
          </div>
          {q.isPending ? (
            <Loading label="Loading SLOs…" />
          ) : q.isError ? (
            <ErrorState error={q.error} onRetry={() => q.refetch()} />
          ) : (
            <div className="mon-table-wrap">
              <table className="mon-table slo-table">
                <thead>
                  <tr>
                    <th className="slo-col-type">TYPE</th>
                    <th>NAME</th>
                    <th className="slo-col-time">TIME</th>
                    <th className="slo-col-num">TARGET</th>
                    <th className="slo-col-status">STATUS</th>
                    <th className="slo-col-budget">ERROR BUDGET LEFT</th>
                    <th className="col-tags">TAGS</th>
                    <th className="slo-col-teams">TEAMS</th>
                  </tr>
                </thead>
                <tbody>
                  {q.data.slos.map((s) => (
                    <SloRow key={s.id} slo={s} selected={open === s.id} onOpen={() => setParam('slo', s.id)} />
                  ))}
                </tbody>
              </table>
              {q.data.slos.length === 0 && <p className="mon-empty">{q.data.summary.total === 0 && !deferred ? 'No SLOs yet: create one with New SLO.' : 'No SLOs match these filters.'}</p>}
              <div className="mon-pager">
                <span>Results per page:</span>
                <Select value={String(limit)} onChange={(v) => setLimit(Number(v))} options={['25', '50', '100'].map((v) => ({ value: v, label: v }))} />
                <Pager offset={offset} limit={limit} total={q.data.total} onChange={setOffset} />
              </div>
            </div>
          )}
        </main>
      </div>
      {open && <SloPanel id={open} window={timeWindow ?? undefined} onClose={() => setParam('slo', null)} />}
    </div>
  );
}

function SloRow({ slo, selected, onOpen }: { slo: SloItem; selected: boolean; onOpen: () => void }) {
  const st = slo.status;
  const budget = st.budgetRemaining;
  return (
    <tr className={selected ? 'selected' : ''} onClick={onOpen}>
      <td className="slo-col-type">
        <TypeIcon type={slo.type} />
      </td>
      <td className="mon-name">
        <button className="link-btn slo-name" onClick={onOpen} title={slo.description}>
          {slo.name}
        </button>
      </td>
      <td className="slo-col-time">{st.window}</td>
      <td className="slo-col-num">{fmtPct(st.target)}</td>
      <td className="slo-col-status">
        <span className={`slo-sli slo-sli-${st.state}`}>{st.state === 'no_data' ? 'No Data' : fmtPct(st.sli)}</span>
        {st.state !== 'ok' && st.state !== 'no_data' && <StateBadge state={st.state} />}
      </td>
      <td className="slo-col-budget">
        {budget === null || budget === undefined ? (
          <span className="muted">—</span>
        ) : (
          <span className="slo-budget" title={budgetLeft(st)}>
            <span className={`slo-budget-value${budget < 0 ? ' neg' : ''}`}>{fmtPct(budget, 1)}</span>
            <span className="slo-bar">
              <i style={{ width: `${Math.max(0, Math.min(100, budget))}%`, background: STATE_COLOR[st.state] }} />
            </span>
          </span>
        )}
      </td>
      <td className="col-tags">
        <TagChips tags={slo.tags} max={2} />
      </td>
      <td className="slo-col-teams">
        <TagChips tags={slo.teams} max={2} />
      </td>
    </tr>
  );
}
