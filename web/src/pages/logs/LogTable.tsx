import { ArrowDownWideNarrow } from 'lucide-react';
import type { LogEvent } from '../../api/types';
import { formatLogDate } from '../../lib/format';
import { STATUS_COLORS } from '../../theme/chartTheme';
import { LogMessage } from './LogMessage';

export interface LogTableOptions {
  wrap: boolean;
  showHost: boolean;
  showService: boolean;
}

export const DEFAULT_TABLE_OPTIONS: LogTableOptions = { wrap: false, showHost: true, showService: true };

interface Props {
  logs: LogEvent[];
  selectedId: string | null;
  onSelect: (log: LogEvent) => void;
  options?: LogTableOptions;
  /** Newly mounted rows flash briefly (live tail). */
  flashNew?: boolean;
}

export function LogTable({ logs, selectedId, onSelect, options = DEFAULT_TABLE_OPTIONS, flashNew = false }: Props) {
  const cols = 3 + Number(options.showHost) + Number(options.showService);
  return (
    <table className={`log-table${options.wrap ? ' wrap' : ''}`}>
      <thead>
        <tr>
          <th className="col-status" />
          <th className="col-date">
            <ArrowDownWideNarrow size={13} /> DATE
          </th>
          {options.showHost && <th className="col-host">HOST</th>}
          {options.showService && <th className="col-service">SERVICE</th>}
          <th>CONTENT</th>
        </tr>
      </thead>
      <tbody>
        {logs.map((l) => (
          <tr
            key={l.id}
            className={`${l.id === selectedId ? 'selected' : ''}${flashNew ? ' row-new' : ''}`}
            onClick={() => onSelect(l)}
          >
            <td className="col-status">
              <span className="status-bar" style={{ background: `var(${STATUS_COLORS[l.status]})` }} />
            </td>
            <td className="mono">{formatLogDate(l.timestamp)}</td>
            {options.showHost && <td className="mono">{l.host}</td>}
            {options.showService && <td className="mono">{l.service}</td>}
            <td className="mono col-content">
              <LogMessage message={l.message} />
            </td>
          </tr>
        ))}
        {logs.length === 0 && (
          <tr>
            <td colSpan={cols} className="log-empty">
              No logs match this query in the selected time range.
            </td>
          </tr>
        )}
      </tbody>
    </table>
  );
}
