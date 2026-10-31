import { useMemo, useState, type ReactNode } from 'react';
import { Link, useNavigate, useSearchParams } from 'react-router-dom';
import { ArrowDownWideNarrow, ChevronDown, ChevronRight, Copy, FileCode, MoreVertical, Plus, Search, ShieldCheck, Table2, X } from 'lucide-react';
import { downloadWpPolicies, type WpAgentRule, type WpFindingRule, type WpPolicy } from '../../../api/workloadProtection/client';
import type { SecurityRuleSummary } from '../../../api/security/client';
import { Empty, ErrorState, Loading } from '../../../components/ui/AsyncState';
import { Menu } from '../../../components/ui/Dropdown';
import { Modal } from '../../../components/ui/Modal';
import { UserAvatar } from '../../../components/ui/UserAvatar';
import { useTime } from '../../../components/shell/TimeContext';
import { compact } from '../../../lib/format';
import { useRuleEnabled, useSecurityRules } from '../queries';
import { POLICY_TABS, WpTabs } from './WorkloadLayout';
import { applyFacets, FacetPanel, Toolbar, type FacetDef, type FacetSel } from './InventoryPages';
import { useDeleteWpAgentRule, useDeleteWpPolicy, useSaveWpAgentRule, useSaveWpPolicy, useWpAgentRules, useWpFindingRules, useWpPolicies } from './manageQueries';
import './manage.css';

const SEV_ORDER = ['critical', 'high', 'medium', 'low', 'info'];
const date = (t: number) => new Date(t).toLocaleString('en-US', { month: 'short', day: 'numeric', hour: 'numeric', minute: '2-digit' });
const day = (t: number) => new Date(t).toLocaleDateString('en-US', { month: 'short', day: 'numeric', year: 'numeric' });

/** MITRE ATT&CK tactics in kill-chain order. */
const TACTICS: [string, string][] = [
  ['TA0043', 'Reconnaissance'],
  ['TA0042', 'Resource Development'],
  ['TA0001', 'Initial Access'],
  ['TA0002', 'Execution'],
  ['TA0003', 'Persistence'],
  ['TA0004', 'Privilege Escalation'],
  ['TA0005', 'Defense Evasion'],
  ['TA0006', 'Credential Access'],
  ['TA0007', 'Discovery'],
  ['TA0008', 'Lateral Movement'],
  ['TA0009', 'Collection'],
  ['TA0011', 'Command and Control'],
  ['TA0010', 'Exfiltration'],
  ['TA0040', 'Impact'],
];
const tacticOf = (tags: string[]) => {
  const t = tags.find((x) => x.startsWith('tactic:'));
  if (!t) return '';
  const id = t.slice(7, 13).toUpperCase();
  return TACTICS.find(([i]) => i === id)?.[1] ?? '';
};

/** Who last changed a record: ObserveCraft's badge for the defaults, the user's avatar otherwise. */
function By({ name, small }: { name: string; small?: boolean }) {
  if (!name || name === 'ObserveCraft')
    return (
      <span className="wpm-by">
        <i className="wpm-oc">
          <ShieldCheck size={small ? 10 : 12} />
        </i>
        {small ? 'OBSERVECRAFT' : 'ObserveCraft'}
      </span>
    );
  return (
    <span className="wpm-by">
      <UserAvatar id={name} name={name} size={small ? 18 : 22} />
      {small ? name.toUpperCase() : name}
    </span>
  );
}

function Sev({ s }: { s: string }) {
  return s ? <span className={`wpm-sev ${s}`}>{s.toUpperCase()}</span> : <span className="muted">—</span>;
}

function Switch({ on, onChange, label, disabled, title }: { on: boolean; onChange?: (v: boolean) => void; label: string; disabled?: boolean; title?: string }) {
  return (
    <label className="wpm-switch" title={title}>
      <input type="checkbox" checked={on} disabled={disabled} onChange={(e) => onChange?.(e.target.checked)} aria-label={label} />
      <i />
    </label>
  );
}

function SearchFor({ value, onChange, placeholder }: { value: string; onChange: (v: string) => void; placeholder: string }) {
  return (
    <div className="wpm-searchbar">
      <span className="wpm-addon">Search for</span>
      <label>
        <Search size={14} />
        <input placeholder={placeholder} value={value} onChange={(e) => onChange(e.target.value)} />
      </label>
    </div>
  );
}

async function download() {
  const blob = await downloadWpPolicies();
  const a = document.createElement('a');
  a.href = URL.createObjectURL(blob);
  a.download = blob.type.includes('zip') ? 'workload-protection-policies.zip' : 'workload-protection-policies.policy';
  a.click();
  URL.revokeObjectURL(a.href);
}

// ---------------------------------------------------------------- agent configuration

