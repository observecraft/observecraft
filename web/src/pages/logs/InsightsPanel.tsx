import { useState } from 'react';
import { ChevronDown, ChevronRight, Radar, ScanSearch } from 'lucide-react';
import type { LogInsight } from '../../api/types';

function pct(n: number): string {
  return `${(n * 100).toFixed(n < 0.1 ? 2 : 1)}%`;
}

function Share({ value, caption, color }: { value: number; caption: string; color: string }) {
  return (
    <div className="insight-share">
      <div className="insight-track">
        <div className="insight-fill" style={{ width: `${Math.max(value * 100, 1)}%`, background: color }} />
      </div>
      <strong>{pct(value)}</strong>
      <span>{caption}</span>
    </div>
  );
}

export function InsightsPanel({ insights, onPick }: { insights: LogInsight[]; onPick: (i: LogInsight) => void }) {
  const [open, setOpen] = useState(true);
  if (insights.length === 0) return null;

  return (
    <section className="insights">
      <header>
        <button className="insights-toggle" onClick={() => setOpen((o) => !o)}>
          {open ? <ChevronDown size={16} /> : <ChevronRight size={16} />}
        </button>
        <span className="insights-badge">
          <Radar size={12} /> {insights.length}
        </span>
        <strong>Anomaly Insights</strong>
        <span className="insights-scope">Logs</span>
        <a href="#" onClick={(e) => e.preventDefault()}>
          View all
        </a>
      </header>
      {open && (
        <div className="insight-cards">
          {insights.map((i) => (
            <button className="insight-card" key={i.value} onClick={() => onPick(i)}>
              <div className="insight-kind">
                <ScanSearch size={14} /> LOGS ERROR OUTLIER
              </div>
              <div className="insight-chip">
                <span>{i.attribute}:</span>
                {i.value}
              </div>
              <div className="insight-shares">
                <Share value={i.shareOfErrors} caption="of total errors" color="var(--status-error)" />
                <Share value={i.shareOfLogs} caption="of total logs" color="var(--chart-1)" />
              </div>
            </button>
          ))}
        </div>
      )}
    </section>
  );
}
