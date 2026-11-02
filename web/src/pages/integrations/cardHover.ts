import type { IntegrationStatus } from '../../api/integrations/types';

/** A hovered catalog tile: the label its coloured top edge grows into, and its button. */
export function cardHover(status: IntegrationStatus): { band: string | null; action: 'install' | 'configure' } {
  switch (status) {
    case 'installed':
      return { band: 'Installed', action: 'configure' };
    case 'warning':
      return { band: 'Missing data', action: 'configure' };
    case 'detected':
      return { band: 'Detected', action: 'install' };
    default:
      return { band: null, action: 'install' };
  }
}
