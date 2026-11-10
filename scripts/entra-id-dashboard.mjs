// The Microsoft Entra ID dashboard. integrations-core's Entra ID integration has a log pipeline and no dashboard, so
// this is ObserveCraft's own, over the tenant's logs as Azure's diagnostic settings export them, `source:azure.activedirectory`
// (read from Microsoft Graph by server/crates/oc-api/src/entra, or forwarded from an Event Hub): `@category`
// (SignInLogs, AuditLogs, UserRiskEvents), `@operationName`, `@properties.*` (Graph's sign-in, audit and risk detection
// fields), and what the integration's pipeline adds (`@evt.outcome`, `@usr.id`, `@network.client.ip`), which turning
// the logs on installs. Written in the Datadog widget format the integration dashboards use and converted like them.
// node scripts/entra-id-dashboard.mjs   (writes server/seeds/integrations/azure-active-directory/dashboards.json)
import { mkdirSync, writeFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { BAD, GOOD, NONE, WARN, bars, buildOwnDashboard, geomap, layout, logStream, logs, note, pie, queryValue, table, timeseries, toplist, treemap } from './own-dashboard-widgets.mjs';

const ALL = 'source:azure.activedirectory';
const SIGN_INS = `${ALL} @category:SignInLogs`;
const OK = `${SIGN_INS} @evt.outcome:success`;
const FAILED = `${SIGN_INS} @evt.outcome:failure`;
const RISKY = `${SIGN_INS} @properties.riskLevelDuringSignIn:(medium OR high)`;
const SINGLE_FACTOR = `${OK} @properties.authenticationRequirement:singleFactorAuthentication`;
const AUDITS = `${ALL} @category:AuditLogs`;
const ROLES = `${AUDITS} @properties.category:RoleManagement`;
const RISK = `${ALL} @category:UserRiskEvents`;
const COUNTRY = '@properties.location.countryOrRegion';
const USER = '@properties.userPrincipalName';

function dashboard() {
  const { at, group } = layout();
  const intro = group(
    '',
    'vivid_blue',
    [
      at(
        note(
          "**Microsoft Entra ID** (formerly Azure Active Directory) is Microsoft's cloud identity service. This dashboard follows a tenant's sign-in logs, audit logs and Identity Protection risk detections: who signs in, to which applications and from where, failed sign-ins and the reasons, users and addresses behind them, sign-ins without multi-factor authentication or rated risky, Conditional Access results, and the changes made to users, groups, applications and roles.\n\nTurn on Entra ID logs for an app registration in [Integrations → Microsoft Azure](/integrations/azure?detail=entra): ObserveCraft reads them from Microsoft Graph (sign-ins need Entra ID P1, risk levels P2), with no Event Hub or forwarder.",
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
    { show_title: false, banner_img: '/images/integrations/azure-active-directory-banner.svg' },
  );
  const overview = group(
    'Overview',
    'blue',
    [
      at(queryValue('Sign-ins', [logs(SIGN_INS)], { formats: NONE, timeseries: true }), 0, 0, 2, 2),
      at(queryValue('Failed sign-ins', [logs(FAILED)], { formats: BAD, timeseries: true }), 2, 0, 2, 2),
      at(queryValue('Failure rate', [logs(FAILED), logs(SIGN_INS, { name: 'query2' })], { formula: '100 * query1 / query2', formats: WARN, unit: '%', precision: 1 }), 4, 0, 2, 2),
      at(queryValue('Users signing in', [logs(OK, { aggregation: 'cardinality', metric: USER })], { formats: GOOD }), 6, 0, 2, 2),
      at(queryValue('Risky sign-ins', [logs(RISKY)], { formats: BAD, timeseries: true }), 8, 0, 2, 2),
      at(queryValue('Risk detections', [logs(RISK)], { formats: BAD }), 10, 0, 2, 2),
      at(queryValue('Single-factor sign-ins', [logs(SINGLE_FACTOR)], { formats: WARN }), 0, 2, 2, 2),
      at(queryValue('Failing IPs', [logs(FAILED, { aggregation: 'cardinality', metric: '@network.client.ip' })], { formats: WARN }), 2, 2, 2, 2),
      at(queryValue('Countries', [logs(SIGN_INS, { aggregation: 'cardinality', metric: COUNTRY })], { formats: NONE }), 4, 2, 2, 2),
      at(queryValue('Applications', [logs(SIGN_INS, { aggregation: 'cardinality', metric: '@properties.appDisplayName' })], { formats: NONE }), 6, 2, 2, 2),
      at(queryValue('Directory changes', [logs(AUDITS)], { formats: NONE, timeseries: true }), 8, 2, 2, 2),
      at(queryValue('Role changes', [logs(ROLES)], { formats: WARN }), 10, 2, 2, 2),
    ],
    5,
    5,
  );
  const signIns = group(
    'Sign-ins',
    'green',
    [
      at(
        timeseries('Sign-ins and failed sign-ins', [logs(OK), logs(FAILED, { name: 'query2' })], {
          formulas: [
            { alias: 'successful', formula: 'query1' },
            { alias: 'failed', formula: 'query2' },
          ],
          display: 'bars',
        }),
        0,
        0,
        8,
        3,
      ),
      at(pie('Client apps', [logs(SIGN_INS, { groupBy: ['@properties.clientAppUsed'] })]), 8, 0, 4, 3),
      at(geomap('Sign-ins around the world', logs(SIGN_INS, { groupBy: [COUNTRY], limit: 250 })), 0, 3, 8, 4),
      at(toplist('Users signing in most', logs(OK, { groupBy: [USER] }), { palette: 'cool' }), 8, 3, 4, 4),
      at(toplist('Applications signed in to', logs(SIGN_INS, { groupBy: ['@properties.appDisplayName'] }), { palette: 'cool' }), 0, 7, 4, 3),
      at(pie('Conditional Access', [logs(SIGN_INS, { groupBy: ['@properties.conditionalAccessStatus'] })]), 4, 7, 4, 3),
      at(pie('Authentication requirement', [logs(OK, { groupBy: ['@properties.authenticationRequirement'] })]), 8, 7, 4, 3),
      at(bars('Sign-ins by operating system', logs(SIGN_INS, { groupBy: ['@properties.deviceDetail.operatingSystem'] })), 0, 10, 6, 3),
      at(treemap('Sign-ins by browser', logs(SIGN_INS, { groupBy: ['@properties.deviceDetail.browser'] })), 6, 10, 6, 3),
    ],
    10,
    14,
  );
  const failures = group(
    'Failed sign-ins',
    'orange',
    [
      at(timeseries('Failed sign-ins by error code', [logs(FAILED, { groupBy: ['@properties.status.errorCode'], limit: 10 })], { display: 'bars', palette: 'warm' }), 0, 0, 8, 3),
      at(toplist('Failure reasons', logs(FAILED, { groupBy: ['@properties.status.failureReason'] }), { palette: 'warm' }), 8, 0, 4, 3),
      at(toplist('Users failing to sign in', logs(FAILED, { groupBy: [USER] }), { palette: 'warm' }), 0, 3, 4, 4),
      at(toplist('Addresses failing to sign in', logs(FAILED, { groupBy: ['@network.client.ip'] }), { palette: 'warm' }), 4, 3, 4, 4),
      at(geomap('Failed sign-ins around the world', logs(FAILED, { groupBy: [COUNTRY], limit: 250 })), 8, 3, 4, 4),
      at(
        table('Addresses with the most failed sign-ins', [
          ['failed sign-ins', logs(FAILED, { groupBy: ['@network.client.ip'], limit: 25 })],
          ['users tried', logs(FAILED, { groupBy: ['@network.client.ip'], limit: 25, aggregation: 'cardinality', metric: USER })],
        ]),
        0,
        7,
        6,
        4,
      ),
      at(logStream('Failed sign-ins', FAILED), 6, 7, 6, 4),
    ],
    24,
    11,
  );
  const risk = group(
    'Risk',
    'red',
    [
      at(timeseries('Risky sign-ins by risk level', [logs(RISKY, { groupBy: ['@properties.riskLevelDuringSignIn'] })], { display: 'bars', palette: 'warm' }), 0, 0, 6, 3),
      at(timeseries('Risk detections by type', [logs(RISK, { groupBy: ['@properties.riskEventType'] })], { display: 'bars', palette: 'warm' }), 6, 0, 6, 3),
      at(toplist('Users with risky sign-ins', logs(RISKY, { groupBy: [USER] }), { palette: 'warm' }), 0, 3, 4, 3),
      at(pie('Risk detections by level', [logs(RISK, { groupBy: ['@properties.riskLevel'] })]), 4, 3, 4, 3),
      at(toplist('Single-factor sign-ins by application', logs(SINGLE_FACTOR, { groupBy: ['@properties.appDisplayName'] }), { palette: 'warm' }), 8, 3, 4, 3),
      at(logStream('Risky sign-ins', RISKY), 0, 6, 6, 4),
      at(logStream('Risk detections', RISK), 6, 6, 6, 4),
    ],
    35,
    10,
  );
  const directory = group(
    'Directory changes',
    'purple',
    [
      at(timeseries('Changes by category', [logs(AUDITS, { groupBy: ['@properties.category'] })], { display: 'bars', palette: 'purple' }), 0, 0, 8, 3),
      at(pie('Changes by result', [logs(AUDITS, { groupBy: ['@properties.result'] })]), 8, 0, 4, 3),
      at(bars('Activities', logs(AUDITS, { groupBy: ['@operationName'] })), 0, 3, 6, 4),
      at(toplist('Administrators changing most', logs(AUDITS, { groupBy: ['@usr.id'] }), { palette: 'purple' }), 6, 3, 3, 4),
      at(toplist('Services logging changes', logs(AUDITS, { groupBy: ['@properties.loggedByService'] }), { palette: 'purple' }), 9, 3, 3, 4),
      at(logStream('Role assignments', ROLES), 0, 7, 6, 4),
      at(logStream('All directory changes', AUDITS), 6, 7, 6, 4),
    ],
    45,
    11,
  );
  return {
    title: 'Microsoft Entra ID Overview',
    description:
      "A Microsoft Entra ID tenant's sign-in logs, audit logs and risk detections: sign-ins and failed sign-ins, where from, MFA and Conditional Access, risky sign-ins, and changes to users, groups, applications and roles.",
    layout_type: 'ordered',
    template_variables: [],
    widgets: [intro, overview, signIns, failures, risk, directory],
  };
}

export const buildDashboard = () => buildOwnDashboard(dashboard(), { integration: 'azure-active-directory', name: 'Microsoft Entra ID', script: 'scripts/entra-id-dashboard.mjs' });

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const dir = new URL('../server/seeds/integrations/azure-active-directory/', import.meta.url);
  mkdirSync(dir, { recursive: true });
  writeFileSync(new URL('dashboards.json', dir), JSON.stringify([buildDashboard()], null, 2) + '\n');
  console.log('wrote server/seeds/integrations/azure-active-directory/dashboards.json');
}
