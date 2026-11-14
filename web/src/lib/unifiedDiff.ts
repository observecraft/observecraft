// A proposed change as a unified diff (every line, no hunks), to copy when the change can't be opened as a merge
// request: `git apply` and patch tools take it as it is.
import { lineDiff } from './lineDiff';

interface ChangedFile {
  path: string;
  before?: string | null;
  after: string;
}

export function unifiedDiff(change: { files: ChangedFile[] }): string {
  let out = '';
  for (const f of change.files) {
    out += `${f.before == null ? '--- /dev/null' : `--- a/${f.path}`}\n+++ b/${f.path}\n`;
    for (const l of lineDiff(f.before ?? '', f.after)) out += `${l.kind === 'add' ? '+' : l.kind === 'remove' ? '-' : ' '}${l.text}\n`;
  }
  return out;
}
