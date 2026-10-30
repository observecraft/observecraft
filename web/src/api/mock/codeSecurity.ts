// In-browser Code Security: the server's demo data (fictional advisories on fictional packages of the
// demo services and example-org repositories), matched and filtered the way the server does.
import type {
  AdvisoryDetail,
  AdvisoryPage,
  AdvisorySummary,
  CodeSecuritySetup,
  CodeSecuritySummary,
  CsFilter,
  FindingKind,
  InventoryPage,
  InventoryService,
  LibraryVulnerability,
  LibraryVulnerabilityPage,
  PullRequestScan,
  RepositorySummary,
  RuleSummary,
  RuntimeCodeVulnerability,
  RuntimeCodeVulnerabilityPage,
  SeverityCounts,
  StaticFinding,
  StaticFindingPage,
} from '../codeSecurity/client';

const DAY = 86_400_000;
const now = Date.now();
const RANK: Record<string, number> = { critical: 4, high: 3, medium: 2, low: 1, unknown: 0 };

interface Adv {
  id: string;
  cve: string;
  summary: string;
  score: number;
  cwe: string;
  ecosystem: string;
  pkg: string;
  introduced: string;
  fixed: string;
  daysAgo: number;
}

const ADVISORIES: Adv[] = [
  {
    id: 'OCSA-DEMO-2026-0001',
    cve: 'CVE-2026-90001',
    summary: 'example-http-server: Digest authenticator accepts unknown users',
    score: 9.1,
    cwe: 'CWE-287',
    ecosystem: 'Maven',
    pkg: 'com.example.web:http-server-core',
    introduced: '11.0.0',
    fixed: '11.0.22',
    daysAgo: 160,
  },
  {
    id: 'OCSA-DEMO-2026-0002',
    cve: 'CVE-2026-90002',
    summary: 'example-http-server: HTTP/2 request headers not validated',
    score: 9.1,
    cwe: 'CWE-20',
    ecosystem: 'Maven',
    pkg: 'com.example.web:http-server-core',
    introduced: '11.0.0',
    fixed: '11.0.15',
    daysAgo: 90,
  },
  {
    id: 'OCSA-DEMO-2026-0003',
    cve: 'CVE-2026-90003',
    summary: 'example-json: polymorphic type validator bypass allows arbitrary class instantiation',
    score: 8.1,
    cwe: 'CWE-502',
    ecosystem: 'Maven',
    pkg: 'com.example.json:json-databind',
    introduced: '2.0.0',
    fixed: '2.19.4',
    daysAgo: 21,
  },
  {
    id: 'OCSA-DEMO-2026-0004',
    cve: 'CVE-2026-90004',
    summary: 'example-json: unbounded token growth in the data input parser',
    score: 7.5,
    cwe: 'CWE-400',
    ecosystem: 'Maven',
    pkg: 'com.example.json:json-core',
    introduced: '2.0.0',
    fixed: '2.19.3',
    daysAgo: 12,
  },
  {
    id: 'OCSA-DEMO-2026-0005',
    cve: 'CVE-2026-90005',
    summary: '@example/session: session fixation through crafted cookies',
    score: 7.1,
    cwe: 'CWE-384',
    ecosystem: 'npm',
    pkg: '@example/session',
    introduced: '1.0.0',
    fixed: '1.4.2',
    daysAgo: 45,
  },
  {
    id: 'OCSA-DEMO-2026-0006',
    cve: 'CVE-2026-90006',
    summary: 'example-yaml: arbitrary code execution when loading untrusted documents',
    score: 9.8,
    cwe: 'CWE-94',
    ecosystem: 'PyPI',
    pkg: 'example-yaml',
    introduced: '5.0',
    fixed: '6.0.1',
    daysAgo: 6,
  },
  {
    id: 'OCSA-DEMO-2026-0007',
    cve: 'CVE-2026-90007',
    summary: 'Example.Http.Client: response splitting via redirect headers',
    score: 4.2,
    cwe: 'CWE-113',
    ecosystem: 'NuGet',
    pkg: 'example.http.client',
    introduced: '3.0.0',
    fixed: '3.2.5',
    daysAgo: 70,
  },
  {
    id: 'OCSA-DEMO-2026-0008',
    cve: 'CVE-2026-90008',
    summary: 'example.com/grpc-gateway: path traversal in static file handler',
    score: 7.5,
    cwe: 'CWE-22',
    ecosystem: 'Go',
    pkg: 'example.com/grpc-gateway',
    introduced: '1.0.0',
    fixed: '1.8.3',
    daysAgo: 30,
  },
];

