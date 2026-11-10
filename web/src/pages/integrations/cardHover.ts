import type { IntegrationStatus } from '../../api/integrations/types';

/** A hovered catalog tile: the label its coloured top edge grows into, and its button. */
export function cardHover(status: IntegrationStatus, waitingForData = false): { band: string | null; action: 'install' | 'configure' | 'details' } {
  switch (status) {
    case 'installed':
      return { band: waitingForData ? 'Installed · Waiting for data' : 'Installed', action: 'configure' };
    case 'warning':
      return { band: 'Missing data', action: 'configure' };
    case 'detected':
      return { band: 'Detected', action: 'install' };
    case 'unavailable':
      return { band: 'Not available', action: 'details' };
    default:
      return { band: null, action: 'install' };
  }
}
