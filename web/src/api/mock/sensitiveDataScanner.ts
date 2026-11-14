// In-browser Sensitive Data Scanner for mock mode (Organization Settings → Sensitive Data Scanner). State lives in
// memory for the session; validation and the rule tester follow the server (sds/config.rs, sds/mod.rs) closely
// enough to try the pages: standard patterns are JavaScript approximations of the server's library (sds/patterns.rs).
import type { SdsConfiguration, SdsGroup, SdsGroupInput, SdsReplacement, SdsRule, SdsRuleInput, SdsStandardPattern, SdsTestRequest, SdsTestResult } from '../sensitiveDataScanner/types';

const GROUP_LIMIT = 50;
const PRODUCTS = ['logs', 'rum', 'events', 'apm'];
const KINDS = ['none', 'hash', 'replacement_string', 'partial_replacement_from_beginning', 'partial_replacement_from_end'];

const fail = (message: string): never => {
  throw new Error(message);
};

interface Library extends SdsStandardPattern {
  /** JavaScript source and flags of the pattern, and the group holding the sensitive part. */
  js: [string, string, number];
  check?: (s: string) => boolean;
}

function luhn(s: string): boolean {
  const d = s.replace(/\D/g, '');
  if (d.length < 13 || d.length > 19) return false;
  let sum = 0;
  for (let i = 0; i < d.length; i++) {
    let n = Number(d[d.length - 1 - i]);
    if (i % 2 === 1) n = n * 2 > 9 ? n * 2 - 9 : n * 2;
    sum += n;
  }
  return sum % 10 === 0 && /^(4|5[1-5]|2[2-7]|3[47]|6|3[05689])/.test(d);
}

const p = (id: string, name: string, category: string, priority: number, js: [string, string, number], keywords: string[], description: string, check?: (s: string) => boolean): Library => ({
  id,
  name,
  category,
  priority,
  pattern: js[0],
  tags: [`sensitive_data:${id}`, `sensitive_data_category:${category}`],
  keywords,
  description,
  js,
  check,
});

