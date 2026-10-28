// Integration log pipelines from integrations-core (BSD-3-Clause), converted by
// scripts/import-integration-pipelines.mjs; the server installs the same files. Each document's
// source field names its upstream path and revision; the license is next to it
// (server/seeds/integrations/<integration>/LICENSE). Used by the in-browser mock.
import coredns from '../../../../server/seeds/integrations/coredns/pipeline.json';
import etcd from '../../../../server/seeds/integrations/etcd/pipeline.json';
import kubeScheduler from '../../../../server/seeds/integrations/kube_scheduler/pipeline.json';
import mysql from '../../../../server/seeds/integrations/mysql/pipeline.json';
import nginx from '../../../../server/seeds/integrations/nginx/pipeline.json';
import nginxIngress from '../../../../server/seeds/integrations/nginx_ingress_controller/pipeline.json';
import postgres from '../../../../server/seeds/integrations/postgres/pipeline.json';
import redisdb from '../../../../server/seeds/integrations/redisdb/pipeline.json';
import type { Processor } from './types';

export interface IntegrationPipelineDoc {
  id: string;
  name: string;
  filter: string;
  processors: Processor[];
  source: { repository: string; revision: string; path: string; license: string };
}

export const INTEGRATION_PIPELINES: IntegrationPipelineDoc[] = [coredns, etcd, kubeScheduler, mysql, nginx, nginxIngress, postgres, redisdb]
  .map((d) => d as unknown as IntegrationPipelineDoc)
  .sort((a, b) => a.name.toLowerCase().localeCompare(b.name.toLowerCase()));
