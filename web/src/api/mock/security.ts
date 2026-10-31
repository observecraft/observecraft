// In-browser Security for mock mode: Cloud SIEM's content packs and detection rules (the server's
// catalog), signals raised on the schedule of oc-tracegen's security logs (web attack bursts every
// 30 minutes, console brute force every 40, MFA fatigue every 50, …), suppressions, the overview,
// the inbox (from the Code Security mock) and a research feed of fictional advisories.
import type {
  ContentPackStatus,
  FeedItem,
  InboxFinding,
  SecurityInbox,
  SecurityOverview,
  SecurityRule,
  SecurityRuleInput,
  SecurityRuleSummary,
  SecuritySignal,
  SiemOverview,
  SignalBulkInput,
  SignalDetail,
  SignalFacet,
  SignalSearchPage,
  SignalSearchRequest,
  SignalStateInput,
  Suppression,
  SuppressionInput,
} from '../security/types';
import { parseQuery, type Term } from '../../lib/searchQuery';
import { rng } from './generator';
import * as cs from './codeSecurity';
import { allMisconfigs, allVulns } from './cloudSecurity';
import { USERS } from './errorTracking';
import { searchUsers } from './identity';
import { WP_AGENT_RULES, wpDetectionRuleId, wpMitreTags } from './workloadCatalog';

const MIN = 60_000;
const HOUR = 60 * MIN;
const DAY = 24 * HOUR;
const delay = <T>(v: T, ms = 140): Promise<T> => new Promise((r) => setTimeout(() => r(structuredClone(v)), ms));
const fail = (m: string): never => {
  throw new Error(m);
};
const ACCOUNT = '123456789012';
const SEVERITY_RANK = ['info', 'low', 'medium', 'high', 'critical'];

// ---------------------------------------------------------------- catalog (the server's security_catalog.rs)

interface DefaultRule {
  id: string;
  name: string;
  message: string;
  query: string;
  groupBy: string[];
  cases: [string, string, string][];
  window: number;
  tags: string[];
}
interface Pack {
  id: string;
  name: string;
  category: string;
  platform: string;
  logo: string;
  description: string;
  query: string;
  setup: string;
  rules: DefaultRule[];
}
const BRUTE = ['tactic:TA0006-credential-access', 'technique:T1110-brute-force'];
const PACKS: Pack[] = [
  {
    id: 'aws-cloudtrail',
    name: 'AWS CloudTrail',
    category: 'Cloud Audit',
    platform: 'AWS',
    logo: 'aws',
    description: 'Watch who does what in your AWS accounts: console sign-ins, root usage, public buckets and tampering with the audit trail.',
    query: 'source:cloudtrail',
    setup: 'Send CloudTrail to the Datadog Forwarder (or an S3 → Lambda forwarder) with source `cloudtrail` and point it at this ObserveCraft intake.',
    rules: [
      {
        id: 'def-ctl-001',
        name: 'Brute force on AWS console sign-ins',
        message: 'Many failed AWS console sign-ins for one identity. Check the source IPs and whether a sign-in finally succeeded.',
        query: 'source:cloudtrail @evt.name:ConsoleLogin @evt.outcome:failure',
        groupBy: ['@userIdentity.arn'],
        cases: [
          ['a > 25', 'high', ''],
          ['a > 5', 'medium', ''],
        ],
        window: 900,
        tags: BRUTE,
      },
      {
        id: 'def-ctl-002',
        name: 'AWS console sign-in without MFA',
        message: 'An IAM user signed in to the AWS console without multi-factor authentication.',
        query: 'source:cloudtrail @evt.name:ConsoleLogin @evt.outcome:success @additionalEventData.MFAUsed:No',
        groupBy: ['@userIdentity.arn'],
        cases: [['a > 0', 'medium', '']],
        window: 300,
        tags: ['tactic:TA0001-initial-access', 'technique:T1078-valid-accounts'],
      },
      {
        id: 'def-ctl-003',
        name: 'AWS root account activity',
        message: "The account's root user made API calls. Root should only be used for the few tasks that require it.",
        query: 'source:cloudtrail @userIdentity.type:Root',
        groupBy: ['@userIdentity.accountId'],
        cases: [['a > 0', 'high', '']],
        window: 300,
        tags: ['tactic:TA0004-privilege-escalation', 'technique:T1078.004-cloud-accounts'],
      },
      {
        id: 'def-ctl-004',
        name: 'S3 bucket policy allows public access',
        message: 'A bucket policy now grants access to everyone. Confirm the bucket is meant to be public.',
        query: 'source:cloudtrail @evt.name:PutBucketPolicy @requestParameters.public:true',
        groupBy: ['@requestParameters.bucketName'],
        cases: [['a > 0', 'high', '']],
        window: 300,
        tags: ['tactic:TA0010-exfiltration', 'technique:T1530-data-from-cloud-storage'],
      },
      {
        id: 'def-ctl-005',
        name: 'CloudTrail logging stopped or deleted',
        message: 'A trail was stopped or deleted, which hides later activity from the audit log.',
        query: 'source:cloudtrail @evt.name:(StopLogging OR DeleteTrail)',
        groupBy: ['@userIdentity.arn'],
        cases: [['a > 0', 'high', '']],
        window: 300,
        tags: ['tactic:TA0005-defense-evasion', 'technique:T1562.008-disable-cloud-logs'],
      },
    ],
  },
  {
    id: 'okta',
    name: 'Okta',
    category: 'Authentication',
    platform: 'Identity',
    logo: 'okta',
    description: 'Spot attacks on your identity provider: MFA fatigue, credential stuffing and new administrators.',
    query: 'source:okta',
    setup: 'Enable the Okta system log integration (or stream the system log) with source `okta`.',
    rules: [
      {
        id: 'def-okt-001',
        name: 'Okta MFA brute force',
        message: 'Repeated failed MFA challenges for one user: a push-fatigue or stolen-password attempt.',
        query: 'source:okta @evt.name:user.authentication.auth_via_mfa @evt.outcome:FAILURE',
        groupBy: ['@usr.email'],
        cases: [
          ['a > 10', 'high', ''],
          ['a > 4', 'medium', ''],
        ],
        window: 900,
        tags: BRUTE,
      },
      {
        id: 'def-okt-002',
        name: 'Okta administrator role granted',
        message: 'A user was given an administrator role in Okta.',
        query: 'source:okta @evt.name:user.account.privilege.grant',
        groupBy: ['@target.email'],
        cases: [['a > 0', 'medium', '']],
        window: 300,
        tags: ['tactic:TA0003-persistence', 'technique:T1098-account-manipulation'],
      },
    ],
  },
  {
    id: 'github',
    name: 'GitHub',
    category: 'Collaboration',
    platform: 'SaaS',
    logo: 'github',
    description: 'Protect your source code: repositories made public, branch protections removed and new deploy keys.',
    query: 'source:github',
    setup: 'Stream the GitHub organization audit log with source `github`.',
    rules: [
      {
        id: 'def-ghb-001',
        name: 'GitHub repository made public',
        message: 'A private repository became public. Check that no secrets or internal code are exposed.',
        query: 'source:github @evt.name:repo.access @visibility:public',
        groupBy: ['@repo'],
        cases: [['a > 0', 'high', '']],
        window: 300,
        tags: ['tactic:TA0010-exfiltration', 'technique:T1567-exfiltration-over-web-service'],
      },
      {
        id: 'def-ghb-002',
        name: 'GitHub branch protection removed',
        message: 'The protection of a branch was removed, so changes can land without review.',
        query: 'source:github @evt.name:protected_branch.destroy',
        groupBy: ['@repo'],
        cases: [['a > 0', 'medium', '']],
        window: 300,
        tags: ['tactic:TA0005-defense-evasion', 'technique:T1562-impair-defenses'],
      },
    ],
  },
  {
    id: 'kubernetes-audit',
    name: 'Kubernetes Audit',
    category: 'Containers',
    platform: 'Kubernetes',
    logo: 'kubernetes',
    description: 'Follow interactive access to your clusters and requests that should never be allowed.',
    query: 'source:kubernetes.audit',
    setup: 'Collect the API server audit log with the Datadog Agent (`logs_config` on the audit file) with source `kubernetes.audit`.',
    rules: [
      {
        id: 'def-k8s-001',
        name: 'Interactive exec into a Kubernetes pod',
        message: 'Someone opened a shell in a running pod.',
        query: 'source:kubernetes.audit @objectRef.subresource:exec',
        groupBy: ['@usr.name', '@objectRef.namespace'],
        cases: [['a > 0', 'low', '']],
        window: 300,
        tags: ['tactic:TA0002-execution', 'technique:T1609-container-administration-command'],
      },
      {
        id: 'def-k8s-002',
        name: 'Anonymous request allowed by the Kubernetes API',
        message: 'The API server allowed a request from an unauthenticated user.',
        query: 'source:kubernetes.audit @usr.name:"system:anonymous" @http.status_code:200',
        groupBy: ['@network.client.ip'],
        cases: [['a > 0', 'high', '']],
        window: 300,
        tags: ['tactic:TA0001-initial-access', 'technique:T1190-exploit-public-facing-application'],
      },
    ],
  },
  {
    id: 'web-attacks',
    name: 'Web Server Attacks',
    category: 'Web',
    platform: 'Web',
    logo: 'nginx',
    description: 'Find attacks against your web servers in their access logs: injection, path traversal and scanners.',
    query: 'source:nginx',
    setup: "Collect nginx access logs with the Datadog Agent's nginx integration (source `nginx`).",
    rules: [
      {
        id: 'def-web-001',
        name: 'Attack attempts on a web service',
        message: 'A burst of requests carrying attack payloads hit the service. Look at the attacking IPs and whether any request succeeded.',
        query: 'source:nginx @security.attack:true',
        groupBy: ['service'],
        cases: [
          ['a > 1000', 'medium', 'Condition 2'],
          ['a > 100', 'info', 'Condition 1'],
        ],
        window: 900,
        tags: ['tactic:TA0043-reconnaissance', 'technique:T1595-active-scanning'],
      },
      {
        id: 'def-web-002',
        name: 'SQL injection attempts from one IP',
        message: 'One client sent many SQL injection payloads.',
        query: 'source:nginx @security.attack_type:sql_injection',
        groupBy: ['@network.client.ip'],
        cases: [['a > 10', 'low', '']],
        window: 600,
        tags: ['tactic:TA0001-initial-access', 'technique:T1190-exploit-public-facing-application'],
      },
    ],
  },
  {
    id: 'azure-activity',
    name: 'Azure Activity Logs',
    category: 'Cloud Audit',
    platform: 'Azure',
    logo: 'azure',
    description: 'Monitor changes to your Azure subscriptions: new role assignments and deleted diagnostic settings.',
    query: 'source:azure.activity_logs',
    setup: 'Forward Azure activity logs (Event Hub → Datadog forwarder) with source `azure.activity_logs`.',
    rules: [
      {
        id: 'def-azr-001',
        name: 'Azure owner role assigned',
        message: 'A principal was given the Owner role on a subscription or resource group.',
        query: 'source:azure.activity_logs @evt.name:"MICROSOFT.AUTHORIZATION/ROLEASSIGNMENTS/WRITE" @properties.roleName:Owner',
        groupBy: ['@identity.claims.name'],
        cases: [['a > 0', 'medium', '']],
        window: 300,
        tags: ['tactic:TA0004-privilege-escalation', 'technique:T1098-account-manipulation'],
      },
    ],
  },
  {
    id: 'gcp-audit',
    name: 'Google Cloud Audit Logs',
    category: 'Cloud Audit',
    platform: 'GCP',
    logo: 'googlecloud',
    description: 'Watch your Google Cloud projects for new service account keys and IAM policy changes.',
    query: 'source:gcp.audit',
    setup: 'Export Cloud Audit Logs through Pub/Sub to the intake with source `gcp.audit`.',
    rules: [
      {
        id: 'def-gcp-001',
        name: 'Google Cloud service account key created',
        message: 'A key was created for a service account. Long-lived keys are a common way in.',
        query: 'source:gcp.audit @evt.name:google.iam.admin.v1.CreateServiceAccountKey',
        groupBy: ['@usr.email'],
        cases: [['a > 0', 'low', '']],
        window: 300,
        tags: ['tactic:TA0003-persistence', 'technique:T1098.001-additional-cloud-credentials'],
      },
    ],
  },
  {
    id: 'cloudflare',
    name: 'Cloudflare',
    category: 'Network',
    platform: 'Web',
    logo: 'cloudflare',
    description: 'Follow firewall events at your edge: blocked attacks and challenges.',
    query: 'source:cloudflare',
    setup: 'Send Cloudflare Logpush (firewall events) with source `cloudflare`.',
    rules: [
      {
        id: 'def-cfl-001',
        name: 'Cloudflare firewall blocking many requests from one IP',
        message: 'The firewall blocked a burst of requests from one client.',
        query: 'source:cloudflare @Action:block',
        groupBy: ['@network.client.ip'],
        cases: [['a > 200', 'low', '']],
        window: 600,
        tags: ['tactic:TA0043-reconnaissance', 'technique:T1595-active-scanning'],
      },
    ],
  },
  {
    id: 'slack-audit',
    name: 'Slack Audit Logs',
    category: 'Collaboration',
    platform: 'SaaS',
    logo: 'slack',
    description: 'Notice risky changes in your Slack workspace, such as new apps with broad access.',
    query: 'source:slack',
    setup: 'Stream the Slack audit logs API with source `slack`.',
    rules: [
      {
        id: 'def-slk-001',
        name: 'Slack app installed with broad scopes',
        message: 'An app was installed with access to every channel or user data.',
        query: 'source:slack @evt.name:app_installed @details.broad_scopes:true',
        groupBy: ['@usr.email'],
        cases: [['a > 0', 'low', '']],
        window: 300,
        tags: ['tactic:TA0003-persistence', 'technique:T1098-account-manipulation'],
      },
    ],
  },
];
/** Sources the demo sends logs for. */
const DEMO_SOURCES: Record<string, number> = { cloudtrail: 3300, okta: 1500, github: 560, 'kubernetes.audit': 1800, nginx: 14000 };

