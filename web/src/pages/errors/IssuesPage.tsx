import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useSearchParams } from 'react-router-dom';
import { ChevronRight, EllipsisVertical, Globe, PanelLeft, PanelLeftClose, Pin, Sparkles, Upload } from 'lucide-react';
import { suggestIssueQuery } from '../../api/errorTracking/client';
import type { DataSource, IssueSort, IssueStateTab, Persona, SuspectedCause } from '../../api/errorTracking/types';
import { FacetPanel } from '../../components/facets/FacetPanel';
import { QueryBar } from '../../components/query/QueryBar';
import { TimePicker } from '../../components/shell/PageHeader';
import { useTime } from '../../components/shell/TimeContext';
import { Empty, ErrorState, Loading } from '../../components/ui/AsyncState';
import { Menu, Select, type Option } from '../../components/ui/Dropdown';
import { formatQuery, parseQuery, type Term } from '../../lib/searchQuery';
import { IssueRow } from './IssueRow';
import { CAUSE_LABEL, PlatformIcon, capCount } from './issueVisuals';
import { IssuePanel } from './panel/IssuePanel';
import { snapRange, useIssueFacets, useIssues, useTeams, useUsers } from './queries';
import './errors.css';

const PAGE_SIZE = 10;

const STATE_TABS: { key: IssueStateTab; label: string }[] = [
  { key: 'for_review', label: 'For Review' },
  { key: 'reviewed', label: 'Reviewed' },
  { key: 'ignored', label: 'Ignored' },
  { key: 'resolved', label: 'Resolved' },
  { key: 'all', label: 'All' },
];

const PERSONAS: Option<Persona>[] = [
  { value: 'all', label: 'All Issues', icon: <Globe size={14} /> },
  { value: 'backend', label: 'Backend Issues' },
  { value: 'browser', label: 'Browser Issues' },
  { value: 'mobile', label: 'Mobile Issues' },
];

const SOURCES: Option<DataSource | 'all'>[] = [
  { value: 'all', label: 'All' },
  { value: 'logs', label: 'Logs' },
  { value: 'rum', label: 'RUM' },
  { value: 'apm', label: 'APM' },
];

const CAUSES: Option<SuspectedCause | 'all'>[] = [
  { value: 'all', label: 'All' },
  ...(Object.keys(CAUSE_LABEL) as SuspectedCause[]).map((c) => ({ value: c, label: CAUSE_LABEL[c] })),
];

const SORTS: Option<IssueSort>[] = [
  { value: 'relevance', label: 'Relevance' },
  { value: 'count', label: 'Count' },
  { value: 'last_seen', label: 'Last seen' },
  { value: 'first_seen', label: 'First seen' },
];

/** Language marker next to Application facet values (JS for browser apps, phone for mobile). */
function appIcon(key: string, value: string) {
  if (key !== 'application') return null;
  return <PlatformIcon platform={value === 'shop-mobile-app' ? 'react-native' : 'browser'} />;
}

