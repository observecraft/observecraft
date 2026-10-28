import { useDeferredValue, useState } from 'react';
import { useNavigate } from 'react-router-dom';
import { BookOpen, Plus } from 'lucide-react';
import type { NotebookCell } from '../../api/notebooks/types';
import { Modal } from '../../components/ui/Modal';
import { timeAgo } from '../../lib/format';
import { useAppendCells, useCreateNotebook, useNotebookSearch } from './queries';
import './notebooks.css';

/** "Add to notebook" from other pages: appends `cells` to a notebook (or a new one) and opens it. */
export function AddToNotebookModal({ cells, newTitle, onClose }: { cells: NotebookCell[]; newTitle: string; onClose: () => void }) {
  const navigate = useNavigate();
  const [query, setQuery] = useState('');
  const search = useNotebookSearch({ query: useDeferredValue(query), view: 'all', sort: 'modified', dir: 'desc', offset: 0, limit: 20 });
  const append = useAppendCells();
  const create = useCreateNotebook();
  const open = (id: string) => {
    onClose();
    navigate(`/notebooks/${id}`);
  };
  const error = append.error ?? create.error;
  const busy = append.isPending || create.isPending;
  return (
    <Modal onClose={onClose} label="Add to notebook" width={520} height="auto">
      <div className="small-modal">
        <h3>Add to notebook</h3>
        <button className="btn btn-primary" disabled={busy} onClick={() => create.mutate({ title: newTitle, cells }, { onSuccess: (nb) => open(nb.id) })}>
          <Plus size={14} /> New notebook
        </button>
        <input autoFocus placeholder="…or find a notebook" value={query} onChange={(e) => setQuery(e.target.value)} />
        <div className="nb-pick-list">
          {(search.data?.notebooks ?? []).map((nb) => (
            <button key={nb.id} disabled={busy} onClick={() => append.mutate({ id: nb.id, cells }, { onSuccess: () => open(nb.id) })}>
              <span>
                <BookOpen size={14} className="muted" /> {nb.title}
              </span>
              <span className="muted-text">{timeAgo(nb.modifiedAt)}</span>
            </button>
          ))}
          {search.data && search.data.notebooks.length === 0 && <p className="muted-text nb-empty">No notebook matches.</p>}
        </div>
        {error && <p className="form-error">{error.message}</p>}
      </div>
    </Modal>
  );
}
