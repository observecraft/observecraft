import { memo } from 'react';
import { Check, Plus, Settings, TriangleAlert } from 'lucide-react';
import type { IntegrationSummary } from '../../api/integrations/types';
import { IntegrationLogo } from './IntegrationLogo';
import { cardHover } from './cardHover';

/**
 * Catalog tile. The top edge encodes status: purple DETECTED, green installed, orange needs attention.
 * Hovered (or focused), the logo gives way to the description and an Install / Configure button, and the
 * coloured edge grows into a band naming the status.
 */
export const IntegrationCard = memo(function IntegrationCard({ item, onOpen }: { item: IntegrationSummary; onOpen: (id: string) => void }) {
  const hover = cardHover(item.status);
  return (
    <button className={`int-card edge-${item.status}`} onClick={() => onOpen(item.id)} title={item.warning}>
      {item.status === 'detected' && <span className="edge-label">DETECTED</span>}
      {item.status === 'warning' && (
        <span className="edge-label edge-warn">
          <TriangleAlert size={13} fill="currentColor" stroke="var(--bg)" />
        </span>
      )}
      <span className="int-card-front">
        <span className="int-card-logo">
          <IntegrationLogo logo={item.logo} name={item.name} />
        </span>
        <span className="int-card-name">{item.name}</span>
        <span className="int-card-by">
          by <b>{item.publisher}</b>
        </span>
      </span>
      <span className="int-card-back" aria-hidden="true">
        {hover.band && (
          <span className="int-card-band">
            {item.status === 'installed' && <Check size={14} strokeWidth={3} />}
            {item.status === 'warning' && <TriangleAlert size={14} fill="currentColor" stroke={`var(--status-warn)`} />}
            {hover.band}
          </span>
        )}
        <span className="int-card-name wrap">{item.name}</span>
        <span className="int-card-by">
          by <b>{item.publisher}</b>
        </span>
        <span className="int-card-desc">{item.description}</span>
        <span className="int-card-action">
          {hover.action === 'install' ? <Plus size={15} strokeWidth={2.5} /> : <Settings size={15} />}
          {hover.action === 'install' ? 'Install' : 'Configure'}
        </span>
      </span>
    </button>
  );
});
