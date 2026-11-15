// Security → Overview: each product's card from that product's own summary (the numbers its pages show). Until a
// summary is read the card says so, instead of offering to enable a product that may well be on.
import type { CloudSecuritySummary } from '../../api/cloudSecurity/types';
import type { WpSummary } from '../../api/workloadProtection/types';

/** on | off (not set up) | pending (its summary is loading) | error (its summary failed). */
export type CapabilityState = 'on' | 'off' | 'pending' | 'error';

interface Read<T> {
  data?: T;
  isError: boolean;
}

const stateOf = <T>(q: Read<T>, on: (d: T) => boolean): CapabilityState => (q.data ? (on(q.data) ? 'on' : 'off') : q.isError ? 'error' : 'pending');

export function cloudSecurityCard(q: Read<CloudSecuritySummary>) {
  const d = q.data;
  const connectedAccounts = d?.coverage.accounts.filter((a) => a.connected).length ?? 0;
  const sbom = stateOf(q, (s) => s.coverage.hostsScanned + s.coverage.images > 0);
  // Misconfigurations come from Kubernetes objects and hosts, the account scans or the tagging policies.
  const posture = stateOf(q, (s) => s.coverage.misconfigurationChecks > 0 || s.coverage.resourcesEvaluated > 0 || connectedAccounts > 0);
  return {
    sbom,
    posture,
    accounts: stateOf(q, () => connectedAccounts > 0),
    connectedAccounts,
    vulnerabilities: sbom === 'on' ? d!.vulnerabilities.total : undefined,
    highExploitRisk: sbom === 'on' ? d!.vulnerabilities.highExploitRisk : undefined,
    misconfigurations: posture === 'on' ? Object.values(d!.misconfigurations).reduce((a, n) => a + n, 0) : undefined,
  };
}

export function workloadCard(q: Read<Pick<WpSummary, 'agents' | 'detectionRules' | 'signals'>>) {
  const d = q.data;
  const runtime = stateOf(q, (s) => s.agents.hostsEnabled + s.agents.serverlessEnabled > 0);
  return {
    runtime,
    detectionRules: stateOf(q, (s) => runtime === 'on' && s.detectionRules > 0),
    rules: d?.detectionRules,
    hosts: d?.agents.hosts,
    hostsEnabled: d?.agents.hostsEnabled,
    signals: d ? Object.values(d.signals).reduce((a, n) => a + n, 0) : undefined,
  };
}
