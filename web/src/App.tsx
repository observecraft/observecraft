import { IndexesPage } from './pages/logs/IndexesPage';
import { DataRetentionPage } from './pages/settings/DataRetentionPage';
import { MonitorSettingsPage } from './pages/monitors/MonitorSettingsPage';
import { ArchiveSearchPage } from './pages/logs/ArchiveSearchPage';
import { ArchivesPage } from './pages/logs/ArchivesPage';
import { RehydrationsPage } from './pages/logs/RehydrationsPage';
import { MetricsLayout } from './pages/metrics/MetricsLayout';
import { MetricsExplorer } from './pages/metrics/MetricsExplorer';
import { MetricsSummary } from './pages/metrics/MetricsSummary';
import { ApiKeysPage } from './pages/settings/ApiKeysPage';
import { InstanceOrgsPage } from './pages/instance/InstanceOrgsPage';
import { Navigate, Route, Routes, useLocation } from 'react-router-dom';
import { PageHeader } from './components/shell/PageHeader';
import { Sidebar, useNavMode } from './components/shell/Sidebar';
import { TimeProvider } from './components/shell/TimeContext';
import { TopBar } from './components/shell/TopBar';
import { ApmComingSoon, ApmLayout } from './pages/apm/ApmLayout';
import { ApmHome } from './pages/apm/ApmHome';
import { ServicePage } from './pages/apm/service/ServicePage';
import { EndpointsPage } from './pages/apm/EndpointsPage';
import { SpanExplorer } from './pages/apm/explorer/SpanExplorer';
import { IncidentPage } from './pages/incidents/IncidentPage';
import { CiLayout, DeliveryLayout } from './pages/delivery/DeliveryLayout';
import { DoraPage } from './pages/delivery/DoraPage';
import { ExplorerPage as CiExplorerPage } from './pages/delivery/ExplorerPage';
import { FlakyPage } from './pages/delivery/FlakyPage';
import { OverviewPage as CiOverviewPage } from './pages/delivery/OverviewPage';
import { SettingsPage as CiSettingsPage } from './pages/delivery/SettingsPage';
import { DowntimesPage } from './pages/monitors/DowntimesPage';
import { MonitorEditor } from './pages/monitors/MonitorEditor';
import { MonitorListPage } from './pages/monitors/MonitorListPage';
import { MonitorsLayout } from './pages/monitors/MonitorsLayout';
import { MonitorStatusPage } from './pages/monitors/MonitorStatusPage';
import { NewMonitorPage } from './pages/monitors/NewMonitorPage';
import { QualityPage } from './pages/monitors/QualityPage';
import { TemplatesPage } from './pages/monitors/TemplatesPage';
import { TriggeredPage } from './pages/monitors/TriggeredPage';
import { IncidentsPage } from './pages/incidents/IncidentsPage';
import { AuditTrailPage } from './pages/settings/AuditTrailPage';
import { LoginMethodsPage } from './pages/settings/LoginMethodsPage';
import { OrgComingSoon, OrgSettingsLayout } from './pages/settings/OrgSettingsLayout';
import { PersonalSettings } from './pages/settings/PersonalSettings';
import { RolesPage, SamlMappingsPage } from './pages/settings/SamlMappingsPage';
import { TeamsPage } from './pages/settings/TeamsPage';
import { UsersPage } from './pages/settings/UsersPage';
import { AuthGate } from './pages/auth/AuthGate';
import { LinkPage } from './pages/auth/LinkPage';
import { LoginPage } from './pages/auth/LoginPage';
import { DashboardListPage, SharedDashboardsPage } from './pages/dashboards/list/DashboardListPage';
import { DashboardPage } from './pages/dashboards/view/DashboardPage';
import { ErrorTrackingLayout } from './pages/errors/ErrorTrackingLayout';
import { IssuesPage } from './pages/errors/IssuesPage';
import { HostsPage } from './pages/infrastructure/HostsPage';
import { KubernetesOverview } from './pages/kubernetes/KubernetesOverview';
import { KubernetesComingSoon } from './pages/kubernetes/KubernetesComingSoon';
import { OrchestrationExplorer } from './pages/kubernetes/OrchestrationExplorer';
import { CatalogPage } from './pages/integrations/CatalogPage';
import { IntegrationsComingSoon, IntegrationsLayout } from './pages/integrations/IntegrationsLayout';
import { MarketplacePage } from './pages/integrations/MarketplacePage';
import { LiveTail } from './pages/logs/LiveTail';
import { LogExplorer } from './pages/logs/LogExplorer';
import { LOGS_CONFIG_SLUGS, LogsConfigComingSoon, LogsConfigLayout } from './pages/logs/LogsConfigLayout';
import { LogsComingSoon, LogsLayout } from './pages/logs/LogsLayout';

function ComingSoon() {
  const { pathname } = useLocation();
  return (
    <>
      <PageHeader>
        <div className="page-tab active">{pathname.slice(1)}</div>
      </PageHeader>
      <div className="placeholder-page">This section hasn't been designed yet.</div>
    </>
  );
}

