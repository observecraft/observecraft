// In-browser stand-in for Integrations → GitLab Audit Events (routes/gitlab_audit_integration.rs). The connection
// lives in memory for the session; any token of 20 characters or more is accepted, and one starting with `glpat-user`
// belongs to a user who isn't an administrator. Once saved, the last sync follows the poller's minute.
// `gitlabAuditLogs` is the mock audit log (`source:gitlab-audit-events`), shaped as crate::gitlab_audit::logs makes it
// and the GitLab Audit Events log pipeline completes it (`gitlab.category`).
import type { components } from '../generated/schema';
import type { JsonValue, LogEvent, LogStatus } from '../types';
import type { Rand } from './generator';

type S = components['schemas'];
type Input = S['GitlabAuditInput'];
type Status = S['GitlabAuditStatus'];
type Test = S['GitlabAuditTest'];

const MIN = 60_000;

let saved: { baseUrl: string; scope: string; path: string; at: number } | null = null;
let syncedAt = 0;

const delay = <T>(value: T, ms = 150) => new Promise<T>((r) => setTimeout(() => r(structuredClone(value)), ms));

function fail(message: string): never {
  throw new Error(message);
}

function checked(input: Input): { baseUrl: string; scope: string; path: string } {
  const baseUrl = ((input.baseUrl ?? '').trim() || 'https://gitlab.com').replace(/\/+$/, '');
  if (!/^https:\/\/[^\s/@]+/.test(baseUrl)) fail('The base URL must use https');
  const scope = (input.scope ?? '').trim() || 'instance';
  if (!['instance', 'group', 'project'].includes(scope)) fail(`${scope} is not a scope: instance, group or project`);
  const path = scope === 'instance' ? '' : (input.path ?? '').trim().replace(/^\/+|\/+$/g, '');
  if (scope !== 'instance' && !/^[\w.-]+(\/[\w.-]+)*$/.test(path))
    fail(scope === 'group' ? "Enter the group's path (example-org or example-org/platform)" : "Enter the project's path (example-org/storefront-api)");
  return { baseUrl, scope, path };
}

function status(): Status {
  if (!saved) return { connection: null, lastSyncAt: null, syncedUntil: null, eventsLastSync: 0, catchingUp: false, error: null };
  const now = Date.now();
  const lastSync = Math.max(syncedAt, now - ((now - saved.at) % MIN));
  return {
    connection: { baseUrl: saved.baseUrl, scope: saved.scope, path: saved.path, updatedAt: saved.at },
    lastSyncAt: lastSync,
    syncedUntil: lastSync - 60_000,
    eventsLastSync: 9,
    catchingUp: false,
    error: null,
  };
}

export const gitlabAuditStatus = (): Promise<Status> => delay(status());

/** Whether the mock connection is saved: the catalog tile is installed, as the server counts a saved connection. */
export const gitlabAuditConnected = (): boolean => !!saved;

export function testGitlabAudit(input: Input): Promise<Test> {
  const c = checked(input);
  const token = input.token?.trim() || (saved?.baseUrl === c.baseUrl ? 'glpat-saved-token-of-the-mock' : '');
  if (!token) fail('GitLab needs an access token');
  if (token.length < 20) return delay({ ok: false, message: 'GitLab refused the token: The provider answered: the credentials were refused (401 Unauthorized)', user: null, admin: null }, 400);
  const admin = !token.startsWith('glpat-user');
  const user = admin ? 'audit-admin' : 'audit-reader';
  if (c.scope === 'instance' && !admin)
    return delay({ ok: false, message: `${user} is not an administrator: the instance's audit events need an administrator's token (or read a group's)`, user, admin }, 400);
  const what = c.scope === 'instance' ? 'the instance' : `${c.scope} ${c.path}`;
  return delay({ ok: true, message: `Connected as ${user}: reading the audit events of ${what}`, user, admin }, 400);
}

export function saveGitlabAudit(input: Input): Promise<Status> {
  const c = checked(input);
  if (!input.token?.trim() && saved?.baseUrl !== c.baseUrl) fail(saved ? 'Enter the token again: the GitLab URL changed' : 'GitLab needs an access token');
  saved = { ...c, at: Date.now() };
  return delay(status(), 250);
}

export function deleteGitlabAudit(): Promise<void> {
  if (!saved) fail('GitLab Audit Events is not connected');
  saved = null;
  return delay(undefined);
}

export function syncGitlabAudit(): Promise<void> {
  if (!saved) fail('GitLab Audit Events is not connected');
  syncedAt = Date.now();
  return delay(undefined);
}

// ---------------------------------------------------------------- the audit log

const HOUR = 60 * MIN;
const HOST = 'gitlab.example.com';

