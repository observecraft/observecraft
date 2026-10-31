import { useState, type ReactNode } from 'react';
import { Link, useNavigate, useSearchParams } from 'react-router-dom';
import {
  Ban,
  ChevronDown,
  ChevronRight,
  Download,
  Eye,
  Filter,
  FolderCog,
  Globe,
  Hand,
  ListChecks,
  MoreVertical,
  PanelLeftClose,
  PanelLeftOpen,
  Pin,
  Plus,
  ScrollText,
  Search,
  Shield,
  ShieldHalf,
  Sparkles,
  Tag,
  X,
} from 'lucide-react';
import type { WafDoc, WafPolicyView } from '../../../api/appsec/client';
import { Modal } from '../../../components/ui/Modal';
import { Menu } from '../../../components/ui/Dropdown';
import { Pager } from '../../../components/ui/Pager';
import { UserAvatar } from '../../../components/ui/UserAvatar';
import { Empty, ErrorState, Loading } from '../../../components/ui/AsyncState';
import { compact } from '../../../lib/format';
import { RulesPage } from '../RulesPage';
import { useSecurityRules } from '../queries';
import { AapTabs, POLICY_TABS } from './AppsecLayout';
import {
  useApiFindings,
  useAppsecServices,
  useAppsecSetting,
  useBlock,
  useBusinessLogic,
  useDeleteWaf,
  useDenylist,
  useSaveSetting,
  useSaveWaf,
  useSecurityTraces,
  useUnblock,
  useUpdateWaf,
  useWaf,
  useWafPolicies,
} from './queries';
import './policies.css';

const DAY = 86_400_000;

/** The last ten days, whole days, fixed for the page's lifetime. */
function useTenDays() {
  return useState(() => {
    const now = Date.now();
    return { from: now - (now % DAY) - 9 * DAY, to: now };
  })[0];
}

/** Per-day counts of one security activity (`attack_attempt.sql_injection`, `business_logic.users.login.failure`, …). */
function daily(hist: { ts: number; counts: Record<string, number> }[] | undefined, from: number, keep: (activity: string) => boolean): number[] {
  const out = Array.from({ length: 10 }, () => 0);
  for (const b of hist ?? []) {
    const i = Math.floor((b.ts - from) / DAY);
    if (i < 0 || i > 9) continue;
    for (const [k, n] of Object.entries(b.counts)) if (keep(k)) out[i] += n;
  }
  return out;
}

/** Policies' frame: sub-tabs and the Remote Configuration notice. */
function PolicyFrame({ children, className = '' }: { children: ReactNode; className?: string }) {
  return (
    <div className={`sec-page aapp-page ${className}`}>
      <AapTabs tabs={POLICY_TABS} />
      <div className="aapp-rc">
        <span>
          Protection is unavailable on <Link to="/security/appsec/inventory/services">your services</Link>: ObserveCraft doesn&rsquo;t deliver Remote Configuration to the tracers yet, so what you
          configure here is stored and shown.
        </span>
        <Link className="btn btn-sm" to="/security/appsec/inventory/services">
          View services
        </Link>
      </div>
      {children}
    </div>
  );
}

const hue = (s: string) => [...s].reduce((h, c) => (h * 31 + c.charCodeAt(0)) % 360, 7);

function ServiceChip({ service, more }: { service: string; more?: number }) {
  return (
    <span className="aapp-svc">
      <span className="aapp-svc-chip">
        <i style={{ background: `hsl(${hue(service)} 55% 45%)` }}>
          <Globe size={11} />
        </i>
        {service}
      </span>
      {!!more && <span className="aapp-more">+{more}</span>}
    </span>
  );
}

function ByUser({ name }: { name?: string }) {
  if (!name) return <span className="muted">—</span>;
  if (name === 'ObserveCraft')
    return (
      <span className="aapp-by">
        <i className="aapp-oc">
          <ShieldHalf size={12} />
        </i>
        OBSERVECRAFT
      </span>
    );
  return (
    <span className="aapp-by">
      <UserAvatar id={name} name={name} size={22} />
      {name.toUpperCase()}
    </span>
  );
}

function EventTag({ event }: { event: string }) {
  return (
    <span className="aapp-event">
      <Tag size={12} /> {event}
    </span>
  );
}

/** Daily bars with the busiest day's count on a dashed line. */
function Spark({ values, color = 'blue' }: { values: number[]; color?: 'blue' | 'orange' }) {
  const max = Math.max(0, ...values);
  return (
    <span className={`aapp-spark ${color}`}>
      <span className="aapp-spark-bars">
        {values.map((n, i) => (
          <i key={i} style={{ height: max ? `${Math.max(n ? 8 : 0, (n / max) * 100)}%` : 0 }} />
        ))}
      </span>
      <span className="aapp-spark-max">{max}</span>
    </span>
  );
}

function Toggle({ on, onChange, label }: { on: boolean; onChange: (v: boolean) => void; label: string }) {
  return <button type="button" role="switch" aria-checked={on} aria-label={label} className={`switch${on ? ' on' : ''}`} onClick={() => onChange(!on)} />;
}

function Kebab({ items }: { items: { label: string; onClick: () => void; disabled?: boolean }[] }) {
  return <Menu triggerClassName="btn btn-ghost aapp-kebab" trigger={<MoreVertical size={15} />} items={items} />;
}

function SearchFor({ placeholder, value, onChange }: { placeholder: string; value: string; onChange: (v: string) => void }) {
  return (
    <label className="aapp-search">
      <span>Search for</span>
      <Search size={14} />
      <input placeholder={placeholder} value={value} onChange={(e) => onChange(e.target.value)} />
    </label>
  );
}

// ---------------------------------------------------------------- checkbox facets

interface FacetDef<T> {
  key: string;
  label: string;
  get: (r: T) => string[];
  icon?: (v: string) => ReactNode;
  color?: (v: string) => string | undefined;
  search?: boolean;
}

/** Facets whose values all start checked: unchecking hides the rows with that value. */
function useFacets<T>(rows: T[], defs: FacetDef<T>[]) {
  const [off, setOff] = useState<Set<string>>(new Set());
  const id = (k: string, v: string) => `${k}\u0000${v}`;
  const visible = rows.filter((r) => defs.every((d) => d.get(r).length === 0 || d.get(r).some((v) => !off.has(id(d.key, v)))));
  const panel = <CheckFacets rows={rows} defs={defs} off={off} setOff={setOff} id={id} />;
  return { visible, panel };
}

function CheckFacets<T>({ rows, defs, off, setOff, id }: { rows: T[]; defs: FacetDef<T>[]; off: Set<string>; setOff: (s: Set<string>) => void; id: (k: string, v: string) => string }) {
  const [closed, setClosed] = useState<Set<string>>(new Set());
  const [filter, setFilter] = useState<Record<string, string>>({});
  return (
    <div className="aapp-facets">
      {defs.map((d) => {
        const counts = new Map<string, number>();
        for (const r of rows) for (const v of d.get(r)) counts.set(v, (counts.get(v) ?? 0) + 1);
        const values = [...counts.entries()].filter(([v]) => !filter[d.key] || v.toLowerCase().includes(filter[d.key].toLowerCase())).sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0]));
        const open = !closed.has(d.key);
        return (
          <section key={d.key}>
            <button
              className="aapp-facet-head"
              onClick={() => {
                const s = new Set(closed);
                if (open) s.add(d.key);
                else s.delete(d.key);
                setClosed(s);
              }}
            >
              {open ? <ChevronDown size={14} /> : <ChevronRight size={14} />} {d.label}
            </button>
            {open && (
              <>
                {d.search && (
                  <label className="aapp-facet-filter">
                    <Search size={12} />
                    <input placeholder="Filter values" value={filter[d.key] ?? ''} onChange={(e) => setFilter({ ...filter, [d.key]: e.target.value })} />
                  </label>
                )}
                {!values.length && <p className="muted aapp-facet-none">No matching values found</p>}
                {values.map(([v, n]) => {
                  const k = id(d.key, v);
                  const color = d.color?.(v);
                  return (
                    <label key={v} className="aapp-facet-value">
                      <input
                        type="checkbox"
                        className="checkbox"
                        checked={!off.has(k)}
                        onChange={() => {
                          const s = new Set(off);
                          if (s.has(k)) s.delete(k);
                          else s.add(k);
                          setOff(s);
                        }}
                      />
                      {color && <i className="aapp-facet-bar" style={{ background: color }} />}
                      {d.icon?.(v)}
                      <span className="aapp-facet-label" title={v}>
                        {v}
                      </span>
                      <span className="muted">{n}</span>
                    </label>
                  );
                })}
              </>
            )}
          </section>
        );
      })}
    </div>
  );
}

// ---------------------------------------------------------------- In-App WAF

const WAF_NAV: [string, string, ReactNode][] = [
  ['policies', 'Policy management', <FolderCog key="p" size={15} />],
  ['custom', 'Custom Rules', <ScrollText key="c" size={15} />],
  ['suggested', 'Suggested Rules', <Sparkles key="s" size={15} />],
  ['responses', 'Custom Responses', <Hand key="r" size={15} />],
];

