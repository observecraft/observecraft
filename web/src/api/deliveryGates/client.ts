// Software Delivery → Deployment Gates and PR Gates. In "live" mode they come from the Rust API
// (gates, evaluations and PR rules in Postgres; verdicts from monitors, APM spans and CI Visibility);
// otherwise the in-browser mock answers (mock/deliveryGates.ts).
//   GET/POST       /api/v1/deployment-gates                    listGates, createGate
//   GET/PUT/DELETE /api/v1/deployment-gates/{id}               getGate, updateGate, deleteGate
//   POST           /api/v1/deployment-gates/evaluate           evaluateGate (CI/CD pipelines, application key)
//   GET            /api/v1/deployment-gates/evaluations[/{id}] listEvaluations, getEvaluation
//   GET/POST       /api/v1/pr-gates/rules                      listPrRules, createPrRule
//   PUT/DELETE     /api/v1/pr-gates/rules/{id}                 updatePrRule, deletePrRule
//   POST           /api/v1/pr-gates/evaluate                   evaluatePr (CI, application key)
import type { components } from '../generated/schema';
import { API_MODE, del, get, post, put } from '../http';
import * as mock from '../mock/deliveryGates';

type S = components['schemas'];
export type DeploymentGate = S['DeploymentGate'];
export type DeploymentGateInput = S['DeploymentGateInput'];
export type GateRule = S['GateRule'];
export type GateRuleKind = S['GateRuleKind'];
export type GateRuleResult = S['GateRuleResult'];
export type GateMonitorState = S['GateMonitorState'];
export type GateEvaluation = S['GateEvaluation'];
export type GateEvaluationStatus = S['GateEvaluationStatus'];
export type GateEvaluationRequest = S['GateEvaluationRequest'];
export type PrGateRule = S['PrGateRule'];
export type PrGateRuleInput = S['PrGateRuleInput'];
export type PrGateSource = S['PrGateSource'];
export type PrGateSeverity = S['PrGateSeverity'];
export type PrGateEvaluation = S['PrGateEvaluation'];
export type PrGateEvaluationRequest = S['PrGateEvaluationRequest'];
export type PrGateRuleResult = S['PrGateRuleResult'];
export type PrGateCheckStatus = S['PrGateCheckStatus'];

const live = API_MODE === 'live';

export const listGates = (): Promise<DeploymentGate[]> => (live ? get('/api/v1/deployment-gates', {}) : mock.listGates());
export const createGate = (i: DeploymentGateInput): Promise<DeploymentGate> => (live ? post('/api/v1/deployment-gates', i) : mock.createGate(i));
export const updateGate = (id: string, i: DeploymentGateInput): Promise<DeploymentGate> => (live ? put('/api/v1/deployment-gates/{id}', { id }, i) : mock.updateGate(id, i));
export const deleteGate = (id: string): Promise<void> => (live ? del('/api/v1/deployment-gates/{id}', { id }) : mock.deleteGate(id));
export const evaluateGate = (r: GateEvaluationRequest): Promise<GateEvaluation> => (live ? post('/api/v1/deployment-gates/evaluate', r) : mock.evaluateGate(r));
export const listEvaluations = (): Promise<GateEvaluation[]> => (live ? get('/api/v1/deployment-gates/evaluations', {}) : mock.listEvaluations());
export const getEvaluation = (id: string): Promise<GateEvaluation> => (live ? get('/api/v1/deployment-gates/evaluations/{id}', { id }) : mock.getEvaluation(id));

export const listPrRules = (): Promise<PrGateRule[]> => (live ? get('/api/v1/pr-gates/rules', {}) : mock.listPrRules());
export const createPrRule = (i: PrGateRuleInput): Promise<PrGateRule> => (live ? post('/api/v1/pr-gates/rules', i) : mock.createPrRule(i));
export const updatePrRule = (id: string, i: PrGateRuleInput): Promise<PrGateRule> => (live ? put('/api/v1/pr-gates/rules/{id}', { id }, i) : mock.updatePrRule(id, i));
export const deletePrRule = (id: string): Promise<void> => (live ? del('/api/v1/pr-gates/rules/{id}', { id }) : mock.deletePrRule(id));
export const evaluatePr = (r: PrGateEvaluationRequest): Promise<PrGateEvaluation> => (live ? post('/api/v1/pr-gates/evaluate', r) : mock.evaluatePr(r));