export function IssuesPage() {
  const { range: liveRange } = useTime();
  const { from, to } = snapRange(liveRange);
  const range = useMemo(() => ({ from, to }), [from, to]);
  const [params, setParams] = useSearchParams();

  // Shareable state lives in the URL; transient UI state stays local.
  const query = params.get('q') ?? '';
  const state = (params.get('state') as IssueStateTab | null) ?? 'for_review';
  const sort = (params.get('sort') as IssueSort | null) ?? 'relevance';
  const openIssue = params.get('issue');
  const setParam = useCallback(
    (key: string, value: string | null) =>
      setParams(
        (p) => {
          const next = new URLSearchParams(p);
          if (value === null || value === '') next.delete(key);
          else next.set(key, value);
          return next;
        },
        { replace: key !== 'issue' },
      ),
    [setParams],
  );

  const [persona, setPersona] = useState<Persona>('all');
  const [source, setSource] = useState<DataSource | 'all'>('all');
  const [team, setTeam] = useState('all');
  const [assignee, setAssignee] = useState('all');
  const [cause, setCause] = useState<SuspectedCause | 'all'>('all');
  const [investigationReady, setInvestigationReady] = useState(false);
  const [showControls, setShowControls] = useState(true);
  const [teamsOpen, setTeamsOpen] = useState(false);

  const filters = { range, query, persona, source, team, assignee, suspectedCause: cause, investigationReady };
  const issuesQ = useIssues({ ...filters, state, sort, limit: PAGE_SIZE });
  const facetsQ = useIssueFacets(filters);
  const users = useUsers();
  const teams = useTeams();

  const terms = useMemo(() => parseQuery(query), [query]);
  const setTerms = useCallback((next: Term[]) => setParam('q', formatQuery(next)), [setParam]);
  const suggest = useCallback((partial: string) => suggestIssueQuery(range, partial), [range]);

  const pages = issuesQ.data?.pages ?? [];
  const issues = pages.flatMap((p) => p.issues);
  const totals = pages[0]?.totals;

  // Infinite scroll: fetch the next cursor page when the sentinel scrolls into view.
  const sentinel = useRef<HTMLDivElement>(null);
  const { hasNextPage, isFetchingNextPage, fetchNextPage } = issuesQ;
  useEffect(() => {
    const el = sentinel.current;
    if (!el || !hasNextPage) return;
    const io = new IntersectionObserver((entries) => {
      if (entries[0].isIntersecting && !isFetchingNextPage) fetchNextPage();
    });
    io.observe(el);
    return () => io.disconnect();
  }, [hasNextPage, isFetchingNextPage, fetchNextPage]);

  const assigneeOptions: Option<string>[] = [
    { value: 'all', label: 'All' },
    { value: 'unassigned', label: 'Unassigned' },
    ...(users.data ?? []).map((u) => ({ value: u.id, label: u.name })),
  ];
  const teamOptions: Option<string>[] = [{ value: 'all', label: 'All' }, ...(teams.data ?? []).map((t) => ({ value: t, label: t }))];

  const facetHeader = (
    <>
      <div className="et-side-card">
        <button className="et-side-toggle" onClick={() => setTeamsOpen((o) => !o)}>
          <ChevronRight size={14} className={teamsOpen ? 'rot90' : ''} /> Teams
        </button>
        <Pin size={14} className="muted" />
      </div>
      {teamsOpen && (
        <ul className="et-team-list">
          {teamOptions.slice(1).map((t) => (
            <li key={t.value}>
              <label>
                <input type="radio" name="team" checked={team === t.value} onChange={() => setTeam(t.value)} /> {t.label}
              </label>
            </li>
          ))}
          <li>
            <button className="link-btn" onClick={() => setTeam('all')}>
              All teams
            </button>
          </li>
        </ul>
      )}
      <label className="et-side-card">
        <span className="et-side-toggle">
          <Sparkles size={14} className="brand-icon" /> Investigation Ready
        </span>
        <button
          role="switch"
          aria-checked={investigationReady}
          className={`switch${investigationReady ? ' on' : ''}`}
          onClick={() => setInvestigationReady((x) => !x)}
        />
      </label>
    </>
  );

  return (
    <div className="issues-page">
      <div className="section-header et-section-header">
        <div className="section-header-left">
          <button className="page-tab et-views">
            <PanelLeft size={15} /> Views
          </button>
          <div className="page-tab active">
            My View
            <EllipsisVertical size={16} className="muted" />
          </div>
        </div>
        <TimePicker compact />
      </div>

      <div className="et-controls">
        <div className="et-query">
          <QueryBar
            value={query}
            onChange={(q) => setParam('q', q)}
            suggest={suggest}
            placeholder="Enter your query"
            prefix={<Select value={persona} options={PERSONAS} onChange={setPersona} triggerClassName="dd-trigger et-persona" />}
          />
        </div>
        <Select label="Data Source" value={source} options={SOURCES} onChange={setSource} triggerClassName="dd-trigger et-filter" />
        <Select label="Teams" value={team} options={teamOptions} onChange={setTeam} triggerClassName="dd-trigger et-filter" />
        <Select label="Assignee" value={assignee} options={assigneeOptions} onChange={setAssignee} triggerClassName="dd-trigger et-filter" />
        <Select label="Suspected Cause" value={cause} options={CAUSES} onChange={setCause} triggerClassName="dd-trigger et-filter" align="right" />
      </div>

      <div className="et-body">
        {showControls && (
          <FacetPanel
            facets={facetsQ.data?.facets ?? []}
            totalAvailable={facetsQ.data?.totalAvailable}
            noun="filters"
            terms={terms}
            onChange={setTerms}
            mode="exclude"
            valueSearch
            valueLimit={8}
            defaultOpen={['service', 'application']}
            verified={[]}
            renderValueIcon={appIcon}
            header={facetHeader}
          />
        )}

        <div className="et-main">
          <div className="et-toolbar">
            <button className="link-btn et-hide" onClick={() => setShowControls((s) => !s)}>
              {showControls ? <PanelLeftClose size={15} /> : <PanelLeft size={15} />}
              {showControls ? 'Hide Controls' : 'Show Controls'}
            </button>
            <div className="et-toolbar-right">
              <span className="muted-text">Sort by:</span>
              <Select value={sort} options={SORTS} onChange={(s) => setParam('sort', s === 'relevance' ? null : s)} triggerClassName="dd-trigger et-sort" />
              <div className="btn-group">
                <button className="btn">
                  <Upload size={14} /> Create Monitor
                </button>
                <Menu
                  trigger="More…"
                  items={[
                    { label: 'Export as CSV', onClick: () => undefined, disabled: true },
                    { label: 'Copy link to view', onClick: () => navigator.clipboard?.writeText(window.location.href) },
                  ]}
                />
              </div>
            </div>
          </div>

          <div className="et-tabs">
            {STATE_TABS.map((t) => (
              <button key={t.key} className={`et-tab${state === t.key ? ' active' : ''}`} onClick={() => setParam('state', t.key === 'for_review' ? null : t.key)}>
                {t.label}
                <span className="et-tab-count">{totals ? capCount(totals[t.key]) : '–'}</span>
              </button>
            ))}
            <span className="et-showing">
              Showing <strong>{issues.length}</strong> out of <strong>{totals ? capCount(totals[state]) : '–'}</strong> issues
            </span>
          </div>

          <div className="issue-list">
            <div className="issue-list-head">
              <span>ISSUE DETAILS</span>
              <span>COUNT</span>
              <span>STATUS</span>
              <span>ASSIGNEE</span>
            </div>
            {issuesQ.isPending ? (
              <Loading label="Loading issues…" />
            ) : issuesQ.isError ? (
              <ErrorState error={issuesQ.error} onRetry={() => issuesQ.refetch()} />
            ) : issues.length === 0 ? (
              <Empty>No issues match these filters in the selected time range.</Empty>
            ) : (
              <div className={issuesQ.isPlaceholderData ? 'is-stale' : undefined}>
                {issues.map((i) => (
                  <IssueRow key={i.id} issue={i} selected={i.id === openIssue} onOpen={(id) => setParam('issue', id)} />
                ))}
              </div>
            )}
            <div ref={sentinel} className="issue-sentinel">
              {isFetchingNextPage && <Loading label="Loading more issues…" />}
            </div>
          </div>
        </div>
      </div>

      {openIssue && <IssuePanel id={openIssue} onClose={() => setParam('issue', null)} />}
    </div>
  );
}
