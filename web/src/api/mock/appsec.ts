// In-browser App & API Protection for mock mode: security traces generated on the schedule of
// oc-tracegen's appsec.rs (a scanner every 30 minutes, injections in between, credential stuffing every
// 45 minutes, blocked requests from a denylisted IP), and what the server derives from them: attackers,
// users, endpoints, API findings (the server's rules), services and the overview. Addresses are
// documentation ranges, people @example.com.
import type {
  ApiEndpointView,
  ApiFinding,
  ApiFindingsPage,
  AppsecOverview,
  AppsecService,
  AppsecUser,
  Attacker,
  AttackerPage,
  BusinessLogic,
  DenylistInput,
  DenylistView,
  EndpointPage,
  FindingRuleStatus,
  SecurityTrace,
  SecurityTracePage,
  TimeRange,
  WafDoc,
  WafKind,
  WafPolicyView,
} from '../appsec/types';
import { parseQuery } from '../../lib/searchQuery';
import { rng } from './generator';

const MIN = 60_000;
const DAY = 86_400_000;
const delay = <T>(v: T, ms = 140): Promise<T> => new Promise((r) => setTimeout(() => r(structuredClone(v)), ms));
const fail = (m: string): never => {
  throw new Error(m);
};

const CUSTOMERS: [string, string][] = [
  ['192.0.2.31', 'RO'],
  ['192.0.2.77', 'DE'],
  ['192.0.2.140', 'NL'],
  ['192.0.2.201', 'US'],
];
const ATTACKERS: [string, string][] = [
  ['203.0.113.7', 'US'],
  ['203.0.113.41', 'BR'],
  ['198.51.100.23', 'VN'],
  ['198.51.100.71', 'CA'],
  ['203.0.113.150', 'IN'],
  ['198.51.100.140', 'BE'],
];
const USERS = ['ana.popescu@example.com', 'maya.chen@example.com', 'dev.kumar@example.com', 'lucia.torres@example.com', 'sam.okafor@example.com'];
const ROUTES: [string, string, string][] = [
  ['api-gateway', 'GET', '/api/products'],
  ['api-gateway', 'GET', '/api/products/{id}'],
  ['api-gateway', 'POST', '/api/v1/login'],
  ['api-gateway', 'POST', '/api/orders/{id}/submit'],
  ['storefront-api', 'GET', '/search'],
  ['storefront-api', 'POST', '/api/cart/items'],
  ['storefront-api', 'GET', '/api/account'],
];
const ATTACKS: [string, string, string, string, string][] = [
  ['security_scanner', 'oc-913-001', 'Security scanner detected', 'server.request.headers.no_cookies', 'nuclei'],
  ['sql_injection', 'oc-942-100', 'SQL injection attempt', 'server.request.query.q', "1' OR '1'='1"],
  ['lfi', 'oc-930-100', 'Path traversal attempt', 'server.request.uri.raw', '/../../etc/passwd'],
  ['xss', 'oc-941-110', 'Cross-site scripting attempt', 'server.request.query.q', '<script>alert(1)</script>'],
  ['php_code_injection', 'oc-933-130', 'PHP code injection attempt', 'server.request.body', "<?php system($_GET['c']); ?>"],
  ['ssrf', 'oc-934-100', 'Server-side request forgery attempt', 'server.request.query.url', 'http://169.254.169.254/latest/meta-data/'],
];

let denylist: DenylistView[] = [
  {
    kind: 'ip',
    value: '203.0.113.41',
    reason: 'Repeated SQL injection attempts',
    source: 'Attack attempts on a service',
    expiresAt: null,
    createdBy: 'Priya Shah',
    createdAt: Date.now() - 12 * DAY,
    activity: [],
  },
  { kind: 'ip', value: '198.51.100.201', reason: 'Credential stuffing', source: 'Attackers Explorer', expiresAt: null, createdBy: 'Priya Shah', createdAt: Date.now() - 6 * DAY, activity: [] },
];

