import type { ReactNode } from 'react';
import { AlertTriangle } from 'lucide-react';
import './ui.css';

/** Standard loading / error / empty rendering for data-backed regions. */
export function Loading({ label = 'Loading…' }: { label?: string }) {
  return (
    <div className="state-box" role="status">
      <span className="spinner" />
      {label}
    </div>
  );
}

export function ErrorState({ error, onRetry }: { error: unknown; onRetry?: () => void }) {
  const message = error instanceof Error ? error.message : 'Something went wrong.';
  return (
    <div className="state-box state-error" role="alert">
      <AlertTriangle size={20} />
      <span>{message}</span>
      {onRetry && (
        <button className="btn" onClick={onRetry}>
          Retry
        </button>
      )}
    </div>
  );
}

export function Empty({ children }: { children: ReactNode }) {
  return <div className="state-box">{children}</div>;
}