const LIBRARY: Library[] = [
  p(
    'credit_card',
    'Credit Card Number',
    'finance',
    1,
    [String.raw`\b\d(?:[ -]?\d){12,18}\b`, 'g', 0],
    ['card', 'credit', 'debit', 'visa', 'mastercard', 'amex', 'pan', 'cc'],
    'Payment card numbers of the major networks (Visa, Mastercard, American Express, Discover, JCB, Diners Club, UnionPay), written with or without spaces or dashes, that pass the Luhn checksum.',
    luhn,
  ),
  p(
    'iban',
    'International Bank Account Number (IBAN)',
    'finance',
    2,
    [String.raw`\b[A-Z]{2}\d{2}(?: ?[A-Z0-9]{4}){2,7}(?: ?[A-Z0-9]{1,3})?\b`, 'g', 0],
    ['iban', 'account', 'bank', 'transfer', 'sepa'],
    'International bank account numbers, grouped by four or not, that pass the mod 97 checksum.',
  ),
  p(
    'us_ssn',
    'US Social Security Number',
    'pii',
    1,
    [String.raw`\b\d{3}-\d{2}-\d{4}\b`, 'g', 0],
    ['ssn', 'social security', 'social_security'],
    'US social security numbers written as AAA-GG-SSSS, without the ranges that are never issued (000, 666 and 9xx areas, 00 groups, 0000 serials).',
    (s) => !/^(000|666|9)/.test(s) && !/-00-/.test(s) && !s.endsWith('-0000'),
  ),
  p(
    'email_address',
    'Email Address',
    'pii',
    3,
    [String.raw`\b[A-Za-z0-9._%+-]+@[A-Za-z0-9-]+(?:\.[A-Za-z0-9-]+)*\.[A-Za-z]{2,24}\b`, 'g', 0],
    ['email', 'e-mail', 'mail', 'address', 'user'],
    'Email addresses with a domain name, leaving out image names such as image@2x.png.',
    (s) => !/\.(png|jpe?g|gif|svg|webp|ico|css|js|map|ts|json|html|pdf|txt)$/i.test(s),
  ),
  p(
    'phone_number',
    'Phone Number',
    'pii',
    3,
    [String.raw`\+\d{1,3}[ .-]?(?:\(\d{1,4}\)|\d{1,4})(?:[ .-]?\d{2,4}){2,4}\b|\(\d{3}\) ?\d{3}[ .-]\d{4}\b|\b\d{3}[.-]\d{3}[.-]\d{4}\b`, 'g', 0],
    ['phone', 'tel', 'mobile', 'cell', 'fax', 'call'],
    'Phone numbers in international form (+ and the country code) or written as North American numbers with separators.',
  ),
  p(
    'ipv4_address',
    'IPv4 Address',
    'network',
    4,
    [String.raw`\b(?:(?:25[0-5]|2[0-4]\d|1\d\d|[1-9]?\d)\.){3}(?:25[0-5]|2[0-4]\d|1\d\d|[1-9]?\d)\b`, 'g', 0],
    ['ip', 'address', 'client', 'remote', 'host'],
    'IPv4 addresses in dotted-decimal form, not part of a longer dotted version number.',
  ),
  p(
    'ipv6_address',
    'IPv6 Address',
    'network',
    4,
    [String.raw`\b(?:[0-9A-Fa-f]{1,4}:){2,7}[0-9A-Fa-f]{1,4}\b|\b(?:[0-9A-Fa-f]{1,4}:){1,6}:[0-9A-Fa-f]{1,4}\b`, 'g', 0],
    ['ip', 'ipv6', 'address', 'client', 'remote'],
    'IPv6 addresses in full or compressed form.',
  ),
  p(
    'aws_access_key_id',
    'AWS Access Key ID',
    'credentials',
    2,
    [String.raw`\b(?:AKIA|ASIA|ABIA|ACCA|AGPA|AIDA|AIPA|ANPA|ANVA|AROA|APKA)[A-Z2-7]{16}\b`, 'g', 0],
    ['aws', 'access', 'key', 'id'],
    'AWS access key ids, long-term (AKIA) and temporary (ASIA).',
  ),
  p(
    'aws_secret_access_key',
    'AWS Secret Access Key',
    'credentials',
    1,
    [String.raw`(?:aws_?)?secret_?access_?key["']?\s*[:=]\s*["']?([A-Za-z0-9/+]{40})`, 'gi', 1],
    ['aws', 'secret', 'access', 'key'],
    'AWS secret access keys assigned to a secret access key setting or field.',
  ),
  p(
    'private_key',
    'Private Key',
    'credentials',
    1,
    [String.raw`-----BEGIN [A-Z0-9 ]*PRIVATE KEY-----[\s\S]*?(?:-----END [A-Z0-9 ]*PRIVATE KEY-----|$)`, 'g', 0],
    ['private', 'key', 'pem', 'rsa', 'ssh'],
    'PEM private keys (RSA, EC, DSA, OpenSSH, PKCS #8, PGP), whole or cut off.',
  ),
  p(
    'jwt',
    'JSON Web Token',
    'credentials',
    2,
    [String.raw`\beyJ[A-Za-z0-9_-]+\.eyJ[A-Za-z0-9_-]+\.[A-Za-z0-9_-]*`, 'g', 0],
    ['jwt', 'token', 'bearer', 'authorization'],
    'JSON Web Tokens: a JSON header and payload, Base64url encoded, and their signature.',
  ),
  p(
    'url_credentials',
    'Password in URL',
    'credentials',
    1,
    [String.raw`\b[A-Za-z][A-Za-z0-9+.-]*://[^\s:/@]+:([^\s@/]+)@[^\s/@]+`, 'g', 1],
    ['url', 'uri', 'dsn', 'connection'],
    "The password of a URL's user information (scheme://user:password@host).",
  ),
  p(
    'password_assignment',
    'Password Assignment',
    'credentials',
    2,
    [String.raw`\b(?:password|passwd|pwd|pass|secret)["']?\s*[:=]\s*["']?([^\s"'&,;}]+)`, 'gi', 1],
    ['password', 'passwd', 'secret'],
    'Values assigned to a password or secret field in query strings, JSON and key-value text.',
  ),
  p(
    'github_token',
    'GitHub Token',
    'credentials',
    1,
    [String.raw`\b(?:gh[pousr]_[A-Za-z0-9]{36}|github_pat_[A-Za-z0-9_]{82})\b`, 'g', 0],
    ['github', 'token'],
    'GitHub personal access, OAuth, app and refresh tokens.',
  ),
  p(
    'gitlab_token',
    'GitLab Token',
    'credentials',
    1,
    [String.raw`\bgl(?:pat|dt|rt|ptt|cbt|oas)-[A-Za-z0-9_-]{20,}`, 'g', 0],
    ['gitlab', 'token'],
    'GitLab personal, deploy, runner, pipeline trigger and CI job tokens.',
  ),
  p('slack_token', 'Slack Token', 'credentials', 1, [String.raw`\bxox[abposre]-[A-Za-z0-9-]{10,}`, 'g', 0], ['slack', 'token'], 'Slack bot, user, app and configuration tokens.'),
  p(
    'slack_webhook',
    'Slack Webhook URL',
    'credentials',
    2,
    [String.raw`https://hooks\.slack\.com/(?:services|workflows|triggers)/[A-Za-z0-9/_+-]+`, 'g', 0],
    ['slack', 'webhook'],
    'Slack incoming webhook and workflow trigger URLs.',
  ),
  p(
    'stripe_api_key',
    'Stripe Secret Key',
    'credentials',
    1,
    [String.raw`\b[rs]k_(?:live|test)_[A-Za-z0-9]{16,}\b`, 'g', 0],
    ['stripe', 'key', 'secret'],
    'Stripe secret and restricted API keys (publishable keys are public).',
  ),
  p('google_api_key', 'Google API Key', 'credentials', 2, [String.raw`\bAIza[0-9A-Za-z_-]{35}`, 'g', 0], ['google', 'gcp', 'firebase', 'key'], 'Google Cloud and Firebase API keys.'),
  p('sendgrid_api_key', 'SendGrid API Key', 'credentials', 2, [String.raw`\bSG\.[A-Za-z0-9_-]{22}\.[A-Za-z0-9_-]{43}`, 'g', 0], ['sendgrid', 'key'], 'SendGrid API keys.'),
  p('npm_token', 'npm Access Token', 'credentials', 2, [String.raw`\bnpm_[A-Za-z0-9]{36}\b`, 'g', 0], ['npm', 'token', 'registry'], 'npm registry access tokens.'),
  p(
    'openai_api_key',
    'OpenAI API Key',
    'credentials',
    1,
    [String.raw`\bsk-(?:proj-|svcacct-|admin-)?[A-Za-z0-9_-]{20,}`, 'g', 0],
    ['openai', 'key'],
    'OpenAI project, service account, admin and legacy API keys.',
    (s) => !s.startsWith('sk-ant-'),
  ),
  p('anthropic_api_key', 'Anthropic API Key', 'credentials', 1, [String.raw`\bsk-ant-(?:api|admin)\d{2}-[A-Za-z0-9_-]{20,}`, 'g', 0], ['anthropic', 'claude', 'key'], 'Anthropic API and admin keys.'),
  p(
    'observecraft_key',
    'ObserveCraft Key',
    'credentials',
    1,
    [String.raw`\boc(?:ak|ik|s|l|sl|wt|ev|oa)_[A-Za-z0-9_-]{43}`, 'g', 0],
    ['key', 'token', 'observecraft'],
    'ObserveCraft application keys, ingest keys and session tokens.',
  ),
];