const CUSTOM_FACETS: FacetDef<WafDoc>[] = [
  { key: 'service', label: 'Service', get: (r) => scopes(r.attributes).map((s) => s.service ?? '*') },
  { key: 'env', label: 'Env', get: (r) => scopes(r.attributes).map((s) => s.env ?? '*') },
  {
    key: 'category',
    label: 'Category',
    get: (r) => [CATEGORY_LABEL[tagsOf(r).category] ?? tagsOf(r).category ?? 'Attack Attempt'],
    color: (v) => ({ 'Attack Attempt': '#e8a33d', 'Business Logic': '#a77de8', 'Security Response': '#5b8def' })[v],
  },
  { key: 'activity', label: 'Security Activity', get: (r) => [`${tagsOf(r).category ?? 'attack_attempt'}.${tagsOf(r).type ?? ''}`] },
  {
    key: 'status',
    label: 'Status',
    get: (r) => [ruleStatus(r.attributes)],
    color: (v) => ({ Blocking: '#5b8def', Disabled: '#9aa3ad', Monitoring: '#e8a33d' })[v],
    icon: (v) => (v === 'Blocking' ? <Hand size={12} /> : v === 'Disabled' ? <Ban size={12} /> : <Eye size={12} />),
  },
];

const CATEGORY_LABEL: Record<string, string> = { attack_attempt: 'Attack Attempt', business_logic: 'Business Logic', security_response: 'Security Response' };
const scopes = (a: WafDoc['attributes']) => (a.scope as { service?: string; env?: string }[] | undefined) ?? [];
const tagsOf = (r: WafDoc) => (r.attributes.tags as Record<string, string>) ?? {};
const ruleStatus = (a: WafDoc['attributes']) => (!a.enabled ? 'Disabled' : a.blocking ? 'Blocking' : 'Monitoring');
const when = (iso?: string) => (iso ? new Date(iso).toLocaleString(undefined, { month: 'short', day: '2-digit', hour: '2-digit', minute: '2-digit', second: '2-digit', hour12: false }) : '');

/** Policies → In-App WAF: policies, custom rules, suggested rules and deny responses. */
export function InAppWafPage() {
  const [params, setParams] = useSearchParams();
  const view = params.get('view') ?? 'policies';
  const [collapsed, setCollapsed] = useState(false);
  const rules = useWaf('custom_rule');
  const logic = useBusinessLogic();
  const facets = useFacets(rules.data ?? [], CUSTOM_FACETS);
  return (
    <PolicyFrame className="aapp-waf-page">
      <div className={`aapp-waf${collapsed ? ' collapsed' : ''}`}>
        <aside className="aapp-side">
          <nav>
            {WAF_NAV.map(([id, label, icon]) => (
              <button key={id} className={view === id ? 'active' : ''} title={label} onClick={() => setParams(id === 'policies' ? {} : { view: id })}>
                {icon}
                {!collapsed && <span>{label}</span>}
                {!collapsed && id === 'suggested' && !!logic.data?.suggested.length && <span className="aapp-count">{logic.data.suggested.length}</span>}
              </button>
            ))}
          </nav>
          {view === 'custom' && !collapsed && !!rules.data?.length && facets.panel}
          <button className="aapp-collapse" onClick={() => setCollapsed((c) => !c)}>
            {collapsed ? <PanelLeftOpen size={15} /> : <PanelLeftClose size={15} />}
            {!collapsed && 'Collapse'}
          </button>
        </aside>
        <div className="aapp-main">
          {view === 'custom' ? <CustomRules q={rules} rows={facets.visible} /> : view === 'suggested' ? <SuggestedRules /> : view === 'responses' ? <CustomResponses /> : <Policies />}
        </div>
      </div>
    </PolicyFrame>
  );
}

