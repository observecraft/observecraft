import { useState } from 'react';
import { FileUp } from 'lucide-react';
import { formatBytes } from '../../components/ui/Pager';
import { MAX_FILE_BYTES, parseCsv, type ParsedCsv } from '../../api/referenceTables/file';

const PREVIEW_ROWS = 5;

export interface CsvFile {
  fileName: string;
  size: number;
  parsed: ParsedCsv | null;
  /** Why the file couldn't be read or parsed. */
  error: string | null;
}

/** The picked CSV file, read and parsed in the browser. */
export function useCsvFile() {
  const [file, setFile] = useState<CsvFile | null>(null);
  const pick = async (f: File | undefined) => {
    if (!f) return setFile(null);
    const base = { fileName: f.name, size: f.size, parsed: null };
    if (f.size > MAX_FILE_BYTES) return setFile({ ...base, error: `The file is larger than ${formatBytes(MAX_FILE_BYTES)}` });
    try {
      setFile({ ...base, parsed: parseCsv(await f.text()), error: null });
    } catch (e) {
      setFile({ ...base, error: (e as Error).message });
    }
  };
  return { file, pick };
}

/** "Choose a CSV file" button, with the picked file's name and size. */
export function CsvFileField({ file, onPick }: { file: CsvFile | null; onPick: (f: File | undefined) => void }) {
  return (
    <div className="rt-file">
      <label className="btn">
        <FileUp size={14} /> {file ? 'Choose another file' : 'Choose a CSV file'}
        <input type="file" accept=".csv,text/csv" hidden onChange={(e) => onPick(e.target.files?.[0])} />
      </label>
      {file ? (
        <span>
          <b>{file.fileName}</b>{' '}
          <span className="muted">
            {formatBytes(file.size)}
            {file.parsed && ` · ${file.parsed.rows.length.toLocaleString()} rows · ${file.parsed.columns.length} columns`}
          </span>
        </span>
      ) : (
        <span className="muted">Comma-separated, with a header row naming the columns; at most {formatBytes(MAX_FILE_BYTES)} and 100,000 rows.</span>
      )}
    </div>
  );
}

/** The header and first rows of a parsed file, the primary key column marked. */
export function CsvPreview({ parsed, primaryKey }: { parsed: ParsedCsv; primaryKey: string }) {
  if (!parsed.columns.length) return null;
  return (
    <div className="rt-preview">
      <table className="settings-table">
        <thead>
          <tr>
            {parsed.columns.map((c, i) => (
              <th key={i} className={c === primaryKey ? 'rt-pk-col' : undefined}>
                {c}
                {c === primaryKey && <span className="tag-chip rt-pk">primary key</span>}
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {parsed.rows.slice(0, PREVIEW_ROWS).map((r, i) => (
            <tr key={i}>
              {parsed.columns.map((c, j) => (
                <td key={j} className={c === primaryKey ? 'rt-pk-col mono' : 'mono'}>
                  {r[j]}
                </td>
              ))}
            </tr>
          ))}
        </tbody>
      </table>
      {parsed.rows.length > PREVIEW_ROWS && <p className="muted">and {(parsed.rows.length - PREVIEW_ROWS).toLocaleString()} more rows</p>}
    </div>
  );
}
