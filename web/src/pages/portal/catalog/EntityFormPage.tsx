import { useMemo, useState } from 'react';
import { Link, useNavigate, useParams, useSearchParams } from 'react-router-dom';
import { ArrowLeft, Plus, Trash2, Upload } from 'lucide-react';
import { CONTACT_TYPES, DEFINITION_KINDS, LINK_TYPES, validateDefinition } from '../../../api/portalCatalog/logic';
import type { EntityDefinition } from '../../../api/portalCatalog/types';
import { ErrorState, Loading } from '../../../components/ui/AsyncState';
import { definitionFromForm, definitionYaml, emptyForm, formFromDefinition, KIND_LABEL, type EntityForm } from './catalogModel';
import { ImportModal } from './ImportModal';
import { useEntityDefinition, useOnCallTeams, useUpdateDefinition, useUpsertDefinition } from './queries';
import { SECTION_ICON } from './visuals';
import './catalog.css';

const LIFECYCLES = ['', 'production', 'experimental', 'deprecated'];
const TIERS = ['', '1', '2', '3'];

/** `/portal/catalog/new` and `/portal/catalog/edit/:id`: an entity definition as a form, with its YAML. */
export function EntityFormPage() {
  const { id } = useParams();
  const q = useEntityDefinition(id ?? null);
  if (id && q.isPending) return <Loading label="Loading the definition…" />;
  if (id && q.isError) return <ErrorState error={q.error} onRetry={() => q.refetch()} />;
  return <EntityFormBody key={id ?? 'new'} id={id ?? null} base={q.data?.definition} />;
}

