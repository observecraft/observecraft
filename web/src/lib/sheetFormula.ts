// Calculated column formulas for Sheets: a tiny, safe expression language evaluated per row.
//
//   =ROUND(errors / requests * 100, 2)      IF(p99 > 500, "slow", "ok")      CONCAT(service, " / ", [p99 latency])
//
// - Numbers (`12`, `1.5e3`), text in double quotes (`"ok"`, `""` for a quote), TRUE / FALSE.
// - Columns by name (`requests`) or in brackets for any name (`[p99 latency]`).
// - Operators, loosest first: comparisons `= <> != < <= > >=`, text join `&`, `+ -`, `* / %`, unary `-`,
//   power `^` (right-associative).
// - Functions (any case): IF(cond, then, [else]), ROUND(x, [digits]), CONCAT(…), ABS(x), MIN(…), MAX(…),
//   AND(…), OR(…), NOT(x).
// Formulas are parsed into closures: nothing is ever passed to eval or Function, and column lookups only
// read the row's own properties. Empty cells count as 0 in arithmetic and as "" in text.

export type Cell = number | string | boolean | null;
export type Row = Record<string, Cell>;

export class FormulaError extends Error {}

type Node = (row: Row) => Cell;

interface Token {
  kind: 'num' | 'str' | 'ident' | 'col' | 'op' | '(' | ')' | ',';
  text: string;
  value?: number;
}

const fail = (message: string): never => {
  throw new FormulaError(message);
};

function tokenize(src: string): Token[] {
  const out: Token[] = [];
  let i = 0;
  while (i < src.length) {
    const c = src[i];
    if (/\s/.test(c)) {
      i++;
    } else if (/[0-9.]/.test(c)) {
      const m = /^(?:\d+\.?\d*|\.\d+)(?:[eE][+-]?\d+)?/.exec(src.slice(i));
      if (!m) fail(`Unexpected "${c}"`);
      out.push({ kind: 'num', text: m![0], value: Number(m![0]) });
      i += m![0].length;
    } else if (c === '"') {
      let s = '';
      let j = i + 1;
      for (;;) {
        if (j >= src.length) fail('Unclosed text: add a closing "');
        if (src[j] === '"') {
          if (src[j + 1] === '"') {
            s += '"';
            j += 2;
            continue;
          }
          break;
        }
        s += src[j++];
      }
      out.push({ kind: 'str', text: s });
      i = j + 1;
    } else if (c === '[') {
      const j = src.indexOf(']', i);
      if (j < 0) fail('Unclosed column name: add a closing ]');
      out.push({ kind: 'col', text: src.slice(i + 1, j).trim() });
      i = j + 1;
    } else if (/[A-Za-z_]/.test(c)) {
      const m = /^[A-Za-z_][A-Za-z0-9_]*/.exec(src.slice(i))!;
      out.push({ kind: 'ident', text: m[0] });
      i += m[0].length;
    } else {
      const two = src.slice(i, i + 2);
      if (['<=', '>=', '<>', '!='].includes(two)) {
        out.push({ kind: 'op', text: two });
        i += 2;
      } else if ('+-*/%^&=<>'.includes(c)) {
        out.push({ kind: 'op', text: c });
        i++;
      } else if (c === '(' || c === ')' || c === ',') {
        out.push({ kind: c, text: c });
        i++;
      } else {
        fail(`Unexpected "${c}"`);
      }
    }
  }
  return out;
}

// ---------------------------------------------------------------- values

function num(v: Cell, what = 'This operation'): number {
  if (v === null || v === '') return 0;
  if (typeof v === 'number') return v;
  if (typeof v === 'boolean') return v ? 1 : 0;
  const n = Number(v);
  if (Number.isFinite(n) && v.trim() !== '') return n;
  return fail(`${what} needs a number, not "${v}"`);
}

const text = (v: Cell): string => (v === null ? '' : typeof v === 'boolean' ? (v ? 'TRUE' : 'FALSE') : String(v));

const truthy = (v: Cell): boolean => (typeof v === 'string' ? v !== '' : Boolean(v));

const isNumeric = (v: Cell) => v === null || typeof v === 'number' || typeof v === 'boolean' || (v.trim() !== '' && Number.isFinite(Number(v)));

function compare(a: Cell, b: Cell): number {
  if (isNumeric(a) && isNumeric(b)) return num(a) - num(b);
  return text(a).localeCompare(text(b));
}

function round(x: number, digits: number): number {
  const f = 10 ** Math.trunc(digits);
  const r = (Math.sign(x) * Math.round(Math.abs(x) * f + Number.EPSILON)) / f;
  return Object.is(r, -0) ? 0 : r;
}

const FUNCTIONS: Record<string, { min: number; max: number; call: (args: Node[], row: Row) => Cell }> = {
  IF: { min: 2, max: 3, call: (a, row) => (truthy(a[0](row)) ? a[1](row) : a[2] ? a[2](row) : null) },
  ROUND: { min: 1, max: 2, call: (a, row) => round(num(a[0](row), 'ROUND'), a[1] ? num(a[1](row), 'ROUND') : 0) },
  CONCAT: { min: 1, max: Infinity, call: (a, row) => a.map((f) => text(f(row))).join('') },
  ABS: { min: 1, max: 1, call: (a, row) => Math.abs(num(a[0](row), 'ABS')) },
  MIN: { min: 1, max: Infinity, call: (a, row) => Math.min(...a.map((f) => num(f(row), 'MIN'))) },
  MAX: { min: 1, max: Infinity, call: (a, row) => Math.max(...a.map((f) => num(f(row), 'MAX'))) },
  AND: { min: 1, max: Infinity, call: (a, row) => a.every((f) => truthy(f(row))) },
  OR: { min: 1, max: Infinity, call: (a, row) => a.some((f) => truthy(f(row))) },
  NOT: { min: 1, max: 1, call: (a, row) => !truthy(a[0](row)) },
};