const findPattern = (id: string) => LIBRARY.find((l) => l.id === id);

// ---------------------------------------------------------------- state

interface State {
  groups: SdsGroup[];
  version: number;
  seq: number;
}

let state: State | null = null;

const replacement = (kind: string, extra: Partial<SdsReplacement> = {}): SdsReplacement => ({ kind, ...extra });

function seedRule(id: string, groupId: string, name: string, over: Partial<SdsRule>): SdsRule {
  return {
    id,
    groupId,
    name,
    description: '',
    pattern: null,
    standardPatternId: null,
    namespaces: [],
    excludedNamespaces: [],
    tags: [],
    enabled: true,
    priority: null,
    replacement: replacement('none'),
    keywords: [],
    keywordCharacters: null,
    useRecommendedKeywords: false,
    suppressions: { startsWith: [], endsWith: [], exactMatch: [] },
    ...over,
  };
}

function db(): State {
  if (state) return state;
  const g = (id: string, name: string, description: string, query: string, samplingRate: number, rules: SdsRule[]): SdsGroup => ({
    id,
    name,
    description,
    query,
    enabled: true,
    products: ['logs'],
    samplingRate,
    rules,
  });
  state = {
    version: 12,
    seq: 100,
    groups: [
      g('3f1c2a10-0000-4000-8000-000000000001', 'PCI compliance', 'Card numbers in payment and checkout logs.', 'service:(billing OR order-service)', 100, [
        seedRule('7a2e0c01-0000-4000-8000-000000000001', '3f1c2a10-0000-4000-8000-000000000001', 'Card numbers', {
          standardPatternId: 'credit_card',
          priority: 1,
          replacement: replacement('partial_replacement_from_beginning', { numberOfChars: 12 }),
        }),
        seedRule('7a2e0c01-0000-4000-8000-000000000002', '3f1c2a10-0000-4000-8000-000000000001', 'Bank accounts', {
          standardPatternId: 'iban',
          replacement: replacement('replacement_string', { replacementString: '[iban]' }),
          keywords: ['iban', 'refund'],
          keywordCharacters: 30,
        }),
      ]),
      g('3f1c2a10-0000-4000-8000-000000000002', 'Credentials', 'Keys and tokens that should never reach the logs.', '', 100, [
        seedRule('7a2e0c01-0000-4000-8000-000000000003', '3f1c2a10-0000-4000-8000-000000000002', 'AWS access keys', {
          standardPatternId: 'aws_access_key_id',
          replacement: replacement('hash'),
        }),
        seedRule('7a2e0c01-0000-4000-8000-000000000004', '3f1c2a10-0000-4000-8000-000000000002', 'GitHub tokens', {
          standardPatternId: 'github_token',
          replacement: replacement('replacement_string', { replacementString: '[github token]' }),
        }),
        seedRule('7a2e0c01-0000-4000-8000-000000000005', '3f1c2a10-0000-4000-8000-000000000002', 'Internal service tokens', {
          description: 'Tokens of the storefront session service.',
          pattern: String.raw`sess_[A-Za-z0-9]{24}`,
          tags: ['sensitive_data:session_token', 'team:storefront'],
          priority: 2,
          replacement: replacement('replacement_string', { replacementString: '[session]' }),
          suppressions: { startsWith: [], endsWith: [], exactMatch: ['sess_000000000000000000000000'] },
        }),
      ]),
      g('3f1c2a10-0000-4000-8000-000000000003', 'Personal data', 'Customer contact details in the storefront.', 'service:storefront-api env:prod', 50, [
        seedRule('7a2e0c01-0000-4000-8000-000000000006', '3f1c2a10-0000-4000-8000-000000000003', 'Customer emails', {
          standardPatternId: 'email_address',
          namespaces: ['message', 'customer'],
          excludedNamespaces: ['customer.support_alias'],
          replacement: replacement('partial_replacement_from_end', { numberOfChars: 8 }),
          useRecommendedKeywords: true,
        }),
        seedRule('7a2e0c01-0000-4000-8000-000000000007', '3f1c2a10-0000-4000-8000-000000000003', 'Phone numbers', {
          standardPatternId: 'phone_number',
          enabled: false,
          replacement: replacement('none'),
        }),
      ]),
    ],
  };
  return state;
}

