import { LayoutDashboard, Radar, Siren } from 'lucide-react';
import { INTEGRATION_LOGOS } from '../../assets/integrationLogos.generated';
import { BrandLogo, hasBrandLogo } from './BrandLogo';

/**
 * A dashboard's icon in compact lists (search, pickers): its integration's logo (`logo`, as the dashboards list shows
 * it), monitors' and synthetics' own icons, else the generic dashboard icon.
 */
export function DashboardIcon({ logo, title, size = 15, className }: { logo?: string | null; title: string; size?: number; className?: string }) {
  if (logo === 'tile:monitors') return <Siren size={size} color="var(--status-warn)" className={className} />;
  if (logo === 'tile:synthetics') return <Radar size={size} color="var(--primary)" className={className} />;
  if (logo && hasBrandLogo(logo)) {
    return (
      <span className={`dash-icon ${className ?? ''}`} style={{ width: size, height: size }}>
        <BrandLogo name={logo} size={size} title={title} />
      </span>
    );
  }
  const file = logo ? INTEGRATION_LOGOS[logo] : undefined;
  if (file) {
    return (
      <span className={`dash-icon ${className ?? ''}`} style={{ width: size, height: size }}>
        <img src={`/images/integrations/logos/${file}`} alt="" loading="lazy" width={size} height={size} />
      </span>
    );
  }
  return <LayoutDashboard size={size} className={className} />;
}
