import { memo } from 'react';
import { TriangleAlert } from 'lucide-react';
import type { IntegrationSummary } from '../../api/integrations/types';
import { IntegrationLogo } from './IntegrationLogo';

/** Catalog tile. The top edge encodes status: purple DETECTED, green installed, orange needs attention. */
export const IntegrationCard = memo(function IntegrationCard({ item, onOpen }: { item: IntegrationSummary; onOpen: (id: string) => void }) {
  return (
    <button className={`int-card edge-${item.status}`} onClick={() => onOpen(item.id)} title={item.warning}>
      {item.status === 'detected' && <span className="edge-label">DETECTED</span>}
      {item.status === 'warning' && (
        <span className="edge-label edge-warn">
          <TriangleAlert size={13} fill="currentColor" stroke="var(--bg)" />
        </span>
      )}
      <span className="int-card-logo">
        <IntegrationLogo logo={item.logo} name={item.name} />
      </span>
      <span className="int-card-name">{item.name}</span>
      <span className="int-card-by">by {item.publisher}</span>
    </button>
  );
});
