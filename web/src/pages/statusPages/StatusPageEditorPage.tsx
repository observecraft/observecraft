import { useState, type ReactNode } from 'react';
import { Link, useNavigate, useParams } from 'react-router-dom';
import { ArrowDown, ArrowUp, Building2, Check, ChevronRight, Globe, GripVertical, Mail, Plus, RadioTower, Trash2, X } from 'lucide-react';
import { checkLogoUrl, checkSlug, currentComponents, groupComponents, MAX_SLUG_CHARS, overallStatus, slugify } from '../../api/statusPages/status';
import {
  VISIBILITY_LABEL,
  VISUALIZATION_LABEL,
  type ComponentStatus,
  type PublicStatusPage,
  type StatusPageDetail,
  type StatusPageInput,
  type StatusPageVisibility,
  type StatusPageVisualization,
} from '../../api/statusPages/types';
import { BrandLogo } from '../../components/brand/BrandLogo';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { useStatusPage, useStatusPageMutations } from './queries';
import { StatusPageView } from './StatusPageView';
import '../settings/settings.css';

/** A component in the editor; `id` and `createdAt` are kept for components the page already has. */
interface Comp {
  key: string;
  id?: string;
  name: string;
  status: ComponentStatus;
  createdAt?: number | null;
}

/** The editor's top level: components, and groups holding components. */
type Item = ({ kind: 'component' } & Comp) | { kind: 'group'; key: string; name: string; children: Comp[] };

let nextKey = 0;
const key = () => `new-${nextKey++}`;
const STEPS = ['Type', 'Settings', 'Components'] as const;

/** The page's components as editor items: groups in the order they first appear. */
function toItems(page?: StatusPageDetail): Item[] {
  return groupComponents(page?.components ?? []).flatMap((g): Item[] => {
    const comps = g.components.map((c) => ({ key: c.id, id: c.id, name: c.name, status: c.status, createdAt: c.createdAt }));
    return g.group ? [{ kind: 'group', key: key(), name: g.group, children: comps }] : comps.map((c) => ({ kind: 'component', ...c }));
  });
}

/** The items back as the page's components, in display order. */
function flatten(items: Item[]): (Comp & { group: string | null })[] {
  return items.flatMap((i) => (i.kind === 'group' ? i.children.map((c) => ({ ...c, group: i.name.trim() || null })) : [{ ...i, group: null }]));
}

/** Incident Response → Status Pages → Create (or edit): type, settings and components, with a live preview. */
export function StatusPageEditorPage() {
  const { id } = useParams();
  const q = useStatusPage(id ?? '', Boolean(id));
  if (!id) return <Editor />;
  if (q.isPending) return <Loading />;
  if (q.isError) return <ErrorState error={q.error} onRetry={() => q.refetch()} />;
  return <Editor page={q.data} />;
}

