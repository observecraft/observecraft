// Cloud Security (Security → Cloud Security): misconfigurations, infrastructure vulnerabilities and
// the inventory. In "live" mode from the Rust API, otherwise the in-browser mock (mock/cloudSecurity.ts).
//   GET  /api/v1/cloud-security/summary             getCloudSecuritySummary
//   POST /api/v1/cloud-security/misconfigurations   listMisconfigurations
//   POST /api/v1/cloud-security/vulnerabilities     listInfraVulnerabilities
//   GET  /api/v1/cloud-security/resources           listResourceCategories
//   GET  /api/v1/cloud-security/packages            listInfraPackages
//   GET  /api/v1/cloud-security/cves                searchCves
//   GET  /api/v1/code-security/advisories/:id       getCve (the advisory, shared with Code Security)
//   POST /api/v1/code-security/triage               triageFinding (mute / reopen)
import { API_MODE, get, post } from '../http';
import * as mock from '../mock/cloudSecurity';
import type { AdvisoryDetail, CloudSecuritySummary, CvePage, CveQuery, FindingStatus, InfraPackage, InfraVulnerability, MisconfigPage, ResourceCategory } from './types';

export * from './types';

const live = API_MODE === 'live';

export const getCloudSecuritySummary = (): Promise<CloudSecuritySummary> => (live ? get('/api/v1/cloud-security/summary', {}) : mock.summary());
export const listMisconfigurations = (status: FindingStatus = 'open'): Promise<MisconfigPage> => (live ? post('/api/v1/cloud-security/misconfigurations', { status }) : mock.misconfigurations(status));
export const listInfraVulnerabilities = (status: FindingStatus = 'open'): Promise<InfraVulnerability[]> =>
  live ? post('/api/v1/cloud-security/vulnerabilities', { status }) : mock.vulnerabilities(status);
export const listResourceCategories = (): Promise<ResourceCategory[]> => (live ? get('/api/v1/cloud-security/resources', {}) : mock.resources());
export const listInfraPackages = (): Promise<InfraPackage[]> => (live ? get('/api/v1/cloud-security/packages', {}) : mock.packages());
export const searchCves = (q: CveQuery): Promise<CvePage> =>
  live
    ? get(
        '/api/v1/cloud-security/cves',
        {},
        undefined,
        Object.fromEntries(
          Object.entries(q)
            .filter(([, v]) => v !== undefined && v !== '')
            .map(([k, v]) => [k, typeof v === 'boolean' ? String(v) : v]),
        ) as Record<string, string | number>,
      )
    : mock.cves(q);
export const getCve = (id: string): Promise<AdvisoryDetail> => (live ? get('/api/v1/code-security/advisories/{id}', { id }) : mock.advisory(id));
export const triageFinding = (key: string, status: 'open' | 'muted'): Promise<void> =>
  live ? post('/api/v1/code-security/triage', { key, status, reason: '' }).then(() => undefined) : mock.setTriage(key, status);
