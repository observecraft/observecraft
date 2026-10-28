import { useCallback, useEffect, useRef, useState } from 'react';
import { Link, useNavigate, useParams } from 'react-router-dom';
import { BookOpen, Check, Copy, EllipsisVertical, LayoutDashboard, Link2, RotateCcw, Star, Trash2 } from 'lucide-react';
import type { CellType, Notebook, NotebookCell, NotebookPatch, NotebookTime } from '../../api/notebooks/types';
import { newCellId } from '../../api/notebooks/templates';
import { TimePicker } from '../../components/shell/PageHeader';
import { TimeProvider, useTime } from '../../components/shell/TimeContext';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { Menu, Select } from '../../components/ui/Dropdown';
import { timeAgo } from '../../lib/format';
import { Initials } from '../dashboards/list/DashboardListPage';
import { cellsToWidgets, duplicateCell, insertCell, moveCell, newCellDefinition } from './cells';
import { CellView, InsertBar } from './NotebookCells';
import { useCloneNotebook, useDeleteNotebook, useExportToDashboard, useNotebook, useNotebookFavorite, useRestoreNotebook, useUpdateNotebook } from './queries';
import '../dashboards/dashboards.css';
import './notebooks.css';

/** Edits are saved this long after the last change. */
const AUTOSAVE_MS = 800;

export function NotebookPage() {
  const { id } = useParams();
  const q = useNotebook(id!);
  if (q.isPending) return <Loading label="Loading notebook…" />;
  if (q.isError) return <ErrorState error={q.error} onRetry={() => q.refetch()} />;
  return (
    <TimeProvider key={q.data.id} initial={q.data.time}>
      <NotebookEditor notebook={q.data} />
    </TimeProvider>
  );
}

type SaveState = 'saved' | 'pending' | 'saving' | 'error';

/** Debounced saving of patches; pending changes are flushed when the editor closes. */
function useAutosave(id: string) {
  const { mutateAsync } = useUpdateNotebook(id);
  const pending = useRef<NotebookPatch>({});
  const timer = useRef<number>();
  const [state, setState] = useState<SaveState>('saved');
  const [error, setError] = useState<string | null>(null);
  const flush = useCallback(async () => {
    window.clearTimeout(timer.current);
    const patch = pending.current;
    pending.current = {};
    if (Object.keys(patch).length === 0) return;
    setState('saving');
    try {
      await mutateAsync(patch);
      setState((s) => (s === 'saving' ? 'saved' : s));
      setError(null);
    } catch (e) {
      setState('error');
      setError((e as Error).message);
    }
  }, [mutateAsync]);
  const save = useCallback(
    (patch: NotebookPatch) => {
      pending.current = { ...pending.current, ...patch };
      setState('pending');
      window.clearTimeout(timer.current);
      timer.current = window.setTimeout(flush, AUTOSAVE_MS);
    },
    [flush],
  );
  /** Drops unsaved changes (the notebook is being deleted). */
  const discard = useCallback(() => {
    window.clearTimeout(timer.current);
    pending.current = {};
  }, []);
  useEffect(
    () => () => {
      void flush();
    },
    [flush],
  );
  return { save, flush, discard, state, error };
}

function SaveIndicator({ state, error }: { state: SaveState; error: string | null }) {
  if (state === 'error')
    return (
      <span className="nb-save nb-save-error" title={error ?? ''}>
        Not saved: {error}
      </span>
    );
  if (state === 'saved')
    return (
      <span className="nb-save">
        <Check size={13} /> Saved
      </span>
    );
  return <span className="nb-save">Saving…</span>;
}

