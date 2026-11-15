import type { InfraVulnerability, InfraVulnerabilityList } from './types';

/** The list's rows joined to their advisory and resource (each sent once), with the triage key the server uses. */
export const expandInfraVulnerabilities = (list: InfraVulnerabilityList): InfraVulnerability[] =>
  list.rows.map(({ advisory, resource, ...row }) => {
    const a = list.advisories[advisory];
    const r = list.resources[resource];
    return { ...a, ...r, ...row, key: `csm-vuln:${a.advisoryId}:${r.resourceId}:${row.package}` };
  });
