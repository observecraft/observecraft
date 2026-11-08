// Downloads the Kubernetes project's icon set (kubernetes/community, icons/svg, Apache-2.0): one icon per resource kind
// (Pod, Deployment, Service, …) and infrastructure component (Node, control plane), the unlabeled variants, into
// public/images/kubernetes/, without the editor's metadata. KubernetesIcon (src/components/brand/KubernetesIcon.tsx)
// shows them; NOTICE.md there records the source and license.
// Run: node scripts/fetch-kubernetes-icons.mjs
import { mkdirSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';

/** kubernetes/community revision the icons are taken from. */
export const REVISION = '307ae50e8e4246de172f195c73e3f026b4f0c7ff';
const OUT = join(import.meta.dirname, '..', 'public', 'images', 'kubernetes');

/** File in the set → its folder. */
const ICONS = {
  resources: [
    'c-role',
    'cm',
    'crb',
    'crd',
    'cronjob',
    'deploy',
    'ds',
    'ep',
    'hpa',
    'ing',
    'job',
    'limits',
    'netpol',
    'ns',
    'pod',
    'pv',
    'pvc',
    'quota',
    'rb',
    'role',
    'rs',
    'sa',
    'sc',
    'secret',
    'sts',
    'svc',
    'vol',
  ],
  infrastructure_components: ['control-plane', 'etcd', 'node'],
};

/** The drawing alone: no XML prolog, comments, Inkscape/Sodipodi data, RDF metadata, ids or physical size. */
export function cleanSvg(svg) {
  return (
    svg
      .replace(/<\?xml[^>]*\?>/g, '')
      .replace(/<!--[\s\S]*?-->/g, '')
      .replace(/<metadata[\s\S]*?<\/metadata>/g, '')
      .replace(/<sodipodi:namedview[\s\S]*?(?:\/>|<\/sodipodi:namedview>)/g, '')
      .replace(/<defs[^>]*\/>/g, '')
      .replace(/\s(?:inkscape|sodipodi):[\w-]+="[^"]*"/g, '')
      .replace(/\sxmlns:\w+="[^"]*"/g, '')
      .replace(/<svg\b[^>]*>/, (tag) => `<svg xmlns="http://www.w3.org/2000/svg" viewBox="${/viewBox="([^"]+)"/.exec(tag)[1]}">`)
      .replace(/\sid="[^"]*"/g, '')
      // Inkscape's text defaults in `style` don't apply to paths.
      .replace(
        /style="([^"]*)"/g,
        (_, s) =>
          `style="${s
            .split(';')
            .filter((d) => /^(?:fill|fill-opacity|fill-rule|stroke|stroke-width|stroke-opacity|stroke-linecap|stroke-linejoin|opacity)\s*:/.test(d.trim()))
            .join(';')}"`,
      )
      .replace(/\sstyle=""/g, '')
      .replace(/>\s+</g, '><')
      .replace(/\s{2,}/g, ' ')
      .trim() + '\n'
  );
}

if (import.meta.url === `file:///${process.argv[1].replace(/\\/g, '/')}` || import.meta.url === `file://${process.argv[1]}`) {
  mkdirSync(OUT, { recursive: true });
  for (const [folder, names] of Object.entries(ICONS)) {
    for (const name of names) {
      const url = `https://raw.githubusercontent.com/kubernetes/community/${REVISION}/icons/svg/${folder}/unlabeled/${name}.svg`;
      const res = await fetch(url);
      if (!res.ok) throw new Error(`${url}: ${res.status}`);
      writeFileSync(join(OUT, `${name}.svg`), cleanSvg(await res.text()));
      console.log(name);
    }
  }
}