function Editor({ page }: { page?: StatusPageDetail }) {
  const navigate = useNavigate();
  const { create, update } = useStatusPageMutations();
  const save = page ? update : create;
  const [step, setStep] = useState(0);
  const [visibility, setVisibility] = useState<StatusPageVisibility>(page?.visibility ?? 'public');
  const [name, setName] = useState(page?.name ?? '');
  const [slug, setSlug] = useState(page?.slug ?? '');
  const [slugEdited, setSlugEdited] = useState(Boolean(page));
  const [logoUrl, setLogoUrl] = useState(page?.logoUrl ?? '');
  const [email, setEmail] = useState(page?.emailSubscriptions ?? true);
  const [slack, setSlack] = useState(page?.slackSubscriptions ?? true);
  const [visualization, setVisualization] = useState<StatusPageVisualization>(page?.visualization ?? 'bars_and_uptime');
  const [items, setItems] = useState<Item[]>(() => toItems(page));

  const shownSlug = slugEdited ? slug : slugify(name);
  const slugError = shownSlug ? checkSlug(shownSlug) : null;
  const logoError = checkLogoUrl(logoUrl);
  const flat = flatten(items).filter((c) => c.name.trim());
  const names = flat.map((c) => c.name.trim().toLowerCase());
  const duplicate = names.find((n, i) => names.indexOf(n) !== i);
  const settingsReady = Boolean(name.trim() && shownSlug && !slugError && !logoError);
  const ready = settingsReady && !duplicate;

  const input = (): StatusPageInput => ({
    name: name.trim(),
    slug: shownSlug,
    visibility,
    description: page?.description ?? '',
    logoUrl: logoUrl.trim(),
    emailSubscriptions: email,
    slackSubscriptions: slack,
    visualization,
    components: flat.map((c) => ({ id: c.id, name: c.name.trim(), group: c.group, status: c.status })),
  });
  const submit = () =>
    page ? update.mutate({ id: page.id, input: input() }, { onSuccess: (d) => navigate(`/status-pages/${d.id}`) }) : create.mutate(input(), { onSuccess: (d) => navigate(`/status-pages/${d.id}`) });

  // The preview: the page as its visitors will see it, with the notices it already has.
  const components = currentComponents(
    flat.map((c) => ({ id: c.id ?? c.key, name: c.name.trim(), group: c.group, status: c.status, createdAt: c.createdAt === undefined ? Date.now() : c.createdAt })),
    page?.notices ?? [],
  );
  const preview: PublicStatusPage = {
    name: name.trim(),
    slug: shownSlug,
    visibility,
    description: page?.description ?? '',
    logoUrl: logoError ? '' : logoUrl.trim(),
    emailSubscriptions: email,
    slackSubscriptions: slack,
    visualization,
    components,
    overallStatus: overallStatus(components),
    notices: page?.notices ?? [],
    updatedAt: Date.now(),
  };
  const canOpen = (i: number) => i <= step || (i === 1 && step === 0) || settingsReady;

  return (
    <div className="stp-editor-page">
      <header className="product-header">
        <div className="product-title stp-crumb-title">
          <RadioTower size={20} />
          <Link to="/status-pages">Status Pages</Link>
          <ChevronRight size={18} className="stp-crumb-sep" />
          {page && (
            <>
              <Link to={`/status-pages/${page.id}`}>{page.name}</Link>
              <ChevronRight size={18} className="stp-crumb-sep" />
            </>
          )}
          <span>{page ? 'Edit Status Page' : 'Create Status Page'}</span>
        </div>
      </header>
      <div className="stp-editor">
        <form
          className="stp-editor-form"
          onSubmit={(e) => {
            e.preventDefault();
            if (step < STEPS.length - 1) setStep(step + 1);
            else if (ready) submit();
          }}
        >
          <div className="stp-editor-scroll">
            <ol className="stp-stepper">
              {STEPS.map((s, i) => (
                <li key={s} className={i === step ? 'active' : i < step ? 'done' : undefined}>
                  <button type="button" disabled={!canOpen(i)} onClick={() => setStep(i)}>
                    <span className="stp-stepper-dot">{i < step ? <Check size={14} strokeWidth={3} /> : i + 1}</span>
                    {s}
                  </button>
                </li>
              ))}
            </ol>

            {step === 0 && (
              <section className="stp-step">
                <h2>Status Page Type</h2>
                <p>Choose who can open this status page. You can change it later.</p>
                <div className="stp-type-cards" role="radiogroup" aria-label="Visibility">
                  <TypeCard value="public" current={visibility} onPick={setVisibility} icon={<Globe size={18} />} title="Public">
                    Anyone with the link can see it, without signing in.
                  </TypeCard>
                  <TypeCard value="internal" current={visibility} onPick={setVisibility} icon={<Building2 size={18} />} title="Internal">
                    Only signed-in members of this organization can see it.
                  </TypeCard>
                </div>
              </section>
            )}

            {step === 1 && (
              <section className="stp-step">
                <h2>Status Page Settings</h2>
                <label className="stp-field">
                  <span className="stp-field-label">Status Page Title</span>
                  <input value={name} onChange={(e) => setName(e.target.value)} placeholder="Enter page title…" maxLength={100} autoFocus />
                  <span className="stp-field-help">Shown as the page header when there is no logo.</span>
                </label>
                <label className="stp-field">
                  <span className="stp-field-label">Status Page URL</span>
                  <span className="stp-url">
                    <span className="stp-url-fixed">{window.location.origin}/status/</span>
                    <input
                      value={shownSlug}
                      onChange={(e) => {
                        setSlugEdited(true);
                        setSlug(e.target.value);
                      }}
                      placeholder="page-name"
                      maxLength={MAX_SLUG_CHARS}
                      aria-label="Page URL slug"
                    />
                  </span>
                  {slugError ? <span className="form-error">{slugError}</span> : <span className="stp-field-help">The page is served at this address.</span>}
                </label>
                <div className="stp-field">
                  <span className="stp-field-label">Subscriptions</span>
                  <span className="stp-field-help">
                    Visitors see a <strong>Subscribe</strong> button for the selected channels. Updates aren&apos;t delivered to subscribers yet.
                  </span>
                  <div className="stp-toggles">
                    <Toggle label="Email" icon={<Mail size={16} />} checked={email} onChange={setEmail} />
                    <Toggle label="Slack" icon={<BrandLogo name="slack" size={16} />} checked={slack} onChange={setSlack} />
                  </div>
                </div>
                <label className="stp-field">
                  <span className="stp-field-label">
                    Company Logo <span className="stp-optional">optional</span>
                  </span>
                  <input value={logoUrl} onChange={(e) => setLogoUrl(e.target.value)} placeholder="https://cdn.example.com/logo.svg" maxLength={2048} />
                  {logoError ? <span className="form-error">{logoError}</span> : <span className="stp-field-help">An http(s) image URL, shown instead of the title.</span>}
                </label>
              </section>
            )}

            {step === 2 && (
              <section className="stp-step">
                <h2>Components</h2>
                <p>
                  Components are the parts of your product your users rely on, such as an API, the checkout or a mobile app. Each shows its status and availability, and notices name the ones they
                  affect.
                </p>
                <ComponentsEditor items={items} setItems={setItems} />
                {duplicate && <p className="form-error">Component &apos;{duplicate}&apos; appears twice</p>}
                <label className="stp-field stp-visualization">
                  <span className="stp-field-label stp-field-label-lg">Visualization Type</span>
                  <select value={visualization} onChange={(e) => setVisualization(e.target.value as StatusPageVisualization)}>
                    {(Object.keys(VISUALIZATION_LABEL) as StatusPageVisualization[]).map((v) => (
                      <option key={v} value={v}>
                        {VISUALIZATION_LABEL[v]}
                      </option>
                    ))}
                  </select>
                </label>
              </section>
            )}
            {save.error && <p className="form-error">{save.error.message}</p>}
          </div>

          <div className="stp-editor-footer">
            {step > 0 ? (
              <button type="button" className="btn stp-footer-btn" onClick={() => setStep(step - 1)}>
                Back
              </button>
            ) : (
              <Link className="btn stp-footer-btn" to={page ? `/status-pages/${page.id}` : '/status-pages'}>
                Cancel
              </Link>
            )}
            {step < STEPS.length - 1 ? (
              <button className="btn btn-primary stp-footer-btn" disabled={step === 1 && !settingsReady}>
                Next
              </button>
            ) : (
              <button className="btn btn-primary stp-footer-btn" disabled={!ready || save.isPending}>
                {save.isPending ? 'Saving…' : page ? 'Save Status Page' : 'Create Status Page'}
              </button>
            )}
          </div>
        </form>
        <aside className="stp-preview" aria-label="Preview">
          <div className="stp-preview-bar">
            <span>Preview of status page configuration</span>
            <span className="stp-preview-vis">
              {VISIBILITY_LABEL[visibility]} {visibility === 'public' ? <Globe size={15} /> : <Building2 size={15} />}
            </span>
          </div>
          <div className="stp-preview-frame">
            <StatusPageView page={preview} preview />
          </div>
        </aside>
      </div>
    </div>
  );
}

