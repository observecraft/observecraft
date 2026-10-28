import { useCallback, useMemo, useState } from 'react';
import { Link, useNavigate, useSearchParams } from 'react-router-dom';
import { ArrowDownWideNarrow, ArrowUpNarrowWide, ChevronDown, ChevronRight, MoreHorizontal, Plus, Search, Tag, Users, X } from 'lucide-react';
import type { CatalogFacet } from '../../api/catalog/client';
import type { ResourcePolicy, ResourcePolicyTemplateView } from '../../api/policies/types';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { Menu, Select, usePopover } from '../../components/ui/Dropdown';
import { Modal } from '../../components/ui/Modal';
import { addTerm, formatQuery, parseQuery, type Term } from '../../lib/searchQuery';
import { TYPE_LABELS } from './catalogModel';
import { CATEGORY_NAMES, groupPolicies, scoreTone, showing, sortPolicies, targetAttribute, type PolicyGroupBy, type PolicySortField } from './policyModel';
import { FilterDropdown } from './ResourceCatalogPage';
import { ResourceLogo } from './ResourceLogo';
import { useDeletePolicy, usePolicies } from './queries';
import './policies.css';

/** Header filters after the pinned Teams box, as catalog keys. */
const FILTERS = ['cloud', 'region', 'env', 'account', 'service'];
const GROUP_OPTIONS: { value: PolicyGroupBy; label: string }[] = [
  { value: 'none', label: 'Select value' },
  { value: 'category', label: 'Category' },
  { value: 'priority', label: 'Priority' },
  { value: 'resource_type', label: 'Resource Type' },
  { value: 'created_by', label: 'Created By' },
];
const COLUMNS: { field: PolicySortField; label: string }[] = [
  { field: 'name', label: 'POLICY NAME' },
  { field: 'priority', label: 'PRIORITY' },
  { field: 'category', label: 'CATEGORY' },
  { field: 'resource_type', label: 'RESOURCE TYPE' },
  { field: 'attribute', label: 'TARGET ATTRIBUTE' },
  { field: 'noncompliant', label: 'NONCOMPLIANT RESOURCES' },
  { field: 'score', label: 'COMPLIANCE SCORE' },
  { field: 'created_by', label: 'CREATED BY' },
];
const BANNER_KEY = 'oc.policies.templateBanner';

function readDismissed(): boolean {
  try {
    return localStorage.getItem(BANNER_KEY) === 'dismissed';
  } catch {
    return false;
  }
}

export function PriorityBadge({ priority }: { priority: number }) {
  return <span className={`pol-priority p${priority}`}>P{priority}</span>;
}

/** "85%" with a pill progress bar. */
export function ScoreBar({ score, compact }: { score: number | null | undefined; compact?: boolean }) {
  const tone = scoreTone(score);
  return (
    <span className={`pol-score${compact ? ' compact' : ''}`}>
      <span className="pol-score-value">{score == null ? '—' : `${Math.round(score)}%`}</span>
      <span className="pol-bar" aria-hidden>
        <span className={`pol-bar-fill ${tone}`} style={{ width: `${score ?? 0}%` }} />
      </span>
    </span>
  );
}

export function TemplateCard({ template: t, onUse }: { template: ResourcePolicyTemplateView; onUse: () => void }) {
  return (
    <button type="button" className="pol-tcard" onClick={onUse} disabled={!t.available} title={t.available ? t.description : `No ${TYPE_LABELS[t.target.resourceType]} resources in the catalog yet`}>
      <span className="pol-tcard-kicker">{CATEGORY_NAMES[t.category].toUpperCase()}</span>
      <span className="pol-tcard-title">
        <ResourceLogo type={t.target.resourceType} size={20} />
        <span>{t.name}</span>
      </span>
      <span className="pol-tcard-meta">
        <span className="muted-text">Priority</span> <PriorityBadge priority={t.priority} />
      </span>
      <span className="pol-tcard-preview">
        <span className="muted-text">Preview Score</span>
        <ScoreBar score={t.available ? (t.preview.score ?? 0) : 0} />
      </span>
    </button>
  );
}

