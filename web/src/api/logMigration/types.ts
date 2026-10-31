// Logs → Configuration → Migration: SPL translation and imports (routes/log_migration.rs, spl.rs).

export interface SplStep {
  /** The SPL this step is about. */
  spl: string;
  /** What it became (empty when dropped). */
  result: string;
  note: string;
}

export interface SplMeasure {
  /** count | avg | sum | min | max | cardinality | pc50 … pc99 */
  function: string;
  field?: string | null;
  name: string;
}

export interface SplAggregation {
  /** timeseries | toplist | table | value */
  viz: string;
  measures: SplMeasure[];
  groupBy: string[];
  interval?: string | null;
  limit?: number | null;
  ascending: boolean;
  condition?: { measure: string; op: string; value: number } | null;
}

export interface SplTranslation {
  query: string;
  aggregation?: SplAggregation | null;
  earliest?: string | null;
  columns: string[];
  explanation: SplStep[];
  warnings: string[];
  complete: boolean;
}

export interface ImportSearch {
  title: string;
  spl: string;
  translation: SplTranslation;
}

export interface ImportAlert {
  comparator?: string | null;
  threshold?: number | null;
  windowMinutes: number;
  recipients: string[];
  schedule?: string | null;
}

export type ImportStatus = 'ready' | 'partial' | 'unsupported' | 'migrated' | 'failed';

export interface ImportAsset {
  id: string;
  kind: 'alert' | 'dashboard' | 'search';
  title: string;
  description?: string | null;
  status: ImportStatus;
  searches: ImportSearch[];
  alert?: ImportAlert | null;
  issues: string[];
  targetKind?: string | null;
  targetId?: string | null;
  error?: string | null;
  migratedAt?: number | null;
}

export interface ImportCounts {
  total: number;
  alerts: number;
  dashboards: number;
  searches: number;
  ready: number;
  partial: number;
  unsupported: number;
  migrated: number;
  failed: number;
}

export interface LogImport {
  id: string;
  name: string;
  source: string;
  createdAt: number;
  createdBy?: string | null;
  macros: number;
  counts: ImportCounts;
}

export interface LogImportDetail extends LogImport {
  assets: ImportAsset[];
}

export interface CreateImport {
  name: string;
  source?: string;
  alerts?: string;
  dashboards?: string;
  macros?: string;
}
