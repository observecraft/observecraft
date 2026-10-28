import { useState } from 'react';
import { Link } from 'react-router-dom';
import { History, Search } from 'lucide-react';
import type { FeatureFlagChange } from '../../api/featureFlags/client';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { useFlagHistory } from './queries';

/** The rows of a Flag History (one flag's, or every flag's with `showFlag`). */
export function HistoryTable({ changes, showFlag }: { changes: FeatureFlagChange[]; showFlag?: boolean }) {
  return (
    <div className="mon-table-wrap">
      <table className="mon-table ff-table">
        <thead>
          <tr>
            <th style={{ width: 210 }}>DATE</th>
            {showFlag && <th>FLAG</th>}
            <th style={{ width: 140 }}>ENVIRONMENT</th>
            <th style={{ width: 110 }}>CHANGE</th>
            <th>DETAILS</th>
            <th style={{ width: 160 }}>USER</th>
            <th className="ff-num" style={{ width: 80 }}>
              VERSION
            </th>
          </tr>
        </thead>
        <tbody>
          {changes.map((c) => (
            <tr key={c.id}>
              <td className="muted ff-when">{new Date(c.at).toLocaleString()}</td>
              {showFlag && (
                <td>
                  {c.flagId ? (
                    <Link to={`/feature-flags/${c.flagId}`} onClick={(e) => e.stopPropagation()}>
                      {c.flagName}
                    </Link>
                  ) : (
                    <span className="muted">{c.flagName}</span>
                  )}
                  <div className="ff-key">{c.flagKey}</div>
                </td>
              )}
              <td>{c.environmentName || <span className="muted">All</span>}</td>
              <td>
                <span className={`ff-history-action ${c.action}`}>{c.action}</span>
              </td>
              <td>{c.summary}</td>
              <td>{c.actor}</td>
              <td className="ff-num muted">v{c.version}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

/** Global Flag History: changes across every flag, newest first. */
export function FlagHistoryPage() {
  const q = useFlagHistory();
  const [text, setText] = useState('');
  if (q.isPending) return <Loading label="Loading flag history…" />;
  if (q.isError) return <ErrorState error={q.error} onRetry={() => q.refetch()} />;
  const words = text.toLowerCase().split(/\s+/).filter(Boolean);
  const shown = q.data.filter((c) => words.every((w) => [c.flagKey, c.flagName, c.environmentName, c.action, c.summary, c.actor].join(' ').toLowerCase().includes(w)));
  return (
    <div className="ff-history">
      <div className="ff-toolbar">
        <div className="mon-filter">
          <Search size={16} />
          <input placeholder="Search changes by flag, environment, user or change" value={text} onChange={(e) => setText(e.target.value)} />
        </div>
        <span className="muted">{shown.length} changes</span>
      </div>
      {shown.length ? (
        <HistoryTable changes={shown} showFlag />
      ) : (
        <div className="ff-no-results">
          <History size={56} strokeWidth={1.2} />
          {q.data.length ? 'No matching results found' : 'No flag changes yet'}
        </div>
      )}
    </div>
  );
}