const POLICY_FACETS: FacetDef<WpPolicy>[] = [
  { key: 'name', label: 'Name', group: 'CORE', value: (p) => p.name },
  { key: 'type', label: 'Policy Type', group: 'CORE', value: (p) => (p.isDefault ? 'ObserveCraft' : 'Custom') },
  { key: 'enabled', label: 'Enabled', group: 'CORE', value: (p) => String(p.enabled), open: true },
  { key: 'creator', label: 'Creator', group: 'CORE', value: (p) => p.updatedBy },
  { key: 'id', label: 'ID', group: 'CORE', value: (p) => p.id },
];
const RULE_FACETS: FacetDef<WpAgentRule>[] = [
  { key: 'policy', label: 'Policy', group: 'CORE', value: (r) => r.policyName, open: true },
  { key: 'category', label: 'Category', group: 'CORE', value: (r) => r.category },
  { key: 'enabled', label: 'Enabled', group: 'CORE', value: (r) => String(r.enabled) },
  { key: 'default', label: 'Default', group: 'CORE', value: (r) => String(r.isDefault) },
  { key: 'tactic', label: 'Tactic', group: 'MITRE ATT&CK', value: (r) => TACTICS.find(([i]) => i === r.tactic)?.[1] ?? '' },
  { key: 'pack', label: 'Content Pack', group: 'MITRE ATT&CK', value: (r) => r.pack },
];

function PolicyModal({ policy, onClose }: { policy?: WpPolicy; onClose: () => void }) {
  const save = useSaveWpPolicy();
  const [name, setName] = useState(policy?.name ?? '');
  const [description, setDescription] = useState(policy?.description ?? '');
  const [tags, setTags] = useState((policy?.hostTagsLists ?? []).map((l) => l.join(', ')).join('\n'));
  const [enabled, setEnabled] = useState(policy?.enabled ?? true);
  return (
    <Modal onClose={onClose} label={policy ? 'Edit policy' : 'New policy'} width={640} height="auto">
      <form
        className="sec-modal wpm-modal"
        onSubmit={(e) => {
          e.preventDefault();
          const hostTagsLists = tags
            .split('\n')
            .map((l) =>
              l
                .split(',')
                .map((t) => t.trim())
                .filter(Boolean),
            )
            .filter((l) => l.length);
          save.mutate({ id: policy?.id, input: { name, description, enabled, hostTagsLists } }, { onSuccess: onClose });
        }}
      >
        <header>
          <h3>{policy ? `Edit ${policy.name}` : 'New Policy'}</h3>
          <button type="button" className="btn btn-ghost" onClick={onClose} aria-label="Close">
            <X size={16} />
          </button>
        </header>
        <label>
          Name
          <input autoFocus required value={name} onChange={(e) => setName(e.target.value)} placeholder="Payments hosts" disabled={policy?.isDefault} />
        </label>
        <label>
          Description
          <textarea rows={2} value={description} onChange={(e) => setDescription(e.target.value)} />
        </label>
        <label>
          Host tags
          <textarea rows={2} className="mono" value={tags} onChange={(e) => setTags(e.target.value)} placeholder="env:prod, team:payments" />
          <span className="muted">One list per line; an Agent gets the policy when it has every tag of a line. Leave empty for every Agent.</span>
        </label>
        <label className="wpm-inline">
          <input type="checkbox" className="checkbox" checked={enabled} onChange={(e) => setEnabled(e.target.checked)} /> Enabled
        </label>
        {save.isError && <p className="sec-banner error">{String(save.error)}</p>}
        <footer>
          <button type="button" className="btn" onClick={onClose}>
            Cancel
          </button>
          <button className="btn btn-primary" disabled={!name.trim() || save.isPending}>
            {policy ? 'Save' : 'Create'}
          </button>
        </footer>
      </form>
    </Modal>
  );
}

