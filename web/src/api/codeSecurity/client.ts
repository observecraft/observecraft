// Code Security: vulnerable libraries (from tracer telemetry and `datadog-ci sbom upload`), static
// findings (`datadog-ci sarif upload`), runtime code vulnerabilities (IAST spans) and the advisories
// libraries are matched against. Outside live mode the mock answers.
//   GET  /api/v1/code-security/summary                       summary
//   POST /api/v1/code-security/library-vulnerabilities       libraryVulnerabilities
//   POST /api/v1/code-security/findings                      findings            (sast | secret | iac)
//   POST /api/v1/code-security/runtime-code-vulnerabilities  runtimeCodeVulnerabilities
//   POST /api/v1/code-security/advisories                    advisories          (CVE explorer)
//   GET  /api/v1/code-security/advisories/{id}               advisory
//   GET  /api/v1/code-security/repositories                  repositories
//   POST /api/v1/code-security/inventory/libraries           inventoryLibraries
//   GET  /api/v1/code-security/inventory/services            inventoryServices
//   POST /api/v1/code-security/rules                         rules
//   GET  /api/v1/code-security/pull-requests                 pullRequests
//   POST /api/v1/code-security/triage                        triage
//   GET  /api/v1/code-security/setup                         setup
import type { components } from '../generated/schema';
import { API_MODE, get, post } from '../http';
import * as mock from '../mock/codeSecurity';

const live = API_MODE === 'live';
type S = components['schemas'];

export type CsFilter = Partial<S['CsFilter']>;
export type CodeSecuritySummary = S['CodeSecuritySummary'];
export type SeverityCounts = S['SeverityCounts'];
export type LibraryVulnerability = S['LibraryVulnerability'];
export type LibraryVulnerabilityPage = S['LibraryVulnerabilityPage'];
export type StaticFinding = S['StaticFinding'];
export type StaticFindingPage = S['StaticFindingPage'];
export type RuntimeCodeVulnerability = S['RuntimeCodeVulnerability'];
export type RuntimeCodeVulnerabilityPage = S['RuntimeCodeVulnerabilityPage'];
export type AdvisorySummary = S['AdvisorySummary'];
export type AdvisoryPage = S['AdvisoryPage'];
export type AdvisoryDetail = S['AdvisoryDetail'];
export type RepositorySummary = S['RepositorySummary'];
export type InventoryLibrary = S['InventoryLibrary'];
export type InventoryPage = S['InventoryPage'];
export type InventoryService = S['InventoryService'];
export type RuleSummary = S['RuleSummary'];
export type PullRequestScan = S['PullRequestScan'];
export type CodeSecuritySetup = S['CodeSecuritySetup'];
export type FindingKind = 'sast' | 'secret' | 'iac';
export type Severity = 'critical' | 'high' | 'medium' | 'low' | 'unknown';

const filter = (f: CsFilter = {}): S['CsFilter'] => ({ query: '', severity: [], repository: '', service: '', status: '', scope: '', language: '', offset: 0, limit: 25, ...f });

export async function summary(signal?: AbortSignal): Promise<CodeSecuritySummary> {
  if (!live) return mock.summary();
  return get('/api/v1/code-security/summary', {}, signal);
}

export async function libraryVulnerabilities(f: CsFilter, signal?: AbortSignal): Promise<LibraryVulnerabilityPage> {
  if (!live) return mock.libraryVulnerabilities(filter(f));
  return post('/api/v1/code-security/library-vulnerabilities', filter(f), signal);
}

export async function findings(kind: FindingKind, f: CsFilter, signal?: AbortSignal): Promise<StaticFindingPage> {
  if (!live) return mock.findings(kind, filter(f));
  return post('/api/v1/code-security/findings', { ...filter(f), kind }, signal);
}

export async function runtimeCodeVulnerabilities(f: CsFilter, signal?: AbortSignal): Promise<RuntimeCodeVulnerabilityPage> {
  if (!live) return mock.runtimeCodeVulnerabilities(filter(f));
  return post('/api/v1/code-security/runtime-code-vulnerabilities', { ...filter(f), range: null }, signal);
}

export async function advisories(f: { query?: string; severity?: string[]; impact?: string; ecosystem?: string; offset?: number; limit?: number }, signal?: AbortSignal): Promise<AdvisoryPage> {
  const req = { query: '', severity: [], impact: '', ecosystem: '', offset: 0, limit: 25, ...f };
  if (!live) return mock.advisories(req);
  return post('/api/v1/code-security/advisories', req, signal);
}

export async function advisory(id: string, signal?: AbortSignal): Promise<AdvisoryDetail> {
  if (!live) return mock.advisory(id);
  return get('/api/v1/code-security/advisories/{id}', { id }, signal);
}

export async function repositories(signal?: AbortSignal): Promise<RepositorySummary[]> {
  if (!live) return mock.repositories();
  return get('/api/v1/code-security/repositories', {}, signal);
}

export async function inventoryLibraries(view: 'static' | 'runtime', f: CsFilter, signal?: AbortSignal): Promise<InventoryPage> {
  if (!live) return mock.inventoryLibraries(view, filter(f));
  return post('/api/v1/code-security/inventory/libraries', { ...filter(f), view }, signal);
}

export async function inventoryServices(signal?: AbortSignal): Promise<InventoryService[]> {
  if (!live) return mock.inventoryServices();
  return get('/api/v1/code-security/inventory/services', {}, signal);
}

export async function rules(kind: FindingKind, f: CsFilter, signal?: AbortSignal): Promise<RuleSummary[]> {
  if (!live) return mock.rules(kind, filter(f));
  return post('/api/v1/code-security/rules', { ...filter(f), kind }, signal);
}

export async function pullRequests(signal?: AbortSignal): Promise<PullRequestScan[]> {
  if (!live) return mock.pullRequests();
  return get('/api/v1/code-security/pull-requests', {}, signal);
}

export async function triage(key: string, status: 'open' | 'muted', reason = ''): Promise<void> {
  if (!live) return mock.triage(key, status);
  await post('/api/v1/code-security/triage', { key, status, reason });
}

export async function setup(signal?: AbortSignal): Promise<CodeSecuritySetup> {
  if (!live) return mock.setup();
  return get('/api/v1/code-security/setup', {}, signal);
}

/** The triage key of a library vulnerability (the advisory on the package, any version). */
export const vulnKey = (v: Pick<LibraryVulnerability, 'advisoryId' | 'ecosystem' | 'package'>) => `vuln:${v.advisoryId}:${v.ecosystem}:${v.package}`;
