// Datadog's newer table requests (`request_type: table`) name their columns apart from their queries: the rows'
// group-bys and the compute columns carry aliases and formats.
import type { ConditionalFormat } from '../../pages/dashboards/widgets/conditionalFormats';

interface TableRequest {
  rows?: { group_by?: { alias?: string; group_keys?: { group_key: string }[] }[] };
  columns?: { alias?: string; compute?: { query: string }; format?: { conditional_formats?: ConditionalFormat[] } }[];
}

/** Labels of the group columns (tag → alias) and of the compute columns (query → alias and formats). */
export function tableColumns(request: TableRequest): { tags: Map<string, string>; computes: Map<string, { label: string; conditional_formats?: ConditionalFormat[] }> } {
  const tags = new Map<string, string>();
  for (const g of request.rows?.group_by ?? []) for (const k of g.group_keys ?? []) if (g.alias) tags.set(k.group_key, g.alias);
  const computes = new Map<string, { label: string; conditional_formats?: ConditionalFormat[] }>();
  for (const c of request.columns ?? [])
    if (c.compute?.query) computes.set(c.compute.query, { label: c.alias ?? c.compute.query, ...(c.format?.conditional_formats ? { conditional_formats: c.format.conditional_formats } : {}) });
  return { tags, computes };
}