function Policies() {
  const q = useWafPolicies();
  const services = useAppsecServices();
  const blocking = useAppsecSetting<boolean>('blocking_allowed');
  const saveBlocking = useSaveSetting('blocking_allowed');
  const del = useDeleteWaf('policy');
  const navigate = useNavigate();
  const [creating, setCreating] = useState(false);
  const [assigning, setAssigning] = useState<WafPolicyView | null>(null);
  const [search, setSearch] = useState('');
  const [openRc, setOpenRc] = useState(false);
  const s = search.trim().toLowerCase();
  const rows = (q.data ?? []).filter((p) => !s || `${p.id} ${p.name} ${p.description} ${p.services.join(' ')}`.toLowerCase().includes(s));
  const allowed = blocking.data !== false;
  return (
    <>
      <p className="aapp-intro">Protect your services by blocking security traces.</p>
      <label className="aapp-allow">
        <Toggle on={allowed} onChange={(v) => saveBlocking.mutate(v)} label="Allow request blocking" /> Allow request blocking
      </label>
      <div className="aapp-bar">
        <SearchFor placeholder="Policy id, policy name, service, env" value={search} onChange={setSearch} />
        <button className="btn btn-primary" onClick={() => setCreating(true)}>
          <Plus size={14} /> New Policy
        </button>
      </div>
      {q.isLoading ? (
        <Loading />
      ) : q.isError ? (
        <ErrorState error={q.error} />
      ) : (
        <table className="mon-table aapp-table">
          <thead>
            <tr>
              <th>POLICY NAME</th>
              <th>LAST MODIFIED BY</th>
              <th>RULES IN POLICY</th>
              <th>ASSIGNED SERVICES</th>
              <th />
            </tr>
          </thead>
          <tbody>
            {rows.map((p) => (
              <tr key={p.id}>
                <td>
                  <div className="aapp-name">
                    {p.managed ? <Pin size={14} /> : <FolderCog size={14} />}
                    {p.isDefault && <span className="aapp-tag blue">DEFAULT</span>}
                    {p.managed && <span className="aapp-tag">NOT EDITABLE</span>}
                    <b>{p.name}</b>
                  </div>
                  <div className="aapp-desc">{p.description}</div>
                  <div className="aapp-version">Managed ruleset (autoupdating)</div>
                </td>
                <td>
                  <ByUser name={p.modifiedBy} />
                </td>
                <td>
                  <div className="aapp-rules">
                    <span className="b">{p.blocking} BLOCKING</span>
                    <span className="m">{p.monitoring} MONITORING</span>
                    <span className="d">{p.disabled} DISABLED</span>
                  </div>
                </td>
                <td>{p.services.length ? <ServiceChip service={p.services[0]} more={p.services.length - 1} /> : <span className="aapp-unassigned">Unassigned</span>}</td>
                <td>
                  <Kebab
                    items={[
                      { label: 'View services', onClick: () => navigate('/security/appsec/inventory/services') },
                      ...(p.managed
                        ? []
                        : [
                            { label: 'Assign services', onClick: () => setAssigning(p) },
                            { label: 'Delete', onClick: () => confirm(`Delete the policy "${p.name}"?`) && del.mutate(p.id) },
                          ]),
                    ]}
                  />
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
      <button className="aapp-group" onClick={() => setOpenRc((o) => !o)}>
        {openRc ? <ChevronDown size={16} /> : <ChevronRight size={16} />} Policies applied on services without remote configuration <span className="aapp-count">{services.data?.length ?? 0}</span>
      </button>
      {openRc && (
        <table className="mon-table aapp-table">
          <thead>
            <tr>
              <th>SERVICE</th>
              <th>ENV</th>
              <th>POLICY</th>
              <th>MODE</th>
            </tr>
          </thead>
          <tbody>
            {(services.data ?? []).map((sv) => (
              <tr key={sv.service}>
                <td>
                  <ServiceChip service={sv.service} />
                </td>
                <td>
                  <span className="tag-chip">env:{sv.env}</span>
                </td>
                <td>{sv.policy}</td>
                <td>
                  <span className={`aap-mode ${sv.mode}`}>{sv.mode.toUpperCase()}</span>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
      {creating && <PolicyModal onClose={() => setCreating(false)} />}
      {assigning && <AssignModal policy={assigning} onClose={() => setAssigning(null)} />}
    </>
  );
}

const PRESETS: { id: string; label: string; desc: string; blocking: number; isNew?: boolean; child?: string }[] = [
  { id: 'exploit-prevention', label: 'Exploit Prevention (RASP)', desc: 'Block exploits at runtime, right before a vulnerability triggers.', blocking: 5, isNew: true },
  {
    id: 'application-attacks',
    label: 'Block Application Attacks',
    desc: 'Block application attacks such as SSRF, SQL injection, Log4Shell and XSS. The rules are chosen to keep the risk of blocking legitimate traffic low.',
    blocking: 117,
    child: 'attack-tools',
  },
  { id: 'attack-tools', label: 'Block Attack Tools', desc: 'Block known attack tools while letting legitimate security scans through.', blocking: 47 },
];
const TOTAL_RULES = 186;
const DISABLED_RULES = 33;

function PolicyModal({ onClose }: { onClose: () => void }) {
  const save = useSaveWaf('policy');
  const [name, setName] = useState('');
  const [description, setDescription] = useState('Monitor security scanners and application attacks such as SSRF, SQL injection, Log4Shell and XSS.');
  const [presets, setPresets] = useState<string[]>([]);
  const has = (id: string) => presets.includes(id) || (id === 'attack-tools' && presets.includes('application-attacks'));
  const blocking = (presets.includes('application-attacks') ? 117 : presets.includes('attack-tools') ? 47 : 0) + (presets.includes('exploit-prevention') ? 5 : 0);
  const monitoring = TOTAL_RULES - blocking;
  const all = TOTAL_RULES + DISABLED_RULES;
  const card = (id: string, nested = false) => {
    const p = PRESETS.find((x) => x.id === id)!;
    const implied = id === 'attack-tools' && presets.includes('application-attacks');
    return (
      <label key={id} className={`aapp-preset${nested ? ' nested' : ''}`}>
        <input type="checkbox" className="checkbox" checked={has(id)} disabled={implied} onChange={(e) => setPresets((ps) => (e.target.checked ? [...ps, id] : ps.filter((x) => x !== id)))} />
        <span>
          <span className="aapp-preset-title">
            <b>{p.label}</b> <span className="aapp-tag blue">{p.blocking} BLOCKING</span>
            {p.isNew && <span className="aapp-new">NEW</span>}
          </span>
          <span className="muted">{p.desc}</span>
        </span>
      </label>
    );
  };
  return (
    <Modal onClose={onClose} label="Create new policy" width={820} height="auto">
      <form
        className="sec-modal aapp-modal"
        onSubmit={(e) => {
          e.preventDefault();
          save.mutate({ name, description, protectionPresets: presets, scope: [], rules: [], version: 0, isDefault: false }, { onSuccess: onClose });
        }}
      >
        <header>
          <h3>
            <FolderCog size={18} /> Create new policy
          </h3>
          <button type="button" className="btn btn-ghost" onClick={onClose} aria-label="Close">
            <X size={16} />
          </button>
        </header>
        <label>
          <span>
            Policy name <span className="aapp-req">*</span>
          </span>
          <input autoFocus required placeholder="Custom Policy 1" value={name} onChange={(e) => setName(e.target.value)} />
        </label>
        <label>
          Description
          <textarea rows={2} value={description} onChange={(e) => setDescription(e.target.value)} />
        </label>
        <h4>Modify In-App WAF rules status with presets (optional)</h4>
        <p className="muted">Choose presets to put the In-App WAF rules for those kinds of attacks in blocking mode. Without a preset, every rule monitors.</p>
        <div className="aapp-presets">
          <h5>RULES CONFIGURATION IN THIS POLICY</h5>
          <p>
            <b>{blocking}</b> <span className="muted">blocking</span> <b>{monitoring}</b> <span className="muted">monitoring</span> <b>{DISABLED_RULES}</b> <span className="muted">disabled</span>
          </p>
          <div className="aapp-preset-bar">
            <i className="b" style={{ width: `${(blocking / all) * 100}%` }} />
            <i className="m" style={{ width: `${(monitoring / all) * 100}%` }} />
            <i className="d" style={{ width: `${(DISABLED_RULES / all) * 100}%` }} />
          </div>
          {card('exploit-prevention')}
          {card('application-attacks')}
          {card('attack-tools', true)}
        </div>
        {save.isError && <p className="sec-error">{(save.error as Error).message}</p>}
        <footer>
          <button type="button" className="btn" onClick={onClose}>
            Cancel
          </button>
          <button className="btn btn-primary" disabled={!name.trim() || save.isPending}>
            Create
          </button>
        </footer>
      </form>
    </Modal>
  );
}

/** Assigns a policy to services: its scope. */
function AssignModal({ policy, onClose }: { policy: WafPolicyView; onClose: () => void }) {
  const docs = useWaf('policy');
  const update = useUpdateWaf('policy');
  const doc = docs.data?.find((d) => d.id === policy.id);
  const [services, setServices] = useState(
    () =>
      scopes(doc?.attributes ?? {})
        .map((s) => s.service ?? '')
        .join(', ') || policy.services.join(', '),
  );
  const [env, setEnv] = useState(() => scopes(doc?.attributes ?? {})[0]?.env ?? 'prod');
  return (
    <Modal onClose={onClose} label="Assign services" width={520} height="auto">
      <form
        className="sec-modal"
        onSubmit={(e) => {
          e.preventDefault();
          if (!doc) return;
          const rest = Object.fromEntries(Object.entries(doc.attributes).filter(([k]) => k !== 'metadata'));
          const scope = services
            .split(/[\s,]+/)
            .filter(Boolean)
            .map((service) => ({ service, ...(env ? { env } : {}) }));
          update.mutate({ id: doc.id, attributes: { ...rest, scope } }, { onSuccess: onClose });
        }}
      >
        <header>
          <h3>Assign services to {policy.name}</h3>
        </header>
        <label>
          Services
          <input autoFocus placeholder="api-gateway, storefront-api" value={services} onChange={(e) => setServices(e.target.value)} />
        </label>
        <label>
          Env
          <input value={env} onChange={(e) => setEnv(e.target.value)} />
        </label>
        {update.isError && <p className="sec-error">{(update.error as Error).message}</p>}
        <footer>
          <button type="button" className="btn" onClick={onClose}>
            Cancel
          </button>
          <button className="btn btn-primary" disabled={!doc || update.isPending}>
            Save
          </button>
        </footer>
      </form>
    </Modal>
  );
}

function CustomRules({ q, rows }: { q: ReturnType<typeof useWaf>; rows: WafDoc[] }) {
  const del = useDeleteWaf('custom_rule');
  const update = useUpdateWaf('custom_rule');
  const range = useTenDays();
  const traces = useSecurityTraces(range, '');
  const [editing, setEditing] = useState<WafDoc | 'new' | null>(null);
  const [search, setSearch] = useState('');
  const s = search.trim().toLowerCase();
  const shown = rows.filter(
    (r) =>
      !s || `${String(r.attributes.name)} ${scopes(r.attributes).map((x) => `${x.service} ${x.env}`)} ${tagsOf(r).category} ${tagsOf(r).type} ${ruleStatus(r.attributes)}`.toLowerCase().includes(s),
  );
  const setStatus = (r: WafDoc, st: string) => {
    const rest = Object.fromEntries(Object.entries(r.attributes).filter(([k]) => k !== 'metadata'));
    update.mutate({ id: r.id, attributes: { ...rest, enabled: st !== 'Disabled', blocking: st === 'Blocking' } });
  };
  return (
    <>
      <p className="aapp-intro">Log or block specific types of requests with Custom Rules.</p>
      <div className="aapp-bar">
        <SearchFor placeholder="Service, env, category, status" value={search} onChange={setSearch} />
        <button className="btn btn-primary" onClick={() => setEditing('new')}>
          <Plus size={14} /> Create New Rule
        </button>
      </div>
      {q.isLoading ? (
        <Loading />
      ) : q.isError ? (
        <ErrorState error={q.error} />
      ) : !shown.length ? (
        <Empty>
          <b>No custom rules.</b>
        </Empty>
      ) : (
        <table className="mon-table aapp-table">
          <thead>
            <tr>
              <th className="aapp-check">
                <input type="checkbox" className="checkbox" aria-label="Select all" />
              </th>
              <th>SERVICE</th>
              <th>RULES</th>
              <th>RULE TYPE</th>
              <th>SECURITY TRACES ACTIVITY</th>
              <th>LAST MODIFIED AT</th>
              <th>LAST MODIFIED BY</th>
              <th>RULE STATUS</th>
              <th />
            </tr>
          </thead>
          <tbody>
            {shown.map((r) => {
              const a = r.attributes;
              const t = tagsOf(r);
              const sc = scopes(a);
              const act = daily(traces.data?.histogram, range.from, (k) => k === `${t.category ?? 'attack_attempt'}.${t.type}`);
              const st = ruleStatus(a);
              return (
                <tr key={r.id}>
                  <td className="aapp-check">
                    <input type="checkbox" className="checkbox" aria-label="Select" />
                  </td>
                  <td>
                    {sc.length ? <ServiceChip service={sc[0].service ?? '*'} more={sc.length - 1} /> : <span className="muted">All services</span>}
                    {sc[0]?.env && <div className="aapp-env">env:{sc[0].env}</div>}
                  </td>
                  <td className="aapp-rule-name">{String(a.name)}</td>
                  <td>{t.type && <EventTag event={t.type} />}</td>
                  <td>
                    <span className="aapp-activity">
                      <span>{compact(act.reduce((x, y) => x + y, 0))}</span>
                      <Spark values={act} color="orange" />
                    </span>
                  </td>
                  <td className="nowrap">{when(a.metadata?.modified_at ?? a.metadata?.added_at)}</td>
                  <td>
                    <ByUser name={a.metadata?.modified_by_name ?? a.metadata?.added_by_name} />
                  </td>
                  <td>
                    <Menu
                      triggerClassName={`aapp-status ${st.toLowerCase()}`}
                      trigger={
                        <>
                          {st.toUpperCase()} <ChevronDown size={12} />
                        </>
                      }
                      items={['Monitoring', 'Blocking', 'Disabled'].map((x) => ({ label: x, onClick: () => setStatus(r, x), disabled: x === st }))}
                    />
                  </td>
                  <td>
                    <Kebab
                      items={[
                        { label: 'Edit', onClick: () => setEditing(r) },
                        { label: 'Delete', onClick: () => confirm('Delete this rule?') && del.mutate(r.id) },
                      ]}
                    />
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
      )}
      {editing && <CustomRuleModal rule={editing === 'new' ? null : editing} onClose={() => setEditing(null)} />}
    </>
  );
}

function CustomRuleModal({ rule, onClose, preset }: { rule: WafDoc | null; onClose: () => void; preset?: Record<string, unknown> }) {
  const save = useSaveWaf('custom_rule', rule?.id);
  const a = (rule?.attributes ?? preset ?? {}) as Record<string, unknown>;
  const scope = ((a.scope as { service?: string; env?: string }[]) ?? [])[0] ?? {};
  const cond = ((a.conditions as { parameters?: { inputs?: { address?: string }[]; regex?: string } }[]) ?? [])[0]?.parameters ?? {};
  const [f, setF] = useState({
    name: String(a.name ?? ''),
    service: scope.service ?? '',
    env: scope.env ?? 'prod',
    path: String(a.path_glob ?? ''),
    address: cond.inputs?.[0]?.address ?? 'server.request.query',
    regex: cond.regex ?? '',
    category: ((a.tags as Record<string, string>) ?? {}).category ?? 'attack_attempt',
    type: ((a.tags as Record<string, string>) ?? {}).type ?? '',
    blocking: Boolean(a.blocking),
    enabled: a.enabled === undefined ? true : Boolean(a.enabled),
  });
  const upd = (p: Partial<typeof f>) => setF((x) => ({ ...x, ...p }));
  return (
    <Modal onClose={onClose} label="Custom rule" width={640} height="auto">
      <form
        className="sec-modal"
        onSubmit={(e) => {
          e.preventDefault();
          save.mutate(
            {
              name: f.name,
              enabled: f.enabled,
              blocking: f.blocking,
              path_glob: f.path || undefined,
              scope: f.service ? [{ service: f.service, env: f.env }] : [],
              conditions: [{ operator: 'match_regex', parameters: { inputs: [{ address: f.address }], regex: f.regex, options: {} } }],
              tags: { category: f.category, type: f.type },
            },
            { onSuccess: onClose },
          );
        }}
      >
        <header>
          <h3>{rule ? 'Edit custom rule' : 'Create a custom rule'}</h3>
          <button type="button" className="btn btn-ghost" onClick={onClose} aria-label="Close">
            <X size={16} />
          </button>
        </header>
        <label>
          Name
          <input required value={f.name} onChange={(e) => upd({ name: e.target.value })} />
        </label>
        <div className="sec-row">
          <label>
            Service
            <input value={f.service} onChange={(e) => upd({ service: e.target.value })} />
          </label>
          <label>
            Env
            <input value={f.env} onChange={(e) => upd({ env: e.target.value })} />
          </label>
        </div>
        <label>
          Path
          <input className="mono" placeholder="/api/v1/login" value={f.path} onChange={(e) => upd({ path: e.target.value })} />
        </label>
        <div className="sec-row">
          <label>
            Request field
            <select value={f.address} onChange={(e) => upd({ address: e.target.value })}>
              {[
                'server.request.query',
                'server.request.body',
                'server.request.headers.no_cookies',
                'server.request.uri.raw',
                'server.request.path_params',
                'server.response.status',
                'http.client_ip',
                'usr.id',
              ].map((x) => (
                <option key={x}>{x}</option>
              ))}
            </select>
          </label>
          <label>
            Matches (regular expression)
            <input className="mono" value={f.regex} onChange={(e) => upd({ regex: e.target.value })} />
          </label>
        </div>
        <div className="sec-row">
          <label>
            Category
            <select value={f.category} onChange={(e) => upd({ category: e.target.value })}>
              <option>attack_attempt</option>
              <option>business_logic</option>
              <option>security_response</option>
            </select>
          </label>
          <label>
            Type
            <input value={f.type} placeholder="users.login.failure" onChange={(e) => upd({ type: e.target.value })} />
          </label>
        </div>
        <label className="sec-check-label">
          <input type="checkbox" checked={f.blocking} onChange={(e) => upd({ blocking: e.target.checked })} /> Block matching requests
        </label>
        <label className="sec-check-label">
          <input type="checkbox" checked={f.enabled} onChange={(e) => upd({ enabled: e.target.checked })} /> Enabled
        </label>
        {save.isError && <p className="sec-error">{(save.error as Error).message}</p>}
        <footer>
          <button type="button" className="btn" onClick={onClose}>
            Cancel
          </button>
          <button className="btn btn-primary" disabled={save.isPending}>
            Save
          </button>
        </footer>
      </form>
    </Modal>
  );
}

function SuggestedRules() {
  const q = useBusinessLogic();
  const [preset, setPreset] = useState<Record<string, unknown> | null>(null);
  const [search, setSearch] = useState('');
  const [picked, setPicked] = useState<Set<string>>(new Set());
  const save = useSaveWaf('custom_rule');
  const s = search.trim().toLowerCase();
  const rows = (q.data?.suggested ?? []).filter((x) => !s || `${x.service} ${x.env} ${x.route} ${x.event}`.toLowerCase().includes(s));
  const key = (x: (typeof rows)[number]) => `${x.service}|${x.env}|${x.route}|${x.event}`;
  const ruleOf = (x: (typeof rows)[number]) => ({
    name: `Track ${x.event} on ${x.service} - ${x.route}`,
    path_glob: x.route,
    scope: [{ service: x.service, env: x.env }],
    conditions: [{ operator: 'match_regex', parameters: { inputs: [{ address: 'server.response.status' }], regex: x.event.endsWith('failure') ? '^401$' : '^200$', options: {} } }],
    tags: { category: 'business_logic', type: x.event },
    enabled: true,
    blocking: false,
  });
  return (
    <>
      <p className="aapp-intro">Based on your application&rsquo;s traffic, these rules are suggested to help you monitor and protect your application.</p>
      <div className="aapp-bar">
        <SearchFor placeholder="Search by service, environment, path, rule type" value={search} onChange={setSearch} />
      </div>
      {q.isLoading ? (
        <Loading />
      ) : !rows.length ? (
        <Empty>
          <b>No suggestions: your login routes already send login events.</b>
        </Empty>
      ) : (
        <table className="mon-table aapp-table">
          <thead>
            <tr>
              <th className="aapp-check">
                <input
                  type="checkbox"
                  className="checkbox"
                  aria-label="Select all"
                  checked={picked.size === rows.length}
                  onChange={(e) => setPicked(e.target.checked ? new Set(rows.map(key)) : new Set())}
                />
              </th>
              <th>SERVICE</th>
              <th>RULES</th>
              <th>RULE TYPE</th>
              <th>ACTION</th>
            </tr>
            <tr className="aapp-subhead">
              <th colSpan={5}>
                <button
                  className="btn btn-sm"
                  disabled={!picked.size || save.isPending}
                  onClick={async () => {
                    for (const x of rows.filter((r) => picked.has(key(r)))) await save.mutateAsync(ruleOf(x));
                    setPicked(new Set());
                  }}
                >
                  Create Selected Suggested Rules
                </button>
              </th>
            </tr>
          </thead>
          <tbody>
            {rows.map((x) => (
              <tr key={key(x)}>
                <td className="aapp-check">
                  <input
                    type="checkbox"
                    className="checkbox"
                    aria-label="Select"
                    checked={picked.has(key(x))}
                    onChange={() => {
                      const p = new Set(picked);
                      if (p.has(key(x))) p.delete(key(x));
                      else p.add(key(x));
                      setPicked(p);
                    }}
                  />
                </td>
                <td>
                  <ServiceChip service={x.service} />
                  <div className="aapp-env">env:{x.env}</div>
                </td>
                <td>
                  Track {x.event} on {x.service} - {x.route}
                </td>
                <td>
                  <EventTag event={x.event} />
                </td>
                <td>
                  <button className="link-btn aapp-link-muted" onClick={() => setPreset(ruleOf(x))}>
                    View suggested rule
                  </button>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
      {preset && <CustomRuleModal rule={null} preset={preset} onClose={() => setPreset(null)} />}
    </>
  );
}

interface CustomResponse {
  mode: 'tracer' | 'redirect';
  status: number;
  redirectUrl: string;
  redirectStatus: number;
  /** Per-service overrides. */
  services?: { service: string; status: number; redirectUrl?: string }[];
}

const STATUS_TEXT: Record<number, string> = {
  301: 'Moved Permanently',
  302: 'Found',
  303: 'See Other',
  307: 'Temporary Redirect',
  401: 'Unauthorized',
  403: 'Forbidden',
  404: 'Not Found',
  418: "I'm a teapot",
  429: 'Too Many Requests',
};

function CustomResponses() {
  const q = useAppsecSetting<CustomResponse>('custom_response');
  const save = useSaveSetting('custom_response');
  const [draft, setDraft] = useState<CustomResponse | null>(null);
  const [adding, setAdding] = useState(false);
  if (q.isLoading) return <Loading />;
  const saved: CustomResponse = { mode: 'tracer', status: 403, redirectUrl: '', redirectStatus: 302, ...(q.data ?? {}) };
  const v = draft ?? saved;
  const upd = (p: Partial<CustomResponse>) => setDraft({ ...v, ...p });
  const perService = saved.services ?? [];
  const saveServices = (services: CustomResponse['services']) => save.mutate({ ...saved, services });
  return (
    <>
      <p className="aapp-intro">Customize protection responses.</p>
      <div className="aapp-responses">
        <h4>Customize deny response served to attackers</h4>
        <div className="aapp-radios">
          <label className={`aapp-radio${v.mode === 'tracer' ? ' on' : ''}`}>
            <span className="aapp-radio-head">
              <input type="radio" checked={v.mode === 'tracer'} onChange={() => upd({ mode: 'tracer' })} /> Use the deny page configured in the tracer library
            </span>
            <span className="aapp-field">Override Response Code</span>
            <select value={v.status} disabled={v.mode !== 'tracer'} onChange={(e) => upd({ status: Number(e.target.value) })}>
              {[403, 401, 404, 418, 429].map((c) => (
                <option key={c} value={c}>
                  {c} {STATUS_TEXT[c]}
                </option>
              ))}
            </select>
          </label>
          <label className={`aapp-radio${v.mode === 'redirect' ? ' on' : ''}`}>
            <span className="aapp-radio-head">
              <input type="radio" checked={v.mode === 'redirect'} onChange={() => upd({ mode: 'redirect' })} /> Use custom redirection
            </span>
            <span className="aapp-field">Redirection URL</span>
            <input disabled={v.mode !== 'redirect'} placeholder="Enter a link" value={v.redirectUrl} onChange={(e) => upd({ redirectUrl: e.target.value })} />
            <span className="muted aapp-help">This is the page where the blocked attackers will be redirected to.</span>
            <span className="aapp-field">Redirect Type</span>
            <select disabled={v.mode !== 'redirect'} value={v.redirectStatus} onChange={(e) => upd({ redirectStatus: Number(e.target.value) })}>
              {[301, 302, 303, 307].map((c) => (
                <option key={c} value={c}>
                  {c} {STATUS_TEXT[c]}
                </option>
              ))}
            </select>
          </label>
        </div>
        <div className="aapp-save">
          <button className="btn btn-primary" disabled={!draft || save.isPending} onClick={() => save.mutate(v, { onSuccess: () => setDraft(null) })}>
            Save Changes
          </button>
        </div>
        <h4 className="aapp-per-service">
          Customize deny response for each service
          <button className="btn btn-sm" onClick={() => setAdding(true)}>
            <Plus size={13} /> Add a Custom Response
          </button>
        </h4>
        <div className="aapp-box">
          {!perService.length ? (
            <div className="aapp-empty">
              <Hand size={30} />
              <p>You have not configured any custom response</p>
              <button className="btn btn-sm" onClick={() => setAdding(true)}>
                <Plus size={13} /> Add a Custom Response
              </button>
            </div>
          ) : (
            <table className="mon-table aapp-table">
              <thead>
                <tr>
                  <th>SERVICE</th>
                  <th>RESPONSE</th>
                  <th />
                </tr>
              </thead>
              <tbody>
                {perService.map((r) => (
                  <tr key={r.service}>
                    <td>
                      <ServiceChip service={r.service} />
                    </td>
                    <td>{r.redirectUrl ? `${r.status} ${STATUS_TEXT[r.status] ?? ''} → ${r.redirectUrl}` : `${r.status} ${STATUS_TEXT[r.status] ?? ''}`}</td>
                    <td>
                      <Kebab items={[{ label: 'Delete', onClick: () => saveServices(perService.filter((x) => x.service !== r.service)) }]} />
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
        </div>
      </div>
      {adding && (
        <ServiceResponseModal
          onClose={() => setAdding(false)}
          onSave={(r) => {
            saveServices([...perService.filter((x) => x.service !== r.service), r]);
            setAdding(false);
          }}
        />
      )}
    </>
  );
}

function ServiceResponseModal({ onClose, onSave }: { onClose: () => void; onSave: (r: { service: string; status: number; redirectUrl?: string }) => void }) {
  const [service, setService] = useState('');
  const [status, setStatus] = useState(403);
  const [url, setUrl] = useState('');
  const redirect = [301, 302, 303, 307].includes(status);
  return (
    <Modal onClose={onClose} label="Add a custom response" width={520} height="auto">
      <form
        className="sec-modal"
        onSubmit={(e) => {
          e.preventDefault();
          onSave({ service, status, ...(redirect ? { redirectUrl: url } : {}) });
        }}
      >
        <header>
          <h3>Add a custom response</h3>
        </header>
        <label>
          Service
          <input autoFocus required placeholder="storefront-api" value={service} onChange={(e) => setService(e.target.value)} />
        </label>
        <label>
          Response
          <select value={status} onChange={(e) => setStatus(Number(e.target.value))}>
            {[403, 401, 404, 418, 429, 302, 303, 307].map((c) => (
              <option key={c} value={c}>
                {c} {STATUS_TEXT[c]}
              </option>
            ))}
          </select>
        </label>
        {redirect && (
          <label>
            Redirection URL
            <input required placeholder="https://shop.example.com/blocked" value={url} onChange={(e) => setUrl(e.target.value)} />
          </label>
        )}
        <footer>
          <button type="button" className="btn" onClick={onClose}>
            Cancel
          </button>
          <button className="btn btn-primary" disabled={!service.trim()}>
            Add
          </button>
        </footer>
      </form>
    </Modal>
  );
}

// ---------------------------------------------------------------- Business Logic

/** Policies → Business Logic: login and custom business events, and the rules tracking them. */
export function BusinessLogicPage() {
  const q = useBusinessLogic();
  const rules = useSecurityRules();
  const navigate = useNavigate();
  const appRules = (rules.data ?? []).filter((r) => r.type === 'application_security');
  const ruleCounts = (event: string) => {
    const hits = appRules.filter((r) => r.queries.some((x) => (x.query ?? '').includes(event)));
    return [hits.filter((r) => r.isEnabled).length, hits.filter((r) => !r.isEnabled).length];
  };
  const suggested = (event: string) => (q.data?.suggested ?? []).filter((s) => s.event === event).length;
  const table = (title: string, sub: string, keep: (e: string) => boolean) => (
    <section className="aapp-logic" key={title}>
      <h3>{title}</h3>
      <p className="muted">{sub}</p>
      <table className="mon-table aapp-table">
        <thead>
          <tr>
            <th>EVENT</th>
            <th>DESCRIPTION</th>
            <th className="num">REQUESTS (7D)</th>
            <th>SERVICES</th>
            <th>SUGGESTED IN-APP WAF RULES</th>
            <th>DETECTION RULES</th>
          </tr>
        </thead>
        <tbody>
          {q
            .data!.events.filter((e) => keep(e.event))
            .map((e) => {
              const [on, off] = ruleCounts(e.event);
              const sug = suggested(e.event);
              return (
                <tr key={e.event}>
                  <td>
                    <EventTag event={e.event} />
                  </td>
                  <td>{e.description}</td>
                  <td className="num">
                    <Link to={`/security/appsec/investigate?q=${encodeURIComponent(`@appsec.events.${e.event}.track:true`)}`}>{compact(e.requests7d)}</Link>
                  </td>
                  <td>{e.services.length ? <ServiceChip service={e.services[0]} more={e.services.length - 1} /> : <span className="aapp-nodata">No data</span>}</td>
                  <td>
                    {!!sug && (
                      <button className="aapp-tag link" onClick={() => navigate('/security/appsec/policies?view=suggested')}>
                        {sug} {sug === 1 ? 'rule' : 'rules'}
                      </button>
                    )}
                  </td>
                  <td>
                    {on || off ? (
                      <span className="aapp-pills">
                        {!!on && <span className="aapp-tag orange">{on} MONITORING</span>}
                        {!!off && <span className="aapp-tag">{off} DISABLED</span>}
                      </span>
                    ) : (
                      <span className="muted">
                        No rule for this event. <Link to="/security/siem/rules/new">Create a new rule</Link>
                      </span>
                    )}
                  </td>
                </tr>
              );
            })}
        </tbody>
      </table>
    </section>
  );
  return (
    <PolicyFrame>
      <p className="aapp-intro">Manage and track business logic events.</p>
      <section className="aapp-callout">
        <b>Track your business logic events</b>
        <p>Create custom rules based on your own business logic and start tracking login events or custom events.</p>
        <div>
          <Link className="btn btn-primary" to="/security/appsec/policies?view=suggested">
            Track with an In-App WAF rule
          </Link>
          <Link className="btn" to="/security/appsec/policies?view=custom">
            Manually tag your spans
          </Link>
        </div>
      </section>
      {q.isLoading ? (
        <Loading />
      ) : q.isError ? (
        <ErrorState error={q.error} />
      ) : (
        <>
          {table('Account Protection', 'Identify potential abuse or exploitation of crucial account protection mechanisms within your system', (e) => e.startsWith('users.'))}
          {table('Custom Business Logic', 'Identify and mitigate misuse or exploitation of critical business logic within your application', (e) => !e.startsWith('users.'))}
        </>
      )}
    </PolicyFrame>
  );
}

// ---------------------------------------------------------------- Detection Rules

/** Policies → Detection Rules: the security rules on the security traces, and the API finding rules. */
export function AppsecRulesPage() {
  const [params, setParams] = useSearchParams();
  const view = params.get('view') ?? 'detection';
  const rules = useSecurityRules();
  const app = (rules.data ?? []).filter((r) => r.type === 'application_security');
  const card = (title: string, list: typeof app) => (
    <div className="aapp-card" key={title}>
      <h5>
        <Filter size={13} /> {title}
      </h5>
      <p>
        <b>0</b> <span className="muted">blocking</span> <b>{list.filter((r) => r.isEnabled).length}</b> <span className="muted">monitoring</span>
      </p>
    </div>
  );
  const isLogic = (r: (typeof app)[number]) => r.queries.some((x) => /users\.|business_logic|payment\.|activity\./.test(x.query ?? ''));
  return (
    <PolicyFrame className="aapp-waf-page">
      <div className="aapp-waf">
        <aside className="aapp-side">
          <nav>
            <button className={view === 'detection' ? 'active' : ''} onClick={() => setParams({})}>
              <span>Detection Rules</span>
            </button>
            <button className={view === 'findings' ? 'active' : ''} onClick={() => setParams({ view: 'findings' })}>
              <span>API Finding Rules</span>
            </button>
          </nav>
        </aside>
        <div className="aapp-main">
          {view === 'findings' ? (
            <FindingRules />
          ) : (
            <>
              <h2 className="aapp-h">Detection Rules</h2>
              <p className="muted">Manage and define the rules that continuously scan your security traces for threats.</p>
              <div className="aapp-cards four">
                {card(
                  'Custom Rules',
                  app.filter((r) => !r.isDefault),
                )}
                {card('Business Logic Rules', app.filter(isLogic))}
                {card(
                  'OWASP Top 10 Rules',
                  app.filter((r) => r.isDefault && !isLogic(r)),
                )}
                <div className="aapp-card">
                  <h5>
                    <Filter size={13} /> Enabled rules
                  </h5>
                  <p>
                    <b>{app.filter((r) => r.isEnabled).length}</b> <span className="muted">out of</span> <b>{app.length}</b> <span className="muted">rules</span>
                  </p>
                </div>
              </div>
              <div className="aapp-embedded-rules">
                <RulesPage ruleType="application_security" />
              </div>
            </>
          )}
        </div>
      </div>
    </PolicyFrame>
  );
}

function FindingRules() {
  const q = useApiFindings();
  const [search, setSearch] = useState('');
  const s = search.trim().toLowerCase();
  const rows = (q.data?.rules ?? []).filter((r) => !s || `${r.name} ${r.owasp} ${r.severity} ${r.description}`.toLowerCase().includes(s));
  return (
    <>
      <h2 className="aapp-h">API Finding Rules</h2>
      <p className="muted">The rules that evaluate your API endpoints for security findings, mapped to the OWASP API Security Top 10.</p>
      <div className="aapp-bar">
        <SearchFor placeholder="Search finding rules" value={search} onChange={setSearch} />
      </div>
      {q.isLoading ? (
        <Loading />
      ) : q.isError ? (
        <ErrorState error={q.error} />
      ) : (
        <>
          <p className="muted">
            <b>{rows.length}</b> rules found
          </p>
          <table className="mon-table aapp-table">
            <thead>
              <tr>
                <th>SEVERITY</th>
                <th>RULE</th>
                <th className="num">ENDPOINTS EVALUATED</th>
                <th className="num">FAILING</th>
                <th>STATUS</th>
              </tr>
            </thead>
            <tbody>
              {rows.map((r) => (
                <tr key={r.id}>
                  <td>
                    <span className={`aapp-sev ${r.severity}`}>{r.severity.toUpperCase()}</span>
                  </td>
                  <td>
                    <b>{r.name}</b>
                    <div className="aapp-pills">
                      <span className="tag-chip">owasp:{r.owasp}</span>
                    </div>
                  </td>
                  <td className="num">{compact(r.evaluated)}</td>
                  <td className="num">{compact(r.failing)}</td>
                  <td>
                    <span className="aapp-status monitoring">ENABLED</span>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </>
      )}
    </>
  );
}

// ---------------------------------------------------------------- Denylist

/** Policies → Denylist: blocked IPs and users. */
export function DenylistPage() {
  const q = useDenylist();
  const range = useTenDays();
  const traces = useSecurityTraces(range, '@appsec.blocked:true');
  const unblock = useUnblock();
  const [adding, setAdding] = useState(false);
  const [search, setSearch] = useState('');
  const [controls, setControls] = useState(true);
  const all = q.data ?? [];
  const facets = useFacets(all, [
    { key: 'status', label: 'Attacker Status', get: (r) => [r.expiresAt ? 'Temporarily blocked' : 'Blocked'] },
    { key: 'by', label: 'Blocked by', get: (r) => [r.createdBy] },
    { key: 'kind', label: 'Entity', get: (r) => [r.kind === 'user' ? 'User' : 'IP'] },
    { key: 'source', label: 'Source', get: (r) => [r.source], search: true },
  ]);
  const s = search.trim().toLowerCase();
  const rows = facets.visible.filter((r) => !s || `${r.value} ${r.source} ${r.reason} ${r.createdBy}`.toLowerCase().includes(s));
  const cards: [string, number, string][] = [
    ['Blocked IPs', all.filter((r) => r.kind === 'ip').length, 'IPs'],
    ['Blocked users', all.filter((r) => r.kind === 'user').length, 'users'],
    ['Blocked user agents', all.filter((r) => r.kind === 'user_agent').length, 'user agents'],
  ];
  const day = (ts: number) => new Date(ts).toLocaleString(undefined, { month: 'short', day: 'numeric', year: 'numeric', hour: 'numeric', minute: '2-digit' });
  const exportCsv = () => {
    const csv = [
      'entity,kind,source,reason,blocked_since,expires_at,blocked_by',
      ...all.map((r) =>
        [r.value, r.kind, r.source, r.reason, new Date(r.createdAt).toISOString(), r.expiresAt ? new Date(r.expiresAt).toISOString() : '', r.createdBy]
          .map((x) => `"${String(x).replace(/"/g, '""')}"`)
          .join(','),
      ),
    ].join('\n');
    const a = document.createElement('a');
    a.href = URL.createObjectURL(new Blob([csv], { type: 'text/csv' }));
    a.download = 'denylist.csv';
    a.click();
    URL.revokeObjectURL(a.href);
  };
  return (
    <PolicyFrame>
      <p className="aapp-intro">Manage IPs and authenticated users that are permanently or temporarily blocked.</p>
      <div className="aapp-cards four">
        {cards.map(([t, n, unit]) => (
          <div key={t} className="aapp-card">
            <h5>
              <Filter size={13} /> {t}
            </h5>
            <p>
              <b>{n}</b> <span className="muted">{unit}</span>
            </p>
          </div>
        ))}
        <Link className="aapp-card" to={`/security/appsec/investigate?q=${encodeURIComponent('@appsec.blocked:true')}`}>
          <h5>
            <Shield size={13} /> Total blocked traffic
          </h5>
          <p>
            <b>{compact(traces.data?.total ?? 0)}</b> <span className="muted">blocked traces</span>
          </p>
        </Link>
      </div>
      <div className="aapp-bar">
        <SearchFor placeholder="Search Attackers" value={search} onChange={setSearch} />
        <button className="btn btn-primary" onClick={() => setAdding(true)}>
          <Hand size={14} /> Block New Attackers
        </button>
        <button className="btn" onClick={exportCsv} aria-label="Export as CSV" title="Export as CSV">
          <Download size={14} />
        </button>
      </div>
      <div className={`aapp-explorer${controls ? '' : ' no-controls'}`}>
        {controls && <aside>{facets.panel}</aside>}
        <div>
          <div className="aapp-count-row">
            <button className="link-btn" onClick={() => setControls((c) => !c)}>
              {controls ? <PanelLeftClose size={14} /> : <PanelLeftOpen size={14} />} {controls ? 'Hide Controls' : 'Show Controls'}
            </button>
            <span>
              <b>{rows.length}</b> attackers found
            </span>
          </div>
          {q.isLoading ? (
            <Loading />
          ) : q.isError ? (
            <ErrorState error={q.error} />
          ) : (
            <table className="mon-table aapp-table">
              <thead>
                <tr>
                  <th className="aapp-check">
                    <input type="checkbox" className="checkbox" aria-label="Select all" />
                  </th>
                  <th>ENTITY</th>
                  <th>LAST 10 DAYS ACTIVITY</th>
                  <th>SOURCE</th>
                  <th>BLOCKING SCOPE</th>
                  <th>DETAILS</th>
                  <th>LAST MODIFIED BY</th>
                  <th>ACTIONS</th>
                </tr>
              </thead>
              <tbody>
                {rows.map((r) => (
                  <tr key={`${r.kind}${r.value}`}>
                    <td className="aapp-check">
                      <input type="checkbox" className="checkbox" aria-label="Select" />
                    </td>
                    <td>
                      <Link className="aapp-entity" to={`/security/appsec/investigate/attackers?q=${encodeURIComponent(r.value)}`}>
                        {r.value} <Hand size={12} />
                      </Link>
                    </td>
                    <td>
                      <span className="aapp-activity">
                        <span>{compact(r.activity.reduce((a, b) => a + b, 0))}</span>
                        <Spark values={r.activity} />
                      </span>
                    </td>
                    <td>
                      {r.source === 'Attackers Explorer' ? r.source : <span className="aapp-source">{r.source}</span>}
                      {r.reason && r.reason !== r.source && <div className="aapp-desc">{r.reason}</div>}
                    </td>
                    <td>
                      <span className="aapp-scope">
                        <ShieldHalf size={12} /> All Services
                      </span>
                    </td>
                    <td>
                      {r.expiresAt ? (
                        <>
                          Blocked until <b>{day(r.expiresAt)}</b>
                        </>
                      ) : (
                        <>
                          Permanently blocked since <b>{day(r.createdAt)}</b>
                        </>
                      )}
                    </td>
                    <td>
                      <ByUser name={r.createdBy} />
                    </td>
                    <td>
                      <button className="btn btn-sm" onClick={() => unblock.mutate({ kind: r.kind, value: r.value })}>
                        <Hand size={13} /> Unblock
                      </button>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
        </div>
      </div>
      {adding && <BlockModal onClose={() => setAdding(false)} />}
    </PolicyFrame>
  );
}

function BlockModal({ onClose }: { onClose: () => void }) {
  const block = useBlock();
  const [kind, setKind] = useState('ip');
  const [values, setValues] = useState('');
  const [minutes, setMinutes] = useState('');
  const [reason, setReason] = useState('');
  return (
    <Modal onClose={onClose} label="Block new attackers" width={520} height="auto">
      <form
        className="sec-modal"
        onSubmit={(e) => {
          e.preventDefault();
          block.mutate(
            { kind, values: values.split(/[\s,]+/).filter(Boolean), reason: reason || 'Blocked from the denylist', durationMinutes: minutes ? Number(minutes) : null },
            { onSuccess: onClose },
          );
        }}
      >
        <header>
          <h3>Block new attackers</h3>
        </header>
        <label>
          Entity
          <select value={kind} onChange={(e) => setKind(e.target.value)}>
            <option value="ip">IP addresses</option>
            <option value="user">User IDs</option>
          </select>
        </label>
        <label>
          {kind === 'ip' ? 'IP addresses' : 'User IDs'} (one per line)
          <textarea autoFocus rows={4} className="mono" value={values} onChange={(e) => setValues(e.target.value)} />
        </label>
        <label>
          Duration
          <select value={minutes} onChange={(e) => setMinutes(e.target.value)}>
            <option value="">Permanently</option>
            <option value="60">1 hour</option>
            <option value="1440">1 day</option>
            <option value="10080">7 days</option>
          </select>
        </label>
        <label>
          Reason
          <input value={reason} onChange={(e) => setReason(e.target.value)} />
        </label>
        {block.isError && <p className="sec-error">{(block.error as Error).message}</p>}
        <footer>
          <button type="button" className="btn" onClick={onClose}>
            Cancel
          </button>
          <button className="btn btn-primary" disabled={!values.trim() || block.isPending}>
            Block
          </button>
        </footer>
      </form>
    </Modal>
  );
}

// ---------------------------------------------------------------- Passlist

/** Policies → Passlist: exclusion filters, unmonitored (no security traces) or monitored (traced, not blocked). */
export function PasslistPage() {
  const q = useWaf('exclusion_filter');
  const save = useSaveWaf('exclusion_filter');
  const del = useDeleteWaf('exclusion_filter');
  const update = useUpdateWaf('exclusion_filter');
  const [adding, setAdding] = useState<'unmonitored' | 'monitored' | null>(null);
  const rows = q.data ?? [];
  const toggle = (r: WafDoc) => {
    const rest = Object.fromEntries(Object.entries(r.attributes).filter(([k]) => k !== 'metadata' && k !== 'search_query'));
    update.mutate({ id: r.id, attributes: { ...rest, enabled: !r.attributes.enabled } });
  };
  const exportJson = (list: WafDoc[], name: string) => {
    const a = document.createElement('a');
    a.href = URL.createObjectURL(new Blob([JSON.stringify(list, null, 2)], { type: 'application/json' }));
    a.download = `passlist-${name}.json`;
    a.click();
    URL.revokeObjectURL(a.href);
  };
  const ipChips = (a: WafDoc['attributes'], green: boolean) =>
    ((a.ip_list as string[]) ?? []).map((ip) => (
      <span key={ip} className={`aapp-ip${green ? ' green' : ''}`}>
        {ip}
      </span>
    ));
  const table = (monitored: boolean) => {
    const list = rows.filter((r) => (r.attributes.on_match === 'monitor') === monitored);
    return (
      <section className="aapp-pass" key={String(monitored)}>
        <div className="aapp-pass-head">
          <h3>
            {monitored ? 'Monitored' : 'Unmonitored'} {monitored && <span className="aapp-new">NEW</span>}
          </h3>
        </div>
        <div className="aapp-pass-sub">
          <p className="muted">
            {monitored
              ? 'This table groups passlist entries that still create security traces for what they match, but never block it.'
              : 'This table groups passlist entries that prevent the creation of security traces for what they match, and therefore never block it.'}
          </p>
          <button className="btn btn-ghost" onClick={() => exportJson(list, monitored ? 'monitored' : 'unmonitored')}>
            <Download size={14} /> Export
          </button>
          <button className="btn btn-primary" onClick={() => setAdding(monitored ? 'monitored' : 'unmonitored')}>
            <Plus size={14} /> Add Entry
          </button>
        </div>
        <table className="mon-table aapp-table">
          <thead>
            <tr>
              {!monitored && <th>SERVICE</th>}
              {!monitored && <th>ENV</th>}
              <th>CONDITION</th>
              <th>DESCRIPTION</th>
              {!monitored && <th>RULES</th>}
              <th>LAST MODIFIED AT</th>
              {!monitored && <th>LAST MODIFIED BY</th>}
              <th>ENABLED</th>
              <th />
            </tr>
          </thead>
          <tbody>
            {!list.length && (
              <tr>
                <td colSpan={monitored ? 5 : 9} className="muted aapp-none">
                  No entries.
                </td>
              </tr>
            )}
            {list.map((r) => {
              const a = r.attributes;
              const sc = scopes(a);
              const targets = (a.rules_target as { tags?: Record<string, string>; rule_id?: string }[] | undefined) ?? [];
              return (
                <tr key={r.id}>
                  {!monitored && <td>{sc.length && sc[0].service ? <ServiceChip service={sc[0].service} more={sc.length - 1} /> : <span className="aapp-star">*</span>}</td>}
                  {!monitored && (
                    <td>
                      <span className="aapp-star">{sc[0]?.env ?? '*'}</span>
                    </td>
                  )}
                  <td>
                    <span className="aapp-cond">
                      {!monitored && !!(a.ip_list as string[] | undefined)?.length && <span className="muted">IP:</span>}
                      {ipChips(a, monitored)}
                      {a.path_glob ? (
                        <>
                          <span className="muted">Path:</span> <span className="aapp-ip">{String(a.path_glob)}</span>
                        </>
                      ) : null}
                      {((a.parameters as string[]) ?? []).map((p) => (
                        <span key={p} className="aapp-ip">
                          param: {p}
                        </span>
                      ))}
                    </span>
                  </td>
                  <td>{String(a.description ?? '')}</td>
                  {!monitored && (
                    <td>
                      {targets.length ? (
                        targets.map((t, i) => (
                          <span key={i} className="tag-chip">
                            {t.rule_id ??
                              Object.entries(t.tags ?? {})
                                .map(([k, v]) => `${k}:${v}`)
                                .join(' ')}
                          </span>
                        ))
                      ) : (
                        <span className="muted">All rules</span>
                      )}
                    </td>
                  )}
                  <td className="nowrap">{when(a.metadata?.modified_at ?? a.metadata?.added_at)}</td>
                  {!monitored && (
                    <td>
                      <ByUser name={a.metadata?.modified_by_name ?? a.metadata?.added_by_name} />
                    </td>
                  )}
                  <td>
                    <Toggle on={Boolean(a.enabled)} onChange={() => toggle(r)} label="Enabled" />
                  </td>
                  <td>
                    <Kebab items={[{ label: 'Delete', onClick: () => confirm('Delete this entry?') && del.mutate(r.id) }]} />
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
      </section>
    );
  };
  return (
    <PolicyFrame>
      <p className="aapp-intro">Manage IPs and service paths ignored by App &amp; API Protection. IPs and paths in the passlist are neither monitored for attacks nor protected.</p>
      {q.isLoading ? <Loading /> : q.isError ? <ErrorState error={q.error} /> : [table(false), table(true)]}
      {adding && <PassModal monitored={adding === 'monitored'} onClose={() => setAdding(null)} save={save} />}
    </PolicyFrame>
  );
}

function PassModal({ monitored, onClose, save }: { monitored: boolean; onClose: () => void; save: ReturnType<typeof useSaveWaf> }) {
  const [f, setF] = useState({ description: '', ips: '', service: '', env: '', path: '' });
  const upd = (p: Partial<typeof f>) => setF((x) => ({ ...x, ...p }));
  return (
    <Modal onClose={onClose} label="Add a passlist entry" width={560} height="auto">
      <form
        className="sec-modal"
        onSubmit={(e) => {
          e.preventDefault();
          const ips = f.ips.split(/[\s,]+/).filter(Boolean);
          const attrs: Record<string, unknown> = { description: f.description, enabled: true };
          if (ips.length) attrs.ip_list = ips;
          if (monitored) attrs.on_match = 'monitor';
          else {
            if (f.service) attrs.scope = [{ service: f.service, ...(f.env ? { env: f.env } : {}) }];
            if (f.path) attrs.path_glob = f.path;
          }
          save.mutate(attrs, { onSuccess: onClose });
        }}
      >
        <header>
          <h3>
            <Shield size={16} /> {monitored ? 'Monitored' : 'Unmonitored'} passlist entry
          </h3>
        </header>
        <label>
          IP addresses or ranges
          <input className="mono" placeholder="192.0.2.0/24" value={f.ips} onChange={(e) => upd({ ips: e.target.value })} />
        </label>
        {!monitored && (
          <>
            <div className="sec-row">
              <label>
                Service
                <input value={f.service} onChange={(e) => upd({ service: e.target.value })} />
              </label>
              <label>
                Env
                <input value={f.env} onChange={(e) => upd({ env: e.target.value })} />
              </label>
            </div>
            <label>
              Path
              <input className="mono" placeholder="/convert" value={f.path} onChange={(e) => upd({ path: e.target.value })} />
            </label>
          </>
        )}
        <label>
          Description
          <input required value={f.description} onChange={(e) => upd({ description: e.target.value })} />
        </label>
        {save.isError && <p className="sec-error">{(save.error as Error).message}</p>}
        <footer>
          <button type="button" className="btn" onClick={onClose}>
            Cancel
          </button>
          <button className="btn btn-primary" disabled={save.isPending}>
            Add
          </button>
        </footer>
      </form>
    </Modal>
  );
}

// ---------------------------------------------------------------- Sensitive Data Detection

/** Sensitive data scanners: what the tracers classify in API traffic (our own catalog). */
const SCANNERS: [string, string, string, string][] = [
  ['aba', 'ABA Routing Number Scanner', 'payment', 'aba_routing'],
  ['amex', 'American Express Card Scanner', 'payment', 'card'],
  ['aws-key', 'AWS Access Key Scanner', 'credentials', 'aws_access_key'],
  ['aws-secret', 'AWS Secret Key Scanner', 'credentials', 'aws_secret_key'],
  ['basic', 'Basic Authentication Scanner', 'credentials', 'basic_auth'],
  ['bearer', 'Bearer Authentication Scanner', 'credentials', 'bearer_token'],
  ['ca-sin', 'Canadian Social Insurance Number Scanner', 'pii', 'canadian_sin'],
  ['diners', 'Diners Club Card Scanner', 'payment', 'card'],
  ['discover', 'Discover Card Scanner', 'payment', 'card'],
  ['email', 'Email Address Scanner', 'pii', 'email'],
  ['iban', 'IBAN Scanner', 'payment', 'iban'],
  ['ipv4', 'IPv4 Address Scanner', 'pii', 'ip_address'],
  ['jcb', 'JCB Card Scanner', 'payment', 'card'],
  ['jwt', 'JSON Web Token Scanner', 'credentials', 'jwt'],
  ['mastercard', 'Mastercard Scanner', 'payment', 'card'],
  ['passport', 'Passport Number Scanner', 'pii', 'passport'],
  ['phone', 'Phone Number Scanner', 'pii', 'phone'],
  ['private-key', 'Private Key Scanner', 'credentials', 'private_key'],
  ['uk-nino', 'UK National Insurance Number Scanner', 'pii', 'uk_nino'],
  ['us-itin', 'US Individual Taxpayer Identification Number Scanner', 'pii', 'us_itin'],
  ['ssn', 'US Social Security Number Scanner', 'pii', 'us_ssn'],
  ['vin', 'US Vehicle Identification Number Scanner', 'pii', 'vin'],
  ['visa', 'Visa Card Scanner', 'payment', 'card'],
  ['zip', 'ZIP Code Scanner', 'address', 'zipcode'],
];

type ScannerFilter = 'enabled' | 'disabled' | 'default' | 'custom' | null;

export function SensitiveDataPage() {
  const q = useAppsecSetting<Record<string, boolean>>('scanners');
  const save = useSaveSetting('scanners');
  const [search, setSearch] = useState('');
  const [chip, setChip] = useState<ScannerFilter>(null);
  const [offset, setOffset] = useState(0);
  const [limit, setLimit] = useState(10);
  const state = q.data ?? {};
  const on = (id: string) => state[id] !== false;
  const s = search.trim().toLowerCase();
  const rows = SCANNERS.filter(
    ([id, name, cat, type]) =>
      (!chip || chip === 'default' || (chip === 'enabled' ? on(id) : chip === 'disabled' ? !on(id) : false)) &&
      (!s || `${name} ${cat} ${type} ${on(id) ? 'enabled' : 'disabled'} observecraft`.toLowerCase().includes(s)),
  );
  const enabled = SCANNERS.filter(([id]) => on(id)).length;
  const chips: [ScannerFilter, string, number, ReactNode][] = [
    ['enabled', 'Enabled scanners', enabled, <ListChecks key="e" size={13} />],
    ['disabled', 'Disabled scanners', SCANNERS.length - enabled, <Ban key="d" size={13} />],
    ['default', 'Default scanners', SCANNERS.length, <ShieldHalf key="f" size={13} />],
    ['custom', 'Custom scanners', 0, <ScrollText key="c" size={13} />],
  ];
  const page = rows.slice(offset, offset + limit);
  return (
    <PolicyFrame>
      <p className="aapp-intro">Scanners detect sensitive data in your API traffic: they analyze requests and responses to classify the API endpoints that process sensitive data.</p>
      <div className="aapp-chips">
        {chips.map(([id, label, n, icon]) => (
          <button
            key={label}
            className={`aapp-chip${chip === id ? ' on' : ''}`}
            onClick={() => {
              setChip(chip === id ? null : id);
              setOffset(0);
            }}
          >
            {icon} {label} <b>{n}</b>
          </button>
        ))}
      </div>
      <div className="aapp-bar">
        <SearchFor
          placeholder="Filter by scanner name, category, type or enabled status"
          value={search}
          onChange={(v) => {
            setSearch(v);
            setOffset(0);
          }}
        />
      </div>
      {q.isLoading ? (
        <Loading />
      ) : (
        <>
          <p className="muted">
            Showing <b>{rows.length ? offset + 1 : 0}</b>–<b>{Math.min(offset + limit, rows.length)}</b> of <b>{rows.length}</b> scanners
          </p>
          <table className="mon-table aapp-table">
            <thead>
              <tr>
                <th className="aapp-check">
                  <input type="checkbox" className="checkbox" aria-label="Select all" />
                </th>
                <th>SCANNER</th>
                <th>CATEGORY</th>
                <th>TYPE</th>
                <th>TRACES ACTIVITY</th>
                <th>LAST MODIFIED BY</th>
                <th>ENABLED</th>
              </tr>
            </thead>
            <tbody>
              {page.map(([id, name, cat, type]) => (
                <tr key={id}>
                  <td className="aapp-check">
                    <input type="checkbox" className="checkbox" aria-label="Select" />
                  </td>
                  <td>
                    <span className="aapp-scanner">
                      <i className="aapp-oc">
                        <ShieldHalf size={12} />
                      </i>
                      {name}
                    </span>
                  </td>
                  <td>
                    <span className="aapp-kv">{cat}</span>
                  </td>
                  <td>
                    <span className="aapp-kv">{type}</span>
                  </td>
                  <td>
                    <span className="aapp-activity">
                      <span>0</span>
                      <span className="aapp-spark-wrap">
                        <span className="muted">Last 10 days</span>
                        <Spark values={Array.from({ length: 10 }, () => 0)} />
                      </span>
                    </span>
                  </td>
                  <td>
                    <ByUser name="ObserveCraft" />
                  </td>
                  <td>
                    <Toggle on={on(id)} onChange={(v) => save.mutate({ ...state, [id]: v })} label={`Enable ${name}`} />
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
          <div className="aapp-paging">
            <label>
              Results per page:
              <select
                value={limit}
                onChange={(e) => {
                  setLimit(Number(e.target.value));
                  setOffset(0);
                }}
              >
                {[10, 25, 50].map((n) => (
                  <option key={n}>{n}</option>
                ))}
              </select>
            </label>
            <Pager offset={offset} limit={limit} total={rows.length} onChange={setOffset} />
          </div>
        </>
      )}
    </PolicyFrame>
  );
}
