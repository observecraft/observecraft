// Cloud Cost: daily cost line items of the cloud and SaaS bills (custom costs files uploaded through
// Datadog's API, or the demo bills), and the budgets kept through Datadog's budget API. Outside live
// mode the mock answers.
//   POST /api/v1/cloud-cost/query     queryCosts    costs per group and interval, with the previous period
//   POST /api/v1/cloud-cost/tags      costTags      the tags costs break down by
//   GET  /api/v1/cloud-cost/budgets   listBudgets   budgets with this month's figures
//   GET  /api/v1/cloud-cost/budgets/{id}   getBudget   a budget with each month's cost and forecast
//   PUT  /api/v1/cloud-cost/budgets   saveBudget    create or replace a budget (Datadog's budget document)
//   DELETE /api/v1/cloud-cost/budgets/{id} deleteBudget
//   POST /api/v1/cloud-cost/unit-costs  unitCosts   usage and its amortized and on-demand cost per tag group
//   POST /api/v1/cloud-cost/commitments commitments savings, coverage, utilization and inventory of commitments
import type { components } from '../generated/schema';
import { API_MODE, del, get, post, put } from '../http';
import * as mock from '../mock/cloudCost';

const live = API_MODE === 'live';

export type CostQueryRequest = components['schemas']['CostQueryRequest'];
export type CostQueryResult = components['schemas']['CostQueryResult'];
export type CostSeries = components['schemas']['CostSeries'];
export type CostTag = components['schemas']['CostTag'];
export type CostTagsRequest = components['schemas']['CostTagsRequest'];
export type CostBudgetSummary = components['schemas']['CostBudgetSummary'];
export type CostInterval = components['schemas']['CostInterval'];
export type CostType = components['schemas']['CostType'];
export type UnitCostsRequest = components['schemas']['UnitCostsRequest'];
export type UnitCostRow = components['schemas']['UnitCostRow'];
export type CommitmentsRequest = components['schemas']['CommitmentsRequest'];
export type CommitmentsResult = components['schemas']['CommitmentsResult'];
export type Commitment = components['schemas']['Commitment'];

/** A budget entry: one month of one tag combination (Datadog's budget API). */
export interface BudgetEntry {
  month: number;
  amount: number;
  tag_filters: { tag_key: string; tag_value: string }[];
  costs?: { amount: number; actual: number | null; forecast: number | null };
}

/** Datadog's budget document (`datadog_cost_budget`). */
export interface BudgetDoc {
  type: 'budget';
  id?: string;
  attributes: {
    name: string;
    metrics_query: string;
    start_month: number;
    end_month: number;
    entries?: BudgetEntry[];
    total_amount?: number;
    costs?: { amount: number; actual: number | null; forecast: number | null };
    created_at?: number;
    updated_at?: number;
  };
}

export async function queryCosts(req: CostQueryRequest, signal?: AbortSignal): Promise<CostQueryResult> {
  if (!live) return mock.queryCosts(req);
  return post('/api/v1/cloud-cost/query', req, signal);
}

export async function costTags(req: CostTagsRequest, signal?: AbortSignal): Promise<CostTag[]> {
  if (!live) return mock.costTags(req);
  return post('/api/v1/cloud-cost/tags', req, signal);
}

export async function listBudgets(signal?: AbortSignal): Promise<CostBudgetSummary[]> {
  if (!live) return mock.listBudgets();
  return get('/api/v1/cloud-cost/budgets', {}, signal);
}

export async function getBudget(id: string, signal?: AbortSignal): Promise<BudgetDoc> {
  if (!live) return mock.getBudget(id);
  return ((await get('/api/v1/cloud-cost/budgets/{budget_id}', { budget_id: id }, signal)) as { data: BudgetDoc }).data;
}

export async function saveBudget(doc: BudgetDoc): Promise<BudgetDoc> {
  if (!live) return mock.saveBudget(doc);
  return ((await put('/api/v1/cloud-cost/budgets', {} as never, { data: doc })) as { data: BudgetDoc }).data;
}

export async function deleteBudget(id: string): Promise<void> {
  if (!live) return mock.deleteBudget(id);
  return del('/api/v1/cloud-cost/budgets/{budget_id}', { budget_id: id });
}

export async function unitCosts(req: UnitCostsRequest, signal?: AbortSignal): Promise<UnitCostRow[]> {
  if (!live) return mock.unitCosts(req);
  return post('/api/v1/cloud-cost/unit-costs', req, signal);
}

export async function commitments(req: CommitmentsRequest, signal?: AbortSignal): Promise<CommitmentsResult> {
  if (!live) return mock.commitments(req);
  return post('/api/v1/cloud-cost/commitments', req, signal);
}