/** id, name, email (fictional) */
const PEOPLE: [number, string, string][] = [
  [42, 'Jordan Patel', 'jordan.patel@example.com'],
  [43, 'Sam Lee', 'sam.lee@example.com'],
  [44, 'Alex Morgan', 'alex.morgan@example.com'],
  [45, 'Riley Chen', 'riley.chen@example.com'],
  [46, 'Casey Nguyen', 'casey.nguyen@example.com'],
];
const MAINTAINERS = PEOPLE.slice(0, 2);
const CLIENTS = ['203.0.113.24', '203.0.113.80', '198.51.100.14', '192.0.2.55'];
const ATTACKERS = ['198.51.100.7', '192.0.2.200', '203.0.113.150'];
const PROJECTS: [number, string][] = [
  [4211, 'example-org/storefront-api'],
  [4212, 'example-org/order-service'],
  [4213, 'example-org/billing'],
  [4214, 'example-org/catalog-service'],
  [4215, 'example-org/platform/inventory-service'],
];
const GROUPS: [number, string][] = [
  [60, 'example-org'],
  [61, 'example-org/platform'],
];
const APPS = ['ObserveCraft', 'Argo CD', 'Renovate', 'Backstage'];
const BOT = [77, 'project_4211_bot', ''] as [number, string, string];
const pick = <T>(r: Rand, xs: T[]) => xs[Math.floor(r() * xs.length)];

/** event name, weight, the pipeline's category */
const EVENTS: [string, number, string][] = [
  ['login_failed_with_standard_authentication', 12, 'authentication'],
  ['login_failed_with_otp_authentication', 3, 'authentication'],
  ['user_access_locked', 1, 'authentication'],
  ['personal_access_token_created', 5, 'token'],
  ['personal_access_token_revoked', 2, 'token'],
  ['personal_access_token_rotated', 2, 'token'],
  ['project_access_token_created', 2, 'token'],
  ['group_deploy_token_created', 1, 'token'],
  ['merge_request_create', 8, 'merge_request'],
  ['merge_request_approval_operation', 12, 'merge_request'],
  ['merge_request_merged', 14, 'merge_request'],
  ['merge_request_merged_by_project_bot', 2, 'merge_request'],
  ['update_approval_rules', 3, 'merge_request'],
  ['member_created', 4, 'membership'],
  ['member_updated', 2, 'membership'],
  ['member_destroyed', 1, 'membership'],
  ['user_authorized_oauth_application', 4, 'authorization'],
  ['user_blocked', 1, 'user_management'],
  ['user_created', 1, 'user_management'],
  ['ci_variable_created', 2, 'ci_cd'],
  ['ci_variable_updated', 2, 'ci_cd'],
  ['ci_runner_created', 1, 'ci_cd'],
  ['protected_branch_allow_force_push_updated', 1, 'repository'],
  ['project_created', 2, 'project'],
  ['project_destroyed', 1, 'project'],
  ['project_visibility_level_updated', 1, 'project'],
  ['project_export_file_download_started', 1, 'project'],
  ['group_created', 1, 'group'],
];
const TOTAL = EVENTS.reduce((n, e) => n + e[1], 0);

/** `merge_request_merged` → `Merge request merged`. */
const words = (name: string) => name.charAt(0).toUpperCase() + name.slice(1).replaceAll('_', ' ');

