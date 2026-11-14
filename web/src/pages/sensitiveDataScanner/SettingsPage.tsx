// Organization Settings → Sensitive Data Scanner: the scanning groups and their rules, in scanning order, and the
// standard pattern library. The log indexers apply changes within seconds; only admins can change them (the
// server answers others with an error, shown here).
import { useEffect, useMemo, useState } from 'react';
import { Link } from 'react-router-dom';
import { ArrowDown, ArrowUp, ChevronDown, ChevronRight, Pencil, Plus, Search, Trash2 } from 'lucide-react';
import { CATEGORY_LABELS, REPLACEMENT_LABELS, type ReplacementKind, type SdsGroup, type SdsRule, type SdsStandardPattern } from '../../api/sensitiveDataScanner/client';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { FilterQuery } from '../logs/config/configUi';
import { GroupEditor, RuleEditor } from './Editors';
import { groupInput, moved, ruleInput, useDeleteGroup, useDeleteRule, useReorderGroups, useReorderRules, useSaveGroup, useSaveRule, useSdsConfiguration, useStandardPatterns } from './queries';
import '../settings/settings.css';
import './sds.css';

type Tab = 'groups' | 'library';
type RuleEdit = { rule: SdsRule | null; groupId: string; standardPatternId?: string };

/** What a rule does with a match, in a few words. */
function action(r: SdsRule): string {
  const k = r.replacement.kind as ReplacementKind;
  if (k === 'replacement_string') return `Replace with ${r.replacement.replacementString ?? ''}`;
  if (k.startsWith('partial_'))
    return `${REPLACEMENT_LABELS[k].replace('the first', `the first ${r.replacement.numberOfChars ?? 0}`).replace('the last', `the last ${r.replacement.numberOfChars ?? 0}`)}`;
  return REPLACEMENT_LABELS[k] ?? k;
}

export function SensitiveDataScannerPage() {
  const cfg = useSdsConfiguration();
  const lib = useStandardPatterns();
  const [tab, setTab] = useState<Tab>('groups');
  const [editingGroup, setEditingGroup] = useState<SdsGroup | 'new' | null>(null);
  const [editingRule, setEditingRule] = useState<RuleEdit | null>(null);
  /** The group a rule was just saved into, shown open. */
  const [focus, setFocus] = useState<string | null>(null);
  const patterns = lib.data ?? [];
  const groups = cfg.data?.groups ?? [];
  const ruleCount = groups.reduce((n, g) => n + g.rules.length, 0);
  return (
    <div className="settings-pad sds-page">
      <div className="settings-title-row">
        <h1 className="settings-title">Sensitive Data Scanner</h1>
        <Link className="link-muted" to="/sensitive-data-scanner/findings">
          Findings
        </Link>
      </div>
      <p className="muted sds-intro">
        Scanning groups pick logs with a search query; their rules find credentials, card numbers and personal data in those logs before they are stored, then mask, hash or replace each match and tag
        the log (<span className="mono">sensitive_data:…</span>). Groups run in the order below, and each group's rules in theirs. Live tail, indexes, archives and forwarding all receive the scanned
        logs.
      </p>
      <nav className="settings-tabs">
        <button className={tab === 'groups' ? 'active' : ''} onClick={() => setTab('groups')}>
          Scanning groups {cfg.data && <span className="sds-count">{groups.length}</span>}
        </button>
        <button className={tab === 'library' ? 'active' : ''} onClick={() => setTab('library')}>
          Library {lib.data && <span className="sds-count">{patterns.length}</span>}
        </button>
      </nav>
      {cfg.isPending ? (
        <Loading />
      ) : cfg.isError ? (
        <ErrorState error={cfg.error} onRetry={() => cfg.refetch()} />
      ) : tab === 'groups' ? (
        <Groups groups={groups} patterns={patterns} ruleCount={ruleCount} focus={focus} onEditGroup={setEditingGroup} onEditRule={setEditingRule} />
      ) : (
        <LibraryTab patterns={patterns} isPending={lib.isPending} groups={groups} onAdd={(standardPatternId) => setEditingRule({ rule: null, groupId: groups[0]?.id ?? '', standardPatternId })} />
      )}
      {editingGroup && <GroupEditor group={editingGroup === 'new' ? null : editingGroup} onClose={() => setEditingGroup(null)} />}
      {editingRule && <RuleEditor {...editingRule} groups={groups} patterns={patterns} onClose={() => setEditingRule(null)} onSaved={(r) => (setTab('groups'), setFocus(r.groupId))} />}
    </div>
  );
}