function TypeCard({
  value,
  current,
  onPick,
  icon,
  title,
  children,
}: {
  value: StatusPageVisibility;
  current: StatusPageVisibility;
  onPick: (v: StatusPageVisibility) => void;
  icon: ReactNode;
  title: string;
  children: ReactNode;
}) {
  return (
    <button type="button" role="radio" aria-checked={current === value} className={`stp-type-card${current === value ? ' selected' : ''}`} onClick={() => onPick(value)}>
      <span className="stp-type-icon">{icon}</span>
      <span>
        <strong>{title}</strong>
        <span className="stp-type-desc">{children}</span>
      </span>
    </button>
  );
}

function Toggle({ label, icon, checked, onChange }: { label: string; icon: ReactNode; checked: boolean; onChange: (v: boolean) => void }) {
  return (
    <label className="stp-toggle-row">
      <span className="stp-toggle-label">
        {icon}
        {label}
      </span>
      <input type="checkbox" role="switch" className="stp-switch" checked={checked} onChange={(e) => onChange(e.target.checked)} />
    </label>
  );
}

/** Components and groups: add, rename, reorder and remove. */
function ComponentsEditor({ items, setItems }: { items: Item[]; setItems: (f: (items: Item[]) => Item[]) => void }) {
  // The row being named: a new or renamed component or group, by key.
  const [editing, setEditing] = useState<string | null>(null);
  const addComponent = (groupKey?: string) => {
    const c: Comp = { key: key(), name: '', status: 'operational' };
    setItems((xs) => (groupKey ? xs.map((x) => (x.kind === 'group' && x.key === groupKey ? { ...x, children: [...x.children, c] } : x)) : [...xs, { kind: 'component', ...c }]));
    setEditing(c.key);
  };
  const addGroup = () => {
    const g: Item = { kind: 'group', key: key(), name: '', children: [] };
    setItems((xs) => [...xs, g]);
    setEditing(g.key);
  };
  const rename = (k: string, name: string) =>
    setItems((xs) => xs.map((x) => (x.key === k ? { ...x, name } : x.kind === 'group' ? { ...x, children: x.children.map((c) => (c.key === k ? { ...c, name } : c)) } : x)));
  const remove = (k: string) => setItems((xs) => xs.filter((x) => x.key !== k).map((x) => (x.kind === 'group' ? { ...x, children: x.children.filter((c) => c.key !== k) } : x)));
  const move = <T,>(list: T[], i: number, by: number) => {
    const out = [...list];
    const [x] = out.splice(i, 1);
    out.splice(i + by, 0, x);
    return out;
  };
  const moveItem = (i: number, by: number) => setItems((xs) => move(xs, i, by));
  const moveChild = (groupKey: string, i: number, by: number) => setItems((xs) => xs.map((x) => (x.kind === 'group' && x.key === groupKey ? { ...x, children: move(x.children, i, by) } : x)));
  // Leaving a new row without a name drops it.
  const finish = (k: string, name: string) => {
    setEditing(null);
    if (!name.trim()) remove(k);
  };

  if (!items.length)
    return (
      <div className="stp-comp-empty">
        <p>Add your first component, or a group to hold related components.</p>
        <div>
          <button type="button" className="btn btn-sm" onClick={() => addComponent()}>
            <Plus size={14} /> Add Component
          </button>
          <button type="button" className="btn btn-sm" onClick={addGroup}>
            <Plus size={14} /> Add Group
          </button>
        </div>
      </div>
    );

  const row = (c: { key: string; name: string }, placeholder: string, i: number, count: number, onMove: (by: number) => void, extra?: ReactNode) =>
    editing === c.key ? (
      <NameInput key={c.key} initial={c.name} placeholder={placeholder} onDone={(name) => (rename(c.key, name), finish(c.key, name))} onCancel={() => finish(c.key, c.name)} />
    ) : (
      <div key={c.key} className="stp-comp-row">
        <GripVertical size={16} className="stp-comp-grip" />
        <button type="button" className="stp-comp-name" title="Rename" onClick={() => setEditing(c.key)}>
          {c.name}
        </button>
        {extra}
        <span className="stp-comp-actions">
          <button type="button" className="icon-plain" title="Move up" disabled={i === 0} onClick={() => onMove(-1)}>
            <ArrowUp size={14} />
          </button>
          <button type="button" className="icon-plain" title="Move down" disabled={i === count - 1} onClick={() => onMove(1)}>
            <ArrowDown size={14} />
          </button>
          <button type="button" className="icon-plain" title="Remove" onClick={() => remove(c.key)}>
            <Trash2 size={15} />
          </button>
        </span>
      </div>
    );

  return (
    <div className="stp-comp-list">
      {items.map((it, i) =>
        it.kind === 'component' ? (
          row(it, 'Component name', i, items.length, (by) => moveItem(i, by))
        ) : (
          <div key={it.key} className="stp-comp-group">
            {row({ key: it.key, name: it.name }, 'Group name', i, items.length, (by) => moveItem(i, by), <span className="stp-comp-group-tag">Group</span>)}
            <div className="stp-comp-children">
              {it.children.map((c, j) => row(c, 'Component name', j, it.children.length, (by) => moveChild(it.key, j, by)))}
              <button type="button" className="link-btn stp-comp-add-child" onClick={() => addComponent(it.key)}>
                <Plus size={14} /> Add Component
              </button>
            </div>
          </div>
        ),
      )}
      <div className="stp-comp-add">
        <button type="button" className="btn btn-sm" onClick={() => addComponent()}>
          <Plus size={14} /> Add Component
        </button>
        <button type="button" className="btn btn-sm" onClick={addGroup}>
          <Plus size={14} /> Add Group
        </button>
      </div>
    </div>
  );
}

function NameInput({ initial, placeholder, onDone, onCancel }: { initial: string; placeholder: string; onDone: (name: string) => void; onCancel: () => void }) {
  const [value, setValue] = useState(initial);
  return (
    <div className="stp-comp-row stp-comp-editing">
      <input
        autoFocus
        value={value}
        placeholder={placeholder}
        maxLength={100}
        onChange={(e) => setValue(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === 'Enter') {
            e.preventDefault();
            onDone(value);
          } else if (e.key === 'Escape') onCancel();
        }}
      />
      <button type="button" className="stp-comp-ok" title="Save" onClick={() => onDone(value)}>
        <Check size={15} strokeWidth={3} />
      </button>
      <button type="button" className="stp-comp-cancel" title="Cancel" onClick={onCancel}>
        <X size={15} />
      </button>
    </div>
  );
}