const clone = <T>(v: T): T => structuredClone(v);
const nextId = (s: State) => `00000000-0000-4000-8000-${String(++s.seq).padStart(12, '0')}`;

function checkGroup(i: SdsGroupInput): SdsGroupInput {
  const name = i.name.trim();
  if (!name) fail('group name should not be blank');
  const rate = i.samplingRate ?? 100;
  if (rate < 0 || rate > 100) fail('A sampling rate is between 0 and 100');
  const products = (i.products ?? []).length ? i.products! : ['logs'];
  const unknown = products.find((p) => !PRODUCTS.includes(p));
  if (unknown) fail(`Unknown product "${unknown}" (one of ${PRODUCTS.join(', ')})`);
  return { ...i, name, products, samplingRate: rate, query: (i.query ?? '').trim() };
}

function compile(source: string, flags = 'g'): RegExp {
  try {
    return new RegExp(source, flags);
  } catch (e) {
    return fail(`Invalid pattern: ${(e as Error).message}`);
  }
}

function checkRule(i: SdsRuleInput): SdsRuleInput {
  const name = i.name.trim();
  if (!name) fail('rule name should not be blank');
  const std = i.standardPatternId?.trim() || null;
  if (std && !findPattern(std)) fail(`Unknown standard pattern "${std}"`);
  if (!std) {
    if (!i.pattern) fail('A rule needs a pattern or a standard pattern');
    compile(i.pattern!);
  }
  const kind = i.replacement?.kind || 'none';
  if (!KINDS.includes(kind)) fail(`Unknown replacement "${kind}"`);
  if (kind === 'replacement_string' && i.replacement?.replacementString == null) fail('A `replacement_string` replacement needs its `replacement_string`');
  if (kind.startsWith('partial_') && (i.replacement?.numberOfChars == null || i.replacement.numberOfChars < 0)) fail('A partial replacement needs `number_of_chars` (0 or more)');
  if (i.priority != null && (i.priority < 1 || i.priority > 5)) fail('`priority` is a number from 1 (highest) to 5');
  if (i.keywordCharacters != null && (i.keywordCharacters < 1 || i.keywordCharacters > 50)) fail('`character_count` is a number from 1 to 50');
  return { ...i, name, standardPatternId: std, pattern: std ? null : i.pattern };
}

