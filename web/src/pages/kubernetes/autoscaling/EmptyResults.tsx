import type { ReactNode } from 'react';
import { Search } from 'lucide-react';

/** The empty state of the Autoscaling tables. */
export function EmptyResults({ title, children }: { title: string; children?: ReactNode }) {
  return (
    <div className="as-empty">
      <Search size={56} strokeWidth={1.6} />
      <h3>{title}</h3>
      {children && <p>{children}</p>}
    </div>
  );
}