function AgentRuleModal({ rule, policies, onClose }: { rule?: WpAgentRule; policies: WpPolicy[]; onClose: () => void }) {
  const save = useSaveWpAgentRule();
  const custom = policies.filter((p) => !p.isDefault);
  const [name, setName] = useState(rule?.name ?? '');
  const [description, setDescription] = useState(rule?.description ?? '');
  const [expression, setExpression] = useState(rule?.expression ?? 'exec.file.name == "nc" && process.ancestors.file.name in ["bash", "sh"]');
  const [policyId, setPolicyId] = useState(rule?.policyId ?? custom[0]?.id ?? policies[0]?.id ?? '');
  const [enabled, setEnabled] = useState(rule?.enabled ?? true);
  const [silent, setSilent] = useState(rule?.silent ?? false);
  const locked = !!rule?.isDefault;
  return (
    <Modal onClose={onClose} label={rule ? 'Edit agent rule' : 'New agent rule'} width={720} height="auto">
      <form
        className="sec-modal wpm-modal"
        onSubmit={(e) => {
          e.preventDefault();
          save.mutate({ id: rule?.id, input: locked ? { enabled, silent } : { name, description, expression, policyId, enabled, silent } }, { onSuccess: onClose });
        }}
      >
        <header>
          <h3>{rule ? rule.title || rule.name : 'New Agent Rule'}</h3>
          <button type="button" className="btn btn-ghost" onClick={onClose} aria-label="Close">
            <X size={16} />
          </button>
        </header>
        {locked && <p className="sec-banner info">Default rules come with their policy: turn them on or off, or make them silent.</p>}
        <div className="sec-row">
          <label>
            Name
            <input
              required
              className="mono"
              pattern="[a-z0-9_]+"
              title="Lowercase letters, digits and underscores"
              value={name}
              onChange={(e) => setName(e.target.value)}
              disabled={locked || !!rule}
              placeholder="netcat_from_shell"
            />
          </label>
          <label>
            Policy
            <select value={policyId} onChange={(e) => setPolicyId(e.target.value)} disabled={locked}>
              {policies.map((p) => (
                <option key={p.id} value={p.id}>
                  {p.name}
                </option>
              ))}
            </select>
          </label>
        </div>
        <label>
          Description
          <input value={description} onChange={(e) => setDescription(e.target.value)} disabled={locked} />
        </label>
        <label>
          Expression (SECL)
          <textarea rows={4} className="mono" required value={expression} onChange={(e) => setExpression(e.target.value)} disabled={locked} />
        </label>
        <div className="wpm-inline">
          <label className="wpm-inline">
            <input type="checkbox" className="checkbox" checked={enabled} onChange={(e) => setEnabled(e.target.checked)} /> Enabled
          </label>
          <label className="wpm-inline">
            <input type="checkbox" className="checkbox" checked={silent} onChange={(e) => setSilent(e.target.checked)} /> Silent (evaluate without sending events)
          </label>
        </div>
        {save.isError && <p className="sec-banner error">{String(save.error)}</p>}
        <footer>
          <button type="button" className="btn" onClick={onClose}>
            Cancel
          </button>
          <button className="btn btn-primary" disabled={save.isPending}>
            {rule ? 'Save' : 'Create'}
          </button>
        </footer>
      </form>
    </Modal>
  );
}

/** Policies → Agent configuration: the policies the Agents load and the agent rules in them. */
export function AgentConfigurationPage() {
  const [params, setParams] = useSearchParams();
  const view = params.get('view') === 'rules' ? 'rules' : 'policies';
  const { range } = useTime();
  const policies = useWpPolicies();
  const rules = useWpAgentRules(range);
  const setView = (v: string) => setParams(v === 'rules' ? { view: 'rules' } : {}, { replace: true });
  const nav = (
    <nav className="wpm-leftnav">
      <button className={view === 'policies' ? 'active' : ''} onClick={() => setView('policies')}>
        Policies <span>{policies.data?.length ?? ''}</span>
      </button>
      <button className={view === 'rules' ? 'active' : ''} onClick={() => setView('rules')}>
        Rules <span>{rules.data?.length ?? ''}</span>
      </button>
    </nav>
  );
  return (
    <div className="sec-page wpm-page">
      <WpTabs tabs={POLICY_TABS} />
      {view === 'policies' ? <PoliciesView nav={nav} /> : <AgentRulesView nav={nav} />}
    </div>
  );
}

