// The Google Workspace dashboard. Datadog's Google Workspace integration is collected by Datadog's own service and its
// dashboards aren't in integrations-core, so this is ObserveCraft's own, over what the Google Workspace integration
// collects (server/crates/oc-api/src/google_workspace): the Admin SDK Reports API's activities, one log per event with
// `source:gsuite` and `service:` the application (`login`, `admin`, `drive`, `token`, `groups`, `mobile`, …),
// `@evt.name` the event, `@usr.email` the actor, `@network.client.ip` and its country, `@event.parameters.*` the
// event's parameters and `@google_workspace.category` from the Google Workspace log pipeline (which saving the
// connection installs), and the Cloud SIEM signals of its content pack. Written in the Datadog widget format the
// integration dashboards use and converted like them (import-integration-dashboards.mjs).
// node scripts/google-workspace-dashboard.mjs   (writes server/seeds/integrations/google-workspace/dashboards.json)
import { mkdirSync, writeFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import {
  BAD,
  GOOD,
  NONE,
  WARN,
  anyOf,
  bars,
  buildOwnDashboard,
  geomap,
  layout,
  logStream,
  logs,
  note,
  pie,
  queryValue,
  signals,
  table,
  timeseries,
  toplist,
  treemap,
} from './own-dashboard-widgets.mjs';

const EVENTS = 'source:gsuite';
const app = (a) => `${EVENTS} service:${a}`;
const COUNTRY = '@network.client.geoip.country.iso_code';
const LOGINS = `${app('login')} ${anyOf(['login_success', 'login_failure'])}`;
const LOGIN_OK = `${app('login')} @evt.name:login_success`;
const LOGIN_FAILED = `${app('login')} @evt.name:login_failure`;
const SUSPICIOUS = `${app('login')} ${anyOf(['suspicious_login', 'suspicious_login_less_secure_app', 'suspicious_programmatic_login', 'gov_attack_warning'])}`;
const CHALLENGES = `${app('login')} @evt.name:login_challenge`;
const TWO_STEP = `${app('user_accounts')} ${anyOf(['2sv_enroll', '2sv_disable'])}`;
const TWO_STEP_OFF = `${app('user_accounts')} @evt.name:2sv_disable`;
const PASSWORDS = `${app('user_accounts')} ${anyOf(['password_edit', 'recovery_email_edit', 'recovery_phone_edit'])}`;
const ADMIN = app('admin');
const ROLES = `${ADMIN} ${anyOf(['ASSIGN_ROLE', 'UNASSIGN_ROLE', 'CREATE_ROLE', 'ADD_PRIVILEGE', 'GRANT_ADMIN_PRIVILEGE'])}`;
const USERS_CHANGED = `${ADMIN} ${anyOf(['CREATE_USER', 'DELETE_USER', 'SUSPEND_USER', 'UNSUSPEND_USER', 'CHANGE_PASSWORD', 'RENAME_USER'])}`;
const SETTINGS = `${ADMIN} @event.type:(APPLICATION_SETTINGS OR SECURITY_SETTINGS OR DOMAIN_SETTINGS)`;
const DRIVE = app('drive');
const SHARED_OUT = `${DRIVE} @event.parameters.visibility:(people_with_link OR public_on_the_web OR shared_externally)`;
const PUBLIC = `${DRIVE} @event.parameters.visibility:(people_with_link OR public_on_the_web)`;
const DOWNLOADS = `${DRIVE} @evt.name:download`;
const TOKENS = `${app('token')} @evt.name:authorize`;
const REVOKED = `${app('token')} @evt.name:revoke`;
const GROUPS = `${EVENTS} @google_workspace.category:groups`;
const DEVICES = app('mobile');
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
          "**Google Workspace audit activities** record who signed in and from where, what administrators changed in the Admin console, who shared Drive files outside the domain, which OAuth apps users authorized, and group, account and device changes. This dashboard follows them, the users and addresses behind them, and the Cloud SIEM signals their detection rules raise.\n\nConnect Google Workspace in [Integrations → Google Workspace](/integrations/google-workspace): a service account reads the Admin SDK Reports API every five minutes, with no access to ObserveCraft needed. Google delivers some applications' activities hours late; they appear here as they arrive.",
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
      at(queryValue('Activities', [logs(EVENTS)], { formats: NONE, timeseries: true }), 0, 0, 2, 2),
      at(queryValue('Users', [logs(EVENTS, { aggregation: 'cardinality', metric: '@usr.email' })], { formats: GOOD }), 2, 0, 2, 2),
      at(queryValue('Failed logins', [logs(LOGIN_FAILED)], { formats: WARN, timeseries: true }), 4, 0, 2, 2),
      at(queryValue('Suspicious logins', [logs(SUSPICIOUS)], { formats: BAD, timeseries: true }), 6, 0, 2, 2),
      at(queryValue('Files shared outside', [logs(SHARED_OUT)], { formats: WARN, timeseries: true }), 8, 0, 2, 2),
      at(queryValue('Security signals', [signals(SIGNALS)], { formats: BAD }), 10, 0, 2, 2),
      at(timeseries('Activities by application', [logs(EVENTS, { groupBy: ['service'] })], { display: 'bars' }), 0, 2, 8, 3),
      at(pie('Activities by category', [logs(EVENTS, { groupBy: ['@google_workspace.category'] })]), 8, 2, 4, 3),
      at(toplist('Most frequent events', logs(EVENTS, { groupBy: ['@evt.name'] })), 0, 5, 6, 4),
      at(geomap('Activities around the world', logs(EVENTS, { groupBy: [COUNTRY], limit: 250 })), 6, 5, 6, 4),
    ],
    3,
    10,
  );
  const logins = group(
    'Logins',
    'green',
    [
      at(queryValue('Successful logins', [logs(LOGIN_OK)], { formats: GOOD, timeseries: true }), 0, 0, 3, 2),
      at(queryValue('Login challenges', [logs(CHALLENGES)], { formats: NONE, timeseries: true }), 3, 0, 3, 2),
      at(queryValue('2-step verification turned off', [logs(TWO_STEP_OFF)], { formats: BAD }), 6, 0, 3, 2),
      at(queryValue('Passwords and recovery changed', [logs(PASSWORDS)], { formats: NONE }), 9, 0, 3, 2),
      at(
        timeseries('Logins and failed logins', [logs(LOGIN_OK), logs(LOGIN_FAILED, { name: 'query2' })], {
          formulas: [
            { alias: 'succeeded', formula: 'query1' },
            { alias: 'failed', formula: 'query2' },
          ],
          display: 'bars',
        }),
        0,
        2,
        8,
        3,
      ),
      at(pie('Logins by type', [logs(LOGINS, { groupBy: ['@event.parameters.login_type'] })]), 8, 2, 4, 3),
      at(toplist('Users failing to log in', logs(LOGIN_FAILED, { groupBy: ['@usr.email'] }), { palette: 'warm' }), 0, 5, 4, 4),
      at(toplist('Addresses failing to log in', logs(LOGIN_FAILED, { groupBy: ['@network.client.ip'] }), { palette: 'warm' }), 4, 5, 4, 4),
      at(geomap('Failed logins around the world', logs(LOGIN_FAILED, { groupBy: [COUNTRY], limit: 250 })), 8, 5, 4, 4),
      at(timeseries('2-step verification enrolled and turned off', [logs(TWO_STEP, { groupBy: ['@evt.name'] })], { display: 'bars', palette: 'purple' }), 0, 9, 6, 3),
      at(logStream('Suspicious logins', SUSPICIOUS), 6, 9, 6, 3),
    ],
    13,
    13,
  );
  const admin = group(
    'Admin activity',
    'purple',
    [
      at(timeseries('Admin console changes', [logs(ADMIN, { groupBy: ['@event.type'] })], { display: 'bars', palette: 'purple' }), 0, 0, 8, 3),
      at(toplist('Most active administrators', logs(ADMIN, { groupBy: ['@usr.email'] }), { palette: 'purple' }), 8, 0, 4, 3),
      at(toplist('Most frequent changes', logs(ADMIN, { groupBy: ['@evt.name'] }), { palette: 'purple' }), 0, 3, 4, 4),
      at(bars('Users created, suspended and deleted', logs(USERS_CHANGED, { groupBy: ['@evt.name'] })), 4, 3, 4, 4),
      at(logStream('Roles and privileges granted', ROLES), 8, 3, 4, 4),
      at(logStream('Security, domain and application settings changed', SETTINGS), 0, 7, 12, 4),
    ],
    26,
    11,
  );
  const drive = group(
    'Drive sharing',
    'orange',
    [
      at(queryValue('Files made public', [logs(PUBLIC)], { formats: BAD, timeseries: true }), 0, 0, 3, 2),
      at(queryValue('Downloads', [logs(DOWNLOADS)], { formats: NONE, timeseries: true }), 3, 0, 3, 2),
      at(queryValue('Users sharing outside', [logs(SHARED_OUT, { aggregation: 'cardinality', metric: '@usr.email' })], { formats: WARN }), 6, 0, 3, 2),
      at(queryValue('Files shared outside', [logs(SHARED_OUT, { aggregation: 'cardinality', metric: '@event.parameters.doc_id' })], { formats: WARN }), 9, 0, 3, 2),
      at(timeseries('Drive activity', [logs(DRIVE, { groupBy: ['@evt.name'] })], { display: 'bars' }), 0, 2, 8, 3),
      at(pie('Sharing by visibility', [logs(SHARED_OUT, { groupBy: ['@event.parameters.visibility'] })]), 8, 2, 4, 3),
      at(toplist('Users sharing outside the domain', logs(SHARED_OUT, { groupBy: ['@usr.email'] }), { palette: 'warm' }), 0, 5, 4, 4),
      at(treemap('Files shared outside the domain', logs(SHARED_OUT, { groupBy: ['@event.parameters.doc_title'] })), 4, 5, 4, 4),
      at(logStream('Files made public', PUBLIC), 8, 5, 4, 4),
    ],
    37,
    9,
  );
  const tokens = group(
    'OAuth apps',
    'yellow',
    [
      at(
        timeseries('Apps authorized and revoked', [logs(TOKENS), logs(REVOKED, { name: 'query2' })], {
          formulas: [
            { alias: 'authorized', formula: 'query1' },
            { alias: 'revoked', formula: 'query2' },
          ],
          display: 'bars',
          palette: 'warm',
        }),
        0,
        0,
        8,
        3,
      ),
      at(pie('Grants by app', [logs(TOKENS, { groupBy: ['@event.parameters.app_name'] })]), 8, 0, 4, 3),
      at(
        table('Apps by grants', [
          ['grants', logs(TOKENS, { groupBy: ['@event.parameters.app_name'], limit: 25 })],
          ['users', logs(TOKENS, { groupBy: ['@event.parameters.app_name'], limit: 25, aggregation: 'cardinality', metric: '@usr.email' })],
        ]),
        0,
        3,
        6,
        4,
      ),
      at(logStream('Apps authorized', TOKENS), 6, 3, 6, 4),
    ],
    46,
    7,
  );
  const people = group(
    'Groups, users and devices',
    'gray',
    [
      at(timeseries('Group changes', [logs(GROUPS, { groupBy: ['@evt.name'] })], { display: 'bars' }), 0, 0, 6, 3),
      at(toplist('Most active users', logs(EVENTS, { groupBy: ['@usr.email'] }), { palette: 'cool' }), 6, 0, 3, 3),
      at(toplist('Most active addresses', logs(EVENTS, { groupBy: ['@network.client.ip'] }), { palette: 'cool' }), 9, 0, 3, 3),
      at(pie('Mobile device events', [logs(DEVICES, { groupBy: ['@evt.name'] })]), 0, 3, 4, 4),
      at(toplist('Top countries', logs(EVENTS, { groupBy: [COUNTRY] }), { palette: 'cool' }), 4, 3, 4, 4),
      at(logStream('Group changes', GROUPS), 8, 3, 4, 4),
    ],
    53,
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
    60,
    5,
  );
  const all = group('All activities', 'white', [at(logStream('Activities', EVENTS), 0, 0, 12, 5)], 65, 6);
  return {
    title: 'Google Workspace Overview',
    description:
      "A Google Workspace account's audit activities: logins and failed or suspicious logins, 2-step verification, Admin console changes, Drive files shared outside the domain, OAuth apps authorized, group and device changes, and the Cloud SIEM signals they raise.",
    layout_type: 'ordered',
    template_variables: [],
    widgets: [intro, overview, logins, admin, drive, tokens, people, siem, all],
  };
}

export const buildDashboard = () => buildOwnDashboard(dashboard(), { integration: 'google-workspace', name: 'Google Workspace', script: 'scripts/google-workspace-dashboard.mjs' });

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const dir = new URL('../server/seeds/integrations/google-workspace/', import.meta.url);
  mkdirSync(dir, { recursive: true });
  writeFileSync(new URL('dashboards.json', dir), JSON.stringify([buildDashboard()], null, 2) + '\n');
  console.log('wrote server/seeds/integrations/google-workspace/dashboards.json');
}
