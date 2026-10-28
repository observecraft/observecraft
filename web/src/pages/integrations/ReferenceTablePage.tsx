import { useEffect, useState } from 'react';
import { Link, useNavigate, useParams } from 'react-router-dom';
import { ArrowLeft, Search, Trash2, Upload, X } from 'lucide-react';
import { checkFile } from '../../api/referenceTables/file';
import type { ReferenceTable } from '../../api/referenceTables/types';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { Modal } from '../../components/ui/Modal';
import { Pager } from '../../components/ui/Pager';
import { formatLongDate, timeAgo } from '../../lib/format';
import { useReferenceTable, useReferenceTableMutations } from './queries';
import { SOURCE_LABELS, useCanEditReferenceTables } from './ReferenceTablesPage';
import { CsvFileField, CsvPreview, useCsvFile } from './ReferenceTableUpload';
import '../settings/settings.css';
import '../logs/logs.css';

const PAGE = 50;

/** `value`, once it has stopped changing for `ms`. */
function useSettled<T>(value: T, ms = 300): T {
  const [settled, setSettled] = useState(value);
  useEffect(() => {
    const t = setTimeout(() => setSettled(value), ms);
    return () => clearTimeout(t);
  }, [value, ms]);
  return settled;
}

/** Integrations → Reference Tables → a table: its schema and rows, searchable; replace the file or delete it. */
export function ReferenceTablePage() {
  const { name = '' } = useParams();
  const navigate = useNavigate();
  const canEdit = useCanEditReferenceTables();
  const { remove } = useReferenceTableMutations();
  const [search, setSearch] = useState('');
  const [column, setColumn] = useState('');
  const [offset, setOffset] = useState(0);
  const [replacing, setReplacing] = useState(false);
  const query = useSettled(search);
  useEffect(() => setOffset(0), [query, column]);
  const q = useReferenceTable(name, { query, column: column || undefined, limit: PAGE, offset });

  if (q.isPending) return <Loading />;
  if (q.isError && !q.data) return <ErrorState error={q.error} onRetry={() => q.refetch()} />;
  const { table, rows, total } = q.data;
  const pk = table.columns.indexOf(table.primaryKey);
  const confirmDelete = () =>
    window.confirm(`Delete the reference table "${table.name}" and its ${table.rowCount.toLocaleString()} rows? This can't be undone.`) &&
    remove.mutate(table.name, { onSuccess: () => navigate('/integrations/reference-tables') });
  return (
    <div className="int-page">
      <Link to="/integrations/reference-tables" className="rt-back">
        <ArrowLeft size={14} /> Reference Tables
      </Link>
      <div className="settings-title-row">
        <h1 className="settings-title mono">{table.name}</h1>
        {canEdit && (
          <div className="pl-title-actions">
            <button className="btn" onClick={() => setReplacing(true)}>
              <Upload size={14} /> Upload New File
            </button>
            <button className="btn" onClick={confirmDelete} disabled={remove.isPending}>
              <Trash2 size={14} /> Delete
            </button>
          </div>
        )}
      </div>
      {table.description && <p>{table.description}</p>}
      <Details table={table} />
      {remove.error && <p className="form-error">{(remove.error as Error).message}</p>}

      <h2 className="settings-subtitle">Schema</h2>
      <table className="settings-table ix-table rt-schema">
        <thead>
          <tr>
            <th>COLUMN</th>
            <th />
          </tr>
        </thead>
        <tbody>
          {table.columns.map((c) => (
            <tr key={c}>
              <td className="mono">{c}</td>
              <td>{c === table.primaryKey && <span className="tag-chip rt-pk">primary key</span>}</td>
            </tr>
          ))}
        </tbody>
      </table>

      <h2 className="settings-subtitle">
        Rows <span className="muted">{total.toLocaleString()}</span>
      </h2>
      <div className="rt-toolbar">
        <label className="settings-search wide">
          <Search size={16} />
          <input placeholder={column ? `Search ${column}` : `Search by ${table.primaryKey} or any value`} value={search} onChange={(e) => setSearch(e.target.value)} />
        </label>
        <select className="rt-column" value={column} onChange={(e) => setColumn(e.target.value)} aria-label="Column to search">
          <option value="">All columns</option>
          {table.columns.map((c) => (
            <option key={c} value={c}>
              {c}
            </option>
          ))}
        </select>
      </div>
      {q.isError && <p className="form-error">{(q.error as Error).message}</p>}
      {rows.length === 0 ? (
        <section className="settings-card settings-empty">
          <p className="muted">No row matches the search.</p>
        </section>
      ) : (
        <div className="rt-rows">
          <table className="settings-table">
            <thead>
              <tr>
                {table.columns.map((c, i) => (
                  <th key={c} className={i === pk ? 'rt-pk-col' : undefined}>
                    {c}
                  </th>
                ))}
              </tr>
            </thead>
            <tbody>
              {rows.map((r) => (
                <tr key={r[pk]}>
                  {r.map((v, i) => (
                    <td key={i} className={i === pk ? 'rt-pk-col mono' : 'mono'}>
                      {v}
                    </td>
                  ))}
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
      <Pager offset={offset} limit={PAGE} total={total} onChange={setOffset} />
      {replacing && <ReplaceModal table={table} onClose={() => setReplacing(false)} />}
    </div>
  );
}

function Details({ table }: { table: ReferenceTable }) {
  return (
    <dl className="rt-details">
      <div>
        <dt>Source</dt>
        <dd>
          {SOURCE_LABELS[table.source] ?? table.source}
          {table.fileName && <span className="muted"> · {table.fileName}</span>}
        </dd>
      </div>
      <div>
        <dt>Rows</dt>
        <dd>{table.rowCount.toLocaleString()}</dd>
      </div>
      <div>
        <dt>Last updated</dt>
        <dd title={formatLongDate(table.updatedAt)}>
          {timeAgo(table.updatedAt)}
          {table.updatedBy && <span className="muted"> by {table.updatedBy}</span>}
        </dd>
      </div>
      <div>
        <dt>Created</dt>
        <dd title={formatLongDate(table.createdAt)}>
          {timeAgo(table.createdAt)}
          {table.createdBy && <span className="muted"> by {table.createdBy}</span>}
        </dd>
      </div>
    </dl>
  );
}

function ReplaceModal({ table, onClose }: { table: ReferenceTable; onClose: () => void }) {
  const { replace } = useReferenceTableMutations();
  const { file, pick } = useCsvFile();
  const fileError = file?.error ?? (file?.parsed ? checkFile({ ...file.parsed, primaryKey: table.primaryKey }) : null);
  const ready = Boolean(file?.parsed && !fileError);
  const submit = () => file?.parsed && replace.mutate({ name: table.name, file: { fileName: file.fileName, columns: file.parsed.columns, rows: file.parsed.rows } }, { onSuccess: onClose });
  return (
    <Modal onClose={onClose} label="Upload a new file" width={760} height="auto">
      <form
        className="small-modal ar-form"
        onSubmit={(e) => {
          e.preventDefault();
          if (ready) submit();
        }}
      >
        <div className="modal-title-row">
          <h3>Upload a New File to {table.name}</h3>
          <button type="button" className="icon-plain" onClick={onClose} title="Close">
            <X size={18} />
          </button>
        </div>
        <p className="muted">
          The file&apos;s rows replace all {table.rowCount.toLocaleString()} current rows. It must have the primary key column <code>{table.primaryKey}</code>; its other columns become the
          table&apos;s new schema.
        </p>
        <CsvFileField file={file} onPick={pick} />
        {file?.parsed && <CsvPreview parsed={file.parsed} primaryKey={table.primaryKey} />}
        {fileError && <p className="form-error">{fileError}</p>}
        {replace.error && <p className="form-error">{(replace.error as Error).message}</p>}
        <div className="small-modal-actions">
          <button type="button" className="btn" onClick={onClose}>
            Cancel
          </button>
          <button className="btn btn-primary" disabled={!ready || replace.isPending}>
            {replace.isPending ? 'Uploading…' : 'Replace Rows'}
          </button>
        </div>
      </form>
    </Modal>
  );
}
