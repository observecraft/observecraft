// The GitLab Audit Events dashboard. Datadog's GitLab Audit Events tile is collected by Datadog's own service and its
// dashboard isn't in integrations-core, so this is ObserveCraft's own, over what the GitLab Audit Events integration
// collects (server/crates/oc-api/src/gitlab_audit): the instance's (or a group's, a project's) audit events, logs with
// `source:gitlab-audit-events` (`@evt.name` the event type, `@evt.outcome`, `@usr.*` the author, `@network.client.ip`
// the author's address, `@gitlab.entity_*` / `@gitlab.target_*` what it is about, `@gitlab.details.*` GitLab's details
// and `@gitlab.category` from the GitLab Audit Events log pipeline, which saving the connection installs), and the
// Cloud SIEM signals of its content pack. Written in the Datadog widget format the integration dashboards use and
// converted like them (import-integration-dashboards.mjs).
// node scripts/gitlab-audit-events-dashboard.mjs   (writes server/seeds/integrations/gitlab-audit-events/dashboards.json)
import { mkdirSync, writeFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { BAD, GOOD, NONE, WARN, bars, buildOwnDashboard, layout, logStream, logs, note, pie, queryValue, signals, table, timeseries, toplist, treemap } from './own-dashboard-widgets.mjs';

const EVENTS = 'source:gitlab-audit-events';
const category = (c) => `${EVENTS} @gitlab.category:${c}`;
const FAILED_LOGINS = `${EVENTS} @evt.name:*login_failed*`;
const TOKENS = category('token');
const TOKENS_CREATED = `${EVENTS} @evt.name:*token_created`;
const TOKENS_REVOKED = `${EVENTS} @evt.name:(*token_revoked OR *token_destroyed)`;
const TOKENS_ROTATED = `${EVENTS} @evt.name:*token_rotated`;
const MRS = category('merge_request');
const MERGED = `${EVENTS} @evt.name:merge_request_merged*`;
const APPROVALS = `${EVENTS} @evt.name:merge_request_approval_operation`;
const APPROVAL_RULES = `${EVENTS} @evt.name:(update_approval_rules OR *approval_rule* OR *approval_setting*)`;
const MEMBERS = category('membership');
const AUTHORIZED = category('authorization');
const PROJECTS = category('project');
const SIGNALS = EVENTS;

function dashboard() {
  const { at, group } = layout();
  const intro = group(
    '',
    'vivid_blue',
    [
      // Our logo tile (buildOwnDashboard).
      at({ type: 'image', url: '' }, 0, 0, 2, 2),
      at(
        note(
          "**GitLab audit events** record who did what on a GitLab instance: sign-ins and failed logins, access tokens created, rotated and revoked, members and their roles, OAuth applications authorized, merge request approvals and merges, approval rules, project and group settings, CI/CD variables and runners. This dashboard follows the connected instance's (or group's) audit events, the users and addresses behind them, and the Cloud SIEM signals their detection rules raise.\n\nConnect GitLab in [Integrations → GitLab Audit Events](/integrations/gitlab-audit-events): ObserveCraft reads the audit events API every minute (GitLab Premium or Ultimate), with no access to ObserveCraft needed.",
          'white',
        ),
        2,
        0,
        10,
        2,
      ),
    ],
    0,
    3,
    { show_title: false },
  );
  const overview = group(
    'Overview',
    'blue',
    [
      at(queryValue('Audit events', [logs(EVENTS)], { formats: NONE, timeseries: true }), 0, 0, 2, 2),
      at(queryValue('Users', [logs(EVENTS, { aggregation: 'cardinality', metric: '@usr.name' })], { formats: GOOD }), 2, 0, 2, 2),
      at(queryValue('Failed logins', [logs(FAILED_LOGINS)], { formats: BAD, timeseries: true }), 4, 0, 2, 2),
      at(queryValue('Tokens created', [logs(TOKENS_CREATED)], { formats: WARN, timeseries: true }), 6, 0, 2, 2),
      at(queryValue('Merge requests merged', [logs(MERGED)], { formats: NONE, timeseries: true }), 8, 0, 2, 2),
      at(queryValue('Security signals', [signals(SIGNALS)], { formats: BAD }), 10, 0, 2, 2),
      at(timeseries('Audit events by category', [logs(EVENTS, { groupBy: ['@gitlab.category'] })], { display: 'bars' }), 0, 2, 8, 3),
      at(pie('Events by category', [logs(EVENTS, { groupBy: ['@gitlab.category'] })]), 8, 2, 4, 3),
      at(toplist('Most frequent events', logs(EVENTS, { groupBy: ['@evt.name'] })), 0, 5, 6, 4),
      at(treemap('Events by group or project', logs(EVENTS, { groupBy: ['@gitlab.entity_path'] })), 6, 5, 6, 4),
    ],
    3,
    10,
  );
  const users = group(
    'User insights',
    'green',
    [
      at(toplist('Most active users', logs(EVENTS, { groupBy: ['@usr.name'] }), { palette: 'cool' }), 0, 0, 4, 4),
      at(toplist('Most active addresses', logs(EVENTS, { groupBy: ['@network.client.ip'] }), { palette: 'cool' }), 4, 0, 4, 4),
      at(pie('Events by entity type', [logs(EVENTS, { groupBy: ['@gitlab.entity_type'] })]), 8, 0, 4, 4),
      at(timeseries('Failed logins by user', [logs(FAILED_LOGINS, { groupBy: ['@usr.name'] })], { display: 'bars', palette: 'warm' }), 0, 4, 6, 3),
      at(toplist('Addresses failing to log in', logs(FAILED_LOGINS, { groupBy: ['@network.client.ip'] }), { palette: 'warm' }), 6, 4, 6, 3),
      at(
        table('Users by activity', [
          ['events', logs(EVENTS, { groupBy: ['@usr.name'], limit: 25 })],
          ['event types', logs(EVENTS, { groupBy: ['@usr.name'], limit: 25, aggregation: 'cardinality', metric: '@evt.name' })],
          ['addresses', logs(EVENTS, { groupBy: ['@usr.name'], limit: 25, aggregation: 'cardinality', metric: '@network.client.ip' })],
        ]),
        0,
        7,
        6,
        4,
      ),
      at(logStream('Failed logins', FAILED_LOGINS), 6, 7, 6, 4),
    ],
    13,
    11,
  );
  const authorization = group(
    'Authorization granted',
    'purple',
    [
      at(timeseries('Members added, changed and removed', [logs(MEMBERS, { groupBy: ['@evt.name'] })], { display: 'bars', palette: 'purple' }), 0, 0, 6, 3),
      at(bars('Membership changes by group or project', logs(MEMBERS, { groupBy: ['@gitlab.entity_path'] })), 6, 0, 6, 3),
      at(toplist('Users granting access', logs(MEMBERS, { groupBy: ['@usr.name'] }), { palette: 'purple' }), 0, 3, 4, 4),
      at(toplist('OAuth applications authorized', logs(AUTHORIZED, { groupBy: ['@gitlab.target_details'] }), { palette: 'purple' }), 4, 3, 4, 4),
      at(logStream('Members and authorizations', `${EVENTS} @gitlab.category:(membership OR authorization)`), 8, 3, 4, 4),
    ],
    24,
    7,
  );
  const mergeRequests = group(
    'Merge requests',
    'orange',
    [
      at(queryValue('Merged', [logs(MERGED)], { formats: GOOD, timeseries: true }), 0, 0, 3, 2),
      at(queryValue('Approvals', [logs(APPROVALS)], { formats: NONE, timeseries: true }), 3, 0, 3, 2),
      at(queryValue('Approval rules changed', [logs(APPROVAL_RULES)], { formats: WARN }), 6, 0, 3, 2),
      at(queryValue('Merged by bots', [logs(`${EVENTS} @evt.name:merge_request_merged_by_project_bot`)], { formats: NONE }), 9, 0, 3, 2),
      at(timeseries('Merge request events', [logs(MRS, { groupBy: ['@evt.name'] })], { display: 'bars' }), 0, 2, 8, 3),
      at(toplist('Projects merging most', logs(MERGED, { groupBy: ['@gitlab.entity_path'] })), 8, 2, 4, 3),
      at(toplist('Users approving most', logs(APPROVALS, { groupBy: ['@usr.name'] })), 0, 5, 4, 4),
      at(logStream('Approval rules and settings changed', APPROVAL_RULES), 4, 5, 8, 4),
    ],
    31,
    9,
  );
  const projects = group(
    'Project activity',
    'gray',
    [
      at(timeseries('Project events', [logs(PROJECTS, { groupBy: ['@evt.name'] })], { display: 'bars' }), 0, 0, 6, 3),
      at(bars('Projects with the most changes', logs(PROJECTS, { groupBy: ['@gitlab.entity_path'] })), 6, 0, 6, 3),
      at(pie('Repository and CI/CD changes', [logs(`${EVENTS} @gitlab.category:(repository OR ci_cd)`, { groupBy: ['@evt.name'] })]), 0, 3, 4, 4),
      at(
        logStream('Projects created, deleted, exported or made more visible', `${EVENTS} @evt.name:(project_created OR project_destroyed OR project_visibility_level_updated OR project_export_*)`),
        4,
        3,
        8,
        4,
      ),
    ],
    40,
    7,
  );
  const tokens = group(
    'Token management',
    'yellow',
    [
      at(
        timeseries('Tokens created, rotated and revoked', [logs(TOKENS_CREATED), logs(TOKENS_ROTATED, { name: 'query2' }), logs(TOKENS_REVOKED, { name: 'query3' })], {
          formulas: [
            { alias: 'created', formula: 'query1' },
            { alias: 'rotated', formula: 'query2' },
            { alias: 'revoked', formula: 'query3' },
          ],
          display: 'bars',
          palette: 'warm',
        }),
        0,
        0,
        8,
        3,
      ),
      at(pie('Token events', [logs(TOKENS, { groupBy: ['@evt.name'] })]), 8, 0, 4, 3),
      at(toplist('Users creating tokens', logs(TOKENS_CREATED, { groupBy: ['@usr.name'] }), { palette: 'warm' }), 0, 3, 4, 4),
      at(logStream('Token events', TOKENS), 4, 3, 8, 4),
    ],
    47,
    7,
  );
  const siem = group(
    'Cloud SIEM signals',
    'red',
    [
      at(queryValue('High and critical', [signals(`${SIGNALS} status:(high OR critical)`)], { formats: BAD }), 0, 0, 3, 2),
      at(queryValue('Medium', [signals(`${SIGNALS} status:medium`)], { formats: WARN }), 0, 2, 3, 2),
      at(timeseries('Signals by severity', [signals(SIGNALS, { groupBy: ['status'] })], { display: 'bars', palette: 'warm' }), 3, 0, 5, 4),
      at(toplist('Signals by rule', signals(SIGNALS, { groupBy: ['@workflow.rule.name'] }), { palette: 'warm' }), 8, 0, 4, 4),
    ],
    54,
    5,
  );
  const all = group('All events', 'white', [at(logStream('Audit events', EVENTS), 0, 0, 12, 5)], 59, 6);
  return {
    title: 'GitLab Audit Events Overview',
    description:
      "A GitLab instance's audit events: users and addresses, failed logins, members and authorizations, merge requests, project activity, access tokens and the Cloud SIEM signals they raise.",
    layout_type: 'ordered',
    template_variables: [],
    widgets: [intro, overview, users, authorization, mergeRequests, projects, tokens, siem, all],
  };
}

export const buildDashboard = () => buildOwnDashboard(dashboard(), { integration: 'gitlab-audit-events', name: 'GitLab Audit Events', script: 'scripts/gitlab-audit-events-dashboard.mjs' });

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const dir = new URL('../server/seeds/integrations/gitlab-audit-events/', import.meta.url);
  mkdirSync(dir, { recursive: true });
  writeFileSync(new URL('dashboards.json', dir), JSON.stringify([buildDashboard()], null, 2) + '\n');
  console.log('wrote server/seeds/integrations/gitlab-audit-events/dashboards.json');
}
