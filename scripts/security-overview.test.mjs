import assert from 'node:assert/strict';
import test from 'node:test';
import { cloudSecurityCard, workloadCard } from '../web/src/pages/security/overviewCards.ts';

const summary = (coverage, extra = {}) => ({
  vulnerabilities: { total: 5200, running: 4000, exploitAvailable: 12, fixAvailable: 3000, highOrCritical: 900, highExploitRisk: 7 },
  coverage: { hosts: 360, hostsScanned: 25, images: 685, imagesRunning: 500, clusters: 3, resourcesEvaluated: 0, misconfigurationChecks: 0, accounts: [], ...coverage },
  misconfigurations: { critical: 1, high: 20, medium: 300, low: 1000 },
  vulnerabilitiesBySeverity: {},
  inventory: [],
  ...extra,
});

test('Cloud Security: pending while the summary loads, never "enable" or a dash for data not read yet', () => {
  const c = cloudSecurityCard({ isError: false });
  assert.deepEqual([c.sbom, c.posture, c.accounts], ['pending', 'pending', 'pending']);
  assert.equal(c.vulnerabilities, undefined);
  assert.deepEqual([cloudSecurityCard({ isError: true }).sbom, cloudSecurityCard({ isError: true }).accounts], ['error', 'error']);
});

test('Cloud Security: SBOMs, account scans and connected accounts turn their capabilities on with their numbers', () => {
  const accounts = [
    { provider: 'aws', id: '123456789012', name: 'shop-prod', connected: true, resources: 0 },
    { provider: 'azure', id: '0b1c2d3e-0000-4000-8000-000000000001', name: 'Storefront Production', connected: true, resources: 0 },
    { provider: 'aws', id: '210987654321', name: '', connected: false, resources: 2 },
  ];
  // Misconfigurations come from the account scans only (no Kubernetes object evaluated).
  const c = cloudSecurityCard({ isError: false, data: summary({ misconfigurationChecks: 1300, accounts }) });
  assert.deepEqual([c.sbom, c.posture, c.accounts, c.connectedAccounts], ['on', 'on', 'on', 2]);
  assert.deepEqual([c.vulnerabilities, c.highExploitRisk, c.misconfigurations], [5200, 7, 1321]);
  const none = cloudSecurityCard({ isError: false, data: summary({ hostsScanned: 0, images: 0 }) });
  assert.deepEqual([none.sbom, none.posture, none.accounts, none.vulnerabilities], ['off', 'off', 'off', undefined]);
});

test('Workload Protection: the detection rules count only once the summary is read', () => {
  assert.deepEqual(workloadCard({ isError: false }), { runtime: 'pending', detectionRules: 'pending', rules: undefined, hosts: undefined, hostsEnabled: undefined, signals: undefined });
  const w = workloadCard({ isError: false, data: { agents: { hosts: 360, hostsEnabled: 300, serverless: 0, serverlessEnabled: 0 }, detectionRules: 27, signals: { high: 2, low: 1 } } });
  assert.deepEqual(w, { runtime: 'on', detectionRules: 'on', rules: 27, hosts: 360, hostsEnabled: 300, signals: 3 });
});
