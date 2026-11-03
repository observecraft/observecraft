// A line diff (longest common subsequence) for showing a proposed change against the file as it was.
// Files of a few thousand lines at most; larger ones fall back to "all removed, all added".

export interface DiffLine {
  kind: 'keep' | 'add' | 'remove';
  text: string;
  /** 1-based line in the old file (none for added lines). */
  oldLine: number | null;
  /** 1-based line in the new file (none for removed lines). */
  newLine: number | null;
}

const MAX_CELLS = 4_000_000;

function lines(s: string): string[] {
  if (!s) return [];
  const out = s.split('\n');
  if (out[out.length - 1] === '') out.pop();
  return out;
}

export function lineDiff(before: string, after: string): DiffLine[] {
  const a = lines(before);
  const b = lines(after);
  const out: DiffLine[] = [];
  if (a.length * b.length > MAX_CELLS) {
    a.forEach((text, i) => out.push({ kind: 'remove', text, oldLine: i + 1, newLine: null }));
    b.forEach((text, i) => out.push({ kind: 'add', text, oldLine: null, newLine: i + 1 }));
    return out;
  }
  // lcs[i][j]: common lines of a[i..] and b[j..].
  const w = b.length + 1;
  const lcs = new Uint32Array((a.length + 1) * w);
  for (let i = a.length - 1; i >= 0; i--) {
    for (let j = b.length - 1; j >= 0; j--) {
      lcs[i * w + j] = a[i] === b[j] ? lcs[(i + 1) * w + j + 1] + 1 : Math.max(lcs[(i + 1) * w + j], lcs[i * w + j + 1]);
    }
  }
  let i = 0;
  let j = 0;
  while (i < a.length || j < b.length) {
    if (i < a.length && j < b.length && a[i] === b[j]) {
      out.push({ kind: 'keep', text: a[i], oldLine: i + 1, newLine: j + 1 });
      i++;
      j++;
    } else if (j < b.length && (i === a.length || lcs[i * w + j + 1] >= lcs[(i + 1) * w + j])) {
      // Removals before additions at the same point read better: take the removal when it keeps as much.
      if (i < a.length && lcs[(i + 1) * w + j] >= lcs[i * w + j + 1]) {
        out.push({ kind: 'remove', text: a[i], oldLine: i + 1, newLine: null });
        i++;
      } else {
        out.push({ kind: 'add', text: b[j], oldLine: null, newLine: j + 1 });
        j++;
      }
    } else {
      out.push({ kind: 'remove', text: a[i], oldLine: i + 1, newLine: null });
      i++;
    }
  }
  return out;
}