const severityOf = (s: number) => (s >= 9 ? 'critical' : s >= 7 ? 'high' : s >= 4 ? 'medium' : s > 0 ? 'low' : 'unknown');
const LANG: Record<string, string> = { Maven: 'java', npm: 'nodejs', PyPI: 'python', NuGet: 'dotnet', Go: 'go' };

interface Lib {
  source: 'runtime' | 'static';
  ecosystem: string;
  name: string;
  version: string;
  file: string;
  service: string;
  env: string;
  repository: string;
  licenses: string[];
}

const SETS: Record<string, [string, string, string, string, string][]> = {
  java: [
    ['Maven', 'com.example.web:http-server-core', '11.0.9', 'pom.xml', 'Apache-2.0'],
    ['Maven', 'com.example.json:json-databind', '2.19.2', 'pom.xml', 'Apache-2.0'],
    ['Maven', 'com.example.json:json-core', '2.19.2', 'pom.xml', 'Apache-2.0'],
    ['Maven', 'com.example.logging:log-core', '3.4.1', 'pom.xml', 'Apache-2.0'],
  ],
  node: [
    ['npm', '@example/session', '1.3.0', 'package-lock.json', 'MIT'],
    ['npm', '@example/router', '4.2.0', 'package-lock.json', 'MIT'],
  ],
  python: [
    ['PyPI', 'example-yaml', '5.4.1', 'requirements.txt', 'MIT'],
    ['PyPI', 'example-http', '2.31.0', 'requirements.txt', 'Apache-2.0'],
  ],
  dotnet: [['NuGet', 'example.http.client', '3.1.0', 'Catalog.csproj', 'MIT']],
  go: [['Go', 'example.com/grpc-gateway', '1.7.0', 'go.mod', 'BSD-3-Clause']],
};

const LIBS: Lib[] = [
  ...(
    [
      ['storefront-api', 'prod', 'java'],
      ['api-gateway', 'prod', 'node'],
      ['billing', 'prod', 'python'],
      ['catalog-service', 'prod', 'dotnet'],
      ['order-service', 'staging', 'go'],
    ] as const
  ).flatMap(([service, env, set]) =>
    SETS[set].map(([ecosystem, name, version, , lic]) => ({ source: 'runtime' as const, ecosystem, name, version, file: '', service, env, repository: '', licenses: [lic] })),
  ),
  ...(
    [
      ['github.com/example-org/storefront', 'java'],
      ['github.com/example-org/api-gateway', 'node'],
      ['github.com/example-org/billing', 'python'],
      ['github.com/example-org/order-service', 'go'],
    ] as const
  ).flatMap(([repository, set]) =>
    SETS[set].map(([ecosystem, name, version, file, lic]) => ({ source: 'static' as const, ecosystem, name, version, file, service: '', env: '', repository, licenses: [lic] })),
  ),
];

function cmp(a: string, b: string): number {
  const pa = a.split(/[.-]/).map(Number);
  const pb = b.split(/[.-]/).map(Number);
  for (let i = 0; i < Math.max(pa.length, pb.length); i++) {
    const d = (pa[i] ?? 0) - (pb[i] ?? 0);
    if (d) return d;
  }
  return 0;
}

const triaged = new Map<string, 'open' | 'muted'>();

