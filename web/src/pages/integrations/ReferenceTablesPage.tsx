import { useMemo, useState } from 'react';
import { Link, useNavigate } from 'react-router-dom';
import { Plus, Search, Table2, X } from 'lucide-react';
import { checkFile, checkName, MAX_DESCRIPTION_CHARS, MAX_NAME_CHARS } from '../../api/referenceTables/file';
import type { ReferenceTable } from '../../api/referenceTables/types';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { Modal } from '../../components/ui/Modal';
import { formatLongDate, timeAgo } from '../../lib/format';
import { useProfile } from '../settings/queries';
import { useReferenceTableMutations, useReferenceTables } from './queries';
import { CsvFileField, CsvPreview, useCsvFile } from './ReferenceTableUpload';
import '../settings/settings.css';
import '../logs/logs.css';

/** Standard users and admins create, replace and delete reference tables. */
export function useCanEditReferenceTables(): boolean {
  const me = useProfile();
  const org = me.data?.org.id;
  return me.data?.roles.some((r) => r.id === `${org}-admin` || r.id === `${org}-standard`) ?? false;
}

export const SOURCE_LABELS: Record<string, string> = { local_file: 'Local file' };

const matches = (t: ReferenceTable, search: string) => {
  const s = search.trim().toLowerCase();
  return !s || [t.name, t.description, t.primaryKey, t.createdBy ?? ''].some((v) => v.toLowerCase().includes(s));
};

/** Integrations → Reference Tables: the org's uploaded lookup tables. */
export function ReferenceTablesPage() {
  const q = useReferenceTables();
  const canEdit = useCanEditReferenceTables();
  const [search, setSearch] = useState('');
  const [creating, setCreating] = useState(false);
  const shown = useMemo(() => (q.data ?? []).filter((t) => matches(t, search)), [q.data, search]);

  if (q.isPending) return <Loading />;
  if (q.isError) return <ErrorState error={q.error} onRetry={() => q.refetch()} />;
  if (q.data.length === 0)
    return (
      <div className="int-page">
        <div className="rt-empty">
          <Table2 size={56} strokeWidth={1.25} className="brand-icon" />
          <h2>Get started with Reference Tables</h2>
          <p className="muted">Upload a CSV of your own data, such as the team owning each service or the name of each SKU, and look its rows up by a primary key.</p>
          {canEdit && (
            <button className="btn btn-primary" onClick={() => setCreating(true)}>
              <Plus size={14} /> Create Your First Table
            </button>
          )}
        </div>
        {creating && <CreateModal onClose={() => setCreating(false)} />}
      </div>
    );
  return (
    <div className="int-page">
      <div className="settings-title-row">
        <h1 className="settings-title">Reference Tables</h1>
        {canEdit && (
          <button className="btn btn-primary" onClick={() => setCreating(true)}>
            <Plus size={14} /> New Reference Table
          </button>
        )}
      </div>
      <p className="muted">
        Tables of your own data, uploaded as CSV files, with a primary key column to look rows up by: the team owning each service, the product name and category of each SKU, the region of each
        account. Uploading a new file replaces every row.
      </p>
      <label className="settings-search wide">
        <Search size={16} />
        <input placeholder="Search tables by name, description, primary key or creator" value={search} onChange={(e) => setSearch(e.target.value)} />
      </label>
      {shown.length === 0 ? (
        <section className="settings-card settings-empty">
          <p className="muted">No reference table matches the search.</p>
        </section>
      ) : (
        <table className="settings-table ix-table">
          <thead>
            <tr>
              <th>NAME</th>
              <th>DESCRIPTION</th>
              <th>SOURCE</th>
              <th className="num">ROWS</th>
              <th>PRIMARY KEY</th>
              <th>LAST UPDATED</th>
              <th>CREATED BY</th>
            </tr>
          </thead>
          <tbody>
            {shown.map((t) => (
              <tr key={t.name}>
                <td>
                  <Link className="mono" to={`/integrations/reference-tables/${encodeURIComponent(t.name)}`}>
                    {t.name}
                  </Link>
                </td>
                <td>{t.description || <span className="muted">—</span>}</td>
                <td>{SOURCE_LABELS[t.source] ?? t.source}</td>
                <td className="num">{t.rowCount.toLocaleString()}</td>
                <td>
                  <code>{t.primaryKey}</code>
                </td>
                <td title={formatLongDate(t.updatedAt)}>{timeAgo(t.updatedAt)}</td>
                <td>{t.createdBy ?? <span className="muted">—</span>}</td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
      {creating && <CreateModal onClose={() => setCreating(false)} />}
    </div>
  );
}

function CreateModal({ onClose }: { onClose: () => void }) {
  const navigate = useNavigate();
  const { create } = useReferenceTableMutations();
  const { file, pick } = useCsvFile();
  const [name, setName] = useState('');
  const [description, setDescription] = useState('');
  const [chosenKey, setChosenKey] = useState('');
  const columns = file?.parsed?.columns ?? [];
  const primaryKey = columns.includes(chosenKey) ? chosenKey : (columns[0] ?? '');
  const nameError = name ? checkName(name.trim()) : null;
  const fileError = file?.error ?? (file?.parsed ? checkFile({ ...file.parsed, primaryKey }) : null);
  const ready = Boolean(name.trim() && file?.parsed && !nameError && !fileError);
  const submit = () =>
    file?.parsed &&
    create.mutate(
      { name: name.trim(), description, primaryKey, fileName: file.fileName, columns: file.parsed.columns, rows: file.parsed.rows },
      { onSuccess: (t) => navigate(`/integrations/reference-tables/${encodeURIComponent(t.name)}`) },
    );
  return (
    <Modal onClose={onClose} label="New reference table" width={760} height="auto">
      <form
        className="small-modal ar-form"
        onSubmit={(e) => {
          e.preventDefault();
          if (ready) submit();
        }}
      >
        <div className="modal-title-row">
          <h3>New Reference Table</h3>
          <button type="button" className="icon-plain" onClick={onClose} title="Close">
            <X size={18} />
          </button>
        </div>
        <label className="form-label">
          Name — lowercase letters, digits and underscores; it can&apos;t be changed later
          <input className="mono" value={name} onChange={(e) => setName(e.target.value)} placeholder="service_owners" maxLength={MAX_NAME_CHARS} autoFocus />
        </label>
        {nameError && <p className="form-error">{nameError}</p>}
        <label className="form-label">
          Description
          <input value={description} onChange={(e) => setDescription(e.target.value)} placeholder="What the table holds and where it comes from" maxLength={MAX_DESCRIPTION_CHARS} />
        </label>
        <CsvFileField file={file} onPick={pick} />
        {columns.length > 0 && (
          <label className="form-label">
            Primary key — the column whose values identify rows (unique and never empty)
            <select value={primaryKey} onChange={(e) => setChosenKey(e.target.value)}>
              {columns.map((c, i) => (
                <option key={i} value={c}>
                  {c || `(column ${i + 1} has no name)`}
                </option>
              ))}
            </select>
          </label>
        )}
        {file?.parsed && <CsvPreview parsed={file.parsed} primaryKey={primaryKey} />}
        {fileError && <p className="form-error">{fileError}</p>}
        {create.error && <p className="form-error">{(create.error as Error).message}</p>}
        <div className="small-modal-actions">
          <button type="button" className="btn" onClick={onClose}>
            Cancel
          </button>
          <button className="btn btn-primary" disabled={!ready || create.isPending}>
            {create.isPending ? 'Uploading…' : 'Create Table'}
          </button>
        </div>
      </form>
    </Modal>
  );
}