/** Security traces of one minute slot, deterministic. */
function slot(minute: number): SecurityTrace[] {
  const r = rng(minute * 7919);
  const pick = <T>(xs: T[]) => xs[Math.floor(r() * xs.length)];
  const out: SecurityTrace[] = [];
  const base = minute * MIN;
  const second = Math.floor(base / 1000);
  const scan = second % 1800 >= 420 && second % 1800 < 540;
  const n = scan ? 40 + Math.floor(r() * 30) : r() < 0.4 ? 1 + Math.floor(r() * 3) : 0;
  const attacker = ATTACKERS[Math.floor(second / 1800) % ATTACKERS.length];
  for (let i = 0; i < n; i++) {
    const [type, id, name, address, value] = scan && r() < 0.7 ? ATTACKS[0] : pick(ATTACKS.slice(1));
    const [service, method, route] = pick(ROUTES);
    const [ip, country] = scan || r() < 0.5 ? attacker : pick(ATTACKERS);
    const blocked = ip === '203.0.113.41';
    out.push({
      traceId: `${minute.toString(16)}${i.toString(16).padStart(4, '0')}`.padStart(32, '0'),
      spanId: `${i}${minute}`.slice(0, 16),
      timestamp: base + Math.floor(r() * MIN),
      service,
      env: 'prod',
      method,
      route,
      url: `https://shop.example.com${route.replace('{id}', '1042')}`,
      status: blocked ? '403' : type === 'security_scanner' ? '404' : type === 'sql_injection' && route === '/search' ? '500' : '400',
      blocked,
      clientIp: ip,
      country,
      userAgent: type === 'security_scanner' ? 'Nuclei - Open-source project (github.com/projectdiscovery/nuclei)' : 'python-requests/2.32.3',
      user: '',
      activities: [`attack_attempt.${type}`, ...(blocked ? ['security_response.block_ip'] : [])],
      triggers: [{ ruleId: id, ruleName: name, type, category: 'attack_attempt', matches: [`${address}: ${value}`] }],
    });
  }
  const stuffing = second % 2700 >= 1200 && second % 2700 < 1320;
  const logins = (stuffing ? 60 : 0) + Math.floor(r() * 4);
  for (let i = 0; i < logins; i++) {
    const bad = stuffing && i < 60;
    const ok = !bad && r() < 0.85;
    const [ip, country] = bad ? ATTACKERS[(Math.floor(second / 2700) + 2) % ATTACKERS.length] : pick(CUSTOMERS);
    out.push({
      traceId: `${minute.toString(16)}l${i}`.padStart(32, '0'),
      spanId: `l${i}${minute}`.slice(0, 16),
      timestamp: base + Math.floor(r() * MIN),
      service: 'api-gateway',
      env: 'prod',
      method: 'POST',
      route: '/api/v1/login',
      url: 'https://shop.example.com/api/v1/login',
      status: ok ? '200' : '401',
      blocked: false,
      clientIp: ip,
      country,
      userAgent: bad ? 'python-requests/2.32.3' : 'Mozilla/5.0 (Macintosh; Intel Mac OS X 14_5)',
      user: ok ? pick(USERS) : '',
      activities: [`business_logic.users.login.${ok ? 'success' : 'failure'}`],
      triggers: [],
    });
  }
  return out;
}

function between(range: TimeRange): SecurityTrace[] {
  const from = Math.max(range.from, Date.now() - 8 * DAY);
  const to = Math.min(range.to, Date.now());
  const out: SecurityTrace[] = [];
  for (let m = Math.floor(from / MIN); m * MIN <= to; m++) for (const t of slot(m)) if (t.timestamp >= from && t.timestamp <= to) out.push(t);
  return out.sort((a, b) => b.timestamp - a.timestamp);
}

function value(t: SecurityTrace, key: string): string[] {
  switch (key.replace(/^@/, '')) {
    case 'service':
      return [t.service];
    case 'env':
      return [t.env];
    case 'appsec.type':
      return t.triggers.map((x) => x.type);
    case 'appsec.category':
      return t.triggers.map((x) => x.category);
    case 'http.status_code':
      return [t.status];
    case 'appsec.blocked':
      return [String(t.blocked)];
    case 'http.client_ip':
      return [t.clientIp];
    case 'country':
      return [t.country];
    case 'http.route':
      return [t.route];
    case 'usr.id':
      return [t.user];
    default:
      return [];
  }
}