function allLibraryVulns(): LibraryVulnerability[] {
  const out = new Map<string, LibraryVulnerability>();
  for (const a of ADVISORIES)
    for (const l of LIBS) {
      if (l.ecosystem !== a.ecosystem || l.name !== a.pkg || cmp(l.version, a.introduced) < 0 || cmp(l.version, a.fixed) >= 0) continue;
      const key = `${a.id}|${l.ecosystem}|${l.name}|${l.version}`;
      let v = out.get(key);
      if (!v) {
        v = {
          key,
          advisoryId: a.id,
          cve: a.cve,
          aliases: [a.cve],
          title: a.summary,
          severity: severityOf(a.score),
          score: a.score,
          cwes: [a.cwe],
          ecosystem: l.ecosystem,
          language: LANG[l.ecosystem],
          package: l.name,
          version: l.version,
          fixed: a.fixed,
          services: [],
          repositories: [],
          envs: [],
          inProduction: false,
          direct: true,
          firstSeen: now - Math.min(a.daysAgo, 150) * DAY,
          lastSeen: now - 60_000,
          status: triaged.get(`vuln:${a.id}:${l.ecosystem}:${l.name}`) ?? 'open',
          occurrences: [],
        };
        out.set(key, v);
      }
      if (l.source === 'runtime') {
        v.services.push(l.service);
        if (!v.envs.includes(l.env)) v.envs.push(l.env);
        v.inProduction ||= l.env === 'prod';
      } else v.repositories.push(l.repository);
      v.occurrences.push({ source: l.source, service: l.service, env: l.env, repository: l.repository, file: l.file, dev: false, firstSeen: v.firstSeen, lastSeen: v.lastSeen });
    }
  return [...out.values()].sort((a, b) => RANK[b.severity] - RANK[a.severity] || (b.score ?? 0) - (a.score ?? 0));
}

function page<T>(rows: T[], f: CsFilter): { total: number; rows: T[] } {
  const limit = f.limit || 25;
  return { total: rows.length, rows: rows.slice(f.offset ?? 0, (f.offset ?? 0) + limit) };
}
const sevOk = (f: CsFilter, s: string) => !f.severity?.length || f.severity.some((x) => x.toLowerCase() === s);
const statusOk = (f: CsFilter, s: string) => (!f.status || f.status === 'open' ? s === 'open' : f.status === 'all' || f.status === s);
const textOk = (f: CsFilter, hay: string[]) => {
  const words = (f.query ?? '').toLowerCase().split(/\s+/).filter(Boolean);
  return words.every((w) => hay.some((h) => h.toLowerCase().includes(w)));
};

export function libraryVulnerabilities(f: CsFilter): LibraryVulnerabilityPage {
  const rows = allLibraryVulns().filter(
    (v) =>
      statusOk(f, v.status) &&
      sevOk(f, v.severity) &&
      (!f.repository || v.repositories.includes(f.repository)) &&
      (!f.service || v.services.includes(f.service)) &&
      (!f.language || v.language === f.language) &&
      textOk(f, [v.advisoryId, v.cve ?? '', v.title, v.package]),
  );
  const p = page(rows, f);
  return { total: p.total, findings: rows.reduce((s, v) => s + v.occurrences.length, 0), vulnerabilities: p.rows };
}

const REPO = 'github.com/example-org/storefront';

function finding(
  id: string,
  kind: FindingKind,
  tool: string,
  ruleId: string,
  ruleName: string,
  severity: string,
  cwes: string[],
  file: string,
  line: number,
  message: string,
  snippet: string,
  daysAgo: number,
  prNumber = '',
  description = '',
): StaticFinding {
  return {
    id,
    kind,
    tool,
    ruleId,
    ruleName,
    description,
    cwes,
    helpUri: '',
    severity,
    message,
    repository: REPO,
    branch: prNumber ? 'feature/gift-cards' : 'main',
    commitSha: prNumber ? 'b71e0d3' : '4f2a9c1',
    file,
    line,
    snippet,
    prNumber,
    firstSeen: now - daysAgo * DAY,
    lastSeen: now - 3600_000,
    resolvedAt: null,
    status: triaged.get(`finding:${id}`) ?? 'open',
  };
}

