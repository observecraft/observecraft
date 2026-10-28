import { useState, type ReactNode } from 'react';
import { Link, useNavigate } from 'react-router-dom';
import { ChevronDown, ChevronRight, FileText, Info, Table2, TriangleAlert } from 'lucide-react';
import { checkFile, checkName, MAX_DESCRIPTION_CHARS, MAX_NAME_CHARS } from '../../api/referenceTables/file';
import { useReferenceTableMutations } from './queries';
import { CsvDropzone, CsvPreview, useCsvFile } from './ReferenceTableUpload';
import '../settings/settings.css';
import './integrations.css';

/** One collapsible step of the form. */
function Step({ n, title, open, onToggle, done, children }: { n: number; title: string; open: boolean; onToggle: () => void; done?: boolean; children: ReactNode }) {
  return (
    <section className={`rt-step${open ? ' open' : ''}${done ? ' done' : ''}`}>
      <button type="button" className="rt-step-head" onClick={onToggle}>
        <span className="rt-step-num">{n}</span>
        {open ? <ChevronDown size={18} /> : <ChevronRight size={18} />}
        {title}
      </button>
      {open && <div className="rt-step-body">{children}</div>}
    </section>
  );
}

/** Integrations → Reference Tables → New: data source, table metadata, then the preview and primary key. */
export function ReferenceTableCreatePage() {
  const navigate = useNavigate();
  const { create } = useReferenceTableMutations();
  const { file, pick } = useCsvFile();
  const [source, setSource] = useState<'csv' | null>(null);
  const [name, setName] = useState('');
  const [description, setDescription] = useState('');
  const [chosenKey, setChosenKey] = useState('');
  const [open, setOpen] = useState<Record<number, boolean>>({ 1: true });
  const toggle = (n: number) => setOpen((o) => ({ ...o, [n]: !o[n] }));
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
  // A parsed file opens the metadata and preview steps.
  const onPick = async (f: File | undefined) => {
    await pick(f);
    setOpen((o) => ({ ...o, 2: true, 3: true }));
  };

  return (
    <div className="rt-create-page">
      <header className="product-header">
        <div className="product-title rt-crumb">
          <Table2 size={20} />
          <Link to="/integrations/reference-tables">Reference Tables</Link>
          <ChevronRight size={18} className="rt-crumb-sep" />
          <span>New</span>
        </div>
      </header>
      <form
        className="rt-create"
        onSubmit={(e) => {
          e.preventDefault();
          if (ready) submit();
        }}
      >
        <Step n={1} title="Select a data source" open={!!open[1]} onToggle={() => toggle(1)} done={Boolean(file?.parsed)}>
          <p>Upload a CSV file. It is read and checked in your browser before anything is saved.</p>
          <div className="rt-sources">
            <button type="button" className={`rt-source${source === 'csv' ? ' selected' : ''}`} onClick={() => setSource('csv')}>
              <FileText size={36} strokeWidth={1.4} />
              Local CSV
            </button>
          </div>
          <div className="rt-warning">
            <TriangleAlert size={16} />
            <span>Everyone in your organization can view the tables uploaded here. Before uploading personal or sensitive data, check that your organization&apos;s policies allow it.</span>
          </div>
          {source === 'csv' && (
            <>
              <label className="rt-field">
                <span>
                  Table name <span className="rt-required">*</span>
                </span>
                <input value={name} onChange={(e) => setName(e.target.value)} placeholder="Name your reference table, e.g. service_owners" maxLength={MAX_NAME_CHARS} autoFocus />
                {nameError ? <span className="form-error">{nameError}</span> : <span className="muted">Lowercase letters, digits and underscores. It can&apos;t be changed later.</span>}
              </label>
              <CsvDropzone file={file} onPick={onPick} />
              {file?.error && <p className="form-error">{file.error}</p>}
            </>
          )}
        </Step>
        <Step n={2} title="Table metadata" open={!!open[2]} onToggle={() => toggle(2)} done={Boolean(description)}>
          <label className="rt-field">
            <span>Description</span>
            <input value={description} onChange={(e) => setDescription(e.target.value)} placeholder="What the table holds and where it comes from" maxLength={MAX_DESCRIPTION_CHARS} />
          </label>
        </Step>
        <Step n={3} title="Preview table and select primary key" open={!!open[3]} onToggle={() => toggle(3)}>
          {file?.parsed ? (
            <>
              <label className="rt-field">
                <span>Primary key</span>
                <select value={primaryKey} onChange={(e) => setChosenKey(e.target.value)}>
                  {columns.map((c, i) => (
                    <option key={i} value={c}>
                      {c || `(column ${i + 1} has no name)`}
                    </option>
                  ))}
                </select>
                <span className="muted">The column whose values identify rows: unique and never empty.</span>
              </label>
              <CsvPreview parsed={file.parsed} primaryKey={primaryKey} />
              {fileError && <p className="form-error">{fileError}</p>}
            </>
          ) : (
            <p className="muted">Upload a file to preview it here.</p>
          )}
        </Step>
        {create.error && <p className="form-error">{(create.error as Error).message}</p>}
        <div className="rt-create-actions">
          <Link className="btn" to="/integrations/reference-tables">
            Cancel
          </Link>
          <button className="btn btn-primary" disabled={!ready || create.isPending}>
            {create.isPending ? 'Saving…' : 'Save'}
          </button>
          <span className="rt-note">
            <Info size={14} /> Uploading a new file later replaces every row of the table.
          </span>
        </div>
      </form>
    </div>
  );
}
