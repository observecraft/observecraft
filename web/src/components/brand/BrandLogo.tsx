import { memo, useId, useMemo } from 'react';
import { BRAND_LOGOS } from '../../assets/brandLogos.generated';

export function hasBrandLogo(key: string): boolean {
  return key in BRAND_LOGOS;
}

/**
 * Full-colour brand logo from the generated SVG Logos subset. Internal ids (gradients, clip
 * paths) are suffixed per instance so repeated logos on one page never reference each other's defs.
 * Bodies come from a build-time, CC0 source — never from user input. `maxRatio` lets wide wordmarks
 * use more horizontal room than the default square box.
 */
export const BrandLogo = memo(function BrandLogo({ name, size = 16, title, maxRatio = 1 }: { name: string; size?: number; title?: string; maxRatio?: number }) {
  const uid = useId().replace(/:/g, '');
  const logo = BRAND_LOGOS[name];
  const body = useMemo(() => {
    if (!logo) return '';
    const ids = [...logo.body.matchAll(/id="([^"]+)"/g)].map((m) => m[1]);
    let out = logo.body;
    for (const id of ids) {
      const esc = id.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
      out = out.replace(new RegExp(`id="${esc}"`, 'g'), `id="${id}-${uid}"`).replace(new RegExp(`#${esc}([")])`, 'g'), `#${id}-${uid}$1`);
    }
    return out;
  }, [logo, uid]);
  if (!logo) return null;
  // Landscape logos (wordmarks) may grow wider than `size`, up to `maxRatio`×, so they stay legible.
  const aspect = logo.width / logo.height;
  const width = size * Math.min(Math.max(aspect, 1), maxRatio);
  return (
    <svg
      viewBox={`0 0 ${logo.width} ${logo.height}`}
      width={width}
      height={size}
      role="img"
      aria-label={title ?? name}
      className="brand-logo"
      dangerouslySetInnerHTML={{ __html: `${title ? `<title>${title.replace(/[<&]/g, '')}</title>` : ''}${body}` }}
    />
  );
});