// ---------------------------------------------------------------- rules

const t0 = Date.now() - 40 * DAY;
const packEnabled = new Map<string, boolean>();
const ruleEnabled = new Map<string, boolean>();
let customRules: SecurityRule[] = [];

function defaultRule(p: Pack, r: DefaultRule): SecurityRule {
  return {
    id: r.id,
    version: 1,
    name: r.name,
    message: r.message,
    type: 'log_detection',
    isEnabled: ruleEnabled.get(r.id) ?? true,
    isDefault: true,
    pack: p.id,
    queries: [{ query: r.query, groupByFields: r.groupBy, distinctFields: [], aggregation: 'count', name: 'a', hasOptionalGroupByFields: false }],
    cases: r.cases.map(([condition, status, name]) => ({ condition, status, name, notifications: [] })),
    options: { detectionMethod: 'threshold', evaluationWindow: r.window, keepAlive: 3600, maxSignalDuration: 86400 } as unknown as SecurityRule['options'],
    filters: [],
    tags: [...r.tags, `source:${p.query.replace('source:', '')}`],
    hasExtendedTitle: true,
    createdAt: t0,
    updatedAt: t0,
    creationAuthorId: 0,
    updateAuthorId: 0,
    notEvaluated: null,
    active: (ruleEnabled.get(r.id) ?? true) && (packEnabled.get(p.id) ?? true),
  };
}