function filter(ts: SecurityTrace[], query: string) {
  const terms = parseQuery(query);
  return ts.filter((t) =>
    terms.every((term) => {
      const hit =
        term.kind === 'text'
          ? `${t.route} ${t.activities.join(' ')}`.toLowerCase().includes(term.text.toLowerCase())
          : term.values.some((v) => {
              const re = new RegExp(
                `^${v
                  .split('*')
                  .map((x) => x.replace(/[.+?^${}()|[\]\\]/g, '\\$&'))
                  .join('.*')}$`,
                'i',
              );
              return value(t, term.key).some((x) => re.test(x));
            });
      return hit !== term.negate;
    }),
  );
}

const count = (xs: string[]) => {
  const m = new Map<string, number>();
  for (const x of xs) if (x) m.set(x, (m.get(x) ?? 0) + 1);
  return [...m.entries()].map(([name, n]) => ({ name, count: n })).sort((a, b) => b.count - a.count);
};

export async function traces(range: TimeRange, query: string): Promise<SecurityTracePage> {
  const all = filter(between(range), query);
  const span = Math.max(range.to - range.from, MIN);
  const step = [MIN, 5 * MIN, 15 * MIN, 30 * MIN, 60 * MIN, 3 * 60 * MIN, 6 * 60 * MIN, DAY].find((s) => span / s <= 120) ?? DAY;
  const hist = new Map<number, Record<string, number>>();
  for (const t of all) {
    const b = t.timestamp - (t.timestamp % step);
    const c = hist.get(b) ?? {};
    for (const a of t.activities) c[a] = (c[a] ?? 0) + 1;
    hist.set(b, c);
  }
  const keys = ['service', 'env', '@appsec.type', '@appsec.category', '@http.status_code', '@appsec.blocked', '@http.client_ip', 'country', '@http.route'];
  return delay({
    total: all.length,
    traces: all.slice(0, 200),
    facets: keys.map((key) => ({ key, values: count(all.flatMap((t) => value(t, key))).slice(0, 25) })),
    histogram: [...hist.entries()].sort((a, b) => a[0] - b[0]).map(([ts, counts]) => ({ ts, counts })),
    topActivities: count(all.flatMap((t) => t.activities)).slice(0, 8),
    topUsers: count(all.map((t) => t.user)).slice(0, 5),
  });
}

function attackerList(range: TimeRange, query: string): Attacker[] {
  const by = new Map<string, SecurityTrace[]>();
  for (const t of filter(between(range), query)) by.set(t.clientIp, [...(by.get(t.clientIp) ?? []), t]);
  const deny = new Set(denylist.filter((d) => d.kind === 'ip').map((d) => d.value));
  return [...by.entries()]
    .map(([ip, ts]) => {
      const types = [...new Set(ts.flatMap((t) => t.triggers.map((x) => x.type)))];
      const attacks = ts.filter((t) => t.triggers.length).length;
      return {
        ip,
        country: ts[0].country,
        services: [...new Set(ts.map((t) => t.service))],
        envs: ['prod'],
        userAgents: [...new Set(ts.map((t) => t.userAgent))].slice(0, 5),
        attackTypes: types,
        traces: ts.length,
        attacks,
        blockedTraces: ts.filter((t) => t.blocked).length,
        firstSeen: ts[ts.length - 1].timestamp,
        lastSeen: ts[0].timestamp,
        denylisted: deny.has(ip),
        suspicious: attacks >= 20 && types.length >= 2,
      };
    })
    .sort((a, b) => b.attacks - a.attacks || b.traces - a.traces);
}

export async function attackers(range: TimeRange, query: string): Promise<AttackerPage> {
  const list = attackerList(range, query);
  return delay({ attackers: list, suspicious: list.filter((a) => a.suspicious && !a.denylisted).length, flagged: list.filter((a) => a.denylisted).length });
}

export async function users(range: TimeRange, query: string): Promise<AppsecUser[]> {
  const by = new Map<string, SecurityTrace[]>();
  for (const t of filter(between(range), query)) if (t.user) by.set(t.user, [...(by.get(t.user) ?? []), t]);
  return delay(
    [...by.entries()].map(([user, ts]) => {
      const countries = [...new Set(ts.map((t) => t.country))];
      return {
        user,
        ips: [...new Set(ts.map((t) => t.clientIp))],
        countries,
        services: [...new Set(ts.map((t) => t.service))],
        loginSuccess: ts.filter((t) => t.activities.includes('business_logic.users.login.success')).length,
        loginFailure: 0,
        attacks: 0,
        traces: ts.length,
        lastSeen: ts[0].timestamp,
        risks: countries.length > 1 ? ['new_geolocation'] : [],
      };
    }),
  );
}