function allFindings(prs = false): StaticFinding[] {
  const rows = [
    finding(
      'f-sql',
      'sast',
      'datadog-static-analyzer',
      'python-security/variable-sql-statement-injection',
      'Potential SQL injection',
      'critical',
      ['CWE-89'],
      'billing/coupons/validators.py',
      138,
      'SQL statement built from user input',
      'admin_query = f"SELECT * FROM coupons WHERE {custom_filter}"',
      24,
      '',
      'SQL built by string formatting from variables can be injected; use query parameters.',
    ),
    finding(
      'f-path',
      'sast',
      'datadog-static-analyzer',
      'java-security/path-traversal',
      'Path traversal',
      'high',
      ['CWE-22'],
      'src/main/java/com/example/storefront/AssetController.java',
      61,
      'File path built from a request parameter',
      'Files.readAllBytes(Paths.get(baseDir, name))',
      40,
    ),
    finding(
      'f-pst',
      'sast',
      'datadog-static-analyzer',
      'java-best-practices/avoid-printstacktrace',
      'Avoid printStackTrace',
      'low',
      [],
      'src/main/java/com/example/storefront/CartService.java',
      112,
      'Use a logger instead of printStackTrace',
      'e.printStackTrace();',
      90,
    ),
    finding(
      'f-secret',
      'secret',
      'datadog-static-analyzer',
      'secrets/example-payments-api-key',
      'Payments API key',
      'critical',
      [],
      'docs/README.md',
      54,
      'Payments API key committed in the repository',
      'PAYMENTS_KEY=pk_live_EXAMPLE…',
      6,
    ),
    finding(
      'f-iac1',
      'iac',
      'Checkov',
      'CKV_AWS_28',
      'DynamoDB point-in-time recovery disabled',
      'medium',
      ['CWE-693'],
      'infra/dynamodb.tf',
      139,
      'aws_dynamodb_table.orders has no point-in-time recovery',
      'resource "aws_dynamodb_table" "orders" {',
      15,
    ),
    finding(
      'f-iac2',
      'iac',
      'Checkov',
      'CKV_AWS_20',
      'S3 bucket is publicly readable',
      'critical',
      ['CWE-284'],
      'infra/s3.tf',
      12,
      'aws_s3_bucket.assets allows public read',
      'acl = "public-read"',
      15,
    ),
  ];
  const pr = [
    finding(
      'f-pr',
      'sast',
      'datadog-static-analyzer',
      'java-security/sql-injection',
      'SQL injection',
      'critical',
      ['CWE-89'],
      'src/main/java/com/example/storefront/GiftCardDao.java',
      37,
      'SQL statement built from user input',
      '"SELECT * FROM gift_cards WHERE code = \'" + code + "\'"',
      1,
      '418',
    ),
  ];
  return prs ? pr : rows;
}

export function findings(kind: FindingKind, f: CsFilter): StaticFindingPage {
  const rows = allFindings()
    .filter((x) => x.kind === kind && statusOk(f, x.status) && sevOk(f, x.severity) && (!f.repository || x.repository === f.repository) && textOk(f, [x.ruleId, x.ruleName, x.file, x.message]))
    .sort((a, b) => RANK[b.severity] - RANK[a.severity]);
  const p = page(rows, f);
  return { total: p.total, findings: p.rows };
}