/** App & API Protection's default detection rules on security traces (the server's appsec::DEFAULT_RULES). */
const APPSEC_RULES: DefaultRule[] = [
  {
    id: 'def-aap-001',
    name: 'Attack attempts on a service',
    message: 'A burst of attack attempts reached the service. Look at the attacking IPs and whether any request succeeded.',
    query: '@appsec.category:attack_attempt',
    groupBy: ['service'],
    cases: [
      ['a > 1000', 'medium', 'Condition 2'],
      ['a > 100', 'info', 'Condition 1'],
    ],
    window: 900,
    tags: ['tactic:TA0001-initial-access', 'technique:T1190-exploit-public-facing-application'],
  },
  {
    id: 'def-aap-002',
    name: 'Security scanner detected',
    message: 'A security scanner is probing the service from one IP.',
    query: '@appsec.type:security_scanner',
    groupBy: ['@http.client_ip'],
    cases: [['a > 50', 'low', '']],
    window: 900,
    tags: ['tactic:TA0043-reconnaissance', 'technique:T1595-active-scanning'],
  },
  {
    id: 'def-aap-003',
    name: 'Credential stuffing attack on a login route',
    message: 'Many failed logins from one IP: a credential stuffing or brute force attempt.',
    query: '@appsec.events.users.login.failure.track:true',
    groupBy: ['@http.client_ip'],
    cases: [['a > 20', 'high', '']],
    window: 900,
    tags: BRUTE,
  },
  {
    id: 'def-aap-004',
    name: 'SQL injection attempts on a service',
    message: 'SQL injection payloads reached the service.',
    query: '@appsec.type:sql_injection',
    groupBy: ['service'],
    cases: [['a > 10', 'low', '']],
    window: 900,
    tags: ['tactic:TA0001-initial-access', 'technique:T1190-exploit-public-facing-application'],
  },
];
/** AI Guard's default detection rules on its evaluations (the server's ai_guard::DEFAULT_RULES). */
const EXFIL = ['category:exploit', 'tactic:ta0010-exfiltration', 'technique:t1567-exfiltration-over-web-service'];
const AI_GUARD_RULES: DefaultRule[] = [
  {
    id: 'def-aig-001',
    name: 'Unblocked & Unsafe Indirect prompt injection on AI service',
    message: 'Content an agent fetched carried instructions to the model, and AI Guard flagged it without blocking. Look at the tool that returned it and turn on blocking for the service.',
    query: '@ai_guard.attack_categories:indirect-prompt-injection @ai_guard.blocked:false',
    groupBy: ['service'],
    cases: [['a > 0', 'high', '']],
    window: 300,
    tags: ['ai_guard:true', 'category:exploit', 'tactic:ta0001-initial-access', 'technique:t1190-exploit-public-facing-application'],
  },
  {
    id: 'def-aig-002',
    name: 'Data exfiltrated from AI service',
    message: "A step of an AI service tried to move data outside the application and wasn't blocked. Check the destination and the data in the conversation.",
    query: '@ai_guard.attack_categories:data-exfiltration @ai_guard.blocked:false',
    groupBy: ['service'],
    cases: [['a > 0', 'critical', '']],
    window: 300,
    tags: ['ai_guard:true', ...EXFIL],
  },
  {
    id: 'def-aig-003',
    name: 'PII or credentials exfiltrated from AI service',
    message: 'Model answers or tool calls of an AI service carried personal data or credentials out of the application.',
    query: '@ai_guard.direction:output @ai_guard.sds.category:(pii OR credentials)',
    groupBy: ['service'],
    cases: [['a > 5', 'high', '']],
    window: 900,
    tags: ['ai_guard:true', ...EXFIL],
  },
  {
    id: 'def-aig-004',
    name: 'Unblocked & Unsafe Traffic on AI service',
    message: 'AI Guard flagged a burst of unsafe steps on an AI service that doesn’t block them.',
    query: '@ai_guard.assessment:unsafe @ai_guard.blocked:false',
    groupBy: ['service'],
    cases: [
      ['a > 50', 'high', 'Condition 2'],
      ['a > 10', 'medium', 'Condition 1'],
    ],
    window: 900,
    tags: ['ai_guard:true', 'category:custom'],
  },
];
/** Workload Protection's default detection rules, one per threat detection agent rule (the server's security_rules::ensure_defaults). */
const WP_RULES: DefaultRule[] = WP_AGENT_RULES.filter((r) => r.policy === 'threat-detection').map((r) => ({
  id: wpDetectionRuleId(r.name),
  name: r.title,
  message: r.description,
  query: `@agent.rule_id:${r.name}`,
  groupBy: ['host'],
  cases: [['a > 0', r.severity, '']],
  window: 300,
  tags: wpMitreTags(r),
}));
const WP_PACK: Pack = {
  id: 'workload-protection',
  name: 'Workload Protection',
  category: 'Workload',
  platform: 'Workload',
  logo: '',
  description: '',
  query: 'source:runtime-security-agent',
  setup: '',
  rules: [],
};
const AI_GUARD_PACK: Pack = {
  id: 'ai-guard',
  name: 'AI Guard',
  category: 'Application',
  platform: 'Application',
  logo: '',
  description: '',
  query: 'source:application-threats',
  setup: '',
  rules: [],
};
const APPSEC_PACK: Pack = { id: 'app-sec', name: 'App & API Protection', category: 'Application', platform: 'Application', logo: '', description: '', query: 'source:appsec', setup: '', rules: [] };

function allRules(): SecurityRule[] {
  return [
    ...customRules,
    ...PACKS.flatMap((p) => p.rules.map((r) => defaultRule(p, r))),
    ...APPSEC_RULES.map((r) => ({ ...defaultRule(APPSEC_PACK, r), type: 'application_security', active: ruleEnabled.get(r.id) ?? true })),
    ...AI_GUARD_RULES.map((r) => ({ ...defaultRule(AI_GUARD_PACK, r), type: 'application_security', active: ruleEnabled.get(r.id) ?? true })),
    ...WP_RULES.map((r) => ({ ...defaultRule(WP_PACK, r), type: 'workload_security', active: ruleEnabled.get(r.id) ?? true })),
  ];
}

function notEvaluated(input: SecurityRuleInput): string | null {
  const type = input.type ?? 'log_detection';
  if (!['log_detection', 'workload_security', 'application_security'].includes(type)) return `${type.replace(/_/g, ' ')} rules are stored but not evaluated yet`;
  const method = (input.options as { detectionMethod?: string })?.detectionMethod ?? 'threshold';
  if (method !== 'threshold') return `the ${method.replace(/_/g, ' ')} detection method is not evaluated yet`;
  const q = input.queries.find((x) => !['count', 'cardinality'].includes(x.aggregation ?? 'count'));
  return q ? `the ${q.aggregation} aggregation is not evaluated yet` : null;
}

function validate(input: SecurityRuleInput) {
  if (!input.name.trim()) fail('the rule needs a name');
  if (!input.queries.length) fail('the rule needs at least one query');
  if (input.queries.some((q) => !q.query?.trim())) fail('Query filter cannot be empty');
  if (!input.cases.length) fail('the rule needs at least one case');
  const names = input.queries.map((q, i) => q.name || String.fromCharCode(97 + i));
  for (const c of input.cases) {
    for (const v of (c.condition ?? '').match(/[a-zA-Z_][a-zA-Z0-9_]*/g) ?? []) if (!names.includes(v)) fail(`the condition \`${c.condition}\` uses \`${v}\`, which is not a query of the rule`);
  }
}

function fromInput(id: string, input: SecurityRuleInput, prev?: SecurityRule): SecurityRule {
  return {
    id,
    version: (prev?.version ?? 0) + 1,
    name: input.name.trim(),
    message: input.message ?? '',
    type: input.type ?? 'log_detection',
    isEnabled: input.isEnabled ?? true,
    isDefault: false,
    pack: null,
    queries: input.queries.map((q) => ({
      query: q.query,
      groupByFields: q.groupByFields ?? [],
      distinctFields: q.distinctFields ?? [],
      aggregation: q.aggregation ?? 'count',
      name: q.name ?? '',
      hasOptionalGroupByFields: false,
    })),
    cases: input.cases.map((c) => ({ name: c.name ?? '', status: c.status, condition: c.condition ?? '', notifications: c.notifications ?? [] })),
    options: { detectionMethod: 'threshold', evaluationWindow: 300, keepAlive: 3600, maxSignalDuration: 86400, ...(input.options as object) } as unknown as SecurityRule['options'],
    filters: input.filters ?? [],
    tags: input.tags ?? [],
    hasExtendedTitle: input.hasExtendedTitle ?? false,
    createdAt: prev?.createdAt ?? Date.now(),
    updatedAt: Date.now(),
    creationAuthorId: 1,
    updateAuthorId: 1,
    notEvaluated: notEvaluated(input),
    active: input.isEnabled ?? true,
  };
}

export async function listRules(): Promise<SecurityRuleSummary[]> {
  const now = Date.now();
  const sigs = signalsBetween(now - 7 * DAY, now);
  return delay(
    allRules().map((rule) => {
      const mine = sigs.filter((s) => s.ruleId === rule.id);
      return { ...rule, signals7d: mine.length, lastSignalAt: mine[0]?.createdAt ?? null, packName: PACKS.find((p) => p.id === rule.pack)?.name ?? null };
    }),
  );
}

export async function getRule(id: string): Promise<SecurityRule> {
  return delay(allRules().find((r) => r.id === id) ?? fail(`no detection rule ${id}`));
}

