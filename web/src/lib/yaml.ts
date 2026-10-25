// JSON → YAML for display (the Kubernetes Explorer's YAML tab). Block style like `kubectl get -o yaml`.

const PLAIN = /^[A-Za-z0-9_./][A-Za-z0-9_./ :@+-]*$/;
const RESERVED = /^(true|false|null|yes|no|on|off|~|-?\d+(\.\d+)?([eE][+-]?\d+)?)$/i;

function scalar(v: string): string {
  if (v === '') return "''";
  if (PLAIN.test(v) && !RESERVED.test(v) && !v.endsWith(':') && !v.includes(': ') && !v.endsWith(' ')) return v;
  return JSON.stringify(v);
}

function key(k: string): string {
  return PLAIN.test(k) && !k.includes(': ') ? k : JSON.stringify(k);
}

function inline(v: unknown): string | null {
  if (v === null || v === undefined) return 'null';
  if (typeof v === 'string') return v.includes('\n') ? null : scalar(v);
  if (typeof v === 'number' || typeof v === 'boolean') return String(v);
  if (Array.isArray(v) && v.length === 0) return '[]';
  if (typeof v === 'object' && Object.keys(v as object).length === 0) return '{}';
  return null;
}

function block(v: unknown, pad: string): string[] {
  if (typeof v === 'string')
    return [
      '|',
      ...v
        .replace(/\n$/, '')
        .split('\n')
        .map((l) => pad + l),
    ];
  const out: string[] = [];
  if (Array.isArray(v)) {
    for (const item of v) {
      const one = inline(item);
      if (one !== null) out.push(`${pad}- ${one}`);
      else if (typeof item === 'string') out.push(`${pad}- ${block(item, pad + '  ').join('\n')}`);
      else {
        // First key on the dash line, the rest aligned under it.
        const lines = block(item, pad + '  ');
        out.push(`${pad}- ${lines[0].slice(pad.length + 2)}`, ...lines.slice(1));
      }
    }
    return out;
  }
  for (const [k, val] of Object.entries(v as Record<string, unknown>)) {
    const one = inline(val);
    if (one !== null) out.push(`${pad}${key(k)}: ${one}`);
    else if (typeof val === 'string') out.push(`${pad}${key(k)}: ${block(val, pad + '  ').join('\n')}`);
    else out.push(`${pad}${key(k)}:`, ...block(val, Array.isArray(val) ? pad : pad + '  '));
  }
  return out;
}

export function toYaml(v: unknown): string {
  const one = inline(v);
  return one !== null ? one : block(v, '').join('\n');
}