/** A week of audit events, as the GitLab Audit Events poller stores them. */
export function gitlabAuditLogs(r: Rand, now: number): LogEvent[] {
  const out: LogEvent[] = [];
  for (let i = 0; i < 650; i++) {
    // A week, denser in the last hours so short ranges look alive.
    const ts = Math.round(now - r() * (i < 250 ? 3 * HOUR : 7 * 24 * HOUR));
    let w = r() * TOTAL;
    const [name, , category] = EVENTS.find((e) => (w -= e[1]) < 0) ?? EVENTS[0];
    const failedLogin = name.startsWith('login_failed');
    let person: [number, string, string] = name.startsWith('member_') || name.startsWith('update_') || name.startsWith('group_') ? pick(r, MAINTAINERS) : pick(r, PEOPLE);
    if (name.endsWith('_by_project_bot')) person = BOT;
    // A failed login's user is the one tried (no author: nobody is signed in).
    if (failedLogin) person = [-1, r() < 0.6 ? pick(r, ['root', 'admin']) : person[1], ''];
    const [projectId, projectPath] = pick(r, PROJECTS);
    const [groupId, groupPath] = pick(r, GROUPS);
    const onGroup = name.startsWith('group_') || (name.startsWith('member_') && r() < 0.3);
    const onUser = failedLogin || name.startsWith('user_') || name.startsWith('personal_access_token');
    const [entityType, entityId, entityPath] = onUser ? ['User', person[0], person[1]] : onGroup ? ['Group', groupId, groupPath] : ['Project', projectId, projectPath];
    const details: Record<string, JsonValue> = {};
    let target = entityPath;
    let targetType = entityType;
    let message = '';
    let ip = pick(r, CLIENTS);
    if (failedLogin) {
      ip = r() < 0.7 ? pick(r, ATTACKERS) : ip;
      details.failed_login = name.includes('otp') ? 'OTP' : 'STANDARD';
      message = `Failed to login with ${details.failed_login} authentication (${person[1]})`;
    } else if (name === 'user_access_locked') {
      message = `User access locked by ${person[1]} on ${person[1]}`;
    } else if (name.endsWith('_token_created') || name.endsWith('_token_revoked') || name.endsWith('_token_rotated')) {
      const id = 100 + Math.floor(r() * 900);
      const verb = name.split('_').at(-1);
      const kind = name.replace(/_(created|revoked|rotated)$/, '').replaceAll('_', ' ');
      message = `${words(verb ?? '')} ${kind} with id ${id} (${person[1]})`;
      details.custom_message = message;
      targetType = 'PersonalAccessToken';
      target = `${kind} ${id}`;
    } else if (name.startsWith('merge_request')) {
      const iid = 100 + Math.floor(r() * 400);
      targetType = 'MergeRequest';
      target = `${projectPath}!${iid}`;
      message =
        name === 'merge_request_approval_operation'
          ? `Approved merge request !${iid} (${person[1]})`
          : name === 'merge_request_create'
            ? `Created merge request !${iid} (${person[1]})`
            : `Merged merge request !${iid} (${person[1]})`;
      details.custom_message = message.replace(/ \(.*\)$/, '');
    } else if (name === 'update_approval_rules') {
      details.change = 'approvals_required';
      details.from = '1';
      details.to = '2';
      message = `${person[1]} changed approvals_required from 1 to 2 on ${projectPath}`;
    } else if (name === 'member_created') {
      const added = pick(r, PEOPLE.slice(2));
      details.add = 'user_access';
      details.as = pick(r, ['Developer', 'Reporter', 'Maintainer']);
      target = added[1];
      targetType = 'User';
      message = `${person[1]} added user_access as ${details.as} on ${added[1]}`;
    } else if (name === 'member_updated') {
      details.change = 'access_level';
      details.from = 'Developer';
      details.to = 'Maintainer';
      target = pick(r, PEOPLE.slice(2))[1];
      targetType = 'User';
      message = `${person[1]} changed access_level from Developer to Maintainer on ${target}`;
    } else if (name === 'member_destroyed') {
      details.remove = 'user_access';
      target = pick(r, PEOPLE.slice(2))[1];
      targetType = 'User';
      message = `${person[1]} removed user_access on ${target}`;
    } else if (name === 'user_authorized_oauth_application') {
      target = pick(r, APPS);
      targetType = 'Doorkeeper::Application';
      message = `User authorized an OAuth application ${target} (${person[1]})`;
      details.custom_message = message.replace(/ \(.*\)$/, '');
    } else if (name === 'project_visibility_level_updated') {
      details.change = 'visibility';
      details.from = 'Private';
      details.to = r() < 0.5 ? 'Public' : 'Internal';
      message = `${person[1]} changed visibility from Private to ${details.to} on ${projectPath}`;
    } else if (name.startsWith('ci_variable')) {
      details.change = 'variable';
      target = pick(r, ['DEPLOY_TOKEN', 'AWS_ROLE_ARN', 'SENTRY_DSN']);
      targetType = 'Ci::Variable';
      message = `${words(name)} by ${person[1]} on ${target}`;
    } else {
      message = `${words(name)} by ${person[1]} on ${entityPath}`;
    }
    const failure = failedLogin;
    const status: LogStatus = failure ? 'error' : name === 'user_access_locked' || name === 'user_blocked' ? 'warn' : 'info';
    const usr: Record<string, JsonValue> = { name: person[1] };
    if (person[0] > 0) usr.id = String(person[0]);
    if (person[2]) usr.email = person[2];
    const gitlab: Record<string, JsonValue> = {
      audit_event_id: String(10_000 + i),
      entity_type: entityType,
      entity_path: entityPath,
      target_type: targetType,
      target_details: target,
      author_class: 'User',
      category,
    };
    if (entityId > 0) gitlab.entity_id = String(entityId);
    if (Object.keys(details).length) gitlab.details = details;
    out.push({
      id: `gitlab-audit-${ts}-${i}`,
      timestamp: ts,
      status,
      service: 'gitlab',
      host: HOST,
      source: 'gitlab-audit-events',
      message,
      tags: {},
      attributes: { evt: { name, outcome: failure ? 'failure' : 'success' }, usr, network: { client: { ip } }, gitlab },
    });
  }
  return out;
}
