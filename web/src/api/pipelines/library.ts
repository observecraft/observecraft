// Integration log pipelines from integrations-core (BSD-3-Clause), converted by
// scripts/import-integration-pipelines.mjs: the few the in-browser mock offers (the server offers every
// integration's, server/seeds/integrations/core/pipelines.json, its LICENSE next to it). Each document's
// source field names its upstream path and revision. Our own pipelines for generic formats (key-value / logfmt lines,
// MIT) are the server's file, as they are.
import library from './library.generated.json' with { type: 'json' };
import own from '../../../../server/seeds/pipelines/observecraft.json' with { type: 'json' };
import type { Processor } from './types';

export interface IntegrationPipelineDoc {
  id: string;
  name: string;
  filter: string;
  processors: Processor[];
  source: { repository: string; revision: string; path: string; license: string };
}

export const INTEGRATION_PIPELINES: IntegrationPipelineDoc[] = ([...library, ...own] as unknown as IntegrationPipelineDoc[]).sort((a, b) => a.name.toLowerCase().localeCompare(b.name.toLowerCase()));