const BINARY: Record<string, (a: Cell, b: Cell) => Cell> = {
  '+': (a, b) => num(a) + num(b),
  '-': (a, b) => num(a) - num(b),
  '*': (a, b) => num(a) * num(b),
  '/': (a, b) => {
    const d = num(b);
    return d === 0 ? fail('Division by zero') : num(a) / d;
  },
  '%': (a, b) => {
    const d = num(b);
    return d === 0 ? fail('Division by zero') : num(a) % d;
  },
  '^': (a, b) => num(a) ** num(b),
  '&': (a, b) => text(a) + text(b),
  '=': (a, b) => compare(a, b) === 0,
  '<>': (a, b) => compare(a, b) !== 0,
  '!=': (a, b) => compare(a, b) !== 0,
  '<': (a, b) => compare(a, b) < 0,
  '<=': (a, b) => compare(a, b) <= 0,
  '>': (a, b) => compare(a, b) > 0,
  '>=': (a, b) => compare(a, b) >= 0,
};

const own = (row: Row, key: string): Cell => (Object.prototype.hasOwnProperty.call(row, key) ? (row[key] ?? null) : null);

// ---------------------------------------------------------------- parser

/** Parses a formula against the sheet's column names; the result evaluates it for one row. */
export function compileFormula(src: string, columns: string[]): (row: Row) => Cell {
  const known = new Set(columns);
  const tokens = tokenize(src.trim().replace(/^=/, ''));
  if (tokens.length === 0) fail('The formula is empty');
  let pos = 0;
  const peek = () => tokens[pos];
  const isOp = (...ops: string[]) => peek()?.kind === 'op' && ops.includes(peek().text);
  const expect = (kind: Token['kind']) => {
    if (peek()?.kind !== kind) fail(peek() ? `Unexpected "${peek().text}"` : `Missing "${kind}" at the end`);
    return tokens[pos++];
  };

  const level = (ops: string[], next: () => Node) => (): Node => {
    let left = next();
    while (isOp(...ops)) {
      const f = BINARY[tokens[pos++].text];
      const l = left;
      const r = next();
      left = (row) => f(l(row), r(row));
    }
    return left;
  };

  const column = (name: string): Node => {
    if (!known.has(name)) fail(`Unknown column ${name}`);
    return (row) => own(row, name);
  };

  const primary = (): Node => {
    const t = peek();
    if (!t) return fail('The formula ends too early');
    pos++;
    switch (t.kind) {
      case 'num': {
        const v = t.value!;
        return () => v;
      }
      case 'str': {
        const v = t.text;
        return () => v;
      }
      case 'col':
        return column(t.text);
      case '(': {
        const inner = comparison();
        expect(')');
        return inner;
      }
      case 'ident': {
        const upper = t.text.toUpperCase();
        if (peek()?.kind === '(') {
          const fn = Object.prototype.hasOwnProperty.call(FUNCTIONS, upper) ? FUNCTIONS[upper] : undefined;
          if (!fn) fail(`Unknown function ${t.text} (use ${Object.keys(FUNCTIONS).join(', ')})`);
          pos++;
          const args: Node[] = [];
          if (peek()?.kind !== ')') {
            args.push(comparison());
            while (peek()?.kind === ',') {
              pos++;
              args.push(comparison());
            }
          }
          expect(')');
          if (args.length < fn!.min || args.length > fn!.max)
            fail(`${upper} takes ${fn!.min === fn!.max ? fn!.min : fn!.max === Infinity ? `at least ${fn!.min}` : `${fn!.min} or ${fn!.max}`} argument${fn!.max === 1 ? '' : 's'}`);
          return (row) => fn!.call(args, row);
        }
        if (upper === 'TRUE') return () => true;
        if (upper === 'FALSE') return () => false;
        return column(t.text);
      }
      default:
        return fail(`Unexpected "${t.text}"`);
    }
  };

  // power := primary ('^' unary)?  (right-associative; `-2 ^ 2` is -(2 ^ 2))
  const power = (): Node => {
    const base = primary();
    if (!isOp('^')) return base;
    pos++;
    const exp = unary();
    return (row) => BINARY['^'](base(row), exp(row));
  };
  const unary = (): Node => {
    if (isOp('-')) {
      pos++;
      const inner = unary();
      return (row) => -num(inner(row));
    }
    if (isOp('+')) {
      pos++;
      const inner = unary();
      return (row) => num(inner(row));
    }
    return power();
  };
  const multiplicative = level(['*', '/', '%'], unary);
  const additive = level(['+', '-'], multiplicative);
  const concat = level(['&'], additive);
  const comparison = level(['=', '<>', '!=', '<', '<=', '>', '>='], concat);

  const root = comparison();
  if (pos < tokens.length) fail(`Unexpected "${tokens[pos].text}"`);
  return (row) => {
    const v = root(row);
    if (typeof v === 'number' && !Number.isFinite(v)) fail('The result isn’t a finite number');
    return v;
  };
}
