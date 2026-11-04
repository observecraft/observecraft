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
//   GET  /api/v1/cloud-cost/bill-configs  listBillConfigs   the providers' bills read (AWS CUR, Azure cost exports)
//   POST /api/v1/cloud-cost/bill-configs/{provider}  createBillConfig   Datadog's create attributes
//   PATCH /api/v1/cloud-cost/bill-configs/{provider}/{id}  setBillConfigEnabled
//   DELETE /api/v1/cloud-cost/bill-configs/{provider}/{id}  deleteBillConfig   archives it; the costs stay
//   GET  /api/v1/cloud-cost/recommendations  costRecommendations  waste found in the AWS and Azure accounts' metrics, with totals
import type { components } from '../generated/schema';
import { API_MODE, del, get, patch, post, postTo, put } from '../http';
import * as mock from '../mock/cloudCost';
import * as recommendationsMock from '../mock/cloudCostRecommendations';

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
export type CostRecommendation = components['schemas']['CostRecommendation'];
export type RecommendationsResult = components['schemas']['RecommendationsResult'];
export type RecommendationGroup = components['schemas']['RecommendationGroup'];

/** Filters of the recommendations; `tags` are `key:value` (`*` wildcards), all required. */
export interface RecommendationFilters {
  provider?: string;
  resourceType?: string;
  recommendationType?: string;
  tags?: string[];
}

export type BillProvider = 'aws' | 'azure';
export type BillStatus = 'active' | 'disabled';

/** Datadog's `aws_cur_config` attributes (timestamps `YYYY-MM-DDTHH:MM:SS.ffffff`, UTC without a zone). */
export interface AwsCurAttributes {
  account_id: string;
  bucket_name: string;
  bucket_region: string;
  report_name: string;
  report_prefix: string;
  months?: number;
  status: BillStatus;
  status_updated_at?: string;
  created_at: string;
  updated_at?: string;
  error_messages: string[];
  account_filters?: unknown;
}

/** One export of Datadog's `azure_uc_configs` (one actual, one amortized). */
export interface AzureExportConfig {
  dataset_type: 'actual' | 'amortized';
  account_id: string;
  client_id: string;
  scope: string;
  export_name: string;
  export_path: string;
  storage_account: string;
  storage_container: string;
  status: BillStatus;
  error_messages: string[];
  created_at: string;
  updated_at?: string;
}

export type BillConfigDoc = { type: 'aws_cur_config'; id: string; attributes: AwsCurAttributes } | { type: 'azure_uc_configs'; id: string; attributes: { configs: AzureExportConfig[] } };

/** A provider bill ObserveCraft reads, with its import progress. */
export interface BillConfig {
  id: string;
  provider: BillProvider;
  doc: BillConfigDoc;
  last_import_at?: number | null;
  imported_months: string[];
}

export interface AwsBillInput {
  account_id: string;
  bucket_name: string;
  bucket_region: string;
  report_name: string;
  report_prefix: string;
}

export interface AzureExportInput {
  export_name: string;
  export_path: string;
  storage_account: string;
  storage_container: string;
}

export interface AzureBillInput {
  account_id: string;
  client_id: string;
  scope: string;
  actual_bill_config: AzureExportInput;
  amortized_bill_config: AzureExportInput;
}

/** What a provider's create takes (Datadog's create attributes). */
export type BillInput<P extends BillProvider = BillProvider> = P extends 'aws' ? AwsBillInput : AzureBillInput;

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

export async function listBillConfigs(signal?: AbortSignal): Promise<BillConfig[]> {
  if (!live) return mock.listBillConfigs();
  return (await get('/api/v1/cloud-cost/bill-configs', {}, signal)) as unknown as BillConfig[];
}

export async function createBillConfig<P extends BillProvider>(provider: P, attributes: BillInput<P>): Promise<BillConfig> {
  if (!live) return mock.createBillConfig(provider, attributes);
  return (await postTo('/api/v1/cloud-cost/bill-configs/{provider}', { provider }, attributes as never)) as unknown as BillConfig;
}

export async function setBillConfigEnabled(provider: BillProvider, id: string, enabled: boolean): Promise<BillConfig> {
  if (!live) return mock.setBillConfigEnabled(provider, id, enabled);
  return (await patch('/api/v1/cloud-cost/bill-configs/{provider}/{id}', { provider, id }, { enabled })) as unknown as BillConfig;
}

export async function deleteBillConfig(provider: BillProvider, id: string): Promise<void> {
  if (!live) return mock.deleteBillConfig(provider, id);
  return del('/api/v1/cloud-cost/bill-configs/{provider}/{id}', { provider, id });
}

export async function costRecommendations(f: RecommendationFilters, signal?: AbortSignal): Promise<RecommendationsResult> {
  if (!live) return recommendationsMock.recommendations(f);
  const query: Record<string, string> = {};
  if (f.provider) query.provider = f.provider;
  if (f.resourceType) query.resourceType = f.resourceType;
  if (f.recommendationType) query.recommendationType = f.recommendationType;
  if (f.tags?.length) query.tags = f.tags.join(',');
  return get('/api/v1/cloud-cost/recommendations', {}, signal, query);
}