/** The signed-in application: sidebar, top bar and every product route. */
function Shell() {
  const [nav, setNav] = useNavMode();
  return (
    <TimeProvider>
      <div className="app" data-nav={nav}>
        <Sidebar mode={nav} setMode={setNav} />
        <TopBar onShowNav={nav === 'hidden' ? () => setNav('expanded') : undefined} />
        <main className="app-content">
          <Routes>
            <Route path="/" element={<Navigate to="/logs" replace />} />
            <Route path="/logs" element={<LogsLayout />}>
              <Route index element={<LogExplorer />} />
              <Route path="livetail" element={<LiveTail />} />
              <Route path="notebooks" element={<LogsComingSoon title="Notebooks" />} />
              <Route path="archive-search" element={<ArchiveSearchPage />} />
            </Route>
            <Route element={<LogsConfigLayout />}>
              <Route path="/logs/archives" element={<ArchivesPage />} />
              <Route path="/logs/indexes" element={<IndexesPage />} />
              <Route path="/logs/rehydrations" element={<RehydrationsPage />} />
              {LOGS_CONFIG_SLUGS.filter((slug) => !['archives', 'rehydrations', 'indexes'].includes(slug)).map((slug) => (
                <Route key={slug} path={`/logs/${slug}`} element={<LogsConfigComingSoon />} />
              ))}
            </Route>
            <Route path="/errors" element={<ErrorTrackingLayout />}>
              <Route index element={<IssuesPage />} />
            </Route>
            <Route path="/integrations" element={<IntegrationsLayout />}>
              <Route index element={<CatalogPage />} />
              <Route path="marketplace" element={<MarketplacePage />} />
              <Route path="reference-tables" element={<IntegrationsComingSoon title="Reference Tables" />} />
              <Route path="source-code" element={<IntegrationsComingSoon title="Link Source Code" />} />
            </Route>
            <Route path="/apm" element={<ApmLayout />}>
              <Route index element={<ApmHome />} />
              <Route path="services/:service" element={<ServicePage />} />
              <Route path="traces" element={<SpanExplorer />} />
              <Route path="endpoints" element={<EndpointsPage />} />
              <Route path="recommendations" element={<Navigate to="/apm?view=recommendations" replace />} />
              <Route path="profiles/*" element={<ApmComingSoon title="Profiles" />} />
              <Route path="debugger/*" element={<ApmComingSoon title="Live Debugger" />} />
              <Route path="settings" element={<ApmComingSoon title="APM Settings" />} />
            </Route>
            <Route path="/infrastructure" element={<HostsPage />} />
            <Route path="/kubernetes" element={<KubernetesOverview />} />
            <Route path="/orchestration/explorer" element={<OrchestrationExplorer />} />
            <Route path="/kubernetes/autoscaling" element={<KubernetesComingSoon tab="Autoscaling" />} />
            <Route path="/kubernetes/remediation" element={<KubernetesComingSoon tab="Remediation" />} />
            <Route path="/kubernetes/rollouts" element={<KubernetesComingSoon tab="Live Rollouts" />} />
            <Route path="/dashboards" element={<DashboardListPage />} />
            <Route path="/dashboards/shared" element={<SharedDashboardsPage />} />
            <Route path="/dashboards/:id" element={<DashboardPage />} />
            <Route path="/ci" element={<CiLayout />}>
              <Route index element={<CiOverviewPage />} />
              <Route path="explorer" element={<CiExplorerPage />} />
              <Route path="flaky" element={<FlakyPage />} />
              <Route path="settings" element={<CiSettingsPage />} />
            </Route>
            <Route path="/delivery" element={<DeliveryLayout />}>
              <Route index element={<Navigate to="/delivery/dora" replace />} />
              <Route path="dora" element={<DoraPage />} />
              <Route path="settings" element={<CiSettingsPage />} />
            </Route>
            <Route path="/monitors" element={<MonitorsLayout />}>
              <Route index element={<MonitorListPage />} />
              <Route path="triggered" element={<TriggeredPage />} />
              <Route path="downtimes" element={<DowntimesPage />} />
              <Route path="quality" element={<QualityPage />} />
            </Route>
            <Route path="/monitors/create" element={<NewMonitorPage />} />
            <Route path="/monitors/create/:type" element={<MonitorEditor />} />
            <Route path="/monitors/templates" element={<TemplatesPage />} />
            <Route path="/monitors/settings" element={<MonitorSettingsPage />} />
            <Route path="/monitors/:id" element={<MonitorStatusPage />} />
            <Route path="/monitors/:id/edit" element={<MonitorEditor />} />
            <Route path="/incidents" element={<IncidentsPage />} />
            <Route path="/incidents/:id" element={<IncidentPage />} />
            <Route path="/account/profile" element={<PersonalSettings />} />
            <Route path="/teams" element={<TeamsPage />} />
            <Route path="/metrics" element={<MetricsLayout />}>
              <Route index element={<MetricsExplorer />} />
              <Route path="summary" element={<MetricsSummary />} />
            </Route>
            <Route path="/instance/orgs" element={<InstanceOrgsPage />} />
            <Route path="/organization-settings" element={<OrgSettingsLayout />}>
              <Route index element={<Navigate to="users" replace />} />
              <Route path="users" element={<UsersPage />} />
              <Route path="api-keys" element={<ApiKeysPage />} />
              <Route path="data-retention" element={<DataRetentionPage />} />
              <Route path="saml-mappings" element={<SamlMappingsPage />} />
              <Route path="roles" element={<RolesPage />} />
              <Route path="login-methods" element={<LoginMethodsPage />} />
              <Route path="audit-trail" element={<AuditTrailPage />} />
              <Route path="*" element={<OrgComingSoon />} />
            </Route>
            <Route path="*" element={<ComingSoon />} />
          </Routes>
        </main>
      </div>
    </TimeProvider>
  );
}

export function App() {
  return (
    <Routes>
      <Route path="/login" element={<LoginPage />} />
      <Route path="/login/link/:token" element={<LinkPage />} />
      <Route
        path="*"
        element={
          <AuthGate>
            <Shell />
          </AuthGate>
        }
      />
    </Routes>
  );
}
