// Integrations wire contract. An integration is a packaged collector configuration
// (OpenTelemetry receiver and/or Vector source) plus the metadata, dashboards and
// monitors that come with it. Live collection is configured on externally managed collectors.

export type IntegrationStatus = 'detected' | 'installed' | 'warning' | 'available';

export interface IntegrationSummary {
  id: string; // "redis", "aws-athena"
  name: string;
  publisher: string;
  /** Logo key resolved by the UI's logo registry (falls back to a monogram). */
  logo: string;
  status: IntegrationStatus;
  categories: string[];
  description: string;
  /** Present when status is "warning": what needs fixing. */
  warning?: string;
}

export interface CatalogRequest {
  query: string;
  status: IntegrationStatus | 'all';
  category: string | null;
}

/** The catalog is bounded (hundreds of entries), so search returns every match without paging. */
export interface CatalogResponse {
  integrations: IntegrationSummary[];
}

export interface CollectorConfig {
  /** Which agent applies it. */
  target: 'otel-collector' | 'vector';
  title: string;
  filename: string;
  language: 'yaml' | 'toml';
  content: string;
}

export interface CollectedMetric {
  name: string;
  type: 'gauge' | 'count' | 'rate' | 'distribution';
  unit?: string;
  description: string;
}

export interface IntegrationHost {
  host: string;
  status: 'ok' | 'warning' | 'error' | 'detected';
  collectorVersion: string;
  lastReported: number;
  message?: string;
}

export interface IntegrationDetail extends IntegrationSummary {
  overview: string;
  homepage?: string;
  offerings: string[];
  supportedOs: string[];
  dataTypes: string[];
  minCollectorVersion: string;
  setupSteps: string[];
  configs: CollectorConfig[];
  /** Credentials / endpoints the user supplies on install (notification channels, SaaS APIs). */
  settings: { key: string; label: string; placeholder: string; secret?: boolean }[];
  metrics: CollectedMetric[];
  logs: string | null;
  hosts: IntegrationHost[];
  resources: { kind: 'dashboard' | 'monitor'; name: string; description: string }[];
  releaseNotes: { version: string; date: string; notes: string[] }[];
}

export type AppOffering = 'Integration' | 'Software License' | 'Professional Services';

export interface MarketplaceApp {
  id: string;
  name: string;
  partner: string;
  logo: string;
  offering: AppOffering;
  categories: string[];
  description: string;
}

export interface MarketplaceRequest {
  query: string;
  category: string | null;
}
