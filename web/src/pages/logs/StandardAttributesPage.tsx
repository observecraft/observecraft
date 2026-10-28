import { useMemo, useState } from 'react';
import { Link } from 'react-router-dom';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { Pencil, Plus, RotateCcw, Search, Trash2, X } from 'lucide-react';
import {
  createStandardAttribute,
  deleteStandardAttribute,
  listStandardAttributes,
  resetStandardAttributes,
  updateStandardAttribute,
  type StandardAttribute,
  type StandardAttributeInput,
  type StandardAttributeType,
} from '../../api/standardAttributes/client';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { Modal } from '../../components/ui/Modal';
import { useProfile } from '../settings/queries';

const KEY = ['standard-attributes'];
const MAX_ALIASES = 20;
const TYPES: StandardAttributeType[] = ['string', 'integer', 'double', 'boolean'];

/** Reference-list sections, by the path's first segment; others are titled by it. */
const NAMESPACES: Record<string, string> = {
  http: 'HTTP',
  network: 'Network',
  duration: 'Performance',
  error: 'Errors',
  usr: 'User',
  db: 'Database',
  logger: 'Source code',
  evt: 'Events',
  syslog: 'Syslog',
  dns: 'DNS',
};

const namespaceOf = (path: string) => {
  const head = path.split('.')[0];
  return NAMESPACES[head] ?? head.charAt(0).toUpperCase() + head.slice(1);
};

/** Attributes matching the search, grouped by namespace in list order. */
function grouped(list: StandardAttribute[], search: string): [string, StandardAttribute[]][] {
  const s = search.trim().toLowerCase();
  const hit = (a: StandardAttribute) => !s || [a.path, a.description, ...a.aliases].some((t) => t.toLowerCase().includes(s));
  const groups = new Map<string, StandardAttribute[]>();
  for (const a of list.filter(hit)) groups.set(namespaceOf(a.path), [...(groups.get(namespaceOf(a.path)) ?? []), a]);
  return [...groups.entries()];
}

