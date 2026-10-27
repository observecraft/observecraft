// Shared generated documents used by backend seeds, the mock store and integration resources.
// See each document's source field and server/seeds/integrations/kubernetes/LICENSE.
import kubernetes from '../../../../server/seeds/integrations/kubernetes/dashboards.json';
import type { Widget, TemplateVariable, LayoutType } from '../dashboards/types';

export const integrationDashboards = kubernetes.map((d) => ({
  ...d,
  kind: 'integration' as const,
  layoutType: d.layoutType as LayoutType,
  widgets: d.widgets as unknown as Widget[],
  templateVariables: d.templateVariables as TemplateVariable[],
}));