// ---------------------------------------------------------------- endpoints and findings

const RULES: Omit<FindingRuleStatus, 'evaluated' | 'failing'>[] = [
  {
    id: 'oc-api-001',
    name: 'Unauthenticated endpoint targeted by attacks',
    severity: 'critical',
    owasp: 'API2',
    description: 'An endpoint that changes data receives attack payloads and none of its requests carries an authenticated user.',
    remediation: 'Require authentication on the endpoint, or set the user on its spans (usr.id) if it already does.',
  },
  {
    id: 'oc-api-002',
    name: 'Authentication route is not protected by account takeover detection',
    severity: 'high',
    owasp: 'API2',
    description: 'A login route sends no login success or failure events, so credential stuffing and brute force go unnoticed.',
    remediation: "Track login events on the route (the tracers' user tracking SDK, or a custom In-App WAF rule).",
  },
  {
    id: 'oc-api-003',
    name: 'Endpoint exposed to attacks without blocking',
    severity: 'high',
    owasp: 'API8',
    description: 'The endpoint receives attack payloads and none was blocked.',
    remediation: 'Assign the service a blocking policy, or block the attacking IPs.',
  },
  {
    id: 'oc-api-004',
    name: 'Endpoint returns server errors to attack payloads',
    severity: 'medium',
    owasp: 'API8',
    description: "Attack payloads make the endpoint fail with 5xx errors: input reaches code that doesn't expect it.",
    remediation: 'Validate the input before it reaches the failing code, and look at the errors in APM.',
  },
  {
    id: 'oc-api-005',
    name: 'Endpoint not protected by App & API Protection',
    severity: 'medium',
    owasp: 'API9',
    description: "The service doesn't run App & API Protection, so attacks on the endpoint are neither detected nor blocked.",
    remediation: "Enable App & API Protection in the service's tracer (DD_APPSEC_ENABLED=true).",
  },
  {
    id: 'oc-api-006',
    name: 'Missing Strict Transport Security HTTP header',
    severity: 'low',
    owasp: 'API8',
    description: "Responses don't set Strict-Transport-Security, so browsers may reach the API over plain HTTP.",
    remediation: 'Set Strict-Transport-Security: max-age=31536000; includeSubDomains on responses.',
  },
  {
    id: 'oc-api-007',
    name: 'Missing X-Frame-Options HTTP header',
    severity: 'low',
    owasp: 'API8',
    description: "Responses don't set X-Frame-Options, so pages can be framed by other sites (clickjacking).",
    remediation: 'Set X-Frame-Options: DENY (or a frame-ancestors Content-Security-Policy).',
  },
  {
    id: 'oc-api-008',
    name: 'Endpoint scanned by many clients',
    severity: 'info',
    owasp: 'API4',
    description: 'Attacks on the endpoint come from many client IPs: it is a known target.',
    remediation: 'Rate limit the endpoint and review what it exposes.',
  },
];
const OTHER: [string, string, string, boolean][] = [
  ['order-service', 'POST', '/orders', false],
  ['billing', 'POST', '/payments/charge', false],
  ['catalog-service', 'GET', '/catalog/items', false],
  ['inventory-service', 'POST', '/inventory/reserve', false],
];
const triageMap = new Map<string, string>();