function TemplatesModal({ templates, onClose, onUse }: { templates: ResourcePolicyTemplateView[]; onClose: () => void; onUse: (key: string) => void }) {
  const [filter, setFilter] = useState('');
  const shown = templates.filter((t) => `${t.name} ${t.description} ${CATEGORY_NAMES[t.category]} ${TYPE_LABELS[t.target.resourceType]}`.toLowerCase().includes(filter.toLowerCase()));
  const categories = [...new Set(shown.map((t) => t.category))];
  return (
    <Modal onClose={onClose} label="Policy templates" width={1040} height="min(760px, 100%)">
      <div className="pol-modal">
        <header className="pol-modal-head">
          <h2>Policy Templates</h2>
          <button className="icon-btn" onClick={onClose} title="Close">
            <X size={16} />
          </button>
        </header>
        <p className="muted-text">Ready-made policies for what your hosts, Kubernetes and ECS resources report. Pick one to review its scope and rule before you save it.</p>
        <label className="rc-filter-search pol-modal-search">
          <Search size={13} />
          <input autoFocus placeholder="Search templates" value={filter} onChange={(e) => setFilter(e.target.value)} />
        </label>
        <div className="pol-modal-body">
          {categories.map((c) => (
            <section key={c}>
              <h3>{CATEGORY_NAMES[c]}</h3>
              <div className="pol-tgrid">
                {shown
                  .filter((t) => t.category === c)
                  .map((t) => (
                    <TemplateCard key={t.key} template={t} onUse={() => onUse(t.key)} />
                  ))}
              </div>
            </section>
          ))}
          {!shown.length && <p className="muted-text">No template matches.</p>}
        </div>
      </div>
    </Modal>
  );
}

function TagsFilter({ terms, onChange }: { terms: Term[]; onChange: (t: Term[]) => void }) {
  const { open, setOpen, ref } = usePopover();
  const [text, setText] = useState('');
  const keys = new Set(['team', ...FILTERS]);
  const tags = terms.filter((t): t is Extract<Term, { kind: 'tag' }> => t.kind === 'tag' && !keys.has(t.key));
  const add = () => {
    const i = text.indexOf(':');
    if (i <= 0 || i === text.length - 1) return;
    onChange(parseQuery(addTerm(formatQuery(terms), text.slice(0, i).trim(), text.slice(i + 1).trim())));
    setText('');
  };
  return (
    <div className="dd dd-labelled rc-filter-dd" ref={ref}>
      <span className="dd-floating-label">Tags</span>
      <button className={`dd-trigger rc-filter${tags.length ? ' active' : ''}`} onClick={() => setOpen((o) => !o)}>
        <span className="dd-trigger-label">{tags.length ? `${tags.length} selected` : 'All'}</span>
        <ChevronDown size={14} className="dd-caret" />
      </button>
      {open && (
        <div className="dd-menu rc-filter-menu">
          <label className="rc-filter-search">
            <Tag size={13} />
            <input autoFocus placeholder="key:value, then Enter" value={text} onChange={(e) => setText(e.target.value)} onKeyDown={(e) => e.key === 'Enter' && add()} />
          </label>
          <div className="pol-tag-list">
            {tags.map((t) => (
              <span key={`${t.key}:${t.values.join(',')}`} className="tag-chip">
                {t.negate && '-'}
                {t.key}:{t.values.join(' OR ')}
                <button title="Remove" onClick={() => onChange(terms.filter((x) => x !== t))}>
                  <X size={11} />
                </button>
              </span>
            ))}
            {!tags.length && <span className="muted-text">Scores count only resources with these tags.</span>}
          </div>
        </div>
      )}
    </div>
  );
}

function TeamsBox({ facet, terms, onChange }: { facet?: CatalogFacet; terms: Term[]; onChange: (t: Term[]) => void }) {
  return (
    <div className="pol-teams">
      <Users size={15} />
      <FilterDropdown facetKey="team" label="Teams" facet={facet} terms={terms} onChange={onChange} />
    </div>
  );
}