function toRule(id: string, i: SdsRuleInput): SdsRule {
  return seedRule(id, i.groupId, i.name, {
    description: i.description ?? '',
    pattern: i.pattern ?? null,
    standardPatternId: i.standardPatternId ?? null,
    namespaces: (i.namespaces ?? []).filter(Boolean),
    excludedNamespaces: (i.excludedNamespaces ?? []).filter(Boolean),
    tags: (i.tags ?? []).filter(Boolean),
    enabled: i.enabled ?? false,
    priority: i.priority ?? null,
    replacement: i.replacement ?? replacement('none'),
    keywords: i.keywords ?? [],
    keywordCharacters: i.keywordCharacters ?? null,
    useRecommendedKeywords: i.useRecommendedKeywords ?? false,
    suppressions: { startsWith: i.suppressions?.startsWith ?? [], endsWith: i.suppressions?.endsWith ?? [], exactMatch: i.suppressions?.exactMatch ?? [] },
  });
}

const groupOf = (s: State, id: string) => s.groups.find((g) => g.id === id) ?? fail(`Scanning group ${id} not found`);

function ruleOf(s: State, id: string): [SdsGroup, number] {
  for (const g of s.groups) {
    const i = g.rules.findIndex((r) => r.id === id);
    if (i >= 0) return [g, i];
  }
  return fail(`Scanning rule ${id} not found`);
}

const same = (a: string[], b: string[]) => a.length === b.length && [...a].sort().join() === [...b].sort().join();

// ---------------------------------------------------------------- API

export async function getConfiguration(): Promise<SdsConfiguration> {
  const s = db();
  return clone({ version: s.version, groups: s.groups });
}

export async function listStandardPatterns(): Promise<SdsStandardPattern[]> {
  return LIBRARY.map(({ js: _js, check: _check, ...p }) => clone(p));
}

export async function createGroup(input: SdsGroupInput): Promise<SdsGroup> {
  const s = db();
  if (s.groups.length >= GROUP_LIMIT) fail(`An organization has at most ${GROUP_LIMIT} scanning groups`);
  const i = checkGroup(input);
  const g: SdsGroup = {
    id: nextId(s),
    name: i.name,
    description: i.description ?? '',
    query: i.query ?? '',
    enabled: i.enabled ?? false,
    products: i.products!,
    samplingRate: i.samplingRate!,
    rules: [],
  };
  s.groups.push(g);
  s.version++;
  return clone(g);
}

export async function updateGroup(id: string, input: SdsGroupInput): Promise<SdsGroup> {
  const s = db();
  const g = groupOf(s, id);
  const i = checkGroup(input);
  Object.assign(g, { name: i.name, description: i.description ?? '', query: i.query ?? '', enabled: i.enabled ?? false, products: i.products!, samplingRate: i.samplingRate! });
  s.version++;
  return clone(g);
}

export async function deleteGroup(id: string): Promise<void> {
  const s = db();
  groupOf(s, id);
  s.groups = s.groups.filter((g) => g.id !== id);
  s.version++;
}

export async function reorderGroups(ids: string[]): Promise<void> {
  const s = db();
  if (
    !same(
      ids,
      s.groups.map((g) => g.id),
    )
  )
    fail('List each scanning group once');
  s.groups = ids.map((id) => groupOf(s, id));
  s.version++;
}

export async function reorderRules(groupId: string, ids: string[]): Promise<void> {
  const s = db();
  const g = groupOf(s, groupId);
  if (
    !same(
      ids,
      g.rules.map((r) => r.id),
    )
  )
    fail("List each of the group's rules once");
  g.rules = ids.map((id) => g.rules.find((r) => r.id === id)!);
  s.version++;
}

export async function createRule(input: SdsRuleInput): Promise<SdsRule> {
  const s = db();
  const g = groupOf(s, input.groupId);
  const r = toRule(nextId(s), checkRule(input));
  g.rules.push(r);
  s.version++;
  return clone(r);
}