function Groups({
  groups,
  patterns,
  ruleCount,
  focus,
  onEditGroup,
  onEditRule,
}: {
  groups: SdsGroup[];
  patterns: SdsStandardPattern[];
  ruleCount: number;
  focus: string | null;
  onEditGroup: (g: SdsGroup | 'new') => void;
  onEditRule: (e: RuleEdit) => void;
}) {
  const [open, setOpen] = useState<Set<string>>(() => new Set(groups.slice(0, 1).map((g) => g.id)));
  useEffect(() => {
    if (focus) setOpen((s) => new Set(s).add(focus));
  }, [focus]);
  const [search, setSearch] = useState('');
  const saveGroup = useSaveGroup();
  const removeGroup = useDeleteGroup();
  const reorder = useReorderGroups();
  const needle = search.trim().toLowerCase();
  const shown = groups.filter(
    (g) =>
      !needle ||
      g.name.toLowerCase().includes(needle) ||
      g.query.toLowerCase().includes(needle) ||
      g.rules.some((r) => r.name.toLowerCase().includes(needle) || (r.pattern ?? '').toLowerCase().includes(needle)),
  );
  const error = saveGroup.error ?? removeGroup.error ?? reorder.error;
  const toggle = (id: string) =>
    setOpen((s) => {
      const n = new Set(s);
      if (n.has(id)) n.delete(id);
      else n.add(id);
      return n;
    });
  return (
    <>
      <div className="mapping-bar">
        <label className="settings-search wide">
          <Search size={16} />
          <input placeholder="Filter groups and rules" value={search} onChange={(e) => setSearch(e.target.value)} />
        </label>
        <button className="btn" onClick={() => onEditRule({ rule: null, groupId: groups[0]?.id ?? '' })} disabled={groups.length === 0}>
          <Plus size={14} /> New rule
        </button>
        <button className="btn btn-primary" onClick={() => onEditGroup('new')}>
          <Plus size={14} /> New group
        </button>
      </div>
      <div className="muted sds-summary">
        <b>{groups.length}</b> groups, <b>{ruleCount}</b> rules, <b>{groups.filter((g) => g.enabled).length}</b> groups enabled
      </div>
      {error && <p className="form-error">{(error as Error).message}</p>}
      {groups.length === 0 ? (
        <section className="settings-card settings-empty">
          <p>No scanning groups yet.</p>
          <p className="muted">Create a group to choose which logs to scan, then add rules from the library or with your own patterns.</p>
        </section>
      ) : (
        <div className="sds-groups">
          {shown.map((g) => {
            const i = groups.indexOf(g);
            const isOpen = open.has(g.id) || !!needle;
            return (
              <section key={g.id} className={`sds-group${g.enabled ? '' : ' off'}`}>
                <div className="sds-group-head">
                  <button className="icon-plain" onClick={() => toggle(g.id)} aria-expanded={isOpen} title={isOpen ? 'Hide rules' : 'Show rules'}>
                    {isOpen ? <ChevronDown size={16} /> : <ChevronRight size={16} />}
                  </button>
                  <span className="sds-num">{i + 1}</span>
                  <span className="sds-group-name">
                    <b>{g.name}</b>
                    {g.description && <span className="muted">{g.description}</span>}
                  </span>
                  <FilterQuery query={g.query} />
                  <span className="sds-chips">
                    {g.products.map((p) => (
                      <span key={p} className="role-chip">
                        {p}
                      </span>
                    ))}
                  </span>
                  <span className="muted sds-nowrap" title="Share of the matching logs scanned">
                    {g.samplingRate}% sampled
                  </span>
                  <span className="muted sds-nowrap">
                    {g.rules.length} {g.rules.length === 1 ? 'rule' : 'rules'}
                  </span>
                  <span className="sds-actions">
                    <button className="icon-plain" title="Move up" disabled={i === 0 || reorder.isPending} onClick={() => reorder.mutate(moved(groups, i, -1).map((x) => x.id))}>
                      <ArrowUp size={14} />
                    </button>
                    <button className="icon-plain" title="Move down" disabled={i === groups.length - 1 || reorder.isPending} onClick={() => reorder.mutate(moved(groups, i, 1).map((x) => x.id))}>
                      <ArrowDown size={14} />
                    </button>
                    <button className="icon-plain" title="Edit group" onClick={() => onEditGroup(g)}>
                      <Pencil size={14} />
                    </button>
                    <button className="icon-plain" title="Delete group" onClick={() => window.confirm(`Delete the group "${g.name}" and its ${g.rules.length} rules?`) && removeGroup.mutate(g.id)}>
                      <Trash2 size={14} />
                    </button>
                    <button
                      role="switch"
                      aria-checked={g.enabled}
                      aria-label={g.enabled ? `Disable ${g.name}` : `Enable ${g.name}`}
                      className={`switch${g.enabled ? ' on' : ''}`}
                      disabled={saveGroup.isPending}
                      onClick={() => saveGroup.mutate({ id: g.id, input: { ...groupInput(g), enabled: !g.enabled } })}
                    />
                  </span>
                </div>
                {isOpen && <Rules group={g} patterns={patterns} onEdit={(rule) => onEditRule({ rule, groupId: g.id })} />}
              </section>
            );
          })}
          {shown.length === 0 && <p className="muted">No group or rule matches “{search}”.</p>}
        </div>
      )}
    </>
  );
}