function PoliciesView({ nav }: { nav: ReactNode }) {
  const q = useWpPolicies();
  const save = useSaveWpPolicy();
  const del = useDeleteWpPolicy();
  const [search, setSearch] = useState('');
  const [sel, setSel] = useState<FacetSel>({});
  const [picked, setPicked] = useState<Set<string>>(new Set());
  const [editing, setEditing] = useState<WpPolicy | 'new' | null>(null);
  const [controls, setControls] = useState(true);
  const all = q.data ?? [];
  const t = search.trim().toLowerCase();
  const rows = applyFacets(all, POLICY_FACETS, sel)
    .filter((p) => !t || `${p.name} ${p.updatedBy} ${p.enabled ? 'enabled' : 'disabled'} ${p.description}`.toLowerCase().includes(t))
    .sort((a, b) => b.updatedAt - a.updatedAt);
  const bulk = (enabled: boolean) => {
    for (const id of picked) save.mutate({ id, input: { enabled } });
    setPicked(new Set());
  };
  return (
    <>
      <div className="wpm-split">
        <div className="wpm-leftcol">
          {nav}
          {controls && <FacetPanel rows={all} defs={POLICY_FACETS} sel={sel} onChange={setSel} />}
        </div>
        <div className="wpm-split-main">
          <SearchFor value={search} onChange={setSearch} placeholder="Name, creator, enabled…" />
          {q.isLoading ? (
            <Loading />
          ) : q.isError ? (
            <ErrorState error={q.error} onRetry={() => q.refetch()} />
          ) : (
            <div className="wpm-main wpm-boxed">
              <Toolbar
                controls={controls}
                onToggle={() => setControls((c) => !c)}
                right={
                  <button className="btn btn-primary" onClick={() => setEditing('new')}>
                    <Plus size={14} /> New Policy
                  </button>
                }
              >
                <span>
                  <b>{rows.length}</b> policies
                </span>
                <span className="wpm-sep" />
                {picked.size ? (
                  <span className="wpm-bulk">
                    {picked.size} selected
                    <button className="link-btn" onClick={() => bulk(true)}>
                      Enable
                    </button>
                    <button className="link-btn" onClick={() => bulk(false)}>
                      Disable
                    </button>
                  </span>
                ) : (
                  <span className="muted">Select rows to take action</span>
                )}
              </Toolbar>
              {!rows.length ? (
                <Empty>
                  <b>No policies match.</b>
                </Empty>
              ) : (
                <table className="mon-table wpm-table">
                  <thead>
                    <tr>
                      <th className="wpm-check">
                        <input
                          type="checkbox"
                          className="checkbox"
                          aria-label="Select all"
                          checked={picked.size > 0 && picked.size === rows.length}
                          onChange={(e) => setPicked(e.target.checked ? new Set(rows.map((p) => p.id)) : new Set())}
                        />
                      </th>
                      <th>NAME</th>
                      <th>POLICY TYPE</th>
                      <th>RULES RELATED</th>
                      <th>DEPLOYMENT</th>
                      <th className="wpm-right">LAST MODIFIED</th>
                      <th />
                    </tr>
                  </thead>
                  <tbody>
                    {rows.map((p) => (
                      <tr key={p.id} className={p.enabled ? '' : 'wpm-dim'}>
                        <td className="wpm-check">
                          <input
                            type="checkbox"
                            className="checkbox"
                            aria-label={`Select ${p.name}`}
                            checked={picked.has(p.id)}
                            onChange={(e) =>
                              setPicked((s) => {
                                const n = new Set(s);
                                if (e.target.checked) n.add(p.id);
                                else n.delete(p.id);
                                return n;
                              })
                            }
                          />
                        </td>
                        <td>
                          <button className="link-btn wpm-name" onClick={() => setEditing(p)}>
                            {p.name}
                          </button>
                          {!p.enabled && <span className="wpm-tag grey">DISABLED</span>}
                          {p.hostTagsLists.length > 0 && (
                            <div className="wpm-subtags">
                              {p.hostTagsLists.map((l) => (
                                <span key={l.join(',')} className="wpm-chip small">
                                  {l.join(' AND ')}
                                </span>
                              ))}
                            </div>
                          )}
                        </td>
                        <td>{p.isDefault ? <By name="ObserveCraft" /> : <span className="muted">Custom</span>}</td>
                        <td>
                          {compact(p.monitoring + p.blocking)} active rules
                          {p.disabled > 0 && <span className="muted"> · {p.disabled} disabled</span>}
                        </td>
                        <td className="nowrap">
                          <span
                            className={`wpm-deploy${p.agents ? '' : ' none'}`}
                            title="ObserveCraft doesn't serve Remote Configuration yet: Agents run the policy files you download and mount, so this is the number of Agents the host tags select."
                          >
                            <ShieldCheck size={12} /> {p.agents} agents
                          </span>
                          <span className="wpm-chip small">v{p.version}</span>
                        </td>
                        <td className="wpm-right nowrap">
                          <span className="wpm-modified">
                            <span className="muted">{date(p.updatedAt)}</span>
                            {p.isDefault ? (
                              <i className="wpm-oc" title="ObserveCraft">
                                <ShieldCheck size={12} />
                              </i>
                            ) : (
                              <UserAvatar id={p.updatedBy} name={p.updatedBy} size={22} />
                            )}
                          </span>
                        </td>
                        <td className="wpm-kebab">
                          <Menu
                            triggerClassName="btn btn-ghost"
                            trigger={<MoreVertical size={15} />}
                            items={[
                              { label: 'Edit', onClick: () => setEditing(p) },
                              { label: p.enabled ? 'Disable' : 'Enable', onClick: () => save.mutate({ id: p.id, input: { enabled: !p.enabled } }) },
                              { label: 'Download policy files', onClick: () => void download() },
                              { label: 'Delete', disabled: p.isDefault, onClick: () => window.confirm(`Delete the policy ${p.name}?`) && del.mutate(p.id) },
                            ]}
                          />
                        </td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              )}
            </div>
          )}
        </div>
      </div>
      {editing && <PolicyModal policy={editing === 'new' ? undefined : editing} onClose={() => setEditing(null)} />}
    </>
  );
}

function AgentRulesView({ nav }: { nav: ReactNode }) {
  const { range } = useTime();
  const q = useWpAgentRules(range);
  const policies = useWpPolicies();
  const save = useSaveWpAgentRule();
  const del = useDeleteWpAgentRule();
  const [search, setSearch] = useState('');
  const [sel, setSel] = useState<FacetSel>({});
  const [editing, setEditing] = useState<WpAgentRule | 'new' | null>(null);
  const [controls, setControls] = useState(true);
  const all = q.data ?? [];
  const t = search.trim().toLowerCase();
  const rows = applyFacets(all, RULE_FACETS, sel).filter((r) => !t || `${r.name} ${r.title} ${r.expression} ${r.policyName} ${r.category}`.toLowerCase().includes(t));
  return (
    <>
      <div className="wpm-split">
        <div className="wpm-leftcol">
          {nav}
          {controls && <FacetPanel rows={all} defs={RULE_FACETS} sel={sel} onChange={setSel} />}
        </div>
        <div className="wpm-split-main">
          <SearchFor value={search} onChange={setSearch} placeholder="Name, expression, policy…" />
          {q.isLoading ? (
            <Loading />
          ) : q.isError ? (
            <ErrorState error={q.error} onRetry={() => q.refetch()} />
          ) : (
            <div className="wpm-main wpm-boxed">
              <Toolbar
                controls={controls}
                onToggle={() => setControls((c) => !c)}
                right={
                  <button className="btn btn-primary" onClick={() => setEditing('new')} disabled={!policies.data?.length}>
                    <Plus size={14} /> New Rule
                  </button>
                }
              >
                <span>
                  <b>{rows.length}</b> agent rules
                </span>
              </Toolbar>
              {!rows.length ? (
                <Empty>
                  <b>No agent rules match.</b>
                </Empty>
              ) : (
                <table className="mon-table wpm-table wpm-rules-table">
                  <thead>
                    <tr>
                      <th>RULE</th>
                      <th>EXPRESSION</th>
                      <th>POLICY</th>
                      <th>CATEGORY</th>
                      <th className="num">MATCHES</th>
                      <th>ENABLED</th>
                      <th />
                    </tr>
                  </thead>
                  <tbody>
                    {rows.map((r) => (
                      <tr key={r.id} className={r.enabled ? '' : 'wpm-dim'}>
                        <td>
                          <button className="link-btn wpm-name" onClick={() => setEditing(r)}>
                            {r.title || r.name}
                          </button>
                          <div className="wpm-subtags">
                            <span className="mono muted">{r.name}</span>
                            {r.silent && <span className="wpm-tag grey">SILENT</span>}
                            {r.blocking && <span className="wpm-tag blue">KILL</span>}
                          </div>
                        </td>
                        <td>
                          <code className="wpm-secl" title={r.expression}>
                            {r.expression}
                          </code>
                        </td>
                        <td>{r.policyName}</td>
                        <td>{r.category}</td>
                        <td className="num">{compact(r.matches)}</td>
                        <td>
                          <Switch on={r.enabled} label={`Enable ${r.name}`} onChange={(v) => save.mutate({ id: r.id, input: { enabled: v } })} />
                        </td>
                        <td className="wpm-kebab">
                          <Menu
                            triggerClassName="btn btn-ghost"
                            trigger={<MoreVertical size={15} />}
                            items={[
                              { label: r.isDefault ? 'View' : 'Edit', onClick: () => setEditing(r) },
                              { label: 'Copy expression', icon: <Copy size={13} />, onClick: () => void navigator.clipboard?.writeText(r.expression) },
                              { label: 'Delete', disabled: r.isDefault, onClick: () => window.confirm(`Delete the agent rule ${r.name}?`) && del.mutate(r.id) },
                            ]}
                          />
                        </td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              )}
            </div>
          )}
        </div>
      </div>
      {editing && policies.data && <AgentRuleModal rule={editing === 'new' ? undefined : editing} policies={policies.data} onClose={() => setEditing(null)} />}
    </>
  );
}

// ---------------------------------------------------------------- detection rules

const maxSev = (r: SecurityRuleSummary) => [...r.cases.map((c) => c.status)].sort((a, b) => SEV_ORDER.indexOf(a) - SEV_ORDER.indexOf(b))[0] ?? 'info';
const tagValue = (tags: string[], key: string) => tags.filter((t) => t.startsWith(`${key}:`)).map((t) => t.slice(key.length + 1));

const DETECTION_FACETS: FacetDef<SecurityRuleSummary>[] = [
  { key: 'enabled', label: 'Enabled', group: 'CORE', value: (r) => String(r.isEnabled), open: true },
  { key: 'type', label: 'Rule Type', group: 'CORE', value: () => 'Workload Protection', open: true },
  { key: 'source', label: 'Source', group: 'CORE', value: (r) => tagValue(r.tags, 'source') },
  { key: 'severity', label: 'Severity', group: 'CORE', value: (r) => maxSev(r) },
  { key: 'default', label: 'Default', group: 'CORE', value: (r) => String(r.isDefault) },
  { key: 'tactic', label: 'Tactic', group: 'CORE', value: (r) => tacticOf(r.tags) },
  { key: 'tag', label: 'Tag', group: 'CORE', value: (r) => r.tags },
];

function DetectionRow({ r }: { r: SecurityRuleSummary }) {
  const toggle = useRuleEnabled();
  return (
    <div className={`wpm-rule${r.isEnabled ? '' : ' wpm-dim'}`}>
      <Sev s={maxSev(r)} />
      <div className="wpm-rule-main">
        <Link to={`/security/siem/rules/${r.id}`} className="wpm-rule-title">
          {r.name}
        </Link>
        <div className="wpm-rule-meta">
          {r.tags.length ? (
            r.tags.slice(0, 4).map((t) => (
              <span key={t} className="wpm-chip small">
                {t}
              </span>
            ))
          ) : (
            <span className="muted">No tags defined</span>
          )}
        </div>
      </div>
      <div className="wpm-rule-side">
        <span className="muted">{day(r.createdAt)}</span>
        <By name={r.isDefault ? 'ObserveCraft' : ''} small />
      </div>
      <div className="wpm-rule-signals">
        <span className="muted">Last 7 days</span>
        <b>{r.signals7d}</b>
      </div>
      <Switch on={r.isEnabled} label={`Enable ${r.name}`} onChange={(v) => toggle.mutate({ id: r.id, enabled: v })} />
    </div>
  );
}

function MitreMap({ rules }: { rules: SecurityRuleSummary[] }) {
  return (
    <div className="wpm-mitre">
      {TACTICS.map(([id, name]) => {
        const rs = rules.filter((r) => tacticOf(r.tags) === name);
        return (
          <div key={id} className="wpm-mitre-col">
            <header>
              <b>{name}</b>
              <span className="muted">
                {rs.length} rule{rs.length === 1 ? '' : 's'}
              </span>
            </header>
            {rs.map((r) => (
              <Link key={r.id} to={`/security/siem/rules/${r.id}`} className={`wpm-mitre-cell ${maxSev(r)}${r.isEnabled ? '' : ' wpm-dim'}`} title={r.name}>
                {r.name}
              </Link>
            ))}
          </div>
        );
      })}
    </div>
  );
}

/** Policies → Detection rules: the workload_security rules that turn agent events into signals. */
export function WorkloadDetectionRulesPage() {
  const q = useSecurityRules();
  const [mode, setMode] = useState<'list' | 'map'>('list');
  const [search, setSearch] = useState('');
  const [groupBy, setGroupBy] = useState<'tactic' | 'none'>('tactic');
  const [sort, setSort] = useState<'created' | 'name' | 'severity'>('created');
  const [mine, setMine] = useState(false);
  const [deprecated, setDeprecated] = useState(false);
  const [sel, setSel] = useState<FacetSel>({});
  const [controls, setControls] = useState(true);
  const [closed, setClosed] = useState<Record<string, boolean>>({});
  const all = useMemo(() => (q.data ?? []).filter((r) => r.type === 'workload_security'), [q.data]);
  const t = search.trim().toLowerCase();
  const rows = applyFacets(all, DETECTION_FACETS, sel)
    .filter((r) => (!mine || !r.isDefault) && (deprecated || r.active !== false || r.isEnabled) && (!t || `${r.name} ${r.tags.join(' ')}`.toLowerCase().includes(t)))
    .sort((a, b) => (sort === 'name' ? a.name.localeCompare(b.name) : sort === 'severity' ? SEV_ORDER.indexOf(maxSev(a)) - SEV_ORDER.indexOf(maxSev(b)) : b.createdAt - a.createdAt));
  const groups =
    groupBy === 'none'
      ? [{ name: '', rules: rows }]
      : [...TACTICS.map(([, n]) => n), ''].map((n) => ({ name: n || 'No tactic', rules: rows.filter((r) => tacticOf(r.tags) === n) })).filter((g) => g.rules.length);
  return (
    <div className="sec-page wpm-page">
      <WpTabs tabs={POLICY_TABS} />
      <div className="wpm-title">
        <div>
          <h2>Detection Rules</h2>
          <p className="muted">Manage and define the rules that turn your Agents&rsquo; events into security signals.</p>
        </div>
        <Link className="btn btn-primary" to="/security/siem/rules/new">
          <Plus size={14} /> New Rule
        </Link>
      </div>
      <div className="wpm-ruleq">
        <span className="wpm-seg">
          <button className={mode === 'list' ? 'active' : ''} onClick={() => setMode('list')}>
            <Table2 size={14} /> Rules List
          </button>
          <button className={mode === 'map' ? 'active' : ''} onClick={() => setMode('map')}>
            <ShieldCheck size={14} /> MITRE ATT&amp;CK Map
          </button>
        </span>
        <label className="wpm-ruleq-search">
          <Search size={14} />
          <input placeholder="Search detection rules" value={search} onChange={(e) => setSearch(e.target.value)} />
        </label>
        <span className="wpm-addon">Group by</span>
        <select value={groupBy} onChange={(e) => setGroupBy(e.target.value as 'tactic' | 'none')} aria-label="Group by">
          <option value="tactic">Tactic</option>
          <option value="none">None</option>
        </select>
        <span className="wpm-addon">Sort by</span>
        <select value={sort} onChange={(e) => setSort(e.target.value as 'created' | 'name' | 'severity')} aria-label="Sort by">
          <option value="created">Creation Date</option>
          <option value="name">Name</option>
          <option value="severity">Severity</option>
        </select>
      </div>
      {q.isLoading ? (
        <Loading />
      ) : q.isError ? (
        <ErrorState error={q.error} onRetry={() => q.refetch()} />
      ) : (
        <div className={`wpm-body${controls ? '' : ' no-facets'}`}>
          {controls && (
            <FacetPanel
              rows={all}
              defs={DETECTION_FACETS}
              sel={sel}
              onChange={setSel}
              top={
                <label className="wpm-mine">
                  My Rules <Switch on={mine} onChange={setMine} label="Only custom rules" title="Only the rules your organization wrote" />
                </label>
              }
            />
          )}
          <div className="wpm-main">
            <Toolbar
              controls={controls}
              onToggle={() => setControls((c) => !c)}
              right={
                <label className="wpm-inline muted">
                  Show deprecated rules <Switch on={deprecated} onChange={setDeprecated} label="Show deprecated rules" />
                </label>
              }
            >
              <span>
                <b>{rows.length}</b> rules found{groupBy === 'tactic' && mode === 'list' ? ' in ' : ''}
                {groupBy === 'tactic' && mode === 'list' && (
                  <>
                    <b>{groups.length}</b> groups
                  </>
                )}
              </span>
            </Toolbar>
            {mode === 'map' ? (
              <MitreMap rules={rows} />
            ) : !rows.length ? (
              <Empty>
                <b>No detection rules match.</b>
              </Empty>
            ) : (
              groups.map((g) =>
                g.name ? (
                  <section key={g.name} className="wpm-group">
                    <button className="wpm-group-head" onClick={() => setClosed((c) => ({ ...c, [g.name]: !(c[g.name] ?? true) }))}>
                      {(closed[g.name] ?? true) ? <ChevronRight size={16} /> : <ChevronDown size={16} />}
                      <span>{g.name}</span>
                      <span className="syn-grow" />
                      <span className="muted">
                        {g.rules.length} rule{g.rules.length === 1 ? '' : 's'}
                      </span>
                    </button>
                    {!(closed[g.name] ?? true) && g.rules.map((r) => <DetectionRow key={r.id} r={r} />)}
                  </section>
                ) : (
                  <section key="all" className="wpm-group open">
                    {g.rules.map((r) => (
                      <DetectionRow key={r.id} r={r} />
                    ))}
                  </section>
                ),
              )
            )}
          </div>
        </div>
      )}
    </div>
  );
}

// ---------------------------------------------------------------- finding rules

const FINDING_FACETS: FacetDef<WpFindingRule>[] = [
  { key: 'sev', label: 'Rule Level Status', group: 'WORKLOAD SECURITY', value: (r) => r.severity, open: true },
  { key: 'resource', label: 'Resource Type', group: 'WORKLOAD SECURITY', value: (r) => r.resourceType, open: true },
  { key: 'agentRule', label: 'Referenced Agent Rule ID', group: 'WORKLOAD SECURITY', value: (r) => r.agentRules },
  { key: 'enabled', label: 'Enabled', group: 'CORE', value: () => 'true' },
  { key: 'type', label: 'Rule Type', group: 'CORE', value: () => 'workload_activity' },
  { key: 'state', label: 'Evaluation', group: 'CORE', value: (r) => (r.failing ? 'fail' : 'pass') },
];

/** Policies → Finding rules: the workload_activity rules that turn agent events into posture findings. */
export function FindingRulesPage() {
  const { range } = useTime();
  const navigate = useNavigate();
  const q = useWpFindingRules(range);
  const [search, setSearch] = useState('');
  const [groupBy, setGroupBy] = useState<'none' | 'resource' | 'severity'>('none');
  const [sort, setSort] = useState<'severity' | 'name' | 'failing'>('severity');
  const [sel, setSel] = useState<FacetSel>({});
  const [controls, setControls] = useState(true);
  const all = q.data ?? [];
  const t = search.trim().toLowerCase();
  const rows = applyFacets(all, FINDING_FACETS, sel)
    .filter((r) => !t || `${r.name} ${r.description} ${r.agentRules.join(' ')}`.toLowerCase().includes(t))
    .sort((a, b) => (sort === 'name' ? a.name.localeCompare(b.name) : sort === 'failing' ? b.failing - a.failing : SEV_ORDER.indexOf(a.severity) - SEV_ORDER.indexOf(b.severity)));
  const groups =
    groupBy === 'none'
      ? [{ name: '', rules: rows }]
      : [...new Set(rows.map((r) => (groupBy === 'resource' ? r.resourceType : r.severity)))].map((n) => ({
          name: n,
          rules: rows.filter((r) => (groupBy === 'resource' ? r.resourceType : r.severity) === n),
        }));
  return (
    <div className="sec-page wpm-page">
      <WpTabs tabs={POLICY_TABS} />
      <div className="wpm-title">
        <div>
          <h2>Finding Rules</h2>
          <p className="muted">The rules that check your hosts&rsquo; and containers&rsquo; runtime activity for risky posture; each failing resource becomes a finding.</p>
        </div>
      </div>
      <div className="wpm-ruleq">
        <label className="wpm-ruleq-search">
          <Search size={14} />
          <input placeholder="Search finding rules" value={search} onChange={(e) => setSearch(e.target.value)} />
        </label>
        <span className="wpm-addon">Group by</span>
        <select value={groupBy} onChange={(e) => setGroupBy(e.target.value as 'none' | 'resource' | 'severity')} aria-label="Group by">
          <option value="none">None</option>
          <option value="resource">Resource Type</option>
          <option value="severity">Severity</option>
        </select>
        <span className="wpm-addon">Sort by</span>
        <select value={sort} onChange={(e) => setSort(e.target.value as 'severity' | 'name' | 'failing')} aria-label="Sort by">
          <option value="severity">Severity</option>
          <option value="name">Name</option>
          <option value="failing">Failing resources</option>
        </select>
      </div>
      {q.isLoading ? (
        <Loading />
      ) : q.isError ? (
        <ErrorState error={q.error} onRetry={() => q.refetch()} />
      ) : (
        <div className={`wpm-body${controls ? '' : ' no-facets'}`}>
          {controls && <FacetPanel rows={all} defs={FINDING_FACETS} sel={sel} onChange={setSel} />}
          <div className="wpm-main">
            <Toolbar controls={controls} onToggle={() => setControls((c) => !c)}>
              <span>
                <b>{rows.length}</b> rules found
              </span>
            </Toolbar>
            {!rows.length ? (
              <Empty>
                <b>No finding rules match.</b>
              </Empty>
            ) : (
              groups.map((g) => (
                <section key={g.name || 'all'} className="wpm-group open">
                  {g.name && (
                    <div className="wpm-group-head static">
                      <span>{g.name}</span>
                      <span className="syn-grow" />
                      <span className="muted">{g.rules.length} rules</span>
                    </div>
                  )}
                  {g.rules.map((r) => (
                    <div key={r.id} className="wpm-frule">
                      <div className="wpm-rule-main">
                        <span className="wpm-rule-title">{r.name}</span>
                        <div className="wpm-frule-meta">
                          <Sev s={r.severity} />
                          <span>workload-activity</span>
                          <span className="wpm-vsep" />
                          <By name="ObserveCraft" small />
                          <span className="wpm-vsep" />
                          <span className="muted">Resource: {r.resourceType}</span>
                          <span className="wpm-vsep" />
                          <span className={r.failing ? 'wpm-fail' : 'muted'}>{r.failing} failing</span>
                          <span className="muted">· {r.passing} passing</span>
                          {r.muted > 0 && <span className="muted">· {r.muted} muted</span>}
                        </div>
                        <p className="muted wpm-frule-desc">{r.description}</p>
                        <div className="wpm-rule-meta">
                          {r.agentRules.length ? (
                            r.agentRules.map((a) => (
                              <span key={a} className="wpm-chip small mono" title="Agent rule this finding rule reads">
                                <FileCode size={11} /> {a}
                              </span>
                            ))
                          ) : (
                            <span className="muted">No tags defined</span>
                          )}
                        </div>
                      </div>
                      <Switch on label={`${r.name} is on`} disabled title="Finding rules always run; mute single findings from the Findings page" />
                      <Menu
                        triggerClassName="btn btn-ghost"
                        trigger={<MoreVertical size={15} />}
                        items={[
                          { label: 'View findings', icon: <ArrowDownWideNarrow size={13} />, onClick: () => navigate(`/security/workload-protection/findings?rule=${encodeURIComponent(r.id)}`) },
                          {
                            label: 'View agent rules',
                            icon: <FileCode size={13} />,
                            onClick: () => navigate('/security/workload-protection/policies?view=rules'),
                          },
                        ]}
                      />
                    </div>
                  ))}
                </section>
              ))
            )}
          </div>
        </div>
      )}
    </div>
  );
}