function allEndpoints(): ApiEndpointView[] {
  const ts = between({ from: Date.now() - 7 * DAY, to: Date.now() });
  const eps: ApiEndpointView[] = ROUTES.map(([service, method, route], i) => {
    const mine = ts.filter((t) => t.service === service && t.route === route);
    const attacks = mine.filter((t) => t.triggers.length);
    return {
      service,
      env: 'prod',
      method,
      route,
      hosts: ['shop.example.com'],
      hits: 40_000 + i * 7_300 + mine.length,
      attacks: attacks.length,
      blocked: attacks.filter((t) => t.blocked).length,
      clientIps: new Set(attacks.map((t) => t.clientIp)).size,
      authenticated: route === '/api/account' || route === '/api/cart/items' ? 3_000 : 0,
      loginEvents: route === '/api/v1/login' ? mine.length : 0,
      protected: true,
      headersSeen: true,
      hsts: service === 'storefront-api',
      frameOptions: service === 'storefront-api',
      serverErrorsOnAttacks: attacks.filter((t) => t.status === '500').length,
      lastSeen: Date.now() - 30_000,
      findings: {},
    };
  });
  for (const [service, method, route] of OTHER)
    eps.push({
      service,
      env: 'prod',
      method,
      route,
      hosts: [],
      hits: 12_000,
      attacks: 0,
      blocked: 0,
      clientIps: 0,
      authenticated: 0,
      loginEvents: 0,
      protected: false,
      headersSeen: false,
      hsts: false,
      frameOptions: false,
      serverErrorsOnAttacks: 0,
      lastSeen: Date.now() - 60_000,
      findings: {},
    });
  return eps;
}

const writes = (e: ApiEndpointView) => ['POST', 'PUT', 'PATCH', 'DELETE'].includes(e.method);
const isAuth = (e: ApiEndpointView) => e.method === 'POST' && /login|signin|auth|token|session/i.test(e.route);
const CHECKS: Record<string, (e: ApiEndpointView) => boolean | null> = {
  'oc-api-001': (e) => (writes(e) ? e.attacks > 0 && e.authenticated === 0 : null),
  'oc-api-002': (e) => (isAuth(e) ? e.loginEvents === 0 : null),
  'oc-api-003': (e) => (e.attacks > 0 ? e.blocked === 0 : null),
  'oc-api-004': (e) => (e.attacks > 0 ? e.serverErrorsOnAttacks > 0 : null),
  'oc-api-005': (e) => !e.protected,
  'oc-api-006': (e) => (e.headersSeen ? !e.hsts : null),
  'oc-api-007': (e) => (e.headersSeen ? !e.frameOptions : null),
  'oc-api-008': (e) => (e.attacks > 0 ? e.clientIps >= 10 : null),
};

function evaluate() {
  const eps = allEndpoints();
  const findings: ApiFinding[] = [];
  const rules = RULES.map((r) => {
    let evaluated = 0;
    let failing = 0;
    for (const e of eps) {
      const res = CHECKS[r.id](e);
      if (res === null) continue;
      evaluated++;
      if (!res) continue;
      failing++;
      const key = `api-finding:${r.id}:${e.service}:${e.method} ${e.route}`;
      findings.push({
        key,
        ruleId: r.id,
        ruleName: r.name,
        severity: r.severity,
        owasp: r.owasp,
        service: e.service,
        env: e.env,
        method: e.method,
        route: e.route,
        status: triageMap.get(key) ?? 'open',
      });
    }
    return { ...r, evaluated, failing };
  });
  for (const f of findings.filter((x) => x.status === 'open')) {
    const e = eps.find((x) => x.service === f.service && x.method === f.method && x.route === f.route)!;
    e.findings[f.severity] = (e.findings[f.severity] ?? 0) + 1;
  }
  return { eps, findings, rules };
}

export async function findings(): Promise<ApiFindingsPage> {
  const { findings: f, rules } = evaluate();
  return delay({
    findings: f,
    rules,
    owasp: ['API1', 'API2', 'API3', 'API4', 'API5', 'API6', 'API7', 'API8', 'API9', 'API10'].map((name) => ({ name, count: f.filter((x) => x.owasp === name && x.status === 'open').length })),
  });
}

export async function endpoints(): Promise<EndpointPage> {
  return delay({ endpoints: evaluate().eps });
}

export async function triage(key: string, status: 'open' | 'muted'): Promise<void> {
  triageMap.set(key, status);
  return delay(undefined, 50);
}

