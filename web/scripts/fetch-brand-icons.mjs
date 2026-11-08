// Downloads the logos of the integrations whose brand is in neither SVG Logos nor Simple Icons: from
// Wikimedia Commons (public-domain logos), permissive Iconify sets, the CNCF artwork repository or the
// vendor's own website (its icon), into src/assets/brand-icons/. NOTICE.md there lists every file's
// source and terms; build-logos.mjs embeds them. Logos stay their owners' trademarks and are shown only
// next to the integration's name, to identify it.
// Run: node scripts/fetch-brand-icons.mjs   (then npm run logos)
import { execFileSync } from 'node:child_process';
import { existsSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { inlineStyles } from './fetch-cloud-icons.mjs';

const commons = (title) => ({ commons: title, source: `Wikimedia Commons, File:${title}`, terms: 'Public domain (logo below the threshold of originality); trademark of its owner' });
const iconify = (set, name, license) => ({ url: `https://api.iconify.design/${set}/${name}.svg`, source: `Iconify ${set}:${name}`, terms: license });
const cncf = (project) => ({
  url: `https://raw.githubusercontent.com/cncf/artwork/main/projects/${project}/icon/color/${project}-icon-color.svg`,
  source: `CNCF artwork, projects/${project}`,
  terms: 'Linux Foundation trademark policy (https://www.linuxfoundation.org/trademark-usage)',
});
const site = (domain) => ({ site: `https://${domain}/`, source: `${domain} (site icon)`, terms: 'Trademark and copyright of its owner; shown only to identify the integration' });

/** Brand file name → where its logo comes from and the integrations (catalog ids) that show it. */
export const BRANDS = {
  anecdote: { ...site('www.anecdoteai.com'), ids: ['anecdote'] },
  appgate: { ...site('appgate.com'), ids: ['appgate-sdp'] },
  appomni: { ...site('appomni.com'), ids: ['appomni'] },
  isc: { ...site('www.isc.org'), ids: ['bind9'] },
  bottomline: { ...site('www.bottomline.com'), ids: ['bottomline-recordandreplay'] },
  celerdata: { ...site('celerdata.com'), ids: ['celerdata'] },
  cloudnatix: { ...site('cloudnatix.com'), ids: ['cloudnatix'] },
  contrastsecurity: { ...site('www.contrastsecurity.com'), ids: ['contrastsecurity', 'contrast-security-adr'] },
  cyral: { ...site('cyral.com'), ids: ['cyral'] },
  dagster: { ...site('dagster.io'), ids: ['dagster-plus'] },
  datazoom: { ...site('www.datazoom.io'), ids: ['datazoom'] },
  doppler: { ...site('www.doppler.com'), ids: ['doppler'] },
  emqx: { ...site('www.emqx.com'), ids: ['emqx'] },
  exim: { ...site('www.exim.org'), ids: ['exim'] },
  'external-secrets': {
    url: 'https://raw.githubusercontent.com/cncf/artwork/main/projects/external-secrets-operator/icon/color/eso-icon-color.svg',
    source: 'CNCF artwork, projects/external-secrets-operator',
    terms: 'Linux Foundation trademark policy (https://www.linuxfoundation.org/trademark-usage)',
    ids: ['external-secrets'],
  },
  fiddler: { ...site('www.fiddler.ai'), ids: ['fiddler'] },
  filemage: { ...site('www.filemage.io'), ids: ['filemage'] },
  // Fleet's mark as its repository ships it (the site's icon is a 32 px favicon).
  fleetdm: {
    url: 'https://raw.githubusercontent.com/fleetdm/fleet/main/orbit/cmd/desktop/fleet-logo.svg',
    source: 'fleetdm/fleet repository, orbit/cmd/desktop/fleet-logo.svg',
    terms: 'Trademark and copyright of its owner; shown only to identify the integration',
    ids: ['fleetdm'],
  },
  opa: {
    url: 'https://raw.githubusercontent.com/cncf/artwork/main/projects/open-policy-agent/icon/color/opa-icon-color.svg',
    source: 'CNCF artwork, projects/open-policy-agent',
    terms: 'Linux Foundation trademark policy (https://www.linuxfoundation.org/trademark-usage)',
    ids: ['gatekeeper'],
  },
  gigamon: { ...site('www.gigamon.com'), ids: ['gigamon'] },
  nats: { ...cncf('nats'), ids: ['gnatsd', 'gnatsd-streaming'] },
  // The site's SVG icon (its <link rel="icon"> files are PNG and ICO only).
  netbird: {
    url: 'https://netbird.io/_next/static/media/netbird-icon.167cd80b.svg',
    source: 'netbird.io (site icon)',
    terms: 'Trademark and copyright of its owner; shown only to identify the integration',
    ids: ['netbird'],
  },
  gravitee: { ...site('www.gravitee.io'), ids: ['gravitee'] },
  invary: { ...site('www.invary.com'), ids: ['invary'] },
  kepler: { ...cncf('kepler'), ids: ['kepler'] },
  tuxcare: { ...site('tuxcare.com'), ids: ['kernelcare'] },
  netnology: { ...site('www.netnology.io'), ids: ['nn-sdwan'] },
  neutrona: { ...site('neutrona.com'), ids: ['neutrona'] },
  ocient: { ...site('ocient.com'), ids: ['ocient'] },
  portworx: { ...site('portworx.com'), ids: ['portworx'] },
  purestorage: { ...site('www.purestorage.com'), ids: ['purefa', 'purefb'] },
  ravendb: { ...site('ravendb.net'), ids: ['ravendb'] },
  redpanda: { ...site('www.redpanda.com'), ids: ['redpanda'] },
  reflectiz: { ...site('www.reflectiz.com'), ids: ['reflectiz'] },
  resilience4j: { ...site('resilience4j.readme.io'), ids: ['resilience4j'] },
  scalr: { ...site('www.scalr.com'), ids: ['scalr'] },
  stardog: { ...site('www.stardog.com'), ids: ['stardog'] },
  sym: { ...site('symops.com'), ids: ['sym'] },
  pingcap: { ...site('www.pingcap.com'), ids: ['tidb'] },
  twingate: { ...site('www.twingate.com'), ids: ['twingate'] },
  typingdna: { ...site('www.typingdna.com'), ids: ['typingdna-activelock'] },
  nlnetlabs: { ...site('nlnetlabs.nl'), ids: ['unbound'] },
  appvia: { ...site('www.appvia.io'), ids: ['wayfinder'] },
  zenoh: { ...site('zenoh.io'), ids: ['zenoh-router'] },
  supervisord: { ...site('supervisord.org'), ids: ['supervisord'] },
  vertica: { ...site('www.vertica.com'), ids: ['vertica'] },
  arista: { ...commons('Arista-networks-logo.svg'), ids: ['snmp-arista'] },
  aruba: { ...commons('Aruba Networks logo.svg'), ids: ['snmp-aruba'] },
  barracuda: { ...commons('Barracuda Networks logo.svg'), ids: ['cloudgen-firewall', 'barracuda-secure-edge'] },
  beyondtrust: { ...commons('BeyondTrust logo.svg'), ids: ['beyondtrust-identity-security-insights', 'beyondtrust-password-safe', 'beyondtrust-privileged-remote-access'] },
  checkpoint: { ...site('www.checkpoint.com'), ids: ['snmp-check-point', 'checkpoint-harmony-email-and-collaboration', 'checkpoint-quantum-firewall', 'checkpoint-harmony-endpoint'] },
  crowdstrike: { ...commons('CrowdStrike logo.svg'), ids: ['crowdstrike-fdr'] },
  eset: { ...commons('ESET logo.svg'), ids: ['eset-protect'] },
  extrahop: { ...commons('ExtraHop-logo-rgb.svg'), ids: ['extrahop'] },
  forcepoint: { ...commons('Forcepoint Logo.svg'), ids: ['forcepoint-secure-web-gateway', 'forcepoint-security-service-edge'] },
  genesys: { ...commons('Genesys Telecommunications Laboratories logo.svg'), ids: ['genesys'] },
  hazelcast: { ...commons('HazelcastLogo-Blue Dark Square.svg'), ids: ['hazelcast'] },
  litellm: { ...site('www.litellm.ai'), ids: ['litellm'] },
  marklogic: { ...commons('MarkLogic logo.svg'), ids: ['marklogic'] },
  nagios: { ...commons('Nagios logo.svg'), ids: ['nagios'] },
  powerdns: { ...commons('Logo of PowerDNS.svg'), ids: ['powerdns'] },
  sophos: { ...commons('Sophos logo.svg'), ids: ['sophos-central-cloud'] },
  tenable: { ...commons('Tenable, Inc. logo.svg'), ids: ['tenable', 'tenable-io'] },
  tibco: { ...commons('TIBCO logo.svg'), ids: ['tibco-ems'] },
  trellix: { ...commons('Trellix Logo.svg'), ids: ['trellix-endpoint-security'] },
  vectra: { ...commons('Vectra AI logo.svg'), ids: ['vectra'] },
  wazuh: { ...commons('Wazuh-2016 2022-Logo.svg'), ids: ['wazuh'] },
  watchguard: { ...commons('Watchguard logo.svg'), ids: ['watchguard-firebox'] },
  pingidentity: { ...commons('Ping Identity logo.svg'), ids: ['ping-federate', 'ping-one', 'ping-one-aic'] },
  proofpoint: { ...commons('Proofpoint-vector-logo.svg'), ids: ['proofpoint-on-demand', 'proofpoint-tap'] },
  crewai: { ...iconify('simple-icons', 'crewai', 'CC0-1.0 (Simple Icons)'), ids: ['crewai'] },
  docusign: { ...iconify('simple-icons', 'docusign', 'CC0-1.0 (Simple Icons)'), ids: ['docusign'] },
  haproxy: { ...iconify('devicon', 'haproxy', 'MIT (devicon)'), ids: ['haproxy'] },
  vllm: { ...iconify('simple-icons', 'vllm', 'CC0-1.0 (Simple Icons)'), ids: ['vllm'] },
  kubeflow: { ...iconify('devicon', 'kubeflow', 'MIT (devicon)'), ids: ['kubeflow'] },
  teleport: { ...iconify('devicon', 'teleport', 'MIT (devicon)'), ids: ['teleport'] },
  silverstripe: { ...iconify('vscode-icons', 'file-type-silverstripe', 'MIT (vscode-icons)'), ids: ['silverstripe-cms'] },
  varnish: { ...iconify('material-icon-theme', 'varnish', 'MIT (Material Icon Theme)'), ids: ['varnish'] },
  powershell: { ...iconify('devicon', 'powershell', 'MIT (devicon)'), ids: ['powershell'] },
  keda: { ...cncf('keda'), ids: ['keda'] },
  kyverno: { ...cncf('kyverno'), ids: ['kyverno'] },
  coredns: { ...cncf('coredns'), ids: ['coredns'] },
  velero: { ...cncf('velero'), ids: ['velero'] },
  zscaler: { ...site('www.zscaler.com'), ids: ['zscaler-private-access', 'z-scaler'] },
  delinea: { ...site('delinea.com'), ids: ['delinea-privilege-manager', 'delinea-secret-server'] },
  mimecast: { ...site('www.mimecast.com'), ids: ['mimecast'] },
  jamf: { ...site('www.jamf.com'), ids: ['jamf-pro', 'jamf-protect'] },
  klaviyo: { ...site('www.klaviyo.com'), ids: ['klaviyo'] },
  tanium: { ...site('www.tanium.com'), ids: ['tanium'] },
  cato: { ...site('www.catonetworks.com'), ids: ['cato-networks'] },
  forescout: { ...site('www.forescout.com'), ids: ['forescout'] },
  workato: { ...site('www.workato.com'), ids: ['workato'] },
  plivo: { ...site('www.plivo.com'), ids: ['plivo'] },
  postmark: { ...site('postmarkapp.com'), ids: ['postmark'] },
  anomali: { ...site('www.anomali.com'), ids: ['anomali-threatstream'] },
  appgate: { ...site('appgate.com'), ids: ['appgate-sdp'] },
  arcticwolf: { ...site('arcticwolf.com'), ids: ['arctic-wolf-aurora-endpoint-security'] },
  cofense: { ...site('cofense.com'), ids: ['cofense-triage'] },
  openldap: { ...site('www.openldap.org'), ids: ['openldap'] },
  zeek: { ...site('zeek.org'), ids: ['zeek'] },
  suricata: { ...site('suricata.io'), ids: ['suricata'] },
  squid: { ...site('www.squid-cache.org'), ids: ['squid'] },
  gearman: { ...site('gearman.org'), ids: ['gearman'] },
  proxysql: { ...site('proxysql.com'), ids: ['proxysql'] },
  pgbouncer: { ...site('www.pgbouncer.org'), ids: ['pgbouncer'] },
  voltdb: { ...site('www.voltactivedata.com'), ids: ['voltdb'] },
  iru: { ...site('www.iru.com'), ids: ['kandji'] },
  orca: { ...site('orca.security'), ids: ['orca-security'] },
  versa: { ...site('versa-networks.com'), ids: ['versa'] },
  zerofox: { ...site('www.zerofox.com'), ids: ['zerofox-cloud-platform'] },
  pushsecurity: { ...site('pushsecurity.com'), ids: ['push-security'] },
  cacti: { ...site('www.cacti.net'), ids: ['cacti'] },
  ossec: { ...site('www.ossec.net'), ids: ['ossec-security'] },
  calico: { ...site('www.tigera.io'), ids: ['calico'] },
  silk: { ...site('silk.us'), ids: ['silk'] },
  dnsfilter: { ...site('www.dnsfilter.com'), ids: ['dnsfilter'] },
  valence: { ...site('www.valencesecurity.com'), ids: ['valence-security'] },
  sglang: { ...site('docs.sglang.ai'), ids: ['sglang'] },
  mux: { ...site('www.mux.com'), ids: ['mux'] },
  supervisord: { ...site('supervisord.org'), ids: ['supervisord'] },
  zeronetworks: { ...site('zeronetworks.com'), ids: ['zero-networks'] },
  ivanti: { ...site('www.ivanti.com'), ids: ['ivanti-connect-secure', 'ivanti-nzta'] },
  slurm: { ...site('www.schedmd.com'), ids: ['slurm'] },
  weaviate: { ...site('weaviate.io'), ids: ['weaviate'] },
  vertica: { ...site('www.vertica.com'), ids: ['vertica'] },
  postfix: { ...site('www.postfix.org'), ids: ['postfix'] },
  iboss: { ...site('www.iboss.com'), ids: ['iboss'] },
  chatsworth: { ...site('www.chatsworth.com'), ids: ['snmp-chatsworth-products'] },
};

export const OUT = new URL('../src/assets/brand-icons/', import.meta.url);

/** Removes what must never reach a page from a downloaded SVG: scripts, event handlers, external references. */
export function sanitizeSvg(svg) {
  let out = svg;
  let before;
  do {
    before = out;
    out = out
      .replace(/<\?xml[^>]*>\s*/g, '')
      .replace(/<!DOCTYPE[^>]*>\s*/gi, '')
      .replace(/<metadata[\s\S]*?<\/metadata>/g, '')
      .replace(/<title>[^<]*<\/title>/g, '')
      .replace(/<foreignObject[\s\S]*?<\/foreignObject>/gi, '')
      // Editor metadata: its namespaces are lost once the logo is rewrapped, which breaks the XML.
      .replace(/<(sodipodi|inkscape|sketch|rdf|cc|dc):[\w-]+[^>]*\/>/g, '')
      .replace(/<(sodipodi|inkscape|sketch|rdf|cc|dc):([\w-]+)[^>]*>[\s\S]*?<\/\1:\2>/g, '')
      .replace(/\s(?:sodipodi|inkscape|sketch):[\w-]+="[^"]*"/g, '')
      .replace(/\s(?:xlink:)?href\s*=\s*(?:"(?!#|data:image\/)[^"]*"|'(?!#|data:image\/)[^']*')/g, '');
    // Each removal repeats on its own until stable (removing one can join the text around it into another,
    // `<scr<script>ipt>`), and the whole pass repeats until no removal changes anything.
    let last;
    do {
      last = out;
      out = out.replace(/<!--[\s\S]*?-->/g, '');
    } while (out !== last);
    do {
      last = out;
      out = out.replace(/<script\b[\s\S]*?<\/script\b[^>]*>/gi, '');
    } while (out !== last);
    do {
      last = out;
      out = out.replace(/<\/?script\b[^>]*>/gi, '');
    } while (out !== last);
    do {
      last = out;
      out = out.replace(/\son[a-z]+\s*=\s*(?:"[^"]*"|'[^']*'|[^\s>]*)/gi, '');
    } while (out !== last);
  } while (out !== before);
  return inlineStyles(out).trim();
}

// Wikimedia asks for a descriptive agent with a project URL (https://meta.wikimedia.org/wiki/User-Agent_policy).
const UA = { 'User-Agent': 'ObserveCraftLogoFetcher/1.0 (https://github.com/observecraft/observecraft) node-fetch' };

/** A website's icons, best first: an SVG icon, its largest declared PNGs (touch icons first), then the conventional files. */
async function siteIcons(base) {
  const html = await (await fetch(base, { headers: UA, redirect: 'follow', signal: AbortSignal.timeout(20000) })).text();
  const links = [...html.matchAll(/<link\b[^>]*>/gi)]
    .map(([tag]) => ({
      rel: /rel=["']([^"']+)/i.exec(tag)?.[1]?.toLowerCase() ?? '',
      href: /href=["']([^"']+)/i.exec(tag)?.[1],
      sizes: Number(/sizes=["'](\d+)/i.exec(tag)?.[1] ?? 0),
      type: /type=["']([^"']+)/i.exec(tag)?.[1] ?? '',
    }))
    .filter((l) => l.href && /icon/.test(l.rel) && !/mask-icon/.test(l.rel));
  const rank = (l) => (/\.svg(\?|$)/i.test(l.href) || /svg/.test(l.type) ? 10000 : 0) + (/apple-touch/.test(l.rel) ? 1000 : 0) + l.sizes;
  const declared = links.sort((a, b) => rank(b) - rank(a)).map((l) => new URL(l.href.replace(/&amp;/g, '&'), base).href);
  return [...new Set([...declared, new URL('/apple-touch-icon.png', base).href, new URL('/favicon.ico', base).href])];
}

/** Another picture format (Windows icon, WebP, JPEG) as PNG, its largest image for icons (macOS sips). */
function toPng(bytes) {
  const dir = mkdtempSync(join(tmpdir(), 'ico-'));
  writeFileSync(join(dir, 'icon.img'), bytes);
  execFileSync('sips', ['-s', 'format', 'png', join(dir, 'icon.img'), '--out', join(dir, 'icon.png')], { stdio: 'ignore' });
  const png = readFileSync(join(dir, 'icon.png'));
  rmSync(dir, { recursive: true });
  return png;
}

/** A PNG no larger than 128 px a side (tiles show 64 px; 2× for sharp screens), so the logos chunk stays small. */
export function shrinkPng(bytes) {
  if (Math.max(bytes.readUInt32BE(16), bytes.readUInt32BE(20)) <= 128) return bytes;
  const dir = mkdtempSync(join(tmpdir(), 'png-'));
  writeFileSync(join(dir, 'in.png'), bytes);
  execFileSync('sips', ['-Z', '128', join(dir, 'in.png'), '--out', join(dir, 'out.png')], { stdio: 'ignore' });
  const png = readFileSync(join(dir, 'out.png'));
  rmSync(dir, { recursive: true });
  return png;
}

/** An SVG that only wraps one large picture: that picture, as a PNG. */
function embeddedPng(svg) {
  const images = [...svg.matchAll(/data:image\/png;base64,([A-Za-z0-9+/=]+)/g)].map((m) => m[1]);
  return svg.length > 40000 && images.length === 1 && images[0].length > svg.length * 0.8 ? Buffer.from(images[0], 'base64') : null;
}

/** The direct file URL of a Wikimedia Commons file (FilePath redirects are rate limited). */
async function commonsFile(title) {
  const api = `https://commons.wikimedia.org/w/api.php?action=query&format=json&prop=imageinfo&iiprop=url&titles=${encodeURIComponent(`File:${title}`)}`;
  const page = Object.values((await (await fetch(api, { headers: UA, signal: AbortSignal.timeout(20000) })).json()).query.pages)[0];
  if (!page.imageinfo) throw new Error(`Commons has no File:${title}`);
  return page.imageinfo[0].url;
}

/** One logo as `{ ext, bytes, url }`, from the first candidate that is an SVG or a picture. */
async function fetchLogo(b) {
  const candidates = b.commons ? [await commonsFile(b.commons)] : b.url ? [b.url] : await siteIcons(b.site);
  const errors = [];
  for (const url of candidates) {
    try {
      const file = await download(url);
      if (/svg/.test(file.type) || file.bytes.subarray(0, 300).toString().includes('<svg')) {
        const svg = sanitizeSvg(file.bytes.toString('utf8'));
        const picture = embeddedPng(svg);
        if (picture) return { ext: 'png', bytes: shrinkPng(picture), url: file.url };
        return { ext: 'svg', bytes: Buffer.from(`${svg}\n`), url: file.url };
      }
      if (file.bytes.subarray(1, 4).toString() !== 'PNG' && (/^image\//.test(file.type) || /\.ico(\?|$)/i.test(file.url))) file.bytes = toPng(file.bytes);
      if (file.bytes.subarray(1, 4).toString() === 'PNG') return { ext: 'png', bytes: shrinkPng(file.bytes), url: file.url };
      errors.push(`${url}: not an image (${file.type})`);
    } catch (e) {
      errors.push(e.message);
    }
  }
  throw new Error(errors.join('; '));
}

async function download(url, attempt = 0) {
  const res = await fetch(url, { headers: UA, redirect: 'follow', signal: AbortSignal.timeout(20000) });
  if (res.status === 429 && attempt < 5) {
    await new Promise((r) => setTimeout(r, 5000 * (attempt + 1)));
    return download(url, attempt + 1);
  }
  if (!res.ok) throw new Error(`${url}: HTTP ${res.status}`);
  return { type: res.headers.get('content-type') ?? '', bytes: Buffer.from(await res.arrayBuffer()), url: res.url };
}

if (process.argv[1] === new URL(import.meta.url).pathname) {
  // Already downloaded logos are kept (their NOTICE rows too); --force downloads everything again.
  const force = process.argv.includes('--force');
  mkdirSync(OUT, { recursive: true });
  const previous = existsSync(new URL('NOTICE.md', OUT))
    ? readFileSync(new URL('NOTICE.md', OUT), 'utf8')
        .split('\n')
        .filter((l) => l.startsWith('| `'))
    : [];
  const files = readdirSync(OUT).filter((f) => f !== 'NOTICE.md');
  for (const f of files) if (force || !(f.replace(/\.\w+$/, '') in BRANDS)) rmSync(new URL(f, OUT));
  const notice = [];
  const failed = [];
  for (const [name, b] of Object.entries(BRANDS)) {
    const kept = !force && files.find((f) => f.replace(/\.\w+$/, '') === name);
    const row = previous.find((l) => l.startsWith(`| \`${name}.`));
    if (kept && row) {
      if (kept.endsWith('.png')) writeFileSync(new URL(kept, OUT), shrinkPng(readFileSync(new URL(kept, OUT))));
      notice.push(row);
      continue;
    }
    if (b.commons) await new Promise((r) => setTimeout(r, 1500));
    try {
      const logo = await fetchLogo(b);
      writeFileSync(new URL(`${name}.${logo.ext}`, OUT), logo.bytes);
      notice.push(`| \`${name}.${logo.ext}\` | ${b.ids.join(', ')} | ${b.source} (${logo.url}) | ${b.terms} |`);
    } catch (e) {
      failed.push(`${name}: ${e.message}`);
    }
  }
  writeFileSync(
    new URL('NOTICE.md', OUT),
    `# Integration logos\n\nDownloaded by \`web/scripts/fetch-brand-icons.mjs\`. These logos are not covered by ObserveCraft's MIT license: each\nremains its owner's trademark and is shown only next to the integration's name, to identify the product it\nmonitors, never to represent ObserveCraft.\n\n| File | Integrations | Source | Terms |\n|---|---|---|---|\n${notice.join('\n')}\n`,
  );
  if (failed.length) console.error(`Failed:\n  ${failed.join('\n  ')}`);
  console.log(`${notice.length} logos written to ${OUT.pathname}`);
}
