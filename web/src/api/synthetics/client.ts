// Synthetic API tests (Digital Experience → Synthetic Tests): HTTP, SSL, DNS and TCP checks the server
// runs on a schedule. In "live" mode they come from the Rust API; otherwise the in-browser mock answers
// (mock/synthetics.ts).
//   GET    /api/v1/synthetics/locations                    listSyntheticLocations
//   POST   /api/v1/synthetics/tests/search                 searchSyntheticTests
//   POST   /api/v1/synthetics/tests                        createSyntheticTest
//   GET    /api/v1/synthetics/tests/:id                    getSyntheticTest
//   PUT    /api/v1/synthetics/tests/:id                    updateSyntheticTest
//   DELETE /api/v1/synthetics/tests/:id                    deleteSyntheticTest
//   POST   /api/v1/synthetics/tests/:id/run                runSyntheticTest
//   POST   /api/v1/synthetics/tests/:id/results            listSyntheticResults
//   GET    /api/v1/synthetics/tests/:id/results/:result    getSyntheticResult
//   POST   /api/v1/synthetics/try                          trySyntheticRequest
//   POST   /api/v1/synthetics/suites/search                searchSyntheticSuites
//   POST   /api/v1/synthetics/suites                       createSyntheticSuite
//   GET    /api/v1/synthetics/suites/:id                   getSyntheticSuite
//   PUT    /api/v1/synthetics/suites/:id                   updateSyntheticSuite
//   DELETE /api/v1/synthetics/suites/:id                   deleteSyntheticSuite
//   POST   /api/v1/synthetics/results/search               searchSyntheticResultsAll
//   POST   /api/v1/synthetics/coverage                     getSyntheticCoverage
//   GET    /api/v1/synthetics/settings                     getSyntheticSettings
//   PUT    /api/v1/synthetics/settings                     updateSyntheticSettings
//   GET    /api/v1/synthetics/variables                    listSyntheticVariables
//   POST   /api/v1/synthetics/variables                    createSyntheticVariable
//   PUT    /api/v1/synthetics/variables/:id                updateSyntheticVariable
//   DELETE /api/v1/synthetics/variables/:id                deleteSyntheticVariable
import { API_MODE, del, get, post, postTo, put } from '../http';
import type { TimeRange } from '../types';
import * as mock from '../mock/synthetics';
import type {
  SyntheticCoverage,
  SyntheticCoverageRequest,
  SyntheticLocation,
  SyntheticResult,
  SyntheticResults,
  SyntheticResultSearchPage,
  SyntheticResultSearchRequest,
  SyntheticSettings,
  SyntheticSuite,
  SyntheticSuiteInput,
  SyntheticSuitesPage,
  SyntheticSuiteSearchRequest,
  SyntheticTest,
  SyntheticTestInput,
  SyntheticTestsPage,
  SyntheticTestSearchRequest,
  SyntheticTryRequest,
  SyntheticTryResult,
  SyntheticVariable,
  SyntheticVariableInput,
} from './types';

export * from './types';

const live = API_MODE === 'live';

export const listSyntheticLocations = (): Promise<SyntheticLocation[]> => (live ? get('/api/v1/synthetics/locations', {}) : mock.listSyntheticLocations());
export const searchSyntheticTests = (req: SyntheticTestSearchRequest): Promise<SyntheticTestsPage> => (live ? post('/api/v1/synthetics/tests/search', req) : mock.searchSyntheticTests(req));
export const getSyntheticTest = (id: string, signal?: AbortSignal): Promise<SyntheticTest> => (live ? get('/api/v1/synthetics/tests/{id}', { id }, signal) : mock.getSyntheticTest(id));
export const createSyntheticTest = (input: SyntheticTestInput): Promise<SyntheticTest> => (live ? post('/api/v1/synthetics/tests', input) : mock.createSyntheticTest(input));
export const updateSyntheticTest = (id: string, input: SyntheticTestInput): Promise<SyntheticTest> =>
  live ? put('/api/v1/synthetics/tests/{id}', { id }, input) : mock.updateSyntheticTest(id, input);
