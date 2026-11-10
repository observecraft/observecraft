// The authentik dashboard. Datadog has no authentik integration, so this is ObserveCraft's own, over what the
// authentik integration collects (server/crates/oc-api/src/authentik): the instance's events, logs with
// `source:authentik` (`@evt.name` the action, `@evt.outcome`, `@usr.*` the user, `@network.client.*` the client's IP,
// geoip and ASN, `@authentik.context.*` the event's context, `@authentik.application` the application authorized and
// `@authentik.category` from the authentik log pipeline, which saving the connection installs). Written in the Datadog
// widget format the integration dashboards use and converted like them (import-integration-dashboards.mjs).
// node scripts/authentik-dashboard.mjs   (writes server/seeds/integrations/authentik/dashboards.json)
import { mkdirSync, writeFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { BAD, GOOD, NONE, WARN, anyOf, bars, buildOwnDashboard, geomap, layout, logStream, logs, note, pie, queryValue, table, timeseries, toplist, treemap } from './own-dashboard-widgets.mjs';

const EVENTS = 'source:authentik';
const LOGINS = `${EVENTS} @evt.name:login`;
const FAILED = `${EVENTS} @evt.name:login_failed`;
const MODEL = ['model_created', 'model_updated', 'model_deleted'];
/** authentik's authenticator (MFA device) models. */
const DEVICES = ['totpdevice', 'webauthndevice', 'staticdevice', 'duodevice', 'smsdevice', 'emaildevice', 'endpointdevice'];
const MFA = `${EVENTS} ${anyOf(MODEL)} ${anyOf(DEVICES, '@authentik.context.model.model_name')}`;
const CHANGES = `${EVENTS} ${anyOf(MODEL)}`;
const PASSWORDS = `${EVENTS} @evt.name:password_set`;
const IMPERSONATION = `${EVENTS} @evt.name:(impersonation_started OR impersonation_ended)`;
const SUSPICIOUS = `${EVENTS} @evt.name:suspicious_request`;
const AUTHORIZED = `${EVENTS} @evt.name:authorize_application`;
const SYSTEM = ['system_exception', 'system_task_exception', 'configuration_error', 'configuration_warning', 'policy_exception', 'property_mapping_exception'];
const COUNTRY = '@network.client.geoip.country.iso_code';

function dashboard() {
  const { at, group } = layout();
  const intro = group(
    '',
    'vivid_blue',
    [
      at(
        note(
          "**authentik** is an open-source identity provider: single sign-on (OAuth2/OIDC, SAML, LDAP, proxy), MFA and application access. This dashboard follows the connected instance's event log: who logs in and from where, failed logins and the users, addresses and countries behind them, MFA authenticators enrolled or removed, passwords changed, impersonation, suspicious requests, the applications users are authorized to, administrators' changes to users, groups, applications, providers, flows and policies, and the instance's own errors.\n\nConnect an instance in [Integrations → authentik](/integrations/authentik): ObserveCraft reads its events API every minute, with no access to ObserveCraft needed.",
          'white',
        ),
        0,
        0,
        12,
        2,
      ),
    ],
    0,
    5,
    { show_title: false, banner_img: '/images/integrations/authentik-banner.svg' },
  );
  const overview = group(
    'Overview',
    'blue',
    [
      at(queryValue('Logins', [logs(LOGINS)], { formats: NONE, timeseries: true }), 0, 0, 2, 2),
      at(queryValue('Failed logins', [logs(FAILED)], { formats: BAD, timeseries: true }), 2, 0, 2, 2),
      at(queryValue('Failure rate', [logs(FAILED), logs(LOGINS, { name: 'query2' })], { formula: '100 * query1 / (query1 + query2)', formats: WARN, unit: '%', precision: 1 }), 4, 0, 2, 2),
      at(queryValue('Users', [logs(LOGINS, { aggregation: 'cardinality', metric: '@usr.name' })], { formats: GOOD }), 6, 0, 2, 2),
      at(queryValue('Suspicious', [logs(SUSPICIOUS)], { formats: BAD, timeseries: true }), 8, 0, 2, 2),
      at(queryValue('Errors', [logs(`${EVENTS} status:error -@evt.name:login_failed`)], { formats: BAD, timeseries: true }), 10, 0, 2, 2),
      at(queryValue('Failing IPs', [logs(FAILED, { aggregation: 'cardinality', metric: '@network.client.ip' })], { formats: WARN }), 0, 2, 2, 2),
      at(queryValue('App sign-ins', [logs(AUTHORIZED)], { formats: NONE, timeseries: true }), 2, 2, 2, 2),
      at(queryValue('MFA changes', [logs(MFA)], { formats: NONE }), 4, 2, 2, 2),
      at(queryValue('Passwords set', [logs(PASSWORDS)], { formats: NONE }), 6, 2, 2, 2),
      at(queryValue('Admin changes', [logs(CHANGES)], { formats: NONE, timeseries: true }), 8, 2, 2, 2),
      at(queryValue('Impersonations', [logs(`${EVENTS} @evt.name:impersonation_started`)], { formats: WARN }), 10, 2, 2, 2),
    ],
    5,
    5,
  );
  const logins = group(
    'Logins',
    'green',
    [
      at(
        timeseries('Logins and failed logins', [logs(LOGINS), logs(FAILED, { name: 'query2' })], {
          formulas: [
            { alias: 'logins', formula: 'query1' },
            { alias: 'failed logins', formula: 'query2' },
          ],
          display: 'bars',
        }),
        0,
        0,
        8,
        3,
      ),
      at(pie('Login methods', [logs(LOGINS, { groupBy: ['@authentik.context.auth_method'] })]), 8, 0, 4, 3),
      at(geomap('Logins around the world', logs(LOGINS, { groupBy: [COUNTRY], limit: 250 })), 0, 3, 8, 4),
      at(toplist('Users logging in most', logs(LOGINS, { groupBy: ['@usr.name'] }), { palette: 'cool' }), 8, 3, 4, 4),
      at(treemap('Logins by network (ASN)', logs(LOGINS, { groupBy: ['@network.client.asn.organization'] })), 0, 7, 4, 3),
      at(bars('Logins by brand', logs(LOGINS, { groupBy: ['@authentik.brand'] })), 4, 7, 4, 3),
      at(toplist('Logins by city', logs(LOGINS, { groupBy: ['@network.client.geoip.city.name'] }), { palette: 'cool' }), 8, 7, 4, 3),
    ],
    10,
    10,
  );
  const failures = group(
    'Failed logins',
    'orange',
    [
      at(timeseries('Failed logins by client address', [logs(FAILED, { groupBy: ['@network.client.ip'], limit: 10 })], { display: 'bars', palette: 'warm' }), 0, 0, 8, 3),
      at(bars('Failed logins by stage', logs(FAILED, { groupBy: ['@authentik.context.stage.name'] })), 8, 0, 4, 3),
      at(toplist('Users failing to log in', logs(FAILED, { groupBy: ['@authentik.context.username'] }), { palette: 'warm' }), 0, 3, 4, 4),
      at(toplist('Addresses failing to log in', logs(FAILED, { groupBy: ['@network.client.ip'] }), { palette: 'warm' }), 4, 3, 4, 4),
      at(geomap('Failed logins around the world', logs(FAILED, { groupBy: [COUNTRY], limit: 250 })), 8, 3, 4, 4),
      at(
        table('Addresses with the most failed logins', [
          ['failed logins', logs(FAILED, { groupBy: ['@network.client.ip'], limit: 25 })],
          ['users tried', logs(FAILED, { groupBy: ['@network.client.ip'], limit: 25, aggregation: 'cardinality', metric: '@authentik.context.username' })],
        ]),
        0,
        7,
        6,
        4,
      ),
      at(logStream('Failed logins', FAILED), 6, 7, 6, 4),
    ],
    20,
    11,
  );
  const security = group(
    'Security',
    'red',
    [
      at(
        timeseries('Suspicious requests and impersonation', [logs(SUSPICIOUS), logs(`${EVENTS} @evt.name:impersonation_started`, { name: 'query2' })], {
          formulas: [
            { alias: 'suspicious requests', formula: 'query1' },
            { alias: 'impersonations', formula: 'query2' },
          ],
          display: 'bars',
          palette: 'warm',
        }),
        0,
        0,
        6,
        3,
      ),
      at(timeseries('MFA authenticators enrolled and removed', [logs(MFA, { groupBy: ['@evt.name'] })], { display: 'bars', palette: 'purple' }), 6, 0, 6, 3),
      at(pie('MFA authenticator types', [logs(MFA, { groupBy: ['@authentik.context.model.model_name'] })]), 0, 3, 4, 3),
      at(toplist('Passwords changed by user', logs(PASSWORDS, { groupBy: ['@usr.name'] }), { palette: 'purple' }), 4, 3, 4, 3),
      at(toplist('Addresses of suspicious requests', logs(SUSPICIOUS, { groupBy: ['@network.client.ip'] }), { palette: 'warm' }), 8, 3, 4, 3),
      at(logStream('Impersonation', IMPERSONATION), 0, 6, 6, 4),
      at(logStream('Suspicious requests, MFA and password changes', `${EVENTS} @authentik.category:(suspicious OR mfa OR password)`), 6, 6, 6, 4),
    ],
    31,
    10,
  );
  const applications = group(
    'Applications',
    'purple',
    [
      at(timeseries('Applications authorized', [logs(AUTHORIZED, { groupBy: ['@authentik.application'] })], { display: 'bars', palette: 'purple' }), 0, 0, 8, 3),
      at(pie('Authorizations by application', [logs(AUTHORIZED, { groupBy: ['@authentik.application'] })]), 8, 0, 4, 3),
      at(toplist('Users of the most applications', logs(AUTHORIZED, { groupBy: ['@usr.name'], aggregation: 'cardinality', metric: '@authentik.application' }), { palette: 'purple' }), 0, 3, 4, 4),
      at(bars('Access requests', logs(`${EVENTS} @evt.name:access_request_*`, { groupBy: ['@evt.name'] })), 4, 3, 4, 4),
      at(logStream('Applications authorized', AUTHORIZED), 8, 3, 4, 4),
    ],
    41,
    7,
  );
  const admin = group(
    'Administration',
    'gray',
    [
      at(timeseries('Objects created, updated and deleted', [logs(CHANGES, { groupBy: ['@evt.name'] })], { display: 'bars' }), 0, 0, 6, 3),
      at(bars('Changes by object type', logs(CHANGES, { groupBy: ['@authentik.context.model.model_name'] })), 6, 0, 6, 3),
      at(toplist('Administrators changing most', logs(CHANGES, { groupBy: ['@usr.name'] })), 0, 3, 4, 4),
      at(logStream('Users, groups, applications, providers, flows and policies changed', CHANGES), 4, 3, 8, 4),
    ],
    48,
    7,
  );
  const health = group(
    'Instance health',
    'yellow',
    [
      at(timeseries('Errors and warnings', [logs(`${EVENTS} ${anyOf(SYSTEM)}`, { groupBy: ['@evt.name'] })], { display: 'bars', palette: 'warm' }), 0, 0, 6, 3),
      at(pie('Events by category', [logs(EVENTS, { groupBy: ['@authentik.category'] })]), 6, 0, 3, 3),
      at(pie('Events by outcome', [logs(EVENTS, { groupBy: ['@evt.outcome'] })]), 9, 0, 3, 3),
      at(logStream('Exceptions and configuration errors', `${EVENTS} ${anyOf(SYSTEM)}`), 0, 3, 6, 4),
      at(logStream('All events', EVENTS), 6, 3, 6, 4),
    ],
    55,
    7,
  );
  return {
    title: 'authentik Overview',
    description: "An authentik instance's event log: logins and failed logins, MFA, passwords, impersonation, suspicious requests, applications authorized, administrative changes and errors.",
    layout_type: 'ordered',
    template_variables: [],
    widgets: [intro, overview, logins, failures, security, applications, admin, health],
  };
}

export const buildDashboard = () => buildOwnDashboard(dashboard(), { integration: 'authentik', name: 'authentik', script: 'scripts/authentik-dashboard.mjs' });

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const dir = new URL('../server/seeds/integrations/authentik/', import.meta.url);
  mkdirSync(dir, { recursive: true });
  writeFileSync(new URL('dashboards.json', dir), JSON.stringify([buildDashboard()], null, 2) + '\n');
  console.log('wrote server/seeds/integrations/authentik/dashboards.json');
}