export async function services(): Promise<AppsecService[]> {
  const now = Date.now();
  const start = now - (now % DAY) - 6 * DAY;
  const ts = between({ from: start, to: now });
  const names = ['api-gateway', 'storefront-api', 'order-service', 'billing', 'catalog-service', 'inventory-service', 'notification-worker'];
  return delay(
    names.map((service) => {
      const mine = ts.filter((t) => t.service === service);
      const protectedService = service === 'api-gateway' || service === 'storefront-api';
      return {
        service,
        env: 'prod',
        language: service === 'api-gateway' ? 'go' : service === 'billing' ? 'python' : 'java',
        protected: protectedService,
        mode: !protectedService ? 'disabled' : service === 'api-gateway' ? 'blocking' : 'monitoring',
        policy: !protectedService ? '' : service === 'api-gateway' ? 'Block attack tools - gateway' : 'Managed - Monitoring-only',
        securityTraces: mine.length,
        trend: Array.from({ length: 7 }, (_, d) => mine.filter((t) => t.timestamp >= start + d * DAY && t.timestamp < start + (d + 1) * DAY).length),
        vulnerabilities: service === 'storefront-api' ? 3 : 0,
        sca: protectedService,
        iast: false,
      };
    }),
  );
}

export async function overview(range: TimeRange): Promise<AppsecOverview> {
  const { eps, findings: f, rules } = evaluate();
  const open = f.filter((x) => x.status === 'open');
  const bySev: Record<string, number> = { critical: 0, high: 0, medium: 0, low: 0, info: 0 };
  for (const x of open) bySev[x.severity]++;
  const rank = ['info', 'low', 'medium', 'high', 'critical'];
  const ep = new Map<string, number>();
  for (const x of open.filter((y) => ['critical', 'high'].includes(y.severity))) ep.set(`${x.service}|${x.method} ${x.route}`, (ep.get(`${x.service}|${x.method} ${x.route}`) ?? 0) + 1);
  const now = between(range).filter((t) => t.triggers.length);
  const len = range.to - range.from;
  const before = between({ from: range.from - len, to: range.from }).filter((t) => t.triggers.length);
  const trend = (get: (t: SecurityTrace) => string[]) => {
    const b = new Map(count(before.flatMap(get)).map((x) => [x.name, x.count]));
    return count(now.flatMap(get))
      .slice(0, 5)
      .map((x) => ({ ...x, change: b.get(x.name) ? Math.round(((x.count - b.get(x.name)!) / b.get(x.name)!) * 100) : null }));
  };
  const svcs = await services();
  return delay({
    endpoints: eps.length,
    endpointsWithTraffic: eps.length,
    unprotectedEndpoints: eps.filter((e) => !e.protected).length,
    authenticatedEndpoints: eps.filter((e) => e.authenticated > 0).length,
    unauthenticatedEndpoints: eps.filter((e) => e.authenticated === 0 && writes(e)).length,
    exposedEndpoints: eps.filter((e) => e.attacks > 0).length,
    findings: bySev,
    topFindingRules: rules
      .filter((r) => r.failing)
      .sort((a, b) => rank.indexOf(b.severity) - rank.indexOf(a.severity) || b.failing - a.failing)
      .slice(0, 5)
      .map((r) => ({ name: r.name, count: r.failing })),
    threatEndpoints: [...ep.entries()]
      .map(([name, c]) => ({ name, count: c }))
      .sort((a, b) => b.count - a.count)
      .slice(0, 5),
    findingRulesEnabled: RULES.length,
    owaspCovered: [...new Set(RULES.map((r) => r.owasp))],
    servicesProtected: svcs.filter((s) => s.protected).length,
    servicesTotal: svcs.length,
    servicesBlocking: svcs.filter((s) => s.mode === 'blocking').length,
    servicesMonitoring: svcs.filter((s) => s.mode === 'monitoring').length,
    servicesWithAttacks: svcs.filter((s) => s.securityTraces).map((s) => ({ name: s.service, count: s.securityTraces })),
    topAttackTypes: trend((t) => t.triggers.map((x) => x.type)),
    topCountries: trend((t) => [t.country]),
  });
}