function Rules({ group, patterns, onEdit }: { group: SdsGroup; patterns: SdsStandardPattern[]; onEdit: (r: SdsRule | null) => void }) {
  const save = useSaveRule();
  const remove = useDeleteRule();
  const reorder = useReorderRules();
  const rules = group.rules;
  const error = save.error ?? remove.error ?? reorder.error;
  const nameOf = (id: string) => patterns.find((p) => p.id === id)?.name ?? id;
  return (
    <div className="sds-rules">
      {error && <p className="form-error">{(error as Error).message}</p>}
      {rules.length === 0 ? (
        <p className="muted sds-rules-empty">This group has no rules: it scans nothing yet.</p>
      ) : (
        <table className="settings-table sds-rule-table">
          <thead>
            <tr>
              <th>#</th>
              <th>RULE</th>
              <th>FINDS</th>
              <th>ACTION</th>
              <th>TAGS</th>
              <th>SCANS</th>
              <th className="num">ENABLED</th>
              <th />
            </tr>
          </thead>
          <tbody>
            {rules.map((r, i) => (
              <tr key={r.id} className={r.enabled ? '' : 'sds-off'}>
                <td className="muted">{i + 1}</td>
                <td>
                  <b>{r.name}</b>
                  {r.priority != null && <span className="muted sds-prio"> P{r.priority}</span>}
                  {r.description && <div className="muted sds-sub">{r.description}</div>}
                </td>
                <td>{r.standardPatternId ? <span className="role-chip">{nameOf(r.standardPatternId)}</span> : <code className="sds-regex">{r.pattern}</code>}</td>
                <td>{action(r)}</td>
                <td>
                  <span className="sds-chips">
                    {(r.tags.length ? r.tags : r.standardPatternId ? (patterns.find((p) => p.id === r.standardPatternId)?.tags ?? []) : []).map((t) => (
                      <span key={t} className="role-chip">
                        {t}
                      </span>
                    ))}
                  </span>
                </td>
                <td className="mono sds-sub">
                  {r.namespaces.length ? r.namespaces.join(', ') : 'whole log'}
                  {r.excludedNamespaces.length > 0 && <div className="muted">except {r.excludedNamespaces.join(', ')}</div>}
                </td>
                <td className="num">
                  <button
                    role="switch"
                    aria-checked={r.enabled}
                    aria-label={r.enabled ? `Disable ${r.name}` : `Enable ${r.name}`}
                    className={`switch${r.enabled ? ' on' : ''}`}
                    disabled={save.isPending}
                    onClick={() => save.mutate({ id: r.id, input: { ...ruleInput(r), enabled: !r.enabled } })}
                  />
                </td>
                <td className="sds-actions">
                  <button
                    className="icon-plain"
                    title="Move up"
                    disabled={i === 0 || reorder.isPending}
                    onClick={() => reorder.mutate({ groupId: group.id, ids: moved(rules, i, -1).map((x) => x.id) })}
                  >
                    <ArrowUp size={14} />
                  </button>
                  <button
                    className="icon-plain"
                    title="Move down"
                    disabled={i === rules.length - 1 || reorder.isPending}
                    onClick={() => reorder.mutate({ groupId: group.id, ids: moved(rules, i, 1).map((x) => x.id) })}
                  >
                    <ArrowDown size={14} />
                  </button>
                  <button className="icon-plain" title="Edit rule" onClick={() => onEdit(r)}>
                    <Pencil size={14} />
                  </button>
                  <button className="icon-plain" title="Delete rule" onClick={() => window.confirm(`Delete the rule "${r.name}"?`) && remove.mutate(r.id)}>
                    <Trash2 size={14} />
                  </button>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
      <button className="btn sds-add-rule" onClick={() => onEdit(null)}>
        <Plus size={14} /> Add a rule to {group.name}
      </button>
    </div>
  );
}

function LibraryTab({ patterns, isPending, groups, onAdd }: { patterns: SdsStandardPattern[]; isPending: boolean; groups: SdsGroup[]; onAdd: (id: string) => void }) {
  const [category, setCategory] = useState('');
  const [search, setSearch] = useState('');
  const used = useMemo(() => new Set(groups.flatMap((g) => g.rules.map((r) => r.standardPatternId).filter(Boolean))), [groups]);
  const needle = search.trim().toLowerCase();
  const rows = patterns.filter((p) => (!category || p.category === category) && (!needle || `${p.name} ${p.description} ${p.tags.join(' ')}`.toLowerCase().includes(needle)));
  if (isPending) return <Loading />;
  return (
    <>
      <div className="mapping-bar">
        <label className="settings-search wide">
          <Search size={16} />
          <input placeholder="Filter patterns" value={search} onChange={(e) => setSearch(e.target.value)} />
        </label>
        <select className="sds-select" value={category} onChange={(e) => setCategory(e.target.value)} aria-label="Category">
          <option value="">All categories</option>
          {Object.entries(CATEGORY_LABELS).map(([id, label]) => (
            <option key={id} value={id}>
              {label}
            </option>
          ))}
        </select>
      </div>
      {groups.length === 0 && <p className="muted">Create a scanning group first to add patterns to it.</p>}
      <table className="settings-table bordered sds-library">
        <thead>
          <tr>
            <th>PATTERN</th>
            <th>CATEGORY</th>
            <th>TAGS</th>
            <th className="num">PRIORITY</th>
            <th />
          </tr>
        </thead>
        <tbody>
          {rows.map((p) => (
            <tr key={p.id}>
              <td>
                <b>{p.name}</b> {used.has(p.id) && <span className="sds-badge hit">in use</span>}
                <div className="muted sds-sub">{p.description}</div>
              </td>
              <td>{CATEGORY_LABELS[p.category] ?? p.category}</td>
              <td>
                <span className="sds-chips">
                  {p.tags.map((t) => (
                    <span key={t} className="role-chip">
                      {t}
                    </span>
                  ))}
                </span>
              </td>
              <td className="num">{p.priority}</td>
              <td className="num">
                <button className="btn" title="Add a rule with this pattern to a group" disabled={groups.length === 0} onClick={() => onAdd(p.id)}>
                  <Plus size={14} /> Add
                </button>
              </td>
            </tr>
          ))}
          {rows.length === 0 && (
            <tr>
              <td colSpan={5} className="muted center">
                No pattern matches.
              </td>
            </tr>
          )}
        </tbody>
      </table>
    </>
  );
}
