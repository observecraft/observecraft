import { useState } from 'react';
import { Link } from 'react-router-dom';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { Pencil, Plus, Trash2, X } from 'lucide-react';
import { createWorkItemAttribute, deleteWorkItemAttribute, listWorkItemAttributes, updateWorkItemAttribute, type WorkItemAttribute } from '../../../api/events/client';
import { ErrorState, Loading } from '../../../components/ui/AsyncState';
import { Modal } from '../../../components/ui/Modal';
import { useAccess } from '../../settings/queries';

export const ATTRIBUTES_KEY = ['events', 'work-item-attributes'];

/** Event Management → Work Item Attributes: custom fields people fill in on work items. */
export function WorkItemAttributesPage() {
  const qc = useQueryClient();
  const { canWrite } = useAccess();
  const q = useQuery({ queryKey: ATTRIBUTES_KEY, queryFn: listWorkItemAttributes });
  const [editing, setEditing] = useState<WorkItemAttribute | 'new' | null>(null);
  const refresh = () => qc.invalidateQueries({ queryKey: ['events'] });
  const remove = useMutation({ mutationFn: deleteWorkItemAttribute, onSuccess: refresh });
  if (q.isPending) return <Loading />;
  if (q.isError) return <ErrorState error={q.error} onRetry={() => q.refetch()} />;
  return (
    <div className="settings-pad">
      <div className="settings-title-row">
        <h1 className="settings-title">Work Item Attributes</h1>
        {canWrite && (
          <button className="btn btn-primary" onClick={() => setEditing('new')}>
            <Plus size={14} /> New Attribute
          </button>
        )}
      </div>
      <p className="muted">
        Fields every work item of the <Link to="/event/inbox">Triage Inbox</Link> has: a choice among values (a severity, a team) or free text (a ticket). Open a work item to fill them in.
      </p>
      {remove.error && <p className="form-error">{(remove.error as Error).message}</p>}
      {q.data.length === 0 ? (
        <section className="settings-card settings-empty">
          <p className="muted">No work item attributes yet.</p>
        </section>
      ) : (
        <table className="settings-table ix-table">
          <thead>
            <tr>
              <th>ATTRIBUTE</th>
              <th>TYPE</th>
              <th>VALUES</th>
              <th />
            </tr>
          </thead>
          <tbody>
            {q.data.map((a) => (
              <tr key={a.id}>
                <td>{a.name}</td>
                <td>{a.kind === 'select' ? 'Select' : 'Text'}</td>
                <td>
                  {a.kind === 'text' ? (
                    <span className="muted">free text</span>
                  ) : (
                    a.options.map((o) => (
                      <code key={o} className="lm-tag">
                        {o}
                      </code>
                    ))
                  )}
                </td>
                <td className="ix-actions">
                  {canWrite && (
                    <>
                      <button className="icon-plain" title="Edit" onClick={() => setEditing(a)}>
                        <Pencil size={15} />
                      </button>
                      <button className="icon-plain" title="Delete" onClick={() => window.confirm(`Delete attribute "${a.name}" and its values on every work item?`) && remove.mutate(a.id)}>
                        <Trash2 size={15} />
                      </button>
                    </>
                  )}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
      {editing && <AttributeModal attribute={editing === 'new' ? null : editing} next={q.data.length} onClose={() => setEditing(null)} onSaved={() => (setEditing(null), void refresh())} />}
    </div>
  );
}

function AttributeModal({ attribute, next, onClose, onSaved }: { attribute: WorkItemAttribute | null; next: number; onClose: () => void; onSaved: () => void }) {
  const [name, setName] = useState(attribute?.name ?? '');
  const [kind, setKind] = useState<WorkItemAttribute['kind']>(attribute?.kind ?? 'select');
  const [options, setOptions] = useState(attribute?.options.join(', ') ?? '');
  const input = {
    name,
    kind,
    options: options
      .split(',')
      .map((o) => o.trim())
      .filter(Boolean),
    position: attribute?.position ?? next,
  };
  const save = useMutation({ mutationFn: () => (attribute ? updateWorkItemAttribute(attribute.id, input) : createWorkItemAttribute(input)), onSuccess: onSaved });
  return (
    <Modal onClose={onClose} label={attribute ? 'Edit attribute' : 'New attribute'} width={600} height="auto">
      <form
        className="small-modal ar-form"
        onSubmit={(e) => {
          e.preventDefault();
          save.mutate();
        }}
      >
        <div className="modal-title-row">
          <h3>{attribute ? `Edit ${attribute.name}` : 'New Attribute'}</h3>
          <button type="button" className="icon-plain" onClick={onClose} title="Close">
            <X size={18} />
          </button>
        </div>
        <div className="ar-grid">
          <label className="form-label">
            Name
            <input value={name} onChange={(e) => setName(e.target.value)} placeholder="Severity" maxLength={200} required autoFocus />
          </label>
          <label className="form-label">
            Type
            <select value={kind} onChange={(e) => setKind(e.target.value as WorkItemAttribute['kind'])}>
              <option value="select">Select — one of a list of values</option>
              <option value="text">Text — free text</option>
            </select>
          </label>
        </div>
        {kind === 'select' && (
          <label className="form-label">
            Values (comma separated)
            <input value={options} onChange={(e) => setOptions(e.target.value)} placeholder="SEV-1, SEV-2, SEV-3" required />
          </label>
        )}
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
