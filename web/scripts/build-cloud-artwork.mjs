// Artwork of the AWS, Azure and Google Cloud overview dashboards: a banner per cloud and a tile per
// service. The compositions, patterns, illustrations and the glyphs of services without a CC0 logo are
// ObserveCraft's own; provider and service logos come from SVG Logos (@iconify-json/logos, CC0) and remain
// trademarks of their owners, shown to identify the services.
// Run: npm run artwork   (writes web/public/images/integrations/)
import { readFileSync, writeFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';

const require = createRequire(import.meta.url);
const set = JSON.parse(readFileSync(require.resolve('@iconify-json/logos/icons.json'), 'utf8'));
const FONT = 'Noto Sans, -apple-system, Segoe UI, sans-serif';

/** An SVG Logos icon, scaled to `height` and centered on (cx, cy). */
function logo(name, cx, cy, height) {
  const icon = set.icons[name];
  if (!icon) throw new Error(`SVG Logos has no ${name}`);
  const [w, h] = [icon.width ?? set.width, icon.height ?? set.height];
  const s = height / h;
  return `<g transform="translate(${(cx - (w * s) / 2).toFixed(1)} ${(cy - height / 2).toFixed(1)}) scale(${s.toFixed(4)})">${icon.body}</g>`;
}

/** One of our glyphs (drawn on a 100 × 100 grid), scaled to `size` and centered on (cx, cy). */
const glyph = (body, cx, cy, size) => `<g transform="translate(${cx - size / 2} ${cy - size / 2}) scale(${size / 100})">${body}</g>`;

const comment = (logos, owners) =>
  logos.length
    ? `<!-- ObserveCraft artwork. ${owners} logos (SVG Logos, CC0: ${logos.join(', ')}) are trademarks of their owners, shown to identify the services. -->`
    : '<!-- ObserveCraft artwork. -->';

const svg = (w, h, logos, owners, content) =>
  `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 ${w} ${h}" preserveAspectRatio="xMidYMid meet" role="img">\n  ${comment(logos, owners)}\n${content}\n</svg>\n`;

// ---------------------------------------------------------------- our glyphs

const AWS_EBS = `<rect x="6" y="6" width="88" height="88" rx="10" fill="#7AA116"/><g fill="none" stroke="#fff" stroke-width="4" stroke-linejoin="round"><rect x="26" y="24" width="48" height="52" rx="4"/><path d="M26 41h48M26 58h48"/><circle cx="64" cy="32.5" r="2.5" fill="#fff"/><circle cx="64" cy="49.5" r="2.5" fill="#fff"/><circle cx="64" cy="66.5" r="2.5" fill="#fff"/></g>`;

const AZ = { blue: '#0078D4', light: '#83B9F9', cyan: '#50E6FF', dark: '#004E8C', yellow: '#FFB900' };
const AZURE = {
  vm: `<rect x="8" y="14" width="84" height="56" rx="6" fill="${AZ.blue}"/><rect x="15" y="21" width="70" height="42" rx="3" fill="${AZ.light}"/><path d="M50 28l14 7v15l-14 7-14-7V35z" fill="${AZ.blue}"/><path d="M50 43l14-8M50 43v14M50 43l-14-8" stroke="${AZ.cyan}" stroke-width="2.5"/><path d="M40 70h20l4 14H36z" fill="${AZ.dark}"/><rect x="28" y="84" width="44" height="6" rx="3" fill="${AZ.blue}"/>`,
  sql: `<path d="M18 22v56c0 7 14.3 13 32 13s32-6 32-13V22z" fill="${AZ.blue}"/><ellipse cx="50" cy="22" rx="32" ry="12" fill="${AZ.cyan}"/><ellipse cx="50" cy="22" rx="24" ry="7" fill="${AZ.light}"/><text x="50" y="66" text-anchor="middle" font-family="${FONT}" font-size="20" font-weight="700" fill="#fff">SQL</text>`,
  app: `<circle cx="50" cy="50" r="40" fill="${AZ.blue}"/><g fill="none" stroke="${AZ.light}" stroke-width="3.5"><ellipse cx="50" cy="50" rx="17" ry="40"/><path d="M10 50h80M16 30h68M16 70h68"/></g><circle cx="74" cy="74" r="14" fill="${AZ.cyan}"/><path d="M68 74l4 4 8-9" fill="none" stroke="${AZ.dark}" stroke-width="3.5" stroke-linecap="round" stroke-linejoin="round"/>`,
  functions: `<path d="M24 14L4 50l20 36" fill="none" stroke="${AZ.blue}" stroke-width="8" stroke-linecap="round" stroke-linejoin="round"/><path d="M76 14l20 36-20 36" fill="none" stroke="${AZ.blue}" stroke-width="8" stroke-linecap="round" stroke-linejoin="round"/><path d="M56 10L32 54h16l-8 36 28-46H52z" fill="${AZ.yellow}"/>`,
  lb: `<path d="M50 6l44 44-44 44L6 50z" fill="${AZ.blue}"/><g fill="none" stroke="#fff" stroke-width="5" stroke-linecap="round" stroke-linejoin="round"><path d="M28 50h44M50 50l14-16M50 50l14 16"/><path d="M58 34h6v6M58 66h6v-6M66 44l6 6-6 6"/></g>`,
  storage: `<rect x="10" y="12" width="80" height="22" rx="5" fill="${AZ.cyan}"/><rect x="10" y="39" width="80" height="22" rx="5" fill="${AZ.light}"/><rect x="10" y="66" width="80" height="22" rx="5" fill="${AZ.blue}"/><g fill="#fff"><circle cx="74" cy="23" r="4"/><circle cx="74" cy="50" r="4"/><circle cx="74" cy="77" r="4"/></g>`,
};

const G = { blue: '#4285F4', mid: '#669DF6', light: '#AECBFA', dark: '#1967D2' };
const GOOGLE = {
  gce: `<g fill="${G.blue}">${[22, 38, 54, 70].map((v) => `<rect x="${v}" y="4" width="8" height="14" rx="2"/><rect x="${v}" y="82" width="8" height="14" rx="2"/><rect x="4" y="${v}" width="14" height="8" rx="2"/><rect x="82" y="${v}" width="14" height="8" rx="2"/>`).join('')}</g><rect x="16" y="16" width="68" height="68" rx="6" fill="${G.light}"/><rect x="30" y="30" width="40" height="40" rx="3" fill="${G.blue}"/><path d="M30 70L70 30v40z" fill="${G.dark}"/>`,
  cloudsql: `<path d="M50 8l38 18-38 18-38-18z" fill="${G.blue}"/><path d="M12 42l38 18 38-18v12L50 72 12 54z" fill="${G.mid}"/><path d="M12 64l38 18 38-18v12L50 94 12 76z" fill="${G.light}"/>`,
  gke: `<path d="M50 6l38 22v44L50 94 12 72V28z" fill="none" stroke="${G.blue}" stroke-width="8" stroke-linejoin="round"/><path d="M50 30l18 10v20L50 70 32 60V40z" fill="${G.mid}"/><path d="M50 50l18-10v20L50 70z" fill="${G.dark}"/><path d="M50 50L32 40l18-10 18 10z" fill="${G.light}"/>`,
  lb: `<circle cx="50" cy="20" r="14" fill="${G.blue}"/><g fill="none" stroke="${G.mid}" stroke-width="6" stroke-linecap="round"><path d="M50 34v18M50 52L20 70M50 52v18M50 52l30 18"/></g><g fill="${G.light}"><circle cx="18" cy="80" r="12"/><circle cx="50" cy="80" r="12"/><circle cx="82" cy="80" r="12"/></g>`,
};

// ---------------------------------------------------------------- tiles

/** A service tile: the icon above its name. */
function tile(id, label, icon, logos, owners) {
  return {
    file: `tiles/${id}.svg`,
    logos,
    svg: svg(
      320,
      220,
      logos,
      owners,
      `  <rect width="320" height="220" fill="#ffffff"/>\n  ${icon}\n  <text x="160" y="198" text-anchor="middle" fill="#3c4043" font-family="${FONT}" font-size="21">${label}</text>`,
    ),
  };
}

const awsTile = (id, label, name) => tile(`aws-${id}`, label, logo(name, 160, 92, 112), [name], 'Amazon Web Services');
const azureTile = (id, label, body) => tile(`azure-${id}`, label, glyph(body, 160, 92, 112), [], '');
const gcpTile = (id, label, body) => tile(`gcp-${id}`, label, glyph(body, 160, 92, 112), [], '');
const gcpLogoTile = (id, label, name) => tile(`gcp-${id}`, label, logo(name, 160, 92, 104), [name], 'Google');

// ---------------------------------------------------------------- banners

/** Small rounded squares scattered on a grid (deterministic). */
function dots(color, opacity) {
  let out = '';
  for (let x = 30; x < 1200; x += 60)
    for (let y = 26; y < 400; y += 60) {
      const k = (x * 7 + y * 13) % 5;
      if (k < 2) out += `<rect x="${x}" y="${y}" width="${8 + k * 4}" height="${8 + k * 4}" rx="3" fill="${color}" fill-opacity="${opacity * (1 + k)}"/>`;
    }
  return `<g>${out}</g>`;
}

const cloud = (x, y, s, fill) => `<path transform="translate(${x} ${y}) scale(${s})" d="M30 70h80a24 24 0 0 0 0-48 34 34 0 0 0-64-8A28 28 0 0 0 30 70z" fill="${fill}"/>`;

/** Our illustrations: a rising chart, a gauge, a bell and a checked badge. */
const chart = (x, y, c) =>
  `<g transform="translate(${x} ${y})"><rect width="120" height="86" rx="8" fill="#fff" stroke="#dadce0" stroke-width="2"/><g fill="${c}" fill-opacity="0.35"><rect x="16" y="52" width="14" height="22" rx="2"/><rect x="38" y="40" width="14" height="34" rx="2"/><rect x="60" y="46" width="14" height="28" rx="2"/><rect x="82" y="26" width="14" height="48" rx="2"/></g><path d="M14 60l26-18 22 8 40-34" fill="none" stroke="${c}" stroke-width="4" stroke-linecap="round" stroke-linejoin="round"/><path d="M92 14h12v12" fill="none" stroke="${c}" stroke-width="4" stroke-linecap="round" stroke-linejoin="round"/></g>`;
const gauge = (x, y, c) =>
  `<g transform="translate(${x} ${y})"><path d="M8 60a52 52 0 0 1 104 0" fill="none" stroke="#dadce0" stroke-width="12" stroke-linecap="round"/><path d="M8 60a52 52 0 0 1 82-42" fill="none" stroke="${c}" stroke-width="12" stroke-linecap="round"/><path d="M60 60l26-30" stroke="#3c4043" stroke-width="5" stroke-linecap="round"/><circle cx="60" cy="60" r="7" fill="#3c4043"/></g>`;
const bell = (x, y, c) =>
  `<g transform="translate(${x} ${y})"><path d="M30 8a22 22 0 0 0-22 22v20L2 60h56l-6-10V30A22 22 0 0 0 30 8z" fill="${c}"/><circle cx="30" cy="66" r="7" fill="${c}"/><circle cx="50" cy="10" r="9" fill="#ea4335"/></g>`;
const badge = (x, y, c) =>
  `<g transform="translate(${x} ${y})"><circle cx="34" cy="34" r="32" fill="${c}"/><path d="M20 35l10 10 18-20" fill="none" stroke="#fff" stroke-width="7" stroke-linecap="round" stroke-linejoin="round"/></g>`;

function banner(cloudId, bg, content, logos, owners) {
  return { file: `${cloudId}-banner.svg`, logos, svg: svg(1200, 400, logos, owners, `  <rect width="1200" height="400" fill="${bg}"/>\n${content}`) };
}

export const ARTWORK = [
  banner(
    'aws',
    '#fdf3e6',
    `  ${dots('#FF9900', 0.08)}\n  ${chart(150, 150, '#FF9900')}\n  ${gauge(930, 170, '#FF9900')}\n  ${logo('aws', 600, 170, 140)}\n  <text x="600" y="300" text-anchor="middle" fill="#232F3E" font-family="${FONT}" font-size="30" font-weight="600">Amazon Web Services</text>`,
    ['aws'],
    'Amazon Web Services',
  ),
  banner(
    'azure',
    '#eaf3fc',
    `  ${cloud(80, 60, 1.6, '#0078D4" fill-opacity="0.08')}\n  ${cloud(900, 210, 1.4, '#0078D4" fill-opacity="0.08')}\n  ${chart(170, 200, '#0078D4')}\n  ${bell(980, 80, '#0078D4')}\n  ${logo('azure', 600, 200, 130)}`,
    ['azure'],
    'Microsoft',
  ),
  banner(
    'gcp',
    '#f8fafd',
    `  ${cloud(40, 40, 1.4, '#e8eaed')}\n  ${cloud(930, 260, 1.2, '#e8eaed')}\n  ${cloud(820, 30, 0.9, '#e8eaed')}\n  ${chart(150, 200, '#4285F4')}\n  ${gauge(920, 110, '#34A853')}\n  ${bell(300, 60, '#FBBC04')}\n  ${badge(880, 250, '#34A853')}\n  ${logo('google-cloud', 600, 165, 150)}\n  <text x="600" y="315" text-anchor="middle" fill="#5f6368" font-family="${FONT}" font-size="34" font-weight="500">Google Cloud</text>`,
    ['google-cloud'],
    'Google',
  ),
  awsTile('ec2', 'Amazon EC2', 'aws-ec2'),
  awsTile('rds', 'Amazon RDS', 'aws-rds'),
  awsTile('elb', 'Elastic Load Balancing', 'aws-elb'),
  awsTile('lambda', 'AWS Lambda', 'aws-lambda'),
  awsTile('s3', 'Amazon S3', 'aws-s3'),
  tile('aws-ebs', 'Amazon EBS', glyph(AWS_EBS, 160, 92, 112), [], ''),
  azureTile('vm', 'Virtual Machines', AZURE.vm),
  azureTile('sql', 'SQL Database', AZURE.sql),
  azureTile('app-service', 'App Service', AZURE.app),
  azureTile('functions', 'Functions', AZURE.functions),
  azureTile('lb', 'Load Balancer', AZURE.lb),
  azureTile('storage', 'Storage', AZURE.storage),
  gcpTile('gce', 'Compute Engine', GOOGLE.gce),
  gcpTile('cloudsql', 'Cloud SQL', GOOGLE.cloudsql),
  gcpTile('gke', 'Kubernetes Engine', GOOGLE.gke),
  gcpLogoTile('run', 'Cloud Run', 'google-cloud-run'),
  gcpLogoTile('functions', 'Cloud Functions', 'google-cloud-functions'),
  gcpTile('lb', 'Cloud Load Balancing', GOOGLE.lb),
];

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  for (const { file, svg: content } of ARTWORK) writeFileSync(new URL(`../public/images/integrations/${file}`, import.meta.url), content);
  console.log(`wrote ${ARTWORK.length} files`);
}
