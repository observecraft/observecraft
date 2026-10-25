import { useEffect, useRef, useState, type KeyboardEvent, type ReactNode } from 'react';
import { Code2, ExternalLink, Search, X } from 'lucide-react';
import type { QuerySuggestion } from '../../api/types';
import { compact } from '../../lib/format';
import { parseTerm, tokenize } from '../../lib/searchQuery';
import './queryBar.css';

interface Props {
  value: string;
  onChange: (query: string) => void;
  /** Autocomplete source for the term being typed. Keep it referentially stable (useCallback). */
  suggest?: (partial: string) => Promise<QuerySuggestion[]>;
  placeholder?: string;
  /** Left-hand addon (e.g. a scope dropdown) replacing the search icon. */
  prefix?: ReactNode;
}

/** True while a token still has an open quote or parenthesis, so space shouldn't commit it. */
function isOpen(token: string): boolean {
  const quotes = (token.match(/"/g) ?? []).length;
  const depth = (token.match(/\(/g) ?? []).length - (token.match(/\)/g) ?? []).length;
  return quotes % 2 === 1 || depth > 0;
}

function Pill({ token }: { token: string }) {
  const t = parseTerm(token);
  if (t.kind === 'text') return <>{token}</>;
  const keyEnd = token.indexOf(':');
  return (
    <>
      <span className={t.negate ? 'pill-neg' : undefined}>{token.slice(0, keyEnd)}</span>
      <span className="pill-value">{token.slice(keyEnd)}</span>
    </>
  );
}

/**
 * Tokenised search bar: committed terms render as pills; the trailing input edits the
 * current term with key/value autocomplete. The `</>` toggle switches to raw text editing.
 */
export function QueryBar({ value, onChange, suggest, placeholder = 'Search…', prefix }: Props) {
  const [tokens, setTokens] = useState<string[]>(() => tokenize(value));
  const [draft, setDraft] = useState('');
  const [focused, setFocused] = useState(false);
  const [raw, setRaw] = useState(false);
  const [rawText, setRawText] = useState(value);
  const [suggestions, setSuggestions] = useState<QuerySuggestion[]>([]);
  const [active, setActive] = useState(-1);
  const input = useRef<HTMLInputElement>(null);
  const bar = useRef<HTMLDivElement>(null);

  // Resync when the query changes from outside (facet clicks, insight cards, tag filters).
  useEffect(() => {
    if (tokens.join(' ') !== value) setTokens(tokenize(value));
    setRawText(value);
  }, [value]);

  useEffect(() => {
    if (!focused || !suggest) return;
    let cancelled = false;
    const id = setTimeout(() => {
      suggest(draft.trim()).then((s) => {
        if (!cancelled) {
          setSuggestions(s);
          setActive(-1);
        }
      });
    }, 80);
    return () => {
      cancelled = true;
      clearTimeout(id);
    };
  }, [draft, focused, suggest]);

  const commit = (next: string[]) => {
    setTokens(next);
    onChange(next.join(' '));
  };

  const commitDraft = () => {
    const d = draft.trim();
    if (!d) return;
    setDraft('');
    commit([...tokens, ...tokenize(d)]);
  };

  const apply = (s: QuerySuggestion) => {
    if (s.text.endsWith(':')) {
      setDraft(s.text);
    } else {
      setDraft('');
      commit([...tokens, s.text]);
    }
    input.current?.focus();
  };

  const edit = (i: number) => {
    const next = tokens.filter((_, j) => j !== i);
    commit(next);
    setDraft(tokens[i]);
    input.current?.focus();
  };

  const onKey = (e: KeyboardEvent<HTMLInputElement>) => {
    if (e.key === 'ArrowDown' && suggestions.length) {
      e.preventDefault();
      setActive((a) => (a + 1) % suggestions.length);
    } else if (e.key === 'ArrowUp' && suggestions.length) {
      e.preventDefault();
      setActive((a) => (a <= 0 ? suggestions.length - 1 : a - 1));
    } else if (e.key === 'Enter') {
      e.preventDefault();
      if (active >= 0 && suggestions[active]) apply(suggestions[active]);
      else commitDraft();
    } else if (e.key === ' ' && draft.trim() && !isOpen(draft)) {
      e.preventDefault();
      commitDraft();
    } else if (e.key === 'Backspace' && draft === '' && tokens.length) {
      e.preventDefault();
      edit(tokens.length - 1);
    } else if (e.key === 'Escape') {
      input.current?.blur();
    }
  };

  // Anchor the dropdown under the caret area, like the reference UI.
  const dropdownLeft = Math.min(input.current?.offsetLeft ?? 0, (bar.current?.clientWidth ?? 800) - 660);

  return (
    <div className={`query-bar${focused ? ' focused' : ''}`}>
      {prefix ?? (
        <span className="query-bar-icon">
          <Search size={15} />
        </span>
      )}
      {raw ? (
        <input
          className="query-raw mono"
          value={rawText}
          autoFocus
          onChange={(e) => setRawText(e.target.value)}
          onKeyDown={(e) => e.key === 'Enter' && onChange(rawText.trim())}
          onBlur={() => onChange(rawText.trim())}
        />
      ) : (
        <div className="query-tokens" ref={bar} onClick={() => input.current?.focus()}>
          {tokens.map((t, i) => (
            <span className="pill mono" key={`${i}-${t}`} onClick={(e) => (e.stopPropagation(), edit(i))}>
              <Pill token={t} />
            </span>
          ))}
          <input
            ref={input}
            className="query-input-field mono"
            value={draft}
            placeholder={tokens.length ? '' : placeholder}
            onChange={(e) => setDraft(e.target.value)}
            onKeyDown={onKey}
            onFocus={() => setFocused(true)}
            onBlur={() => {
              setFocused(false);
              commitDraft();
            }}
          />
          {focused && suggestions.length > 0 && (
            <div className="suggest" style={{ left: Math.max(0, dropdownLeft) }} onMouseDown={(e) => e.preventDefault()}>
              <div className="suggest-title">Suggestions</div>
              <ul>
                {suggestions.map((s, i) => (
                  <li key={s.text}>
                    <button className={i === active ? 'active' : ''} onMouseEnter={() => setActive(i)} onClick={() => apply(s)}>
                      <Search size={14} className="muted" />
                      <span className="mono suggest-text">{s.text}</span>
                      <span className="suggest-count">{compact(s.count)}</span>
                    </button>
                  </li>
                ))}
              </ul>
              <div className="suggest-footer">
                <span>
                  Wildcard <code>key:*</code>
                </span>
                <span>
                  Union <code>key:(a OR b)</code>
                </span>
                <a href="/help/search-syntax" target="_blank" rel="noreferrer">
                  Syntax Help <ExternalLink size={12} />
                </a>
              </div>
            </div>
          )}
        </div>
      )}
      {(tokens.length > 0 || draft) && !raw && (
        <button
          className="query-clear"
          title="Clear query"
          onClick={() => {
            setDraft('');
            commit([]);
          }}
        >
          <X size={18} />
        </button>
      )}
      <button
        className={`query-raw-toggle${raw ? ' on' : ''}`}
        title={raw ? 'Edit as tokens' : 'Edit as raw text'}
        onClick={() => setRaw((r) => !r)}
      >
        <Code2 size={16} />
      </button>
    </div>
  );
}