/** Log Configuration → Standard Attributes: the attribute names logs are remapped onto at ingest, with their aliases. */
export function StandardAttributesPage() {
  const qc = useQueryClient();
  const me = useProfile();
  const org = me.data?.org.id;
  const canEdit = me.data?.roles.some((r) => r.id === `${org}-admin` || r.id === `${org}-standard`) ?? false;
  const q = useQuery({ queryKey: KEY, queryFn: listStandardAttributes });
  const [search, setSearch] = useState('');
  const [editing, setEditing] = useState<StandardAttribute | 'new' | null>(null);
  const refresh = () => qc.invalidateQueries({ queryKey: KEY });
  const remove = useMutation({ mutationFn: deleteStandardAttribute, onSuccess: refresh });
  const reset = useMutation({ mutationFn: resetStandardAttributes, onSuccess: (list) => qc.setQueryData(KEY, list) });
  const groups = useMemo(() => grouped(q.data ?? [], search), [q.data, search]);

  if (q.isPending) return <Loading />;
  if (q.isError) return <ErrorState error={q.error} onRetry={() => q.refetch()} />;
  const changed = q.data.some((a) => a.modified || !a.isDefault);
  const confirmRemove = (a: StandardAttribute) =>
    window.confirm(
      a.isDefault
        ? `Remove ${a.path}? Its aliases stop being remapped; "Reset to defaults" brings it back. Logs already stored keep their attributes.`
        : `Delete ${a.path}? Its aliases stop being remapped. Logs already stored keep their attributes.`,
    ) && remove.mutate(a.path);
  return (
    <div className="settings-pad">
      <div className="settings-title-row">
        <h1 className="settings-title">Standard Attributes</h1>
        {canEdit && (
          <div className="pl-title-actions">
            <button
              className="btn"
              disabled={reset.isPending}
              title={changed ? 'Drop your changes and use the default list' : 'The list is the default one'}
              onClick={() => window.confirm('Reset to the default standard attributes? Attributes you added or edited are lost.') && reset.mutate()}
            >
              <RotateCcw size={14} /> Reset to defaults
            </button>
            <button className="btn btn-primary" onClick={() => setEditing('new')}>
              <Plus size={14} /> New Attribute
            </button>
          </div>
        )}
      </div>
      <p className="muted">
        One name per kind of value, whatever your services call it. Every ingested log that has an alias but not the standard attribute gets the alias&apos;s value copied to it, cast to its type,
        after <Link to="/logs/pipelines">pipelines</Link> and before <Link to="/logs/generate-metrics">generated metrics</Link> and <Link to="/logs/indexes">indexes</Link>. The first alias present
        wins; a standard value the log already has is kept. Standard attributes are listed first when you add a facet.
      </p>
      {(remove.error || reset.error) && <p className="form-error">{((remove.error ?? reset.error) as Error).message}</p>}
      <label className="settings-search wide">
        <Search size={16} />
        <input placeholder="Search attributes, aliases or descriptions" value={search} onChange={(e) => setSearch(e.target.value)} />
      </label>
      {groups.length === 0 ? (
        <section className="settings-card settings-empty">
          <p className="muted">{q.data.length ? 'No attribute matches the search.' : 'No standard attributes. Reset to defaults to get the default list back.'}</p>
        </section>
      ) : (
        groups.map(([namespace, list]) => (
          <section key={namespace} className="sa-group">
            <h2 className="sa-group-title">
              {namespace} <span className="muted">{list.length}</span>
            </h2>
            <table className="settings-table ix-table sa-table">
              <thead>
                <tr>
                  <th>ATTRIBUTE</th>
                  <th>TYPE</th>
                  <th>ALIASES</th>
                  <th>DESCRIPTION</th>
                  <th />
                </tr>
              </thead>
              <tbody>
                {list.map((a) => (
                  <tr key={a.path}>
                    <td>
                      <Link className="mono" to={`/logs?query=${encodeURIComponent(`@${a.path}:*`)}`} title="Logs with this attribute">
                        @{a.path}
                      </Link>
                      {!a.isDefault && <span className="pill-chip small">custom</span>}
                      {a.modified && <span className="pill-chip small">edited</span>}
                    </td>
                    <td>{a.type}</td>
                    <td>
                      {a.aliases.length === 0 ? (
                        <span className="muted">—</span>
                      ) : (
                        a.aliases.map((x) => (
                          <code key={x} className="tag-chip sa-alias">
                            {x}
                          </code>
                        ))
                      )}
                      {a.aliases.length > 0 && !a.preserveSource && <span className="muted sa-moved"> (moved)</span>}
                    </td>
                    <td className="sa-description">{a.description}</td>
                    <td className="ix-actions">
                      {canEdit && (
                        <>
                          <button className="icon-plain" title="Edit" onClick={() => setEditing(a)}>
                            <Pencil size={15} />
                          </button>
                          <button className="icon-plain" title={a.isDefault ? 'Remove (until a reset)' : 'Delete'} onClick={() => confirmRemove(a)}>
                            <Trash2 size={15} />
                          </button>
                        </>
                      )}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </section>
        ))
      )}
      {editing && <AttributeModal attribute={editing === 'new' ? null : editing} onClose={() => setEditing(null)} onSaved={() => (setEditing(null), void refresh())} />}
    </div>
  );
}

/** Aliases as removable chips; Enter, comma or "Add" adds the typed one. */
function AliasInput({ aliases, onChange }: { aliases: string[]; onChange: (aliases: string[]) => void }) {
  const [typed, setTyped] = useState('');
  const add = () => {
    const parts = typed
      .split(/[\s,]+/)
      .map((p) => p.trim().replace(/^@+/, ''))
      .filter(Boolean);
    const next = [...aliases];
    for (const p of parts) if (!next.includes(p)) next.push(p);
    onChange(next.slice(0, MAX_ALIASES));
    setTyped('');
  };
  return (
    <div className="sa-aliases">
      {aliases.map((a, i) => (
        <span key={a} className="tag-chip sa-alias">
          <span className="mono">{a}</span>
          <button type="button" className="icon-plain" title={`Remove ${a}`} onClick={() => onChange(aliases.filter((_, j) => j !== i))}>
            <X size={12} />
          </button>
        </span>
      ))}
      {aliases.length < MAX_ALIASES && (
        <>
          <input
            className="mono sa-alias-input"
            value={typed}
            onChange={(e) => setTyped(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === 'Enter' || e.key === ',') {
                e.preventDefault();
                add();
              } else if (e.key === 'Backspace' && !typed && aliases.length) onChange(aliases.slice(0, -1));
            }}
            onBlur={() => typed.trim() && add()}
            placeholder={aliases.length ? 'Add an alias' : 'status_code, http.response.status_code…'}
          />
          <button type="button" className="btn btn-sm" onClick={add} disabled={!typed.trim()}>
            <Plus size={13} /> Add
          </button>
        </>
      )}
    </div>
  );
}

function AttributeModal({ attribute, onClose, onSaved }: { attribute: StandardAttribute | null; onClose: () => void; onSaved: () => void }) {
  const [path, setPath] = useState(attribute?.path ?? '');
  const [type, setType] = useState<StandardAttributeType>(attribute?.type ?? 'string');
  const [description, setDescription] = useState(attribute?.description ?? '');
  const [aliases, setAliases] = useState<string[]>(attribute?.aliases ?? []);
  const [preserveSource, setPreserveSource] = useState(attribute?.preserveSource ?? true);
  const input: StandardAttributeInput = { path, type, description, aliases, preserveSource };
  const save = useMutation({ mutationFn: () => (attribute ? updateStandardAttribute(attribute.path, input) : createStandardAttribute(input)), onSuccess: onSaved });
  return (
    <Modal onClose={onClose} label={attribute ? 'Edit standard attribute' : 'New standard attribute'} width={680} height="auto">
      <form
        className="small-modal ar-form"
        onSubmit={(e) => {
          e.preventDefault();
          save.mutate();
        }}
      >
        <div className="modal-title-row">
          <h3>{attribute ? `Edit @${attribute.path}` : 'New Standard Attribute'}</h3>
          <button type="button" className="icon-plain" onClick={onClose} title="Close">
            <X size={18} />
          </button>
        </div>
        <div className="ar-grid">
          <label className="form-label">
            Attribute path
            <input
              className="mono"
              value={path}
              onChange={(e) => setPath(e.target.value)}
              placeholder="checkout.cart_id"
              maxLength={200}
              required
              disabled={Boolean(attribute)}
              autoFocus={!attribute}
            />
          </label>
          <label className="form-label">
            Type
            <select value={type} onChange={(e) => setType(e.target.value as StandardAttributeType)}>
              {TYPES.map((t) => (
                <option key={t} value={t}>
                  {t}
                </option>
              ))}
            </select>
          </label>
        </div>
        <label className="form-label">
          Description
          <textarea value={description} onChange={(e) => setDescription(e.target.value)} rows={2} maxLength={500} placeholder="What the value means, and its unit" />
        </label>
        <div className="form-label">
          Aliases — tried in order; the first one a log has (with a value of this type) is remapped
          <AliasInput aliases={aliases} onChange={setAliases} />
        </div>
        <label className="ar-check">
          <input type="checkbox" className="checkbox" checked={preserveSource} onChange={(e) => setPreserveSource(e.target.checked)} />
          Keep the alias attribute on the log (otherwise its value is moved)
        </label>
        {attribute && <p className="muted lm-preview">An attribute&apos;s path can&apos;t change: add a new one instead. Changes apply to logs ingested from now on.</p>}
        {save.error && <p className="form-error">{(save.error as Error).message}</p>}
        <div className="small-modal-actions">
          <button type="button" className="btn" onClick={onClose}>
            Cancel
          </button>
          <button className="btn btn-primary" disabled={save.isPending}>
            {attribute ? 'Save' : 'Create Attribute'}
          </button>
        </div>
      </form>
    </Modal>
  );
}
