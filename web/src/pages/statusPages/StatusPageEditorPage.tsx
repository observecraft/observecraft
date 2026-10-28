import { useState, type ReactNode } from 'react';
import { Link, useNavigate, useParams } from 'react-router-dom';
import { ArrowDown, ArrowUp, ChevronRight, Globe, Lock, Plus, Trash2 } from 'lucide-react';
import { checkLogoUrl, checkSlug, currentComponents, MAX_SLUG_CHARS, overallStatus, slugify } from '../../api/statusPages/status';
import {
  COMPONENT_STATUSES,
  COMPONENT_STATUS_LABEL,
  type PublicStatusPage,
  type StatusPageComponentInput,
  type StatusPageDetail,
  type StatusPageInput,
  type StatusPageVisibility,
} from '../../api/statusPages/types';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { useStatusPage, useStatusPageMutations } from './queries';
import { StatusPageView } from './StatusPageView';
import '../settings/settings.css';
import '../logs/logs.css';

interface Row extends StatusPageComponentInput {
  /** Stable React key: the component id, or a local one for a new component. */
  key: string;
}

let nextKey = 0;
const newRow = (): Row => ({ key: `new-${nextKey++}`, name: '', group: '', status: 'operational' });
const STEPS = ['Type', 'Settings', 'Components'] as const;

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
  const [description, setDescription] = useState(page?.description ?? '');
  const [logoUrl, setLogoUrl] = useState(page?.logoUrl ?? '');
  const [email, setEmail] = useState(page?.emailSubscriptions ?? false);
  const [slack, setSlack] = useState(page?.slackSubscriptions ?? false);
  const [rows, setRows] = useState<Row[]>(() => page?.components.map((c) => ({ key: c.id, id: c.id, name: c.name, group: c.group ?? '', status: c.status })) ?? [newRow()]);

  const shownSlug = slugEdited ? slug : slugify(name);
  const slugError = shownSlug ? checkSlug(shownSlug) : null;
  const logoError = checkLogoUrl(logoUrl);
  const filled = rows.filter((r) => r.name.trim());
  const names = filled.map((r) => r.name.trim().toLowerCase());
  const duplicate = names.find((n, i) => names.indexOf(n) !== i);
  const settingsReady = Boolean(name.trim() && shownSlug && !slugError && !logoError);
  const ready = settingsReady && !duplicate;

  const set = (i: number, patch: Partial<Row>) => setRows((rs) => rs.map((r, j) => (j === i ? { ...r, ...patch } : r)));
  const move = (i: number, by: number) =>
    setRows((rs) => {
      const out = [...rs];
      const [r] = out.splice(i, 1);
      out.splice(i + by, 0, r);
      return out;
    });
  const input = (): StatusPageInput => ({
    name: name.trim(),
    slug: shownSlug,
    visibility,
    description,
    logoUrl: logoUrl.trim(),
    emailSubscriptions: email,
    slackSubscriptions: slack,
    components: filled.map(({ id, name, group, status }) => ({ id, name: name.trim(), group: group?.trim() || null, status })),
  });
  const submit = () =>
    page ? update.mutate({ id: page.id, input: input() }, { onSuccess: (d) => navigate(`/status-pages/${d.id}`) }) : create.mutate(input(), { onSuccess: (d) => navigate(`/status-pages/${d.id}`) });

  // The preview: the page as its visitors will see it, with the notices it already has.
  const components = currentComponents(
    filled.map((r) => ({ id: r.id ?? r.key, name: r.name.trim(), group: r.group?.trim() || null, status: r.status ?? 'operational' })),
    page?.notices ?? [],
  );
  const preview: PublicStatusPage = {
    name: name.trim(),
    slug: shownSlug,
    visibility,
    description,
    logoUrl: logoError ? '' : logoUrl.trim(),
    emailSubscriptions: email,
    slackSubscriptions: slack,
    components,
    overallStatus: overallStatus(components),
    notices: page?.notices ?? [],
    updatedAt: Date.now(),
  };
  const canOpen = (i: number) => i === 0 || settingsReady;
  const title = page ? `Edit ${page.name}` : 'Create Status Page';

  return (
    <div className="stp-page">
      <nav className="stp-crumbs">
        <Link to="/status-pages">Status Pages</Link>
        <ChevronRight size={14} />
        {page && (
          <>
            <Link to={`/status-pages/${page.id}`}>{page.name}</Link>
            <ChevronRight size={14} />
          </>
        )}
        <span>{page ? 'Edit' : 'Create Status Page'}</span>
      </nav>
      <h1 className="settings-title">{title}</h1>
      <div className="stp-editor">
        <form
          className="stp-editor-form ar-form"
          onSubmit={(e) => {
            e.preventDefault();
            if (step < STEPS.length - 1) setStep(step + 1);
            else if (ready) submit();
          }}
        >
          <ol className="stp-steps">
            {STEPS.map((s, i) => (
              <li key={s} className={i === step ? 'active' : i < step ? 'done' : undefined}>
                <button type="button" disabled={!canOpen(i)} onClick={() => setStep(i)}>
                  <span className="stp-step-num">{i + 1}</span> {s}
                </button>
              </li>
            ))}
          </ol>

          {step === 0 && (
            <div className="stp-step-body">
              <p className="muted">Who can see the page. You can change it later.</p>
              <div className="stp-type-cards" role="radiogroup" aria-label="Visibility">
                <TypeCard value="public" current={visibility} onPick={setVisibility} icon={<Globe size={22} />} title="Public">
                  Anyone with the link can see it, without signing in.
                </TypeCard>
                <TypeCard value="internal" current={visibility} onPick={setVisibility} icon={<Lock size={22} />} title="Internal">
                  Only signed-in members of this organization can see it.
                </TypeCard>
              </div>
            </div>
          )}

          {step === 1 && (
            <div className="stp-step-body">
              <label className="form-label">
                Page title — the header when there is no logo
                <input value={name} onChange={(e) => setName(e.target.value)} placeholder="Example Shop Status" maxLength={100} autoFocus />
              </label>
              <label className="form-label">
                Page URL
                <span className="stp-url">
                  <span className="stp-url-fixed mono">{window.location.origin}/status/</span>
                  <input
                    className="mono"
                    value={shownSlug}
                    onChange={(e) => {
                      setSlugEdited(true);
                      setSlug(e.target.value);
                    }}
                    placeholder="shop-status"
                    maxLength={MAX_SLUG_CHARS}
                    aria-label="Page URL slug"
                  />
                </span>
              </label>
              {slugError && <p className="form-error">{slugError}</p>}
              <label className="form-label">
                Description
                <input value={description} onChange={(e) => setDescription(e.target.value)} placeholder="What the page covers" maxLength={1000} />
              </label>
              <label className="form-label">
                Company logo URL (optional) — an http(s) image shown instead of the title
                <input value={logoUrl} onChange={(e) => setLogoUrl(e.target.value)} placeholder="https://cdn.example.com/logo.svg" maxLength={2048} />
              </label>
              {logoError && <p className="form-error">{logoError}</p>}
              <div className="form-label">
                Subscriptions — show a Subscribe button for these channels
                <label className="stp-check">
                  <input type="checkbox" checked={email} onChange={(e) => setEmail(e.target.checked)} /> Email
                </label>
                <label className="stp-check">
                  <input type="checkbox" checked={slack} onChange={(e) => setSlack(e.target.checked)} /> Slack
                </label>
                <span className="muted stp-note">Updates aren&apos;t sent to subscribers yet; the button tells visitors so.</span>
              </div>
            </div>
          )}

          {step === 2 && (
            <div className="stp-step-body">
              <p className="muted">The parts of your service your users rely on, in display order. Components with the same group are shown together.</p>
              <div className="stp-editor-components">
                {rows.map((r, i) => (
                  <div key={r.key} className="stp-editor-component">
                    <input aria-label="Component name" value={r.name} onChange={(e) => set(i, { name: e.target.value })} placeholder="Checkout" maxLength={100} />
                    <input aria-label="Group" value={r.group ?? ''} onChange={(e) => set(i, { group: e.target.value })} placeholder="Group (optional)" maxLength={100} />
                    <select aria-label="Status" value={r.status} onChange={(e) => set(i, { status: e.target.value as Row['status'] })}>
                      {COMPONENT_STATUSES.map((s) => (
                        <option key={s} value={s}>
                          {COMPONENT_STATUS_LABEL[s]}
                        </option>
                      ))}
                    </select>
                    <div className="stp-actions">
                      <button type="button" className="icon-plain" title="Move up" disabled={i === 0} onClick={() => move(i, -1)}>
                        <ArrowUp size={14} />
                      </button>
                      <button type="button" className="icon-plain" title="Move down" disabled={i === rows.length - 1} onClick={() => move(i, 1)}>
                        <ArrowDown size={14} />
                      </button>
                      <button type="button" className="icon-plain" title="Remove" onClick={() => setRows((rs) => rs.filter((_, j) => j !== i))}>
                        <Trash2 size={14} />
                      </button>
                    </div>
                  </div>
                ))}
              </div>
              <div>
                <button type="button" className="btn" onClick={() => setRows((rs) => [...rs, newRow()])}>
                  <Plus size={14} /> Add Component
                </button>
              </div>
              {duplicate && <p className="form-error">Component &apos;{duplicate}&apos; appears twice</p>}
            </div>
          )}

          {save.error && <p className="form-error">{save.error.message}</p>}
          <div className="stp-step-actions">
            {step > 0 ? (
              <button type="button" className="btn" onClick={() => setStep(step - 1)}>
                Back
              </button>
            ) : (
              <Link className="btn" to={page ? `/status-pages/${page.id}` : '/status-pages'}>
                Cancel
              </Link>
            )}
            {step < STEPS.length - 1 ? (
              <button className="btn btn-primary" disabled={step === 1 && !settingsReady}>
                Next
              </button>
            ) : (
              <button className="btn btn-primary" disabled={!ready || save.isPending}>
                {save.isPending ? 'Saving…' : page ? 'Save' : 'Create Status Page'}
              </button>
            )}
          </div>
        </form>
        <aside className="stp-preview" aria-label="Preview">
          <div className="stp-preview-label muted">Preview</div>
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
      {icon}
      <strong>{title}</strong>
      <span className="muted">{children}</span>
    </button>
  );
}