function EmptyPolicies({ onCreate }: { onCreate: () => void }) {
  return (
    <div className="pol-empty">
      <svg width="96" height="96" viewBox="0 0 96 96" aria-hidden>
        <circle cx="40" cy="40" r="26" fill="var(--bg-subtle)" stroke="var(--border-strong)" strokeWidth="5" />
        <path d="M59 59 82 82" stroke="var(--border-strong)" strokeWidth="9" strokeLinecap="round" />
        <path d="M28 40h24M40 28v24" stroke="var(--brand)" strokeWidth="4" strokeLinecap="round" opacity="0.7" />
      </svg>
      <h2>Get Started with Policies</h2>
      <p className="muted-text">Write down how your infrastructure should be set up, such as owners on every host or pinned image tags, and see which resources don't follow it yet.</p>
      <button className="btn btn-primary" onClick={onCreate}>
        <Plus size={14} /> Create a Policy
      </button>
    </div>
  );
}

function PolicyRowMenu({ policy }: { policy: ResourcePolicy }) {
  const navigate = useNavigate();
  const del = useDeletePolicy();
  return (
    <span onClick={(e) => e.stopPropagation()}>
      <Menu
        triggerClassName="icon-btn"
        trigger={<MoreHorizontal size={15} />}
        items={[
          { label: 'Edit', onClick: () => navigate(`/infrastructure/policies/${policy.id}/edit`) },
          { label: 'Delete', onClick: () => window.confirm(`Delete the policy "${policy.name}" and its history?`) && del.mutate(policy.id) },
        ]}
      />
    </span>
  );
}

