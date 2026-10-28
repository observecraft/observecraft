// Formulas between metric queries in the explorer ("a / b * 100"). The API evaluates the expanded formula
// (server: routes/monitor_formula.rs); the in-browser mock uses evalFormula.

/**
 * The formula with each query letter replaced by its query in parentheses, or null when it names an
 * unknown letter or holds anything but letters, numbers, + - * / and parentheses.
 */
export function expandFormula(formula: string, queries: Record<string, string>): string | null {
  const tokens = formula.match(/[a-z]+|\d+(?:\.\d+)?|[-+*/()]|\S/gi) ?? [];
  if (tokens.length === 0) return null;
  const out: string[] = [];
  for (const t of tokens) {
    if (/^[a-z]+$/i.test(t)) {
      const q = queries[t.toLowerCase()];
      if (!q) return null;
      out.push(`(${q})`);
    } else if (/^(\d+(\.\d+)?|[-+*/()])$/.test(t)) out.push(t);
    else return null;
  }
  return out.join(' ').replace(/\( /g, '(').replace(/ \)/g, ')');
}

/**
 * Evaluates a formula whose queries are written Q0, Q1, … with their values at one point; null when a
 * value is missing, a division is by zero or the formula doesn't parse.
 */
export function evalFormula(formula: string, values: (number | null)[]): number | null {
  const tokens = formula.match(/Q\d+|\d+(?:\.\d+)?|[-+*/()]/g) ?? [];
  let i = 0;
  const factor = (): number | null => {
    const t = tokens[i++];
    if (t === '(') {
      const v = expr();
      return tokens[i++] === ')' ? v : null;
    }
    if (t?.startsWith('Q')) return values[Number(t.slice(1))] ?? null;
    return t !== undefined && /^\d/.test(t) ? Number(t) : null;
  };
  const term = (): number | null => {
    let v = factor();
    while (tokens[i] === '*' || tokens[i] === '/') {
      const op = tokens[i++];
      const r = factor();
      v = v === null || r === null ? null : op === '*' ? v * r : r === 0 ? null : v / r;
    }
    return v;
  };
  const expr = (): number | null => {
    let v = term();
    while (tokens[i] === '+' || tokens[i] === '-') {
      const op = tokens[i++];
      const r = term();
      v = v === null || r === null ? null : op === '+' ? v + r : v - r;
    }
    return v;
  };
  const v = expr();
  return i === tokens.length && v !== null && Number.isFinite(v) ? v : null;
}