function EntityFormBody({ id, base }: { id: string | null; base?: EntityDefinition }) {
  const [params] = useSearchParams();
  const navigate = useNavigate();
  const [form, setForm] = useState<EntityForm>(() => {
    if (base) return formFromDefinition(base);
    const kind = params.get('kind');
    return { ...emptyForm(kind && (DEFINITION_KINDS as readonly string[]).includes(kind) ? kind : 'service'), name: params.get('name') ?? '' };
  });
  const [importing, setImporting] = useState(false);
  const [touched, setTouched] = useState(false);
  const teams = useOnCallTeams();
  const create = useUpsertDefinition();
  const update = useUpdateDefinition();
  const save = id ? update : create;
  const set = (patch: Partial<EntityForm>) => setForm((f) => ({ ...f, ...patch }));
  const def = useMemo(() => definitionFromForm(form, base), [form, base]);
  // The same checks as the server, as the user types.
  const problem = useMemo(() => {
    try {
      validateDefinition(def);
      return null;
    } catch (e) {
      return (e as Error).message;
    }
  }, [def]);
  const submit = () => {
    setTouched(true);
    if (problem) return;
    const done = (saved: { kind: string; name: string }) => navigate(`/portal/catalog?entity=${encodeURIComponent(`${saved.kind}:${saved.name}`)}`);
    if (id) update.mutate({ id, def }, { onSuccess: done });
    else create.mutate(def, { onSuccess: done });
  };

  return (
    <div className="pc-form-page">
      <div className="pc-form-top">
        <Link to="/portal/catalog" className="btn btn-sm">
          <ArrowLeft size={14} /> Catalog
        </Link>
        <h1>{id ? `Edit ${form.kind}:${form.name}` : 'Add an entity'}</h1>
        {!id && (
          <button className="btn btn-sm" onClick={() => setImporting(true)}>
            <Upload size={14} /> Import definitions
          </button>
        )}
      </div>
      <div className="pc-form-grid">
        <form
          className="pc-form"
          onSubmit={(e) => {
            e.preventDefault();
            submit();
          }}
        >
          <fieldset className="pc-fieldset">
            <legend>Kind</legend>
            <div className="pc-kinds">
              {DEFINITION_KINDS.map((k) => {
                const Icon = SECTION_ICON[k];
                return (
                  <button type="button" key={k} className={`pc-kind${form.kind === k ? ' active' : ''}`} aria-pressed={form.kind === k} onClick={() => set({ kind: k })}>
                    <Icon size={16} /> {KIND_LABEL[k]}
                  </button>
                );
              })}
            </div>
          </fieldset>
          <div className="pc-form-row">
            <label className="pc-label">
              Name
              <input className="sc-input" value={form.name} onChange={(e) => set({ name: e.target.value })} placeholder="order-service" required />
              <span className="pc-hint">Matches the telemetry name (service.name, the database or the queue).</span>
            </label>
            <label className="pc-label">
              Display name
              <input className="sc-input" value={form.displayName} onChange={(e) => set({ displayName: e.target.value })} placeholder="Order Service" />
            </label>
          </div>
          <div className="pc-form-row">
            <label className="pc-label">
              Owner team
              <input className="sc-input" list="pc-teams" value={form.owner} onChange={(e) => set({ owner: e.target.value })} placeholder="checkout" />
              <datalist id="pc-teams">
                {(teams.data ?? []).map((t) => (
                  <option key={t.handle} value={t.handle}>
                    {t.name}
                  </option>
                ))}
              </datalist>
            </label>
            <label className="pc-label">
              System
              <input className="sc-input" value={form.system} onChange={(e) => set({ system: e.target.value })} placeholder="checkout" />
              <span className="pc-hint">The systems it belongs to, comma-separated.</span>
            </label>
          </div>
          <label className="pc-label">
            Description
            <textarea className="sc-input pc-textarea" value={form.description} onChange={(e) => set({ description: e.target.value })} rows={3} />
          </label>
          <div className="pc-form-row pc-form-row-3">
            <label className="pc-label">
              Lifecycle
              <select className="sc-input" value={LIFECYCLES.includes(form.lifecycle) ? form.lifecycle : ''} onChange={(e) => set({ lifecycle: e.target.value })}>
                {LIFECYCLES.map((l) => (
                  <option key={l} value={l}>
                    {l || '—'}
                  </option>
                ))}
              </select>
            </label>
            <label className="pc-label">
              Tier
              <select className="sc-input" value={TIERS.includes(form.tier) ? form.tier : ''} onChange={(e) => set({ tier: e.target.value })}>
                {TIERS.map((t) => (
                  <option key={t} value={t}>
                    {t || '—'}
                  </option>
                ))}
              </select>
            </label>
            <label className="pc-label">
              Type
              <input
                className="sc-input"
                value={form.type}
                onChange={(e) => set({ type: e.target.value })}
                placeholder={form.kind === 'datastore' ? 'postgres' : form.kind === 'queue' ? 'kafka' : 'web'}
              />
            </label>
          </div>
          <div className="pc-form-row">
            <label className="pc-label">
              Tags
              <input className="sc-input" value={form.tags} onChange={(e) => set({ tags: e.target.value })} placeholder="tier:1, env:prod" />
            </label>
            <label className="pc-label">
              Languages
              <input className="sc-input" value={form.languages} onChange={(e) => set({ languages: e.target.value })} placeholder="go, python" />
            </label>
          </div>
          <label className="pc-label">
            Depends on
            <input className="sc-input" value={form.dependsOn} onChange={(e) => set({ dependsOn: e.target.value })} placeholder="service:billing, datastore:orders-db" />
          </label>

          <fieldset className="pc-fieldset">
            <legend>Links</legend>
            {form.links.map((l, i) => (
              <div key={i} className="pc-list-row">
                <input
                  className="sc-input"
                  value={l.name}
                  onChange={(e) => set({ links: form.links.map((x, j) => (j === i ? { ...x, name: e.target.value } : x)) })}
                  placeholder="Name"
                  aria-label="Link name"
                />
                <select className="sc-input" value={l.type} onChange={(e) => set({ links: form.links.map((x, j) => (j === i ? { ...x, type: e.target.value } : x)) })} aria-label="Link type">
                  {LINK_TYPES.map((t) => (
                    <option key={t}>{t}</option>
                  ))}
                </select>
                <input
                  className="sc-input pc-grow"
                  value={l.url}
                  onChange={(e) => set({ links: form.links.map((x, j) => (j === i ? { ...x, url: e.target.value } : x)) })}
                  placeholder="https://"
                  aria-label="Link URL"
                />
                <button type="button" className="sc-icon-btn" aria-label="Remove link" onClick={() => set({ links: form.links.filter((_, j) => j !== i) })}>
                  <Trash2 size={15} />
                </button>
              </div>
            ))}
            <button type="button" className="btn btn-sm" onClick={() => set({ links: [...form.links, { name: '', type: form.links.some((l) => l.type === 'repo') ? 'doc' : 'repo', url: '' }] })}>
              <Plus size={13} /> Add link
            </button>
          </fieldset>
          <fieldset className="pc-fieldset">
            <legend>Contacts</legend>
            {form.contacts.map((c, i) => (
              <div key={i} className="pc-list-row">
                <input
                  className="sc-input"
                  value={c.name}
                  onChange={(e) => set({ contacts: form.contacts.map((x, j) => (j === i ? { ...x, name: e.target.value } : x)) })}
                  placeholder="Name"
                  aria-label="Contact name"
                />
                <select className="sc-input" value={c.type} onChange={(e) => set({ contacts: form.contacts.map((x, j) => (j === i ? { ...x, type: e.target.value } : x)) })} aria-label="Contact type">
                  {CONTACT_TYPES.map((t) => (
                    <option key={t}>{t}</option>
                  ))}
                </select>
                <input
                  className="sc-input pc-grow"
                  value={c.contact}
                  onChange={(e) => set({ contacts: form.contacts.map((x, j) => (j === i ? { ...x, contact: e.target.value } : x)) })}
                  placeholder={c.type === 'email' ? 'team@example.com' : c.type === 'slack' ? '#channel' : 'https://'}
                  aria-label="Contact"
                />
                <button type="button" className="sc-icon-btn" aria-label="Remove contact" onClick={() => set({ contacts: form.contacts.filter((_, j) => j !== i) })}>
                  <Trash2 size={15} />
                </button>
              </div>
            ))}
            <button type="button" className="btn btn-sm" onClick={() => set({ contacts: [...form.contacts, { name: '', type: 'email', contact: '' }] })}>
              <Plus size={13} /> Add contact
            </button>
          </fieldset>
          {form.kind === 'service' && <p className="pc-hint">A service's team, contacts, repository and docs are shared with Scorecards.</p>}
          {(touched && problem) || save.isError ? <div className="pc-error">{touched && problem ? problem : (save.error as Error).message}</div> : null}
          <div className="pc-form-actions">
            <Link to="/portal/catalog" className="btn">
              Cancel
            </Link>
            <button type="submit" className="btn btn-primary" disabled={save.isPending}>
              {save.isPending ? 'Saving…' : id ? 'Save' : 'Create entity'}
            </button>
          </div>
        </form>
        <aside className="pc-preview" aria-label="Definition preview">
          <div className="pc-preview-head">entity.yaml</div>
          <pre className="pc-code">{definitionYaml(def)}</pre>
        </aside>
      </div>
      {importing && <ImportModal onClose={() => setImporting(false)} />}
    </div>
  );
}