function allIast(): RuntimeCodeVulnerability[] {
  const v = (
    type: string,
    title: string,
    severity: string,
    cwe: string,
    hash: string,
    path: string,
    method: string,
    line: number,
    evidence: string,
    service: string,
    resource: string,
    count: number,
  ): RuntimeCodeVulnerability => ({
    type,
    title,
    severity,
    cwe,
    hash,
    path,
    method,
    line,
    evidence,
    services: [service],
    envs: ['prod'],
    resources: [resource],
    count,
    firstSeen: now - 6 * DAY,
    lastSeen: now - 120_000,
    traceId: '0af7651916cd43dd8448eb211c80319c',
    status: triaged.get(`iast:${hash}`) ?? 'open',
  });
  return [
    v(
      'SQL_INJECTION',
      'SQL Injection',
      'critical',
      'CWE-89',
      '4118201',
      'com.example.storefront.orders.OrderDao',
      'findById',
      44,
      'SELECT * FROM orders WHERE id = 1042',
      'storefront-api',
      'GET /api/orders/{id}',
      212,
    ),
    v(
      'SSRF',
      'Server-Side Request Forgery',
      'high',
      'CWE-918',
      '9334507',
      'billing/providers/callback.py',
      'notify',
      112,
      'https://payments.example.com/hooks/42',
      'billing',
      'POST /payments/charge',
      87,
    ),
    v('WEAK_HASH', 'Weak Hashing Algorithm', 'low', 'CWE-328', '2779013', 'com.example.storefront.catalog.ProductCache', 'cacheKey', 87, 'MD5', 'storefront-api', 'GET /api/products', 404),
  ];
}

export function runtimeCodeVulnerabilities(f: CsFilter): RuntimeCodeVulnerabilityPage {
  const rows = allIast().filter((v) => statusOk(f, v.status) && sevOk(f, v.severity) && (!f.service || v.services.includes(f.service)) && textOk(f, [v.title, v.path, v.method, v.services.join(' ')]));
  const p = page(rows, f);
  return { total: p.total, vulnerabilities: p.rows };
}

function advisoryRows(): AdvisorySummary[] {
  const impacted = new Set(allLibraryVulns().map((v) => v.advisoryId));
  return [...ADVISORIES]
    .sort((a, b) => a.daysAgo - b.daysAgo)
    .map((a) => ({
      id: a.id,
      cve: a.cve,
      aliases: [a.cve],
      summary: a.summary,
      severity: severityOf(a.score),
      score: a.score,
      packages: [a.pkg],
      ecosystems: [a.ecosystem],
      published: now - a.daysAgo * DAY,
      impacted: impacted.has(a.id),
    }));
}

export function advisories(f: { query: string; severity: string[]; impact: string; ecosystem: string; offset: number; limit: number }): AdvisoryPage {
  const cf: CsFilter = { query: f.query, severity: f.severity, offset: f.offset, limit: f.limit };
  const rows = advisoryRows().filter(
    (a) =>
      sevOk(cf, a.severity) &&
      (!f.ecosystem || a.ecosystems.includes(f.ecosystem)) &&
      (f.impact === 'impacted' ? a.impacted : f.impact === 'not_impacted' ? !a.impacted : true) &&
      textOk(cf, [a.id, a.aliases.join(' '), a.summary, a.packages.join(' ')]),
  );
  const p = page(rows, cf);
  return { total: p.total, advisories: p.rows };
}

export function advisory(id: string): AdvisoryDetail {
  const a = ADVISORIES.find((x) => x.id === id);
  if (!a) throw new Error('advisory not found');
  return {
    id: a.id,
    aliases: [a.cve],
    summary: a.summary,
    details: `${a.summary}. Versions from ${a.introduced} up to (not including) ${a.fixed} are affected; upgrade to ${a.fixed} or later. This is a fictional advisory of the ObserveCraft demo.`,
    severity: severityOf(a.score),
    score: a.score,
    vector: 'CVSS:3.1/AV:N/AC:L/PR:N/UI:N/S:U/C:H/I:H/A:N',
    cwes: [a.cwe],
    refs: [{ type: 'ADVISORY', url: `https://advisories.example.com/${a.id}` }],
    published: now - a.daysAgo * DAY,
    modified: now - a.daysAgo * DAY,
    source: 'demo',
    packages: [{ ecosystem: a.ecosystem, package: a.pkg, ranges: [{ introduced: a.introduced, fixed: a.fixed }], fixed: a.fixed }],
    vulnerabilities: allLibraryVulns().filter((v) => v.advisoryId === id),
    servicesAnalyzed: 5,
    repositoriesAnalyzed: 4,
  };
}