export async function createRule(input: SecurityRuleInput): Promise<SecurityRule> {
  validate(input);
  const r = fromInput(`usr-${Math.random().toString(36).slice(2, 5)}-${Math.random().toString(36).slice(2, 5)}`, input);
  customRules = [r, ...customRules];
  return delay(r);
}

export async function updateRule(id: string, input: SecurityRuleInput): Promise<SecurityRule> {
  const cur = allRules().find((r) => r.id === id) ?? fail(`no detection rule ${id}`);
  if (cur.isDefault) {
    ruleEnabled.set(id, input.isEnabled ?? true);
    return getRule(id);
  }
  validate(input);
  const r = fromInput(id, input, cur);
  customRules = customRules.map((x) => (x.id === id ? r : x));
  return delay(r);
}

export async function deleteRule(id: string): Promise<void> {
  const cur = allRules().find((r) => r.id === id) ?? fail(`no detection rule ${id}`);
  if (cur.isDefault) fail("default rules can't be deleted; disable the rule instead");
  customRules = customRules.filter((r) => r.id !== id);
  return delay(undefined);
}

export async function setRuleEnabled(id: string, enabled: boolean): Promise<SecurityRule> {
  const cur = allRules().find((r) => r.id === id) ?? fail(`no detection rule ${id}`);
  if (cur.isDefault) ruleEnabled.set(id, enabled);
  else customRules = customRules.map((r) => (r.id === id ? { ...r, isEnabled: enabled, active: enabled } : r));
  return getRule(id);
}

// ---------------------------------------------------------------- suppressions

let suppressions: Suppression[] = [
  {
    id: 'sup-ci1-dpl',
    version: 1,
    name: 'CI deployer console sign-ins',
    description: 'The deployment pipeline signs in from a known runner without MFA.',
    enabled: true,
    ruleQuery: 'source:cloudtrail',
    suppressionQuery: '@userIdentity.arn:*ci-deployer',
    dataExclusionQuery: '',
    startDate: null,
    expirationDate: null,
    tags: ['team:platform'],
    creator: 'Priya Shah',
    updater: 'Priya Shah',
    createdAt: t0 + 3 * DAY,
    updatedAt: t0 + 3 * DAY,
  },
];

export async function listSuppressions(): Promise<Suppression[]> {
  return delay(suppressions);
}

function checkSuppression(input: SuppressionInput) {
  if (!input.name.trim()) fail('the suppression needs a name');
  if (!input.suppressionQuery?.trim() && !input.dataExclusionQuery?.trim()) fail('set a suppression query, a data exclusion query, or both');
}

export async function createSuppression(input: SuppressionInput): Promise<Suppression> {
  checkSuppression(input);
  if (suppressions.some((s) => s.name === input.name.trim())) fail(`a suppression named ${input.name.trim()} already exists`);
  const s: Suppression = {
    id: `sup-${Math.random().toString(36).slice(2, 5)}-${Math.random().toString(36).slice(2, 5)}`,
    version: 1,
    name: input.name.trim(),
    description: input.description ?? '',
    enabled: input.enabled ?? true,
    ruleQuery: input.ruleQuery ?? '',
    suppressionQuery: input.suppressionQuery ?? '',
    dataExclusionQuery: input.dataExclusionQuery ?? '',
    startDate: input.startDate ?? null,
    expirationDate: input.expirationDate ?? null,
    tags: input.tags ?? [],
    creator: 'Alex Morgan',
    updater: 'Alex Morgan',
    createdAt: Date.now(),
    updatedAt: Date.now(),
  };
  suppressions = [...suppressions, s];
  return delay(s);
}

export async function updateSuppression(id: string, input: SuppressionInput): Promise<Suppression> {
  const cur = suppressions.find((s) => s.id === id) ?? fail(`no suppression ${id}`);
  checkSuppression(input);
  const s: Suppression = {
    ...cur,
    ...input,
    name: input.name.trim(),
    description: input.description ?? '',
    enabled: input.enabled ?? true,
    ruleQuery: input.ruleQuery ?? '',
    suppressionQuery: input.suppressionQuery ?? '',
    dataExclusionQuery: input.dataExclusionQuery ?? '',
    startDate: input.startDate ?? null,
    expirationDate: input.expirationDate ?? null,
    tags: input.tags ?? [],
    version: cur.version + 1,
    updater: 'Alex Morgan',
    updatedAt: Date.now(),
  };
  suppressions = suppressions.map((x) => (x.id === id ? s : x));
  return delay(s);
}

export async function deleteSuppression(id: string): Promise<void> {
  suppressions.find((s) => s.id === id) ?? fail(`no suppression ${id}`);
  suppressions = suppressions.filter((s) => s.id !== id);
  return delay(undefined);
}

// ---------------------------------------------------------------- signals