export async function businessLogic(): Promise<BusinessLogic> {
  const ts = between({ from: Date.now() - 7 * DAY, to: Date.now() });
  const n = (a: string) => ts.filter((t) => t.activities.includes(a)).length;
  const events = [
    ['users.login.failure', 'Tracks failed user logins.', n('business_logic.users.login.failure')],
    ['users.login.success', 'Tracks successful user logins.', n('business_logic.users.login.success')],
    ['users.signup', 'Tracks user signups.', 0],
    ['users.delete', 'Tracks user deletions.', 0],
    ['users.password_reset', 'Tracks user password resets.', 0],
    ['activity.sensitive', 'Tracks any sensitive activity in the application.', 0],
    ['payment.attempt', 'Tracks payment attempts.', 0],
    ['payment.failure', 'Tracks failed payment attempts.', 0],
  ] as [string, string, number][];
  return delay({
    events: events.map(([event, description, requests7d]) => ({ event, description, requests7d, services: requests7d ? ['api-gateway'] : [] })),
    suggested: [
      { service: 'billing', env: 'prod', route: '/auth/token', event: 'users.login.success' },
      { service: 'billing', env: 'prod', route: '/auth/token', event: 'users.login.failure' },
      { service: 'storefront-api', env: 'prod', route: '/api/account/login', event: 'users.login.success' },
      { service: 'storefront-api', env: 'prod', route: '/api/account/login', event: 'users.login.failure' },
      { service: 'order-service', env: 'staging', route: '/internal/auth', event: 'users.login.success' },
      { service: 'order-service', env: 'staging', route: '/internal/auth', event: 'users.login.failure' },
    ],
  });
}

// ---------------------------------------------------------------- configuration

const MANAGED: [string, string, string, boolean, number, number, number][] = [
  [
    'recommended',
    'Managed - Monitoring-only',
    'Monitor security scanners and application attacks such as Server-Side-Request-Forgery (SSRF), SQL Injection, Log4Shell, and Cross-Site-Scripting (XSS).',
    true,
    0,
    186,
    33,
  ],
  [
    'recommended-attack-tools',
    'Managed - Block attack tools',
    'Block known attack tools without impacting legitimate security scans. Monitor application attacks such as SSRF, SQL Injection, Log4Shell, and XSS.',
    false,
    47,
    139,
    33,
  ],
  [
    'recommended-blocking',
    'Managed - Block attack tools & application attacks',
    'Block known attack tools without impacting legitimate security scans. Block application attacks such as SSRF, SQL Injection, Log4Shell, and XSS.',
    false,
    117,
    69,
    33,
  ],
];
const meta = (by = 'Priya Shah') => ({ added_at: new Date(Date.now() - 9 * DAY).toISOString(), added_by_name: by });
let waf: Record<WafKind, WafDoc[]> = {
  policy: [
    {
      id: 'pol-0001',
      type: 'policy',
      attributes: {
        name: 'Block attack tools - gateway',
        description: 'Blocks scanners on the public gateway.',
        protectionPresets: ['attack-tools'],
        scope: [{ service: 'api-gateway', env: 'prod' }],
        rules: [],
        version: 0,
        isDefault: false,
      },
      meta: meta(),
    },
  ],
  exclusion_filter: [
    {
      id: 'exf-0001',
      type: 'exclusion_filter',
      attributes: {
        description: 'Internal vulnerability scanner (weekly scan)',
        enabled: true,
        ip_list: ['192.0.2.250'],
        on_match: 'monitor',
        search_query: '@http.client_ip:192.0.2.250',
        metadata: meta(),
      },
    },
    {
      id: 'exf-0002',
      type: 'exclusion_filter',
      attributes: {
        description: 'Search accepts code snippets',
        enabled: true,
        path_glob: '/search',
        parameters: ['q'],
        rules_target: [{ tags: { category: 'attack_attempt', type: 'xss' } }],
        scope: [{ env: 'prod', service: 'storefront-api' }],
        metadata: meta('Alex Morgan'),
      },
    },
  ],
  custom_rule: [
    {
      id: 'cr-0001',
      type: 'custom_rule',
      attributes: {
        name: 'Track users.login.failure on api-gateway - /api/v1/login',
        enabled: true,
        blocking: false,
        path_glob: '/api/v1/login',
        scope: [{ env: 'prod', service: 'api-gateway' }],
        conditions: [{ operator: 'match_regex', parameters: { inputs: [{ address: 'server.response.status' }], regex: '^401$', options: {} } }],
        tags: { category: 'business_logic', type: 'users.login.failure' },
        metadata: meta(),
      },
    },
  ],
};