const counts = (sevs: string[]): SeverityCounts => {
  const c: SeverityCounts = { critical: 0, high: 0, medium: 0, low: 0, unknown: 0 };
  for (const s of sevs) c[(s in c ? s : 'unknown') as keyof SeverityCounts]++;
  return c;
};

function groupBy(pairs: [string, string][]) {
  const m = new Map<string, { name: string; count: number; critical: number; high: number }>();
  for (const [name, sev] of pairs) {
    if (!name) continue;
    const g = m.get(name) ?? { name, count: 0, critical: 0, high: 0 };
    g.count++;
    if (sev === 'critical') g.critical++;
    if (sev === 'high') g.high++;
    m.set(name, g);
  }
  return [...m.values()].sort((a, b) => b.critical - a.critical || b.high - a.high || b.count - a.count).slice(0, 10);
}

export function summary(): CodeSecuritySummary {
  const libs = allLibraryVulns().filter((v) => v.status === 'open');
  const f = allFindings().filter((x) => x.status === 'open');
  const iast = allIast().filter((x) => x.status === 'open');
  const today = Math.floor(now / DAY) * DAY;
  return {
    libraries: counts(libs.map((v) => v.severity)),
    code: counts(f.filter((x) => x.kind === 'sast').map((x) => x.severity)),
    runtimeCode: counts(iast.map((x) => x.severity)),
    secrets: counts(f.filter((x) => x.kind === 'secret').map((x) => x.severity)),
    iac: counts(f.filter((x) => x.kind === 'iac').map((x) => x.severity)),
    inRepositories: libs.filter((v) => v.repositories.length).length,
    inProduction: libs.filter((v) => v.inProduction).length,
    fixable: libs.filter((v) => v.fixed).length,
    repositoriesAnalyzed: 4,
    servicesAnalyzed: 5,
    emerging: advisoryRows().filter((a) => a.published! > now - 30 * DAY && (a.severity === 'critical' || a.severity === 'high')),
    byService: groupBy(libs.flatMap((v) => v.services.map((s) => [s, v.severity] as [string, string]))),
    byRepository: groupBy([...libs.flatMap((v) => v.repositories.map((s) => [s, v.severity] as [string, string])), ...f.map((x) => [x.repository, x.severity] as [string, string])]),
    history: Array.from({ length: 30 }, (_, i) => {
      const day = today - (29 - i) * DAY;
      const end = day + DAY;
      return {
        day,
        libraries: libs.filter((v) => v.firstSeen < end).length,
        code: f.filter((x) => x.kind !== 'secret' && x.firstSeen < end).length,
        secrets: f.filter((x) => x.kind === 'secret' && x.firstSeen < end).length,
      };
    }),
  };
}

export function repositories(): RepositorySummary[] {
  const vulns = allLibraryVulns();
  const f = allFindings();
  return [...new Set(LIBS.filter((l) => l.source === 'static').map((l) => l.repository))].map((repository) => {
    const open = (k: string) => f.filter((x) => x.repository === repository && x.kind === k && x.status === 'open');
    return {
      repository,
      codeVulnerabilities: open('sast').filter((x) => x.cwes.length).length,
      codeViolations: open('sast').filter((x) => !x.cwes.length).length,
      libraryVulnerabilities: vulns.filter((v) => v.status === 'open' && v.repositories.includes(repository)).length,
      libraries: LIBS.filter((l) => l.source === 'static' && l.repository === repository).length,
      secrets: open('secret').length,
      iacViolations: open('iac').length,
      lastScan: now - 3600_000,
      lastBranch: 'main',
      lastCommit: '4f2a9c1',
    };
  });
}

