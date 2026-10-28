// Reference table files: the browser parses the uploaded CSV and checks it before sending its header
// and rows. The checks and their messages follow the server (routes/reference_tables.rs), which
// checks again; the mock uses them too.

export const MAX_NAME_CHARS = 100;
export const MAX_DESCRIPTION_CHARS = 500;
export const MAX_COLUMNS = 50;
export const MAX_ROWS = 100_000;
export const MAX_COLUMN_CHARS = 100;
/** The uploaded file's size, and on the server every value together. */
export const MAX_FILE_BYTES = 10 * 1024 * 1024;

export interface ParsedCsv {
  /** The first line, trimmed. */
  columns: string[];
  /** Every later non-blank line, values as written (quotes removed). */
  rows: string[][];
}

/** Parses comma-separated values (RFC 4180: quoted values may hold commas, quotes as `""` and line breaks). */
export function parseCsv(text: string): ParsedCsv {
  const src = text.startsWith('﻿') ? text.slice(1) : text;
  const records: string[][] = [];
  let record: string[] = [];
  let value = '';
  let line = 1;
  let i = 0;
  /** Whether the current record has anything yet (blank lines are skipped). */
  let started = false;
  const endValue = () => {
    record.push(value);
    value = '';
  };
  const endRecord = () => {
    if (started) {
      endValue();
      records.push(record);
    }
    record = [];
    value = '';
    started = false;
  };
  while (i < src.length) {
    const c = src[i];
    if (c === '"' && value === '') {
      // A quoted value: up to the closing quote, `""` being a quote.
      const opened = line;
      started = true;
      i++;
      for (;;) {
        if (i >= src.length) throw new Error(`Line ${opened}: a quoted value is not closed`);
        const q = src[i];
        if (q === '"') {
          if (src[i + 1] === '"') {
            value += '"';
            i += 2;
            continue;
          }
          i++;
          break;
        }
        if (q === '\n') line++;
        value += q;
        i++;
      }
      const next = src[i];
      if (next !== undefined && next !== ',' && next !== '\n' && next !== '\r') throw new Error(`Line ${line}: unexpected character after a closing quote`);
      if (next === ',') {
        endValue();
        i++;
      }
      continue;
    }
    if (c === ',') {
      started = true;
      endValue();
      i++;
    } else if (c === '\r' || c === '\n') {
      endRecord();
      i += c === '\r' && src[i + 1] === '\n' ? 2 : 1;
      line++;
    } else {
      started = true;
      value += c;
      i++;
    }
  }
  endRecord();
  const [header = [], ...rows] = records;
  return { columns: header.map((c) => c.trim()), rows };
}

export interface FileCheck {
  columns: string[];
  rows: string[][];
  primaryKey: string;
}

/** Why the server would reject the file, or null. Column names are expected trimmed (parseCsv does). */
export function checkFile({ columns, rows, primaryKey }: FileCheck): string | null {
  if (columns.length === 0) return 'The file has no columns: its first line must name them';
  if (columns.length > MAX_COLUMNS) return `A reference table has at most ${MAX_COLUMNS} columns`;
  const seen = new Set<string>();
  for (const [i, raw] of columns.entries()) {
    const c = raw.trim();
    if (!c) return `Column ${i + 1} has no name`;
    if ([...c].length > MAX_COLUMN_CHARS) return `Column names are at most ${MAX_COLUMN_CHARS} characters`;
    if (seen.has(c)) return `Column '${c}' appears twice`;
    seen.add(c);
  }
  const pk = columns.findIndex((c) => c.trim() === primaryKey);
  if (pk < 0) return `Primary key '${primaryKey}' is not a column of the file`;
  if (rows.length === 0) return 'The file has no rows after its header';
  if (rows.length > MAX_ROWS) return 'A reference table has at most 100,000 rows';
  const keys = new Map<string, number>();
  for (const [i, row] of rows.entries()) {
    if (row.length !== columns.length) return `Row ${i + 1} has ${row.length} values, expected ${columns.length}`;
    const key = row[pk];
    if (!key.trim()) return `Row ${i + 1} has an empty primary key (${primaryKey})`;
    const first = keys.get(key);
    if (first !== undefined) return `Primary key value '${key}' appears twice (rows ${first + 1} and ${i + 1})`;
    keys.set(key, i);
  }
  return null;
}

/** Why the server would reject the table name, or null. */
export function checkName(name: string): string | null {
  return name.length >= 1 && name.length <= MAX_NAME_CHARS && /^[a-z0-9_]+$/.test(name) ? null : `The name must be 1–${MAX_NAME_CHARS} characters: lowercase letters, digits and underscores`;
}