export const deleteSyntheticTest = (id: string): Promise<void> => (live ? del('/api/v1/synthetics/tests/{id}', { id }) : mock.deleteSyntheticTest(id));
/** Runs a saved test once now; the result is recorded but doesn't change the alert state. */
export const runSyntheticTest = (id: string): Promise<SyntheticResult> => (live ? postTo('/api/v1/synthetics/tests/{id}/run', { id }, undefined) : mock.runSyntheticTest(id));
export const listSyntheticResults = (id: string, range: TimeRange, signal?: AbortSignal): Promise<SyntheticResults> =>
  live ? postTo('/api/v1/synthetics/tests/{id}/results', { id }, { range }, signal) : mock.listSyntheticResults(id, range);
export const getSyntheticResult = (id: string, resultId: number, signal?: AbortSignal): Promise<SyntheticResult> =>
  live ? get('/api/v1/synthetics/tests/{id}/results/{result_id}', { id, result_id: String(resultId) }, signal) : mock.getSyntheticResult(id, resultId);
/** Sends a request once from the editor (nothing is stored) and suggests assertions for the response. */
export const trySyntheticRequest = (req: SyntheticTryRequest): Promise<SyntheticTryResult> => (live ? post('/api/v1/synthetics/try', req) : mock.trySyntheticRequest(req));

// Test suites.
export const searchSyntheticSuites = (req: SyntheticSuiteSearchRequest): Promise<SyntheticSuitesPage> => (live ? post('/api/v1/synthetics/suites/search', req) : mock.searchSyntheticSuites(req));
export const getSyntheticSuite = (id: string, signal?: AbortSignal): Promise<SyntheticSuite> => (live ? get('/api/v1/synthetics/suites/{id}', { id }, signal) : mock.getSyntheticSuite(id));
export const createSyntheticSuite = (input: SyntheticSuiteInput): Promise<SyntheticSuite> => (live ? post('/api/v1/synthetics/suites', input) : mock.createSyntheticSuite(input));
export const updateSyntheticSuite = (id: string, input: SyntheticSuiteInput): Promise<SyntheticSuite> =>
  live ? put('/api/v1/synthetics/suites/{id}', { id }, input) : mock.updateSyntheticSuite(id, input);
export const deleteSyntheticSuite = (id: string): Promise<void> => (live ? del('/api/v1/synthetics/suites/{id}', { id }) : mock.deleteSyntheticSuite(id));

// Results Explorer and Test Coverage.
export const searchSyntheticResultsAll = (req: SyntheticResultSearchRequest, signal?: AbortSignal): Promise<SyntheticResultSearchPage> =>
  live ? post('/api/v1/synthetics/results/search', req, signal) : mock.searchSyntheticResultsAll(req);
export const getSyntheticCoverage = (req: SyntheticCoverageRequest, signal?: AbortSignal): Promise<SyntheticCoverage> =>
  live ? post('/api/v1/synthetics/coverage', req, signal) : mock.getSyntheticCoverage(req);

// Settings and global variables.
export const getSyntheticSettings = (): Promise<SyntheticSettings> => (live ? get('/api/v1/synthetics/settings', {}) : mock.getSyntheticSettings());
export const updateSyntheticSettings = (s: SyntheticSettings): Promise<SyntheticSettings> => (live ? put('/api/v1/synthetics/settings', {}, s) : mock.updateSyntheticSettings(s));
export const listSyntheticVariables = (): Promise<SyntheticVariable[]> => (live ? get('/api/v1/synthetics/variables', {}) : mock.listSyntheticVariables());
export const createSyntheticVariable = (input: SyntheticVariableInput): Promise<SyntheticVariable> => (live ? post('/api/v1/synthetics/variables', input) : mock.createSyntheticVariable(input));
export const updateSyntheticVariable = (id: string, input: SyntheticVariableInput): Promise<SyntheticVariable> =>
  live ? put('/api/v1/synthetics/variables/{id}', { id }, input) : mock.updateSyntheticVariable(id, input);
export const deleteSyntheticVariable = (id: string): Promise<void> => (live ? del('/api/v1/synthetics/variables/{id}', { id }) : mock.deleteSyntheticVariable(id));