interface Schedule {
  rule: string;
  every: number;
  offset: number;
  severity: (r: () => number) => [string, string];
  group: (slot: number) => Record<string, string>;
  events: (r: () => number) => number;
  entities: (r: () => number, slot: number) => Record<string, { top: string[]; count: number }>;
  services: (slot: number) => string[];
}
const ATTACKERS = ['203.0.113.7', '203.0.113.41', '203.0.113.88', '198.51.100.23', '198.51.100.71', '198.51.100.140', '203.0.113.150', '198.51.100.201'];
const ips = (r: () => number, n: number, lead: number) => ({ top: Array.from({ length: Math.min(n, 5) }, (_, i) => ATTACKERS[(lead + i) % ATTACKERS.length]), count: n + Math.floor(r() * 3) });
const AI_SERVICES = ['shopping-assistant', 'support-agent', 'catalog-enricher', 'review-summarizer'];
/** Workload Protection signals on the schedule of oc-tracegen's cws.rs, thinned: the server's keep-alive extends a signal while its events repeat. */
const WP_HOST = (n: number) => `i-0a1b2c3d4e5f6000${n}`;
const wpHost = (n: number, events: (r: () => number) => number, extra: Record<string, { top: string[]; count: number }> = {}) => ({
  group: () => ({ host: WP_HOST(n) }),
  events,
  entities: () => ({ host: { top: [WP_HOST(n)], count: 1 }, ...extra }),
});
const WP_SCHEDULES: Schedule[] = [
  {
    rule: wpDetectionRuleId('web_server_spawned_shell'),
    every: 6 * HOUR,
    offset: 25 * MIN,
    severity: () => ['high', ''],
    ...wpHost(1, () => 1, { container: { top: ['storefront-api-7d9f6c5b8-x2kq4'], count: 1 } }),
    services: () => ['storefront-api'],
  },
  {
    rule: wpDetectionRuleId('reverse_shell'),
    every: 6 * HOUR,
    offset: 26 * MIN,
    severity: () => ['critical', ''],
    ...wpHost(1, () => 1, { ip: { top: ['203.0.113.41'], count: 1 } }),
    services: () => ['storefront-api'],
  },
  { rule: wpDetectionRuleId('service_account_token_read'), every: 6 * HOUR, offset: 25 * MIN, severity: () => ['medium', ''], ...wpHost(1, () => 1), services: () => ['storefront-api'] },
  { rule: wpDetectionRuleId('network_scanner_executed'), every: 6 * HOUR, offset: 26 * MIN, severity: () => ['medium', ''], ...wpHost(1, () => 1), services: () => ['storefront-api'] },
  {
    rule: wpDetectionRuleId('shell_in_container'),
    every: 8 * HOUR,
    offset: 31 * MIN,
    severity: () => ['medium', ''],
    ...wpHost(1, (r) => 1 + Math.floor(r() * 3)),
    services: () => ['storefront-api'],
  },
  {
    rule: wpDetectionRuleId('imds_v1_request'),
    every: 12 * HOUR,
    offset: 7 * MIN,
    severity: () => ['medium', ''],
    ...wpHost(4, (r) => 12 + Math.floor(r() * 4)),
    services: () => ['runtime-security-agent'],
  },
  { rule: wpDetectionRuleId('sudoers_modified'), every: 8 * HOUR, offset: 10 * MIN, severity: () => ['medium', ''], ...wpHost(5, () => 1), services: () => ['runtime-security-agent'] },
  { rule: wpDetectionRuleId('cryptominer_executed'), every: 9 * HOUR, offset: 40 * MIN, severity: () => ['high', ''], ...wpHost(5, () => 1), services: () => ['runtime-security-agent'] },
  { rule: wpDetectionRuleId('mining_pool_dns'), every: 9 * HOUR, offset: 40 * MIN, severity: () => ['medium', ''], ...wpHost(5, () => 1), services: () => ['runtime-security-agent'] },
  { rule: wpDetectionRuleId('user_account_created'), every: 12 * HOUR, offset: 20 * MIN, severity: () => ['medium', ''], ...wpHost(4, () => 1), services: () => ['runtime-security-agent'] },
  { rule: wpDetectionRuleId('package_manager_in_container'), every: 12 * HOUR, offset: 97 * MIN, severity: () => ['low', ''], ...wpHost(2, () => 1), services: () => ['order-service'] },
  { rule: wpDetectionRuleId('compiler_in_container'), every: 16 * HOUR, offset: 2 * HOUR, severity: () => ['low', ''], ...wpHost(2, () => 1), services: () => ['catalog-service'] },
];
const SCHEDULES: Schedule[] = [
  ...WP_SCHEDULES,
  {
    rule: 'def-aig-001',
    every: 7 * HOUR,
    offset: 52 * MIN,
    severity: () => ['high', ''],
    group: (slot) => ({ service: AI_SERVICES[slot % 2 ? 0 : 2] }),
    events: (r) => 1 + Math.floor(r() * 4),
    entities: (_r, slot) => ({
      ip: { top: [ATTACKERS[slot % ATTACKERS.length]], count: 1 },
      attack_category: { top: ['indirect-prompt-injection', 'instruction-override'], count: 2 },
      tool: { top: ['fetch_url'], count: 1 },
    }),
    services: (slot) => [AI_SERVICES[slot % 2 ? 0 : 2]],
  },
  {
    rule: 'def-aig-002',
    every: 15 * HOUR,
    offset: 3 * HOUR,
    severity: () => ['critical', ''],
    group: (slot) => ({ service: AI_SERVICES[slot % 2] }),
    events: (r) => 1 + Math.floor(r() * 3),
    entities: (_r, slot) => ({
      ip: { top: [ATTACKERS[(slot + 2) % ATTACKERS.length]], count: 1 },
      attack_category: { top: ['data-exfiltration', 'credential-access'], count: 2 },
      tool: { top: ['http_request', 'send_email'], count: 2 },
    }),
    services: (slot) => [AI_SERVICES[slot % 2]],
  },
  {
    rule: 'def-aig-003',
    every: 11 * HOUR,
    offset: 6 * HOUR,
    severity: () => ['high', ''],
    group: (slot) => ({ service: AI_SERVICES[(slot % 2) + 2] }),
    events: (r) => 6 + Math.floor(r() * 30),
    entities: (_r, slot) => ({ ip: { top: [ATTACKERS[(slot + 4) % ATTACKERS.length]], count: 1 }, user: { top: ['ana.popescu@example.com'], count: 1 } }),
    services: (slot) => [AI_SERVICES[(slot % 2) + 2]],
  },
  {
    rule: 'def-aig-004',
    every: 5 * HOUR,
    offset: 2 * HOUR,
    severity: (r) => (r() < 0.3 ? ['high', 'Condition 2'] : ['medium', 'Condition 1']),
    group: (slot) => ({ service: AI_SERVICES[slot % AI_SERVICES.length] }),
    events: (r) => 12 + Math.floor(r() * 80),
    entities: (r, slot) => ({ ip: ips(r, 3, slot + 1), attack_category: { top: ['jailbreak', 'system-prompt-extraction', 'instruction-override'], count: 3 } }),
    services: (slot) => [AI_SERVICES[slot % AI_SERVICES.length]],
  },
  {
    rule: 'def-aap-001',
    every: 4 * HOUR,
    offset: 37 * MIN,
    severity: (r) => (r() < 0.25 ? ['medium', 'Condition 2'] : ['info', 'Condition 1']),
    group: (slot) => ({ service: slot % 3 ? 'api-gateway' : 'storefront-api' }),
    events: (r) => 180 + Math.floor(r() * 1400),
    entities: (r, slot) => ({ ip: ips(r, 12, slot), country: { top: ['BE', 'IN', 'VN'], count: 3 } }),
    services: (slot) => [slot % 3 ? 'api-gateway' : 'storefront-api'],
  },
  {
    rule: 'def-aap-002',
    every: 9 * HOUR,
    offset: 83 * MIN,
    severity: () => ['low', ''],
    group: (slot) => ({ '@http.client_ip': ATTACKERS[slot % ATTACKERS.length] }),
    events: (r) => 60 + Math.floor(r() * 200),
    entities: (_r, slot) => ({ ip: { top: [ATTACKERS[slot % ATTACKERS.length]], count: 1 } }),
    services: () => ['storefront-api'],
  },
  {
    rule: 'def-aap-003',
    every: 18 * HOUR,
    offset: 5 * HOUR,
    severity: () => ['high', ''],
    group: (slot) => ({ '@http.client_ip': ATTACKERS[(slot + 3) % ATTACKERS.length] }),
    events: (r) => 25 + Math.floor(r() * 60),
    entities: (_r, slot) => ({ ip: { top: [ATTACKERS[(slot + 3) % ATTACKERS.length]], count: 1 }, user: { top: ['maria.chen@example.com', 'sam.ito@example.com'], count: 14 } }),
    services: () => ['api-gateway'],
  },
  {
    rule: 'def-web-001',
    every: 30 * MIN,
    offset: 7 * MIN,
    severity: (r) => (r() < 0.12 ? ['medium', 'Condition 2'] : ['info', 'Condition 1']),
    group: (slot) => ({ service: slot % 2 ? 'api-gateway' : 'storefront-api' }),
    events: (r) => 250 + Math.floor(r() * 300),
    entities: (r, slot) => ({ ip: ips(r, 6, slot), country: { top: ['CA', 'BR', 'VN'], count: 3 } }),
    services: (slot) => [slot % 2 ? 'api-gateway' : 'storefront-api'],
  },
  {
    rule: 'def-ctl-001',
    every: 40 * MIN,
    offset: 11 * MIN,
    severity: (r) => (r() < 0.8 ? ['high', ''] : ['medium', '']),
    group: () => ({ '@userIdentity.arn': `arn:aws:iam::${ACCOUNT}:user/dev.kumar` }),
    events: (r) => 20 + Math.floor(r() * 20),
    entities: (r, slot) => ({ ip: ips(r, 4, slot + 2), user: { top: [], count: 0 } }),
    services: () => ['cloudtrail'],
  },
  {
    rule: 'def-okt-001',
    every: 50 * MIN,
    offset: 16 * MIN,
    severity: () => ['high', ''],
    group: () => ({ '@usr.email': 'maya.chen@example.com' }),
    events: (r) => 18 + Math.floor(r() * 10),
    entities: (r, slot) => ({ ip: ips(r, 5, slot + 4), user: { top: ['maya.chen@example.com'], count: 1 }, country: { top: ['BR', 'VN'], count: 2 } }),
    services: () => ['okta'],
  },
  {
    rule: 'def-ctl-002',
    every: HOUR,
    offset: 20 * MIN,
    severity: () => ['medium', ''],
    group: () => ({ '@userIdentity.arn': `arn:aws:iam::${ACCOUNT}:user/sam.okafor` }),
    events: () => 1,
    entities: () => ({ ip: { top: ['192.0.2.24'], count: 1 } }),
    services: () => ['cloudtrail'],
  },
  {
    rule: 'def-k8s-001',
    every: HOUR,
    offset: 25 * MIN,
    severity: () => ['low', ''],
    group: () => ({ '@usr.name': 'lucia.torres@example.com', '@objectRef.namespace': 'payments' }),
    events: () => 1,
    entities: () => ({ ip: { top: ['192.0.2.24'], count: 1 }, user_name: { top: ['lucia.torres@example.com'], count: 1 } }),
    services: () => ['kube-apiserver'],
  },
  {
    rule: 'def-ghb-002',
    every: HOUR,
    offset: 50 * MIN,
    severity: () => ['medium', ''],
    group: () => ({ '@repo': 'example-org/billing' }),
    events: () => 1,
    entities: () => ({ user_name: { top: ['dev.kumar'], count: 1 } }),
    services: () => ['github'],
  },
  {
    rule: 'def-ctl-003',
    every: 2 * HOUR,
    offset: 35 * MIN,
    severity: () => ['high', ''],
    group: () => ({ '@userIdentity.accountId': ACCOUNT }),
    events: () => 1,
    entities: () => ({ ip: { top: ['192.0.2.57'], count: 1 } }),
    services: () => ['cloudtrail'],
  },
  {
    rule: 'def-ghb-001',
    every: 3 * HOUR,
    offset: 45 * MIN,
    severity: () => ['high', ''],
    group: () => ({ '@repo': 'example-org/infra' }),
    events: () => 1,
    entities: () => ({ user_name: { top: ['arjun.patel'], count: 1 } }),
    services: () => ['github'],
  },
];