/** Infrastructure → Resources → Policies: rules on the catalog's resources and how well they are followed. */
export function PoliciesPage() {
  const navigate = useNavigate();
  const [params, setParams] = useSearchParams();
  const query = params.get('q') ?? '';
  const groupBy = (params.get('group') ?? 'none') as PolicyGroupBy;
  const [sortField, sortDir] = (params.get('sort') ?? 'priority:asc').split(':') as [PolicySortField, 'asc' | 'desc'];
  const [bannerHidden, setBannerHidden] = useState(readDismissed);
  const [showTemplates, setShowTemplates] = useState(false);

  const setParam = useCallback(
    (updates: Record<string, string | null>) =>
      setParams(
        (p) => {
          const next = new URLSearchParams(p);
          for (const [k, v] of Object.entries(updates)) {
            if (v === null || v === '') next.delete(k);
            else next.set(k, v);
          }
          return next;
        },
        { replace: true },
      ),
    [setParams],
  );
  const terms = useMemo(() => parseQuery(query), [query]);
  const setTerms = useCallback((t: Term[]) => setParam({ q: formatQuery(t) }), [setParam]);
  const q = usePolicies(query);
  const data = q.data;
  const facet = (key: string) => data?.facets.find((f) => f.key === key);
  const sorted = useMemo(() => sortPolicies(data?.policies ?? [], sortField, sortDir === 'desc'), [data, sortField, sortDir]);
  const groups = groupPolicies(sorted, groupBy);
  const templates = data?.templates ?? [];
  const featured = [...templates].filter((t) => t.available).slice(0, 3);
  const openTemplate = (key: string) => navigate(`/infrastructure/policies/new?template=${encodeURIComponent(key)}`);
  const dismiss = () => {
    setBannerHidden(true);
    try {
      localStorage.setItem(BANNER_KEY, 'dismissed');
    } catch {
      // Private windows may refuse storage; the banner just comes back next time.
    }
  };
  const onSort = (field: PolicySortField) => setParam({ sort: `${field}:${field === sortField && sortDir === 'asc' ? 'desc' : 'asc'}` });
  const SortIcon = sortDir === 'desc' ? ArrowDownWideNarrow : ArrowUpNarrowWide;
  const total = sorted.length;

  return (
    <div className="rc-page">
      <div className="hosts-scroll pol-page">
        <div className="pol-filterbar">
          <Select label="Group by" value={groupBy} options={GROUP_OPTIONS} onChange={(g) => setParam({ group: g === 'none' ? null : g })} />
          <span className="pol-divider" />
          <TeamsBox facet={facet('team')} terms={terms} onChange={setTerms} />
          {FILTERS.map((k) => (
            <FilterDropdown key={k} facetKey={k} facet={facet(k)} terms={terms} onChange={setTerms} />
          ))}
          <TagsFilter terms={terms} onChange={setTerms} />
          {terms.length > 0 && (
            <button className="link-btn pol-clear" onClick={() => setTerms([])}>
              Clear filters
            </button>
          )}
        </div>

        {!bannerHidden && featured.length > 0 && (
          <section className="pol-banner" aria-label="Policy templates">
            <button className="pol-banner-close icon-btn" onClick={dismiss} title="Dismiss">
              <X size={16} />
            </button>
            <div className="pol-banner-intro">
              <h2>Start with a Policy Template</h2>
              <p>Templates cover common checks for the resources you already send. Each one shows how your infrastructure scores today.</p>
              <button className="link-btn" onClick={() => setShowTemplates(true)}>
                View All <ChevronRight size={14} />
              </button>
            </div>
            <div className="pol-banner-cards">
              {featured.map((t) => (
                <TemplateCard key={t.key} template={t} onUse={() => openTemplate(t.key)} />
              ))}
            </div>
          </section>
        )}

        <div className="hosts-toolbar pol-toolbar">
          <span className="rc-count">{showing(1, total, total)}</span>
          <span className="hosts-toolbar-right">
            <button className="btn" onClick={() => setShowTemplates(true)}>
              View Policy Templates
            </button>
            <span className="btn-group pol-split">
              <button className="btn btn-primary" onClick={() => navigate('/infrastructure/policies/new')}>
                Create a Policy
              </button>
              <Menu
                triggerClassName="btn btn-primary pol-split-caret"
                trigger={<ChevronDown size={14} aria-label="More ways to create" />}
                items={[
                  { label: 'From scratch', onClick: () => navigate('/infrastructure/policies/new') },
                  { label: 'From a template', onClick: () => setShowTemplates(true) },
                ]}
              />
            </span>
          </span>
        </div>

        {q.isPending ? (
          <Loading label="Loading policies…" />
        ) : q.isError ? (
          <ErrorState error={q.error} onRetry={() => q.refetch()} />
        ) : total === 0 ? (
          <EmptyPolicies onCreate={() => navigate('/infrastructure/policies/new')} />
        ) : (
          <table className={`host-table rc-table pol-table${q.isPlaceholderData ? ' is-stale' : ''}`}>
            <thead>
              <tr>
                {COLUMNS.map((c) => (
                  <th key={c.field} className="sortable" onClick={() => onSort(c.field)}>
                    {sortField === c.field && <SortIcon size={13} />} {c.label}
                  </th>
                ))}
                <th aria-label="Actions" />
              </tr>
            </thead>
            <tbody>
              {groups.map((g) => [
                g.label && (
                  <tr key={`group:${g.key}`} className="ht-group pol-group-row">
                    <td colSpan={COLUMNS.length + 1}>
                      {g.label} <span className="muted-text">({g.policies.length})</span>
                    </td>
                  </tr>
                ),
                ...g.policies.map((p) => (
                  <tr key={p.id} onClick={() => navigate(`/infrastructure/policies/${p.id}${query ? `?q=${encodeURIComponent(query)}` : ''}`)}>
                    <td className="pol-name">
                      <Link to={`/infrastructure/policies/${p.id}`} onClick={(e) => e.stopPropagation()}>
                        {p.name}
                      </Link>
                    </td>
                    <td>
                      <PriorityBadge priority={p.priority} />
                    </td>
                    <td>{CATEGORY_NAMES[p.category]}</td>
                    <td>
                      <span className="rc-cloud">
                        <ResourceLogo type={p.resourceType} size={15} />
                        {TYPE_LABELS[p.resourceType]}
                      </span>
                    </td>
                    <td>
                      <code className="pol-attr">{targetAttribute(p.conditions)}</code>
                    </td>
                    <td className="num">
                      <span className={p.evaluation.noncompliant ? 'pol-noncompliant' : undefined}>{p.evaluation.noncompliant.toLocaleString()}</span>
                      <span className="muted-text"> / {p.evaluation.evaluated.toLocaleString()}</span>
                    </td>
                    <td>
                      <ScoreBar score={p.evaluation.score} compact />
                    </td>
                    <td>{p.createdBy ?? <span className="muted-text">—</span>}</td>
                    <td className="pol-actions">
                      <PolicyRowMenu policy={p} />
                    </td>
                  </tr>
                )),
              ])}
            </tbody>
          </table>
        )}
      </div>
      {showTemplates && <TemplatesModal templates={templates} onClose={() => setShowTemplates(false)} onUse={openTemplate} />}
    </div>
  );
}