export async function updateRule(id: string, input: SdsRuleInput): Promise<SdsRule> {
  const s = db();
  const [g, at] = ruleOf(s, id);
  // A rule stays in its group.
  const r = toRule(id, { ...checkRule(input), groupId: g.id });
  g.rules[at] = r;
  s.version++;
  return clone(r);
}

export async function deleteRule(id: string): Promise<void> {
  const s = db();
  const [g, at] = ruleOf(s, id);
  g.rules.splice(at, 1);
  s.version++;
}

// ---------------------------------------------------------------- tester

/** 16 hex digits standing for the value (FNV-1a, stable for the same value). */
function hash16(value: string): string {
  let h = 0xcbf29ce484222325n;
  for (const c of new TextEncoder().encode(`mock-sds:${value}`)) h = BigInt.asUintN(64, (h ^ BigInt(c)) * 0x100000001b3n);
  return h.toString(16).padStart(16, '0');
}

function replace(found: string, r: SdsReplacement): string {
  const chars = [...found];
  const n = Math.min(r.numberOfChars ?? 0, chars.length);
  switch (r.kind) {
    case 'hash':
      return hash16(found);
    case 'replacement_string':
      return r.replacementString ?? '[sensitive_data]';
    case 'partial_replacement_from_beginning':
      return '*'.repeat(n) + chars.slice(n).join('');
    case 'partial_replacement_from_end':
      return chars.slice(0, chars.length - n).join('') + '*'.repeat(n);
    default:
      return found;
  }
}

export async function testRule(req: SdsTestRequest): Promise<SdsTestResult> {
  if (req.sample.length > 64 * 1024) fail('The sample is at most 64 KiB');
  const rule = checkRule(req.rule);
  const std = rule.standardPatternId ? findPattern(rule.standardPatternId) : undefined;
  const [source, flags, group] = std ? std.js : [rule.pattern!, 'g', 0];
  const re = compile(source, flags);
  const keywords = [...(rule.keywords ?? []), ...(rule.useRecommendedKeywords && std ? std.keywords : [])].map((k) => k.toLowerCase()).filter(Boolean);
  const window = rule.keywordCharacters ?? 30;
  const s = rule.suppressions ?? {};
  const kind = rule.replacement ?? replacement('none');
  let matches = 0;
  const redacted = req.sample.replace(re, (...args) => {
    const whole = args[0] as string;
    const offset = args[args.length - 2] as number;
    const part = group ? ((args[group] as string | undefined) ?? '') : whole;
    if (!part) return whole;
    const at = offset + whole.indexOf(part);
    if (std?.check && !std.check(part)) return whole;
    if (keywords.length) {
      const before = req.sample.slice(Math.max(0, at - window), at).toLowerCase();
      if (!keywords.some((k) => before.includes(k))) return whole;
    }
    if (s.exactMatch?.includes(part) || s.startsWith?.some((x) => x && part.startsWith(x)) || s.endsWith?.some((x) => x && part.endsWith(x))) return whole;
    matches++;
    return whole.replace(part, replace(part, kind));
  });
  const tags = matches ? ((rule.tags ?? []).filter(Boolean).length ? rule.tags!.filter(Boolean) : (std?.tags ?? [])) : [];
  return { matches, redacted, tags: [...tags].sort() };
}

// ---------------------------------------------------------------- findings (the `sds.matches` metric's series)

/** The mock series of `sds.matches`: one per rule and service that has matches. */
export const SDS_MATCH_SERIES: Record<string, string>[] = [
  { sds_group: 'PCI compliance', sds_rule: 'Card numbers', service: 'billing' },
  { sds_group: 'PCI compliance', sds_rule: 'Card numbers', service: 'order-service' },
  { sds_group: 'PCI compliance', sds_rule: 'Bank accounts', service: 'billing' },
  { sds_group: 'Credentials', sds_rule: 'AWS access keys', service: 'inventory-service' },
  { sds_group: 'Credentials', sds_rule: 'GitHub tokens', service: 'catalog-service' },
  { sds_group: 'Credentials', sds_rule: 'Internal service tokens', service: 'storefront-api' },
  { sds_group: 'Credentials', sds_rule: 'Internal service tokens', service: 'api-gateway' },
  { sds_group: 'Personal data', sds_rule: 'Customer emails', service: 'storefront-api' },
].map((e) => {
  const group = db().groups.find((g) => g.name === e.sds_group)!;
  const rule = group.rules.find((r) => r.name === e.sds_rule)!;
  return { ...e, sds_group_id: group.id, sds_rule_id: rule.id };
});