export async function policies(): Promise<WafPolicyView[]> {
  const svcs = await services();
  return delay([
    ...MANAGED.map(([id, name, description, isDefault, blocking, monitoring, disabled]) => ({
      id,
      name,
      description,
      managed: true,
      isDefault,
      blocking,
      monitoring,
      disabled,
      services: svcs.filter((s) => s.policy === name).map((s) => s.service),
      modifiedBy: 'ObserveCraft',
    })),
    ...waf.policy.map((p) => {
      const presets = (p.attributes.protectionPresets as string[]) ?? [];
      const [, , , , b, m, d] = presets.includes('application-attacks') ? MANAGED[2] : presets.includes('attack-tools') ? MANAGED[1] : MANAGED[0];
      return {
        id: p.id,
        name: String(p.attributes.name),
        description: String(p.attributes.description ?? ''),
        managed: false,
        isDefault: false,
        blocking: b,
        monitoring: m,
        disabled: d,
        services: svcs.filter((s) => s.policy === p.attributes.name).map((s) => s.service),
        modifiedBy: p.meta?.added_by_name as string,
      };
    }),
  ]);
}

export async function wafList(kind: WafKind): Promise<WafDoc[]> {
  return delay(waf[kind]);
}

function check(kind: WafKind, a: Record<string, unknown>) {
  if (kind !== 'exclusion_filter' && !String(a.name ?? '').trim()) fail('The name is required');
  if (kind === 'exclusion_filter' && a.on_match === 'monitor' && (a.path_glob || a.parameters || a.rules_target || a.scope)) fail('only IPs are supported for monitored exclusion filters');
}

export async function wafCreate(kind: WafKind, attributes: Record<string, unknown>): Promise<WafDoc> {
  check(kind, attributes);
  const d: WafDoc = { id: `${kind}-${Math.random().toString(36).slice(2, 8)}`, type: kind, attributes: { ...attributes, metadata: meta('Alex Morgan') } };
  waf = { ...waf, [kind]: [...waf[kind], d] };
  return delay(d);
}

export async function wafUpdate(kind: WafKind, id: string, attributes: Record<string, unknown>): Promise<WafDoc> {
  check(kind, attributes);
  const cur = waf[kind].find((d) => d.id === id) ?? fail(`no ${kind} ${id}`);
  const d = { ...cur, attributes: { ...attributes, metadata: { ...cur.attributes.metadata, modified_at: new Date().toISOString(), modified_by_name: 'Alex Morgan' } } };
  waf = { ...waf, [kind]: waf[kind].map((x) => (x.id === id ? d : x)) };
  return delay(d);
}

export async function wafDelete(kind: WafKind, id: string): Promise<void> {
  waf = { ...waf, [kind]: waf[kind].filter((x) => x.id !== id) };
  return delay(undefined);
}

async function denylistView(): Promise<DenylistView[]> {
  const now = Date.now();
  const start = now - (now % DAY) - 9 * DAY;
  const ts = between({ from: start, to: now });
  return denylist.map((d) => ({
    ...d,
    activity: Array.from({ length: 10 }, (_, i) => ts.filter((t) => t.clientIp === d.value && t.timestamp >= start + i * DAY && t.timestamp < start + (i + 1) * DAY).length),
  }));
}

export async function denylistEntries(): Promise<DenylistView[]> {
  return delay(await denylistView());
}

export async function block(input: DenylistInput): Promise<void> {
  for (const v of input.values) {
    denylist = [
      {
        kind: input.kind,
        value: v,
        reason: input.reason ?? '',
        source: 'Attackers Explorer',
        expiresAt: input.durationMinutes ? Date.now() + input.durationMinutes * MIN : null,
        createdBy: 'Alex Morgan',
        createdAt: Date.now(),
        activity: [],
      },
      ...denylist.filter((d) => !(d.kind === input.kind && d.value === v)),
    ];
  }
  return delay(undefined);
}

export async function unblock(kind: string, value: string): Promise<void> {
  denylist = denylist.filter((d) => !(d.kind === kind && d.value === value));
  return delay(undefined);
}

const settings = new Map<string, unknown>([
  ['custom_response', { mode: 'tracer', status: 403, redirectUrl: '', redirectStatus: 302 }],
  ['blocking_allowed', true],
]);

export async function setting<T>(key: string): Promise<T> {
  return delay((settings.get(key) ?? {}) as T);
}

export async function setSetting(key: string, value: unknown): Promise<void> {
  settings.set(key, value);
  return delay(undefined);
}
