import { useState } from 'react';
import { ChevronDown, ChevronRight } from 'lucide-react';
import type { JsonValue } from '../../api/types';
import './jsonTree.css';

interface Props {
  data: Record<string, JsonValue>;
  filter?: string;
  /** Clicking a scalar value; receives `@dotted.path` and the value. */
  onFilter?: (path: string, value: string) => void;
  /** Objects deeper than this start collapsed. Arrays always start collapsed. */
  expandDepth?: number;
}

function matches(key: string, value: JsonValue, filter: string): boolean {
  if (!filter) return true;
  const f = filter.toLowerCase();
  if (key.toLowerCase().includes(f)) return true;
  if (value !== null && typeof value === 'object') {
    return Object.entries(value).some(([k, v]) => matches(k, v as JsonValue, filter));
  }
  return String(value).toLowerCase().includes(f);
}

interface NodeProps {
  name: string;
  value: JsonValue;
  path: string;
  depth: number;
  filter: string;
  expandDepth: number;
  onFilter?: Props['onFilter'];
}

function Node({ name, value, path, depth, filter, expandDepth, onFilter }: NodeProps) {
  const isArray = Array.isArray(value);
  const [open, setOpen] = useState(!isArray && depth < expandDepth);

  if (value !== null && typeof value === 'object') {
    const entries = isArray ? value.map((v, i) => [String(i), v] as const) : Object.entries(value);
    const [o, c] = isArray ? ['[', ']'] : ['{', '}'];
    // An active filter forces matching branches open.
    const expanded = open || (!!filter && entries.some(([k, v]) => matches(k, v, filter)));
    return (
      <div className="jt-node">
        <button className="jt-toggle" onClick={() => setOpen((x) => !x)}>
          {expanded ? <ChevronDown size={13} /> : <ChevronRight size={13} />}
          <span className="jt-key">{name}</span>
          <span className="jt-brace">{expanded ? o : `${o}…${c}`}</span>
        </button>
        {expanded && (
          <>
            <div className="jt-children">
              {entries
                .filter(([k, v]) => matches(k, v, filter))
                .map(([k, v]) => (
                  <Node key={k} name={k} value={v} path={`${path}.${k}`} depth={depth + 1} filter={filter} expandDepth={expandDepth} onFilter={onFilter} />
                ))}
            </div>
            <span className="jt-brace jt-close">{c}</span>
          </>
        )}
      </div>
    );
  }

  const kind = typeof value === 'string' ? 'str' : 'lit';
  return (
    <div className="jt-row">
      <span className="jt-key">{name}</span>
      {onFilter ? (
        <button className={`jt-value jt-${kind}`} title={`Filter by @${path}:${String(value)}`} onClick={() => onFilter(`@${path}`, String(value))}>
          {String(value)}
        </button>
      ) : (
        <span className={`jt-value jt-${kind}`}>{String(value)}</span>
      )}
    </div>
  );
}

/** Collapsible JSON view of event attributes; clicking a value can filter by it. */
export function JsonTree({ data, filter = '', onFilter, expandDepth = 3 }: Props) {
  return (
    <div className="json-tree mono">
      <span className="jt-brace">{'{'}</span>
      <div className="jt-children">
        {Object.entries(data)
          .filter(([k, v]) => matches(k, v, filter))
          .map(([k, v]) => (
            <Node key={k} name={k} value={v} path={k} depth={0} filter={filter} expandDepth={expandDepth} onFilter={onFilter} />
          ))}
      </div>
      <span className="jt-brace">{'}'}</span>
    </div>
  );
}
