// Logo registry for integrations/apps: the generated brand logos (SVG Logos, Simple Icons, LobeHub),
// then the integrations catalog's logo files (they include the cloud providers' service icons; the
// browser loads only the visible ones), then a wordmark / coloured tile. Brands remain trademarks of
// their owners and are shown only to identify the integration.
import { BrandLogo, hasBrandLogo } from '../../components/brand/BrandLogo';
import { INTEGRATION_LOGOS } from '../../assets/integrationLogos.generated';

/** Fallbacks: "word" renders a coloured wordmark, "tile" a coloured square with short text. */
const FALLBACK: Record<string, { text: string; color: string; style: 'word' | 'tile' }> = {
  keda: { text: 'KEDA', color: '#326ce5', style: 'word' },
  velero: { text: 'VELERO', color: '#2c7fb8', style: 'word' },
  coredns: { text: 'CoreDNS', color: '#7b42bc', style: 'word' },
  supervisord: { text: 'SUPERVISORD', color: '#3b3b3b', style: 'word' },
  'tile:aws-appstream': { text: 'AS', color: '#01a88d', style: 'tile' },
  'tile:aws-bedrock': { text: 'Br', color: '#01a88d', style: 'tile' },
  'tile:haproxy': { text: 'HA', color: '#106da9', style: 'tile' },
  'tile:azure-devops': { text: 'AzD', color: '#0078d4', style: 'tile' },
  'tile:servicenow': { text: 'SN', color: '#293e40', style: 'tile' },
};

function initials(text: string): string {
  return text
    .split(/[\s-]+/)
    .map((w) => w[0])
    .join('')
    .slice(0, 3)
    .toUpperCase();
}

/** Deterministic colour for unknown keys. */
function hashColor(key: string): string {
  const palette = ['#5b5fc7', '#d6336c', '#1c7ed6', '#0ca678', '#f08c00', '#7048e8', '#e8590c'];
  let h = 0;
  for (const c of key) h = (h * 31 + c.charCodeAt(0)) >>> 0;
  return palette[h % palette.length];
}

interface Props {
  logo: string;
  name: string;
  size?: number;
}

export function IntegrationLogo({ logo, name, size = 64 }: Props) {
  if (hasBrandLogo(logo)) return <BrandLogo name={logo} size={size} title={name} maxRatio={2.8} />;
  const file = INTEGRATION_LOGOS[logo];
  if (file) return <img className="logo-file" src={`/images/integrations/logos/${file}`} alt={name} loading="lazy" style={{ maxWidth: size * 2.8, height: size }} />;
  const fb = FALLBACK[logo] ?? { text: logo.startsWith('tile:') ? initials(name) : name, color: hashColor(logo), style: logo.startsWith('tile:') ? 'tile' : 'word' };
  if (fb.style === 'tile') {
    return (
      <span className="logo-tile" style={{ width: size * 1.25, height: size * 1.25, background: fb.color, fontSize: size * 0.42 }} aria-label={name}>
        {fb.text}
      </span>
    );
  }
  return (
    <span className="logo-word" style={{ color: fb.color, fontSize: Math.max(14, size * (fb.text.length > 8 ? 0.36 : 0.62)) }} aria-label={name}>
      {fb.text}
    </span>
  );
}