export function inventoryLibraries(view: 'static' | 'runtime', f: CsFilter): InventoryPage {
  const vulns = allLibraryVulns().filter((v) => v.status === 'open');
  const by = new Map<string, InventoryPage['libraries'][number]>();
  for (const l of LIBS.filter((x) => x.source === view)) {
    const k = `${l.ecosystem}|${l.name}|${l.version}`;
    let e = by.get(k);
    if (!e) {
      const vs = vulns.filter((v) => v.ecosystem === l.ecosystem && v.package === l.name && v.version === l.version);
      e = {
        ecosystem: l.ecosystem,
        language: LANG[l.ecosystem],
        name: l.name,
        version: l.version,
        licenses: l.licenses,
        vulnerabilities: vs.length,
        worstSeverity: vs.map((v) => v.severity).sort((a, b) => RANK[b] - RANK[a])[0] ?? '',
        locations: [],
        files: [],
        firstSeen: now - 150 * DAY,
        direct: true,
      };
      by.set(k, e);
    }
    const loc = view === 'runtime' ? l.service : l.repository;
    if (!e.locations.includes(loc)) e.locations.push(loc);
    if (l.file && !e.files.includes(l.file)) e.files.push(l.file);
  }
  const rows = [...by.values()]
    .filter((l) => (!f.language || l.language === f.language) && (!f.severity?.length || sevOk(f, l.worstSeverity)) && textOk(f, [l.name, l.version, l.licenses.join(' ')]))
    .sort((a, b) => b.vulnerabilities - a.vulnerabilities || a.name.localeCompare(b.name));
  const p = page(rows, f);
  return { total: p.total, libraries: p.rows };
}

export function inventoryServices(): InventoryService[] {
  const vulns = allLibraryVulns().filter((v) => v.status === 'open');
  const iast = allIast();
  return [...new Set(LIBS.filter((l) => l.source === 'runtime').map((l) => l.service))].sort().map((service) => {
    const libs = LIBS.filter((l) => l.source === 'runtime' && l.service === service);
    return {
      service,
      language: LANG[libs[0].ecosystem],
      envs: [...new Set(libs.map((l) => l.env))],
      libraries: libs.length,
      vulnerabilities: counts(vulns.filter((v) => v.services.includes(service)).map((v) => v.severity)),
      runtimeCodeVulnerabilities: iast.filter((v) => v.services.includes(service)).length,
      lastSeen: now - 60_000,
    };
  });
}

export function rules(kind: FindingKind, f: CsFilter): RuleSummary[] {
  const all = allFindings();
  const seen = new Map<string, RuleSummary>();
  for (const x of all.filter((y) => y.kind === kind)) {
    const r = seen.get(x.ruleId) ?? {
      kind,
      tool: x.tool,
      ruleId: x.ruleId,
      name: x.ruleName,
      description: x.description,
      severity: x.severity,
      cwes: x.cwes,
      helpUri: '',
      language: x.ruleId.split('-')[0] === 'java' ? 'java' : x.ruleId.startsWith('python') ? 'python' : '',
      openFindings: 0,
      repositories: [],
    };
    if (x.status === 'open') r.openFindings++;
    if (!r.repositories.includes(x.repository)) r.repositories.push(x.repository);
    seen.set(x.ruleId, r);
  }
  return [...seen.values()].filter((r) => sevOk(f, r.severity) && (!f.language || r.language === f.language) && textOk(f, [r.ruleId, r.name, r.cwes.join(' ')]));
}

export function pullRequests(): PullRequestScan[] {
  return [
    {
      repository: REPO,
      prNumber: '418',
      branch: 'feature/gift-cards',
      baseBranch: 'main',
      commitSha: 'b71e0d3',
      receivedAt: now - 2 * 3600_000,
      tool: 'datadog-static-analyzer',
      findings: allFindings(true),
    },
  ];
}

export function triage(key: string, status: 'open' | 'muted'): void {
  triaged.set(key, status);
}

export function setup(): CodeSecuritySetup {
  return { runtimeServices: 5, repositoriesWithLibraries: 4, repositoriesWithCodeScans: 1, iastServices: 2, advisories: ADVISORIES.length, osvEnabled: false, lastOsvCheck: null };
}