function NotebookEditor({ notebook: nb }: { notebook: Notebook }) {
  const navigate = useNavigate();
  const { range, relative } = useTime();
  const { save, flush, discard, state, error } = useAutosave(nb.id);
  const favorite = useNotebookFavorite();
  const clone = useCloneNotebook();
  const del = useDeleteNotebook();
  const restore = useRestoreNotebook();
  const exportDashboard = useExportToDashboard();
  const canEdit = nb.deletedAt === null;

  const [title, setTitle] = useState(nb.title);
  const [cells, setCells] = useState<NotebookCell[]>(nb.cells);
  const [teams, setTeams] = useState(nb.teams.join(', '));
  const [tags, setTags] = useState(nb.tags.join(', '));
  const [status, setStatus] = useState(nb.status);
  const [editing, setEditing] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);
  const isFavorite = favorite.isPending ? favorite.variables.favorite : nb.favorite;

  const changeCells = (next: NotebookCell[]) => {
    setCells(next);
    save({ cells: next });
  };
  const insert = (index: number, type: CellType) => {
    const cell = { id: newCellId(), definition: newCellDefinition(type) };
    changeCells(insertCell(cells, index, cell));
    setEditing(cell.id);
  };

  // The time picker is the notebook's time range: changing it is saved with the notebook.
  const time = relative.key as NotebookTime;
  const savedTime = useRef(nb.time);
  useEffect(() => {
    if (!canEdit || time === savedTime.current) return;
    savedTime.current = time;
    save({ time });
  }, [time, canEdit, save]);

  const labels = (text: string) =>
    text
      .split(',')
      .map((s) => s.trim())
      .filter(Boolean);

  const copyLink = () => {
    navigator.clipboard?.writeText(window.location.href);
    setCopied(true);
    window.setTimeout(() => setCopied(false), 1500);
  };

  return (
    <div className="dl-page nb-page">
      <header className="product-header nb-header">
        <Link to="/notebooks" className="product-title nb-back" title="All notebooks">
          <BookOpen size={20} />
        </Link>
        <input
          className="nb-title-input"
          aria-label="Notebook title"
          value={title}
          disabled={!canEdit}
          onChange={(e) => setTitle(e.target.value)}
          onBlur={() => title.trim() && title.trim() !== nb.title && save({ title: title.trim() })}
          onKeyDown={(e) => e.key === 'Enter' && (e.target as HTMLInputElement).blur()}
        />
        <button
          className={`icon-plain dl-star${isFavorite ? ' on' : ''}`}
          title={isFavorite ? 'Remove from favorites' : 'Add to favorites'}
          onClick={() => favorite.mutate({ id: nb.id, favorite: !isFavorite })}
        >
          <Star size={18} fill={isFavorite ? 'currentColor' : 'none'} />
        </button>
        {canEdit && (
          <Select
            value={status}
            options={[
              { value: 'published', label: 'Published' },
              { value: 'draft', label: 'Draft' },
            ]}
            onChange={(next) => (setStatus(next), save({ status: next }))}
            triggerClassName="dd-trigger nb-status"
          />
        )}
        <div className="dl-header-actions">
          {canEdit && <SaveIndicator state={state} error={error} />}
          <TimePicker compact />
          <button className="btn" onClick={copyLink}>
            {copied ? <Check size={14} /> : <Link2 size={14} />} {copied ? 'Copied' : 'Share'}
          </button>
          <Menu
            trigger={<EllipsisVertical size={16} />}
            triggerClassName="btn"
            items={[
              { label: 'Clone', icon: <Copy size={14} />, onClick: () => void flush().then(() => clone.mutate(nb.id, { onSuccess: (c) => navigate(`/notebooks/${c.id}`) })) },
              {
                label: 'Export to dashboard',
                icon: <LayoutDashboard size={14} />,
                onClick: () => exportDashboard.mutate({ title: title.trim() || nb.title, widgets: cellsToWidgets(cells) }, { onSuccess: (d) => navigate(`/dashboards/${d.id}`) }),
              },
              ...(canEdit ? [{ label: 'Delete', icon: <Trash2 size={14} />, onClick: () => (discard(), del.mutate(nb.id, { onSuccess: () => navigate('/notebooks') })) }] : []),
            ]}
          />
        </div>
      </header>

      <div className="dl-scroll">
        <div className="nb-doc">
          {nb.deletedAt !== null && (
            <div className="nb-deleted">
              This notebook was deleted {timeAgo(nb.deletedAt)}. Restore it to edit it again.
              <button className="btn btn-sm" onClick={() => restore.mutate(nb.id)}>
                <RotateCcw size={14} /> Restore
              </button>
            </div>
          )}
          <div className="nb-meta">
            {nb.author && (
              <span className="dl-author">
                <Initials name={nb.author.name} /> {nb.author.name}
              </span>
            )}
            <span className="muted-text">Modified {timeAgo(nb.modifiedAt)}</span>
            <label className="nb-meta-field">
              Teams
              <input value={teams} disabled={!canEdit} placeholder="checkout, payments" onChange={(e) => setTeams(e.target.value)} onBlur={() => save({ teams: labels(teams) })} />
            </label>
            <label className="nb-meta-field">
              Tags
              <input value={tags} disabled={!canEdit} placeholder="service:order-service" onChange={(e) => setTags(e.target.value)} onBlur={() => save({ tags: labels(tags) })} />
            </label>
          </div>

          {cells.map((cell, i) => (
            <div key={cell.id}>
              {canEdit && <InsertBar onInsert={(type) => insert(i, type)} />}
              <CellView
                cell={cell}
                index={i}
                count={cells.length}
                range={range}
                canEdit={canEdit}
                editing={editing === cell.id}
                onEditing={(on) => setEditing(on ? cell.id : null)}
                onChange={(next) => changeCells(cells.map((c) => (c.id === cell.id ? next : c)))}
                onMove={(direction) => changeCells(moveCell(cells, cell.id, direction))}
                onDuplicate={() => changeCells(duplicateCell(cells, cell.id))}
                onDelete={() => changeCells(cells.filter((c) => c.id !== cell.id))}
              />
            </div>
          ))}
          {cells.length === 0 && <p className="muted-text nb-empty">This notebook is empty. Add text to explain what you are looking at, and graphs or logs to show it.</p>}
          {canEdit && <InsertBar prominent onInsert={(type) => insert(cells.length, type)} />}
        </div>
      </div>
    </div>
  );
}