const overrides = new Map<string, Partial<SecuritySignal>>();

function hash(s: string): number {
  let h = 2166136261;
  for (const c of s) h = Math.imul(h ^ c.charCodeAt(0), 16777619);
  return h >>> 0;
}

function term(k: string, v: string) {
  return /[\s():"]/.test(v) ? `${k}:"${v}"` : `${k}:${v}`;
}

function makeSignal(sch: Schedule, slot: number, now: number): SecuritySignal | null {
  const rule = allRules().find((r) => r.id === sch.rule);
  if (!rule) return null;
  const pack = PACKS.find((p) => p.id === rule.pack) ?? (rule.pack === 'ai-guard' ? AI_GUARD_PACK : rule.pack === 'workload-protection' ? WP_PACK : APPSEC_PACK);
  const r = rng(hash(`${sch.rule}:${slot}`));
  const created = slot * sch.every + sch.offset;
  const [severity, caseName] = sch.severity(r);
  const group = sch.group(slot);
  const age = now - created;
  const id = `AQAAA${hash(`${sch.rule}/${slot}`).toString(36)}${hash(`${slot}/${sch.rule}`).toString(36)}`;
  // Older signals were triaged.
  const roll = r();
  const state = age > DAY ? (roll < 0.7 ? 'archived' : roll < 0.85 ? 'under_review' : 'open') : age > 6 * HOUR && roll < 0.35 ? 'under_review' : 'open';
  const reasons = ['false_positive', 'testing_or_maintenance', 'true_positive_benign', 'investigated_case_opened'];
  const assignee = state !== 'open' || roll < 0.15 ? USERS[Math.floor(r() * USERS.length)] : null;
  const queryText = rule.queries[0]?.query ?? '';
  const base: SecuritySignal = {
    id,
    ruleId: rule.id,
    ruleName: rule.name,
    ruleType: rule.type,
    title: [rule.name, Object.values(group).join(', '), caseName].filter(Boolean).join(' - '),
    message: rule.message,
    severity,
    caseName,
    condition: rule.cases.find((c) => c.status === severity)?.condition ?? rule.cases[0]?.condition ?? '',
    groupValues: group,
    queries: [{ name: 'a', query: queryText }],
    logsQuery: [queryText, ...Object.entries(group).map(([k, v]) => term(k, v))].join(' '),
    source: pack.query.replace('source:', ''),
    services: sch.services(slot),
    env: 'prod',
    tags: [
      ...rule.tags,
      ...Object.entries(group)
        .filter(([k]) => !k.startsWith('@'))
        .map(([k, v]) => `${k}:${v}`),
    ],
    entities: sch.entities(r, slot) as unknown as SecuritySignal['entities'],
    eventsMatched: sch.events(r),
    firstSeen: created - 4 * MIN,
    lastSeen: created + Math.floor(r() * 20) * MIN,
    createdAt: created,
    state,
    archiveReason: state === 'archived' ? reasons[Math.floor(r() * reasons.length)] : 'none',
    archiveComment: '',
    assignee: assignee ? { id: assignee.id, name: assignee.name, email: assignee.email } : null,
    stateUpdatedAt: state !== 'open' ? created + 2 * HOUR : null,
    stateUpdatedBy: state !== 'open' ? (assignee?.name ?? 'Alex Morgan') : null,
    incidentIds: [],
  };
  return { ...base, ...overrides.get(id) };
}

/** Signals created in [from, to], newest first. */
export function signalsBetween(from: number, to: number): SecuritySignal[] {
  const now = Date.now();
  const out: SecuritySignal[] = [];
  const start = Math.max(from, now - 14 * DAY);
  for (const sch of SCHEDULES) {
    const rule = allRules().find((r) => r.id === sch.rule);
    if (!rule?.active) continue;
    for (let slot = Math.floor((start - sch.offset) / sch.every); slot * sch.every + sch.offset <= Math.min(to, now); slot++) {
      const created = slot * sch.every + sch.offset;
      if (created < start) continue;
      const s = makeSignal(sch, slot, now);
      if (s) out.push(s);
    }
  }
  return out.sort((a, b) => b.createdAt - a.createdAt);
}

function valuesOf(s: SecuritySignal, key: string): string[] {
  const k = key.replace(/^@/, '');
  switch (k) {
    case 'status':
    case 'severity':
      return [s.severity];
    case 'state':
    case 'workflow.triage.state':
      return [s.state];
    case 'source':
      return s.source ? [s.source] : [];
    case 'service':
      return s.services;
    case 'env':
      return s.env ? [s.env] : [];
    case 'rule':
    case 'workflow.rule.id':
      return [s.ruleId];
    case 'workflow.rule.name':
      return [s.ruleName];
    case 'type':
    case 'workflow.rule.type':
      return [s.ruleType];
    case 'assignee':
      return s.assignee ? [s.assignee.name, s.assignee.email] : ['unassigned'];
    default: {
      const out = s.tags.filter((t) => t.split(':')[0] === k).map((t) => t.slice(k.length + 1));
      const g = s.groupValues[key] ?? s.groupValues[`@${k}`] ?? s.groupValues[k];
      return g ? [...out, g] : out;
    }
  }
}

function glob(p: string, v: string) {
  const re = new RegExp(
    `^${p
      .toLowerCase()
      .split('*')
      .map((x) => x.replace(/[.+?^${}()|[\]\\]/g, '\\$&'))
      .join('.*')}$`,
  );
  return re.test(v.toLowerCase());
}

function matches(s: SecuritySignal, terms: Term[]) {
  return terms.every((t) => {
    const hit = t.kind === 'text' ? (s.title + ' ' + s.ruleName).toLowerCase().includes(t.text.toLowerCase()) : t.values.some((p) => valuesOf(s, t.key).some((v) => glob(p, v)));
    return hit !== t.negate;
  });
}

const FACETS = ['@workflow.triage.state', 'status', 'source', '@workflow.rule.type', '@workflow.rule.name', 'service', 'env', 'assignee', 'tactic'];

function facet(key: string, list: SecuritySignal[]): SignalFacet {
  const counts = new Map<string, number>();
  for (const s of list) {
    const vals = key === 'assignee' ? [s.assignee?.name ?? 'Unassigned'] : valuesOf(s, key);
    for (const v of vals) counts.set(v, (counts.get(v) ?? 0) + 1);
  }
  return {
    key,
    values: [...counts.entries()]
      .map(([value, count]) => ({ value, count }))
      .sort((a, b) => b.count - a.count || a.value.localeCompare(b.value))
      .slice(0, 25),
  };
}

export async function searchSignals(req: SignalSearchRequest): Promise<SignalSearchPage> {
  const terms = parseQuery(req.query ?? '');
  const matched = signalsBetween(req.range.from, req.range.to).filter((s) => matches(s, terms));
  const states: Record<string, number> = { open: 0, under_review: 0, archived: 0 };
  for (const s of matched) states[s.state] = (states[s.state] ?? 0) + 1;
  let shown = matched.filter((s) => !req.state || s.state === req.state);
  if (req.ascending) shown = [...shown].reverse();
  const span = Math.max(req.range.to - req.range.from, MIN);
  const step = [MIN, 5 * MIN, 15 * MIN, 30 * MIN, HOUR, 3 * HOUR, 6 * HOUR, DAY].find((s) => span / s <= 120) ?? DAY;
  const start = req.range.from - (req.range.from % step);
  const histogram = [];
  for (let ts = start; ts <= req.range.to; ts += step) histogram.push({ ts, info: 0, low: 0, medium: 0, high: 0, critical: 0 });
  for (const s of shown) {
    const b = histogram[Math.floor((s.createdAt - start) / step)];
    if (b) b[s.severity as 'info']++;
  }
  const offset = req.offset ?? 0;
  return delay({ total: shown.length, states, signals: shown.slice(offset, offset + (req.limit ?? 100)), facets: FACETS.map((k) => facet(k, shown)), histogram });
}

function findSignal(id: string): SecuritySignal {
  const now = Date.now();
  return signalsBetween(now - 14 * DAY, now).find((s) => s.id === id) ?? fail(`no signal ${id}`);
}

function detail(s: SecuritySignal): SignalDetail {
  const now = Date.now();
  const same = signalsBetween(now - 14 * DAY, now).filter((x) => x.ruleId === s.ruleId);
  return { ...s, ruleSignalsMonth: same.length, ruleSignalsDay: same.filter((x) => x.createdAt > now - DAY).length, ruleExists: true };
}

export async function getSignal(id: string): Promise<SignalDetail> {
  return delay(detail(findSignal(id)));
}

function applyState(id: string, input: SignalStateInput) {
  if (!['open', 'under_review', 'archived'].includes(input.state)) fail(`invalid state \`${input.state}\``);
  overrides.set(id, {
    ...overrides.get(id),
    state: input.state,
    archiveReason: input.state === 'archived' ? (input.archiveReason ?? 'other') : 'none',
    archiveComment: input.archiveComment ?? '',
    stateUpdatedAt: Date.now(),
    stateUpdatedBy: 'Alex Morgan',
  });
}

/** Assigns to a member of the mock org (Organization Settings → Users). */
async function applyAssignee(id: string, assigneeId: string | null) {
  const members = (await searchUsers({ limit: 500, offset: 0 })).users;
  const u = assigneeId ? (members.find((x) => x.id === assigneeId) ?? fail(`${assigneeId} is not an active member of the organization`)) : null;
  overrides.set(id, { ...overrides.get(id), assignee: u ? { id: u.id, name: u.name, email: u.email } : null });
}

export async function setSignalState(id: string, input: SignalStateInput): Promise<SignalDetail> {
  findSignal(id);
  applyState(id, input);
  return getSignal(id);
}

export async function setSignalAssignee(id: string, assigneeId: string | null): Promise<SignalDetail> {
  findSignal(id);
  await applyAssignee(id, assigneeId);
  return getSignal(id);
}

export async function bulkUpdateSignals(input: SignalBulkInput): Promise<{ updated: number }> {
  for (const id of input.ids) {
    if (input.state) applyState(id, input.state);
    if (input.assigneeId != null) await applyAssignee(id, input.assigneeId || null);
  }
  return delay({ updated: input.ids.length });
}

// ---------------------------------------------------------------- content packs and SIEM overview

function hourly(source: string): number[] {
  const perDay = DEMO_SOURCES[source] ?? 0;
  const r = rng(hash(source));
  return Array.from({ length: 24 }, () => (perDay ? Math.round((perDay / 24) * (0.7 + r() * 0.6)) : 0));
}

function packStatuses(): ContentPackStatus[] {
  const now = Date.now();
  const sigs = signalsBetween(now - 7 * DAY, now);
  return PACKS.map((p) => {
    const source = p.query.replace('source:', '');
    const on = packEnabled.get(p.id) ?? true;
    const hours = hourly(source);
    const logs24h = hours.reduce((a, b) => a + b, 0);
    const rules = p.rules.map((r) => ({
      id: r.id,
      name: r.name,
      severity: [...r.cases.map((c) => c[1])].sort((a, b) => SEVERITY_RANK.indexOf(b) - SEVERITY_RANK.indexOf(a))[0],
      enabled: ruleEnabled.get(r.id) ?? true,
    }));
    return {
      id: p.id,
      name: p.name,
      category: p.category,
      platform: p.platform,
      logo: p.logo,
      description: p.description,
      query: p.query,
      setup: p.setup,
      enabled: on,
      status: !on ? 'disabled' : logs24h > 0 ? 'active' : 'available',
      logs24h,
      hourly: hours,
      lastLogAt: logs24h ? now - (now % HOUR) : null,
      rules,
      signals7d: sigs.filter((s) => rules.some((r) => r.id === s.ruleId)).length,
    };
  });
}

export async function listPacks(): Promise<ContentPackStatus[]> {
  return delay(packStatuses());
}

export async function setPack(id: string, enabled: boolean): Promise<void> {
  PACKS.find((p) => p.id === id) ?? fail(`no content pack ${id}`);
  packEnabled.set(id, enabled);
  return delay(undefined);
}

export async function siemOverview(range: { from: number; to: number }): Promise<SiemOverview> {
  const packs = packStatuses();
  const rules = allRules();
  const statuses: Record<string, number> = { active: 0, warning: 0, available: 0, disabled: 0 };
  const coverage = new Map<string, { active: number; total: number }>();
  for (const p of packs) {
    statuses[p.status]++;
    const c = coverage.get(p.category) ?? { active: 0, total: 0 };
    c.total++;
    if (p.status === 'active') c.active++;
    coverage.set(p.category, c);
  }
  const span = Math.max(range.to - range.from, HOUR);
  const step = [5 * MIN, 15 * MIN, HOUR, 3 * HOUR, 6 * HOUR, DAY].find((s) => span / s <= 96) ?? DAY;
  const start = range.from - (range.from % step);
  const logsAnalyzed = Object.entries(DEMO_SOURCES).map(([source, perDay]) => {
    const r = rng(hash(source + step));
    const points: [number, number][] = [];
    for (let t = start; t <= range.to; t += step) points.push([t, Math.round(((perDay * step) / DAY) * (0.75 + r() * 0.5))]);
    return { source, points };
  });
  const sigs = signalsBetween(range.from, range.to);
  const bySeverity: Record<string, number> = { info: 0, low: 0, medium: 0, high: 0, critical: 0 };
  for (const s of sigs) bySeverity[s.severity]++;
  const open = sigs.filter((s) => s.state === 'open');
  return delay({
    setup: {
      packsEnabled: packs.filter((p) => p.enabled).length,
      receivingLogs: packs.filter((p) => p.logs24h > 0).length,
      rulesActive: rules.filter((r) => r.active && !r.notEvaluated).length,
      rulesNotifying: rules.filter((r) => r.cases.some((c) => c.notifications?.length)).length,
    },
    coverage: [...coverage.entries()].map(([category, c]) => ({ category, ...c })),
    statuses,
    logsAnalyzed,
    signalsBySeverity: bySeverity,
    importantSignals: open.filter((s) => ['high', 'critical'].includes(s.severity)).slice(0, 10),
    openSignals: open.length,
  });
}

// ---------------------------------------------------------------- overview, inbox and feed

const EXPLOITED = new Set(['CVE-2026-31337']);

export async function securityOverview(): Promise<SecurityOverview> {
  const now = Date.now();
  const sum = cs.summary();
  const setup = cs.setup();
  const vulns = cs.libraryVulnerabilities({ limit: 5000 }).vulnerabilities;
  const total = (c: { critical: number; high: number; medium: number; low: number; unknown: number }) => c.critical + c.high + c.medium + c.low + c.unknown;
  const sigs = signalsBetween(now - 7 * DAY, now);
  const attacked = new Set(sigs.filter((s) => s.ruleId === 'def-web-001').flatMap((s) => s.services));
  const start = now - (now % DAY) - 6 * DAY;
  return delay({
    repositories: cs.repositories().length,
    services: 9,
    endpoints: 64,
    hosts: 12,
    logSources: Object.keys(DEMO_SOURCES).length,
    codeSecurity: {
      sast: setup.repositoriesWithCodeScans > 0,
      scaStatic: setup.repositoriesWithLibraries > 0,
      iac: setup.repositoriesWithCodeScans > 0,
      secrets: setup.repositoriesWithCodeScans > 0,
      iast: setup.iastServices > 0,
      scaRuntime: setup.runtimeServices > 0,
      staticCodeVulnerabilities: total(sum.code),
      iacFindings: total(sum.iac),
      secretsFound: total(sum.secrets),
      staticLibraryVulnerabilities: sum.inRepositories,
      withExploit: vulns.filter((v) => [v.cve, ...v.aliases].some((c) => c && EXPLOITED.has(c))).length,
      runtimeCodeVulnerabilities: total(sum.runtimeCode),
      runtimeLibraryVulnerabilities: vulns.filter((v) => v.services.length).length,
      runtimeLibrariesInProduction: sum.inProduction,
      exposedToAttacks: vulns.filter((v) => v.inProduction && v.services.some((s) => attacked.has(s))).length,
    },
    siemPacksEnabled: PACKS.filter((p) => packEnabled.get(p.id) ?? true).length,
    importantSignals: sigs.filter((s) => s.state === 'open' && ['high', 'critical'].includes(s.severity)).length,
    appSecSignals: Array.from({ length: 7 }, (_, i) => [start + i * DAY, 0] as [number, number]),
  });
}

function inboxFindings(): InboxFinding[] {
  const out: InboxFinding[] = [];
  for (const v of cs.libraryVulnerabilities({ severity: ['critical', 'high'], limit: 5000 }).vulnerabilities) {
    const exploited = [v.cve, ...v.aliases].some((c) => c && EXPLOITED.has(c));
    if (!v.inProduction && !exploited) continue;
    out.push({
      key: `vuln:${v.advisoryId}:${v.ecosystem}:${v.package}`,
      title: `${v.title} in ${v.package}`,
      findingType: 'Library Vulnerability',
      severity: v.severity,
      resourceType: v.services.length ? 'Service' : 'Repository',
      resources: [...v.services, ...v.repositories],
      services: v.services,
      repositories: v.repositories,
      reasons: [...(v.inProduction ? ['Loaded in production'] : []), ...(exploited ? ['Exploited in the wild'] : [])],
      firstSeen: v.firstSeen,
      triage: 'pending',
      cve: v.cve ?? null,
      link: `/code-security/vulnerabilities/libraries?q=${v.advisoryId}`,
    });
  }
  for (const [kind, label, path] of [
    ['sast', 'Code Vulnerability', 'static-code'],
    ['secret', 'Secret', 'secrets'],
    ['iac', 'IaC Misconfiguration', 'iac'],
  ] as const) {
    for (const f of cs.findings(kind, { severity: kind === 'secret' ? [] : ['critical', 'high'], limit: 5000 }).findings) {
      out.push({
        key: `finding:${f.id}`,
        title: f.ruleName || f.message,
        findingType: label,
        severity: kind === 'secret' && !['critical', 'high'].includes(f.severity) ? 'high' : f.severity,
        resourceType: 'Repository',
        resources: [f.repository],
        services: [],
        repositories: [f.repository],
        reasons: [`${f.file}:${f.line}`],
        firstSeen: f.firstSeen,
        triage: 'pending',
        cve: null,
        link: `/code-security/vulnerabilities/${path}`,
      });
    }
  }
  for (const m of allMisconfigs().filter((x) => x.status === 'open' && ['critical', 'high'].includes(x.severity))) {
    out.push({
      key: m.key,
      title: m.ruleName,
      findingType: 'Misconfiguration',
      severity: m.severity,
      resourceType: m.resourceType,
      resources: [m.resourceName],
      services: m.service ? [m.service] : [],
      repositories: [],
      reasons: [m.detail],
      firstSeen: m.lastSeen,
      triage: 'pending',
      cve: null,
      link: `/security/cloud/findings?rule=${m.ruleId}`,
    });
  }
  for (const v of allVulns().filter((x) => x.status === 'open' && x.running && ['critical', 'high'].includes(x.severity))) {
    out.push({
      key: v.key,
      title: `${v.title} in ${v.package}`,
      findingType: 'Infrastructure Vulnerability',
      severity: v.severity,
      resourceType: v.resourceKind === 'host' ? 'Host' : 'Container Image',
      resources: [v.resourceName],
      services: [],
      repositories: [],
      reasons: ['Running', ...(v.exploitAvailable ? ['Exploited in the wild'] : []), ...(v.fixed ? [`Fixed in ${v.fixed}`] : [])],
      firstSeen: v.firstSeen,
      triage: 'pending',
      cve: v.cve ?? null,
      link: `/security/cloud/vulnerabilities?q=${v.advisoryId}`,
    });
  }
  return out.sort((a, b) => SEVERITY_RANK.indexOf(b.severity) - SEVERITY_RANK.indexOf(a.severity) || b.firstSeen - a.firstSeen);
}

export async function securityInbox(): Promise<SecurityInbox> {
  const findings = inboxFindings();
  const weekAgo = Date.now() - 7 * DAY;
  const count = (sev: string, before?: number) => findings.filter((f) => f.severity === sev && (before === undefined || f.firstSeen <= before)).length;
  const byType: Record<string, number> = {};
  for (const f of findings) byType[f.findingType] = (byType[f.findingType] ?? 0) + 1;
  return delay({ findings, critical: count('critical'), high: count('high'), criticalWeekAgo: count('critical', weekAgo), highWeekAgo: count('high', weekAgo), byType });
}

/** Fictional catalog entries (vendors and CVE numbers are made up for the demo). */
const KEV: [string, string, string, string, string, number, boolean, string[]][] = [
  [
    'CVE-2026-31337',
    'Example Software Foundation',
    'Example HTTP Parser',
    'Example HTTP Parser Request Smuggling Vulnerability',
    'The parser accepts conflicting length headers, letting an attacker smuggle a second request past a proxy.',
    2,
    true,
    ['CWE-444'],
  ],
  [
    'CVE-2026-28810',
    'Demo Networks',
    'EdgeGate VPN',
    'Demo Networks EdgeGate VPN Authentication Bypass',
    'A crafted session cookie bypasses authentication on the management interface.',
    5,
    true,
    ['CWE-287'],
  ],
  [
    'CVE-2026-27755',
    'Sample Systems',
    'FileShare Server',
    'Sample Systems FileShare Path Traversal Vulnerability',
    'Unauthenticated requests can read files outside the share root.',
    9,
    false,
    ['CWE-22'],
  ],
  [
    'CVE-2026-25104',
    'Acme Data',
    'Report Builder',
    'Acme Data Report Builder Deserialization of Untrusted Data',
    'Uploaded report templates are deserialized without checks, leading to remote code execution.',
    14,
    false,
    ['CWE-502'],
  ],
  ['CVE-2026-22917', 'Placeholder Inc.', 'Mail Gateway', 'Placeholder Mail Gateway OS Command Injection', 'An attachment name reaches a shell command unescaped.', 21, true, ['CWE-78']],
  [
    'CVE-2026-20001',
    'Example Software Foundation',
    'Example Template Engine',
    'Example Template Engine Server-Side Template Injection',
    'User input rendered as a template evaluates expressions on the server.',
    33,
    false,
    ['CWE-94'],
  ],
];
const WEB_CWES = ['CWE-77', 'CWE-78', 'CWE-79', 'CWE-89', 'CWE-22', 'CWE-94', 'CWE-502', 'CWE-918'];

export async function researchFeed(): Promise<FeedItem[]> {
  const now = Date.now();
  const vulns = cs.libraryVulnerabilities({ status: 'all', scope: 'all', limit: 5000 }).vulnerabilities;
  const byCve = new Map<string, Set<string>>();
  for (const v of vulns) for (const id of [v.cve, ...v.aliases, v.advisoryId]) if (id) byCve.set(id, new Set([...(byCve.get(id) ?? []), ...v.services, ...v.repositories]));
  const items: FeedItem[] = KEV.map(([cve, vendor, product, name, description, days, ransomware, cwes]) => {
    const res = [...(byCve.get(cve) ?? [])];
    return {
      id: cve,
      articleType: 'exploited_vulnerability',
      title: `${name} (${cve})`,
      summary: description,
      publishedAt: now - (now % DAY) - days * DAY,
      source: 'CISA Known Exploited Vulnerabilities',
      url: '',
      cve,
      severity: null,
      products: ['Code Security', ...(cwes.some((c) => WEB_CWES.includes(c)) ? ['App & API Protection'] : []), 'Cloud SIEM'],
      vendor,
      product,
      ransomware,
      requiredAction: 'Apply mitigations per vendor instructions or discontinue use of the product if mitigations are unavailable.',
      dueDate: new Date(now + (21 - days) * DAY).toISOString().slice(0, 10),
      impacted: res.length > 0 || cve === 'CVE-2026-31337',
      impactedResources: res.length ? res : cve === 'CVE-2026-31337' ? ['storefront-api', 'api-gateway'] : [],
    };
  });
  for (const a of cs.summary().emerging.filter((x) => ['critical', 'high'].includes(x.severity))) {
    items.push({
      id: a.id,
      articleType: 'emerging_vulnerability',
      title: `${a.summary} (${a.cve ?? a.id})`,
      summary: a.summary,
      publishedAt: a.published ?? now,
      source: 'Code Security advisories',
      url: '',
      cve: a.cve ?? null,
      severity: a.severity,
      products: ['Code Security'],
      vendor: '',
      product: a.packages.join(', '),
      ransomware: false,
      requiredAction: '',
      dueDate: null,
      impacted: a.impacted,
      impactedResources: [...(byCve.get(a.id) ?? [])],
    });
  }
  return delay(items.sort((a, b) => b.publishedAt - a.publishedAt));
}
