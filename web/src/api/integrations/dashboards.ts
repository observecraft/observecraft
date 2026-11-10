// Shared generated documents used by backend seeds, the mock store and integration resources.
// See each document's source field and server/seeds/integrations/kubernetes/LICENSE.
import kubernetes from '../../../../server/seeds/integrations/kubernetes/dashboards.json';
// ObserveCraft's own AWS, Azure and Google Cloud overviews (scripts/cloud-dashboards.mjs).
import aws from '../../../../server/seeds/integrations/aws/dashboards.json';
import azure from '../../../../server/seeds/integrations/azure/dashboards.json';
import gcp from '../../../../server/seeds/integrations/gcp/dashboards.json';
// ObserveCraft's own NetBird dashboard (scripts/netbird-dashboard.mjs).
import netbird from '../../../../server/seeds/integrations/netbird/dashboards.json';
// ObserveCraft's own FleetDM dashboard (scripts/fleetdm-dashboard.mjs).
import fleetdm from '../../../../server/seeds/integrations/fleetdm/dashboards.json';
// ObserveCraft's own authentik dashboard (scripts/authentik-dashboard.mjs).
import authentik from '../../../../server/seeds/integrations/authentik/dashboards.json';
// ObserveCraft's own Microsoft Entra ID dashboard (scripts/entra-id-dashboard.mjs).
import entraId from '../../../../server/seeds/integrations/azure-active-directory/dashboards.json';
// ObserveCraft's own GitLab Audit Events dashboard (scripts/gitlab-audit-events-dashboard.mjs).
import gitlabAudit from '../../../../server/seeds/integrations/gitlab-audit-events/dashboards.json';
// ObserveCraft's own Google Workspace dashboard (scripts/google-workspace-dashboard.mjs).
import googleWorkspace from '../../../../server/seeds/integrations/google-workspace/dashboards.json';
// ObserveCraft's own health (scripts/observecraft-dashboard.mjs), over the server's self-monitoring metrics.
import observecraft from '../../../../server/seeds/integrations/observecraft/dashboards.json';
// ObserveCraft's own dashboard over the synthetics.* metrics of synthetic API tests.
import synthetics from '../../../../server/seeds/dashboards/synthetics-api-tests.json';
// The integrations-core dashboards of the integrations the mock shows installed (the server installs every
// integration's on its first data: scripts/import-integration-dashboards.mjs --all).
import core from './coreDashboards.generated.json';
// Our dashboards of the AWS services the demo account collects (scripts/cloud-service-dashboards.mjs).
import cloudServices from './cloudServiceDashboards.generated.json';
import type { Widget, TemplateVariable, LayoutType } from '../dashboards/types';

export const integrationDashboards = [
  ...kubernetes,
  ...aws,
  ...azure,
  ...gcp,
  ...netbird,
  ...fleetdm,
  ...authentik,
  ...gitlabAudit,
  ...googleWorkspace,
  ...entraId,
  ...observecraft,
  synthetics,
  ...core.dashboards,
  ...cloudServices.dashboards,
].map((d) => ({
  ...d,
  kind: 'integration' as const,
  layoutType: d.layoutType as LayoutType,
  widgets: d.widgets as unknown as Widget[],
  templateVariables: d.templateVariables as TemplateVariable[],
}));
