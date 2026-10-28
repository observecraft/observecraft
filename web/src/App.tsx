import { IndexesPage } from './pages/logs/IndexesPage';
import { PipelinesPage } from './pages/logs/PipelinesPage';
import { GenerateMetricsPage } from './pages/logs/GenerateMetricsPage';
import { StandardAttributesPage } from './pages/logs/StandardAttributesPage';
import { DataAccessPage } from './pages/logs/DataAccessPage';
import { DataRetentionPage } from './pages/settings/DataRetentionPage';
import { MonitorSettingsPage } from './pages/monitors/MonitorSettingsPage';
import { ArchiveSearchPage } from './pages/logs/ArchiveSearchPage';
import { ArchivesPage } from './pages/logs/ArchivesPage';
import { RehydrationsPage } from './pages/logs/RehydrationsPage';
import { MetricsLayout } from './pages/metrics/MetricsLayout';
import { MetricsExplorer } from './pages/metrics/MetricsExplorer';
import { MetricsSummary } from './pages/metrics/MetricsSummary';
import { MetricsVolume } from './pages/metrics/MetricsVolume';
import { DerivedMetrics } from './pages/metrics/DerivedMetrics';
import { FilteredMetricsPage, MetricRulesPage, MetricsSettingsLayout } from './pages/metrics/settings/MetricsSettings';
import { CheckSummary } from './pages/checks/CheckSummary';
import { ExternalProviderStatus, WatchdogLayout } from './pages/watchdog/WatchdogLayout';
import { WatchdogPage } from './pages/watchdog/WatchdogPage';
import { ReleaseNotesPage } from './pages/releaseNotes/ReleaseNotesPage';
import { SetupGuidePage } from './pages/setup/SetupGuidePage';
import { ReferenceTableCreatePage } from './pages/integrations/ReferenceTableCreatePage';
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
import { DeploymentGatesLayout, EvaluationsTab, GatesTab } from './pages/delivery/DeploymentGatesPage';
import { PrGatesPage } from './pages/delivery/PrGatesPage';
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
import { ApplicationsPage } from './pages/llm/ApplicationsPage';
import { CostPage } from './pages/llm/CostPage';
import { EvaluationsPage } from './pages/llm/EvaluationsPage';
import { LlmLayout } from './pages/llm/LlmLayout';
import { LlmTracesPage } from './pages/llm/LlmTracesPage';
import { CorrectionsPage } from './pages/slo/CorrectionsPage';
import { SloEditor } from './pages/slo/SloEditor';
import { SloLayout } from './pages/slo/SloLayout';
import { SloListPage } from './pages/slo/SloListPage';
import { IncidentsPage } from './pages/incidents/IncidentsPage';
import { PublicStatusPage } from './pages/statusPages/PublicStatusPage';
import { StatusPageEditorPage } from './pages/statusPages/StatusPageEditorPage';
import { StatusPagePage } from './pages/statusPages/StatusPagePage';
import { StatusPagesPage } from './pages/statusPages/StatusPagesPage';
import { AnalyticsPage as OnCallAnalyticsPage } from './pages/onCall/AnalyticsPage';
import { OnCallLayout } from './pages/onCall/OnCallLayout';
import { PagesPage as OnCallPagesPage } from './pages/onCall/PagesPage';
import { PoliciesPage as OnCallPoliciesPage } from './pages/onCall/PoliciesPage';
import { PolicyEditor as OnCallPolicyEditor } from './pages/onCall/PolicyEditor';
import { ProfilePage as OnCallProfilePage } from './pages/onCall/ProfilePage';
import { ScheduleEditor as OnCallScheduleEditor } from './pages/onCall/ScheduleEditor';
import { SchedulesPage as OnCallSchedulesPage } from './pages/onCall/SchedulesPage';
import { SummaryPage as OnCallSummaryPage } from './pages/onCall/SummaryPage';
import { OnCallTeamsPage } from './pages/onCall/TeamsPage';
import { SyntheticTestEditor } from './pages/synthetics/SyntheticTestEditor';
import { SyntheticTestPage } from './pages/synthetics/SyntheticTestPage';
import { SyntheticTestsPage } from './pages/synthetics/SyntheticTestsPage';
import { SyntheticsLayout, SyntheticsNotAvailable } from './pages/synthetics/SyntheticsLayout';
import { SyntheticsOverviewPage } from './pages/synthetics/SyntheticsOverviewPage';
import { TestSuitesPage } from './pages/synthetics/TestSuitesPage';
import { SuiteEditorPage } from './pages/synthetics/SuiteEditorPage';
import { ResultsExplorerPage } from './pages/synthetics/ResultsExplorerPage';
import { TestCoveragePage } from './pages/synthetics/TestCoveragePage';
import {
  ContinuousTestingSettings,
  DefaultSettingsPage,
  GlobalVariablesPage,
  MobileApplicationsSettings,
  SyntheticsDowntimesSettings,
  SyntheticsIntegrationSettings,
  SyntheticsPrivateLocations,
  SyntheticsProfilesSettings,
  SyntheticsSettingsLayout,
} from './pages/synthetics/SyntheticsSettings';
import { CreateFlagPage } from './pages/featureFlags/CreateFlagPage';
import { EnvironmentsPage } from './pages/featureFlags/EnvironmentsPage';
import { FeatureFlagsLayout, FlagsOverview } from './pages/featureFlags/FeatureFlagsPage';
import { FlagDetailPage } from './pages/featureFlags/FlagDetailPage';
import { FlagHistoryPage } from './pages/featureFlags/FlagHistoryPage';
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
import { ReportsPage } from './pages/dashboards/reports/ReportsPage';
import { SheetsListPage } from './pages/sheets/SheetsListPage';
import { SpreadsheetPage } from './pages/sheets/SpreadsheetPage';
import { NotebookListPage, NotebookReportsPage, NotebookTemplatesPage } from './pages/notebooks/NotebookListPage';
import { NotebookPage } from './pages/notebooks/NotebookPage';
import { ErrorTrackingLayout } from './pages/errors/ErrorTrackingLayout';
import { IssuesPage } from './pages/errors/IssuesPage';
import { ChangesPage } from './pages/infrastructure/ChangesPage';
import { ResourceCatalogPage } from './pages/infrastructure/ResourceCatalogPage';
import { ResourcesLayout } from './pages/infrastructure/ResourcesLayout';
import { PoliciesPage } from './pages/infrastructure/PoliciesPage';
import { PolicyDetailPage } from './pages/infrastructure/PolicyDetailPage';
import { PolicyEditorPage } from './pages/infrastructure/PolicyEditorPage';
import { CloudcraftPage } from './pages/infrastructure/CloudcraftPage';
import { HostsPage } from './pages/infrastructure/HostsPage';
import { GpuMonitoringPage } from './pages/gpu/GpuMonitoringPage';
import { ServerlessPage } from './pages/serverless/ServerlessPage';
import { SqlEditorPage } from './pages/sqlEditor/SqlEditorPage';
import { ConfigurationPage as FleetConfigurationPage } from './pages/fleet/ConfigurationPage';
import { FleetLayout } from './pages/fleet/FleetLayout';
import { FleetViewPage } from './pages/fleet/FleetViewPage';
import { UpgradesPage as FleetUpgradesPage } from './pages/fleet/UpgradesPage';
import { DatabaseMonitoringPage } from './pages/data/DatabaseMonitoringPage';
import { DataJobsPage } from './pages/data/DataJobsPage';
import { KubernetesOverview } from './pages/kubernetes/KubernetesOverview';
import { KubernetesComingSoon } from './pages/kubernetes/KubernetesComingSoon';
import { OrchestrationExplorer, ProcessMetrics } from './pages/kubernetes/OrchestrationExplorer';
import { ProfilesLayout } from './pages/apm/profiles/ProfilesLayout';
import { NetworkLayout } from './pages/network/NetworkLayout';
import { AnalyticsPage } from './pages/network/AnalyticsPage';
import { NetworkMap } from './pages/network/NetworkMap';
import { ProfileExplorer } from './pages/apm/profiles/ProfileExplorer';
import { ProfilesOverview } from './pages/apm/profiles/ProfilesOverview';
import { ProfileComparison } from './pages/apm/profiles/ProfileComparison';
import { ProfileInsights } from './pages/apm/profiles/ProfileInsights';
import { CatalogPage } from './pages/integrations/CatalogPage';
import { IntegrationsComingSoon, IntegrationsLayout } from './pages/integrations/IntegrationsLayout';
import { MarketplacePage } from './pages/integrations/MarketplacePage';
import { ReferenceTablePage } from './pages/integrations/ReferenceTablePage';
import { ReferenceTablesPage } from './pages/integrations/ReferenceTablesPage';
import { LiveTail } from './pages/logs/LiveTail';
import { LogExplorer } from './pages/logs/LogExplorer';
import { LOGS_CONFIG_SLUGS, LogsConfigComingSoon, LogsConfigLayout } from './pages/logs/LogsConfigLayout';
import { LogsLayout } from './pages/logs/LogsLayout';
import { EventExplorer } from './pages/events/EventExplorer';
import { EventsLayout, EventsSummary } from './pages/events/EventsLayout';
import { TriageInbox } from './pages/events/TriageInbox';
import { EventGenerateMetricsPage, EventPipelinesPage, EventProjectsPage, EventSettingsLayout, IntelligentCorrelationPage } from './pages/events/settings/EventSettingsLayout';
import { CorrelatorsPage } from './pages/events/settings/CorrelatorsPage';
import { MaintenanceWindowsPage } from './pages/events/settings/MaintenanceWindowsPage';
import { WorkItemAttributesPage } from './pages/events/settings/WorkItemAttributesPage';
import { PortalLayout } from './pages/portal/PortalLayout';
import { PortalHome } from './pages/portal/PortalHome';
import { PortalSettings, PortalSettingsSoon } from './pages/portal/PortalSettings';
import { SettingsConfigurations } from './pages/portal/SettingsConfigurations';
import { SettingsSources } from './pages/portal/SettingsSources';
import { ScorecardsPage } from './pages/portal/ScorecardsPage';
import { SurveysPage } from './pages/portal/surveys/SurveysPage';
import { SurveyEditorPage } from './pages/portal/surveys/SurveyEditorPage';
import { SurveyResultsPage } from './pages/portal/surveys/SurveyResultsPage';
import { SurveyRespondPage } from './pages/portal/surveys/SurveyRespondPage';
import { ActionsPage } from './pages/portal/actions/ActionsPage';
import { ActionEditorPage } from './pages/portal/actions/ActionEditorPage';
import { ActionRunPage } from './pages/portal/actions/ActionRunPage';
import { SoftwareCatalogPage } from './pages/portal/catalog/CatalogPage';
import { EntityFormPage } from './pages/portal/catalog/EntityFormPage';
import { CatalogHealth } from './pages/portal/reports/CatalogHealth';
import { ReliabilityOverview } from './pages/portal/reports/ReliabilityOverview';
import { ReportsLayout } from './pages/portal/reports/ReportsLayout';
import { ScorecardsPerformance } from './pages/portal/reports/ScorecardsPerformance';
import { TeamsPerformance } from './pages/portal/reports/TeamsPerformance';
import { AutomationLayout } from './pages/automation/AutomationLayout';
import { WorkflowsIntro } from './pages/automation/workflows/WorkflowsIntro';
import { WorkflowsList } from './pages/automation/workflows/WorkflowsList';
import { WorkflowsBlueprints } from './pages/automation/workflows/WorkflowsBlueprints';
import { WorkflowsAnalytics } from './pages/automation/workflows/WorkflowsAnalytics';
import { ActionCatalogPage } from './pages/automation/workflows/ActionCatalogPage';
import { NewWorkflowPage } from './pages/automation/workflows/NewWorkflowPage';
import { WorkflowEditorPage } from './pages/automation/workflows/WorkflowEditorPage';
import { AppsIntro } from './pages/automation/apps/AppsIntro';
import { AppsList } from './pages/automation/apps/AppsList';
import { AppsBlueprints } from './pages/automation/apps/AppsBlueprints';
import { AppBuilderPage } from './pages/automation/apps/AppBuilderPage';
import { AppViewPage } from './pages/automation/apps/AppViewPage';

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
              <Route path="archive-search" element={<ArchiveSearchPage />} />
            </Route>
            <Route path="/logs/notebooks" element={<Navigate to="/notebooks" replace />} />
            <Route element={<LogsConfigLayout />}>
              <Route path="/logs/archives" element={<ArchivesPage />} />
              <Route path="/logs/indexes" element={<IndexesPage />} />
              <Route path="/logs/pipelines" element={<PipelinesPage />} />
              <Route path="/logs/generate-metrics" element={<GenerateMetricsPage />} />
              <Route path="/logs/attributes" element={<StandardAttributesPage />} />
              <Route path="/logs/data-access" element={<DataAccessPage />} />
              <Route path="/logs/rehydrations" element={<RehydrationsPage />} />
              {LOGS_CONFIG_SLUGS.filter((slug) => !['archives', 'rehydrations', 'indexes', 'pipelines', 'generate-metrics', 'attributes', 'data-access'].includes(slug)).map((slug) => (
                <Route key={slug} path={`/logs/${slug}`} element={<LogsConfigComingSoon />} />
              ))}
            </Route>
            <Route path="/errors" element={<ErrorTrackingLayout />}>
              <Route index element={<IssuesPage />} />
            </Route>
            <Route path="/integrations/reference-tables/new" element={<ReferenceTableCreatePage />} />
            <Route path="/integrations" element={<IntegrationsLayout />}>
              <Route index element={<CatalogPage />} />
              <Route path="marketplace" element={<MarketplacePage />} />
              <Route path="reference-tables" element={<ReferenceTablesPage />} />
              <Route path="reference-tables/:name" element={<ReferenceTablePage />} />
              <Route path="source-code" element={<IntegrationsComingSoon title="Link Source Code" />} />
            </Route>
            <Route path="/fleet" element={<FleetLayout />}>
              <Route index element={<FleetViewPage />} />
              <Route path="configuration" element={<FleetConfigurationPage />} />
              <Route path="upgrades" element={<FleetUpgradesPage />} />
            </Route>
            <Route path="/apm" element={<ApmLayout />}>
              <Route index element={<ApmHome />} />
              <Route path="services/:service" element={<ServicePage />} />
              <Route path="traces" element={<SpanExplorer />} />
              <Route path="endpoints" element={<EndpointsPage />} />
              <Route path="recommendations" element={<Navigate to="/apm?view=recommendations" replace />} />
              <Route path="profiles" element={<ProfilesLayout />}>
                <Route index element={<ProfileExplorer />} />
                <Route path="overview" element={<ProfilesOverview />} />
                <Route path="comparison" element={<ProfileComparison />} />
                <Route path="insights" element={<ProfileInsights />} />
              </Route>
              <Route path="debugger/*" element={<ApmComingSoon title="Live Debugger" />} />
              <Route path="settings" element={<ApmComingSoon title="APM Settings" />} />
            </Route>
            <Route path="/infrastructure" element={<HostsPage />} />
            <Route path="/infrastructure/map" element={<Navigate to="/infrastructure?view=map" replace />} />
            <Route path="/infrastructure/add" element={<SetupGuidePage kind="infrastructure" />} />
            <Route element={<ResourcesLayout />}>
              <Route path="/infrastructure/catalog" element={<ResourceCatalogPage />} />
              <Route path="/infrastructure/changes" element={<ChangesPage />} />
              <Route path="/infrastructure/policies" element={<PoliciesPage />} />
              <Route path="/infrastructure/policies/new" element={<PolicyEditorPage />} />
              <Route path="/infrastructure/policies/:id" element={<PolicyDetailPage />} />
              <Route path="/infrastructure/policies/:id/edit" element={<PolicyEditorPage />} />
              <Route path="/cloudcraft" element={<CloudcraftPage />} />
            </Route>
            <Route path="/gpu-monitoring" element={<GpuMonitoringPage view="summary" />} />
            <Route path="/gpu-monitoring/explorer" element={<GpuMonitoringPage view="explorer" />} />
            <Route path="/serverless" element={<ServerlessPage />} />
            <Route path="/ddsql" element={<SqlEditorPage />} />
            <Route path="/data" element={<DatabaseMonitoringPage view="overview" />} />
            <Route path="/data/setup" element={<DatabaseMonitoringPage view="setup" />} />
            <Route path="/data/jobs" element={<DataJobsPage />} />
            <Route path="/databases" element={<Navigate to="/data" replace />} />
            <Route path="/logs/onboarding" element={<SetupGuidePage kind="logs" />} />
            <Route path="/release-notes" element={<ReleaseNotesPage />} />
            <Route path="/kubernetes" element={<KubernetesOverview />} />
            <Route path="/orchestration/explorer" element={<OrchestrationExplorer />} />
            <Route path="/network" element={<NetworkLayout />}>
              <Route index element={<AnalyticsPage />} />
              <Route path="map" element={<NetworkMap />} />
            </Route>
            <Route path="/process" element={<OrchestrationExplorer />} />
            <Route path="/process/metrics" element={<ProcessMetrics />} />
            <Route path="/kubernetes/autoscaling" element={<KubernetesComingSoon tab="Autoscaling" />} />
            <Route path="/kubernetes/remediation" element={<KubernetesComingSoon tab="Remediation" />} />
            <Route path="/kubernetes/rollouts" element={<KubernetesComingSoon tab="Live Rollouts" />} />
            <Route path="/dashboards" element={<DashboardListPage />} />
            <Route path="/dashboards/shared" element={<SharedDashboardsPage />} />
            <Route path="/dashboards/reports" element={<ReportsPage />} />
            <Route path="/dashboards/quick-graph" element={<DashboardListPage quickGraph />} />
            <Route path="/sheets" element={<SheetsListPage />} />
            <Route path="/sheets/:id" element={<SpreadsheetPage />} />
            <Route path="/dashboards/:id" element={<DashboardPage />} />
            <Route path="/notebooks" element={<NotebookListPage />} />
            <Route path="/notebooks/templates" element={<NotebookTemplatesPage />} />
            <Route path="/notebooks/reports" element={<NotebookReportsPage />} />
            <Route path="/notebooks/:id" element={<NotebookPage />} />
            <Route path="/ci" element={<CiLayout />}>
              <Route index element={<CiOverviewPage />} />
              <Route path="explorer" element={<CiExplorerPage />} />
              <Route path="flaky" element={<FlakyPage />} />
              <Route path="settings" element={<CiSettingsPage />} />
            </Route>
            <Route path="/delivery/deployment-gates" element={<DeploymentGatesLayout />}>
              <Route index element={<GatesTab />} />
              <Route path="evaluations" element={<EvaluationsTab />} />
            </Route>
            <Route path="/delivery/pr-gates" element={<PrGatesPage />} />
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
            <Route path="/slo" element={<SloLayout />}>
              <Route index element={<SloListPage />} />
              <Route path="corrections" element={<CorrectionsPage />} />
            </Route>
            <Route path="/slo/create" element={<SloEditor />} />
            <Route path="/slo/:id/edit" element={<SloEditor />} />
            <Route path="/llm" element={<LlmLayout />}>
              <Route index element={<ApplicationsPage />} />
              <Route path="traces" element={<LlmTracesPage />} />
              <Route path="cost" element={<CostPage />} />
              <Route path="evaluations" element={<EvaluationsPage />} />
              <Route path="setup" element={null} />
            </Route>
            <Route path="/synthetics" element={<SyntheticsLayout />}>
              <Route index element={<SyntheticsOverviewPage />} />
              <Route path="tests" element={<SyntheticTestsPage />} />
              <Route path="suites" element={<TestSuitesPage />} />
              <Route path="suites/new" element={<SuiteEditorPage />} />
              <Route path="suites/:id" element={<SuiteEditorPage />} />
              <Route path="explorer" element={<ResultsExplorerPage />} />
              <Route path="test-coverage" element={<TestCoveragePage />} />
              <Route
                path="ai-testing"
                element={
                  <SyntheticsNotAvailable title="AI Testing isn't available yet">
                    Generating and maintaining tests with an AI assistant isn't available yet. Create API tests with New Test.
                  </SyntheticsNotAvailable>
                }
              />
              <Route
                path="continuous-testing"
                element={
                  <SyntheticsNotAvailable title="Continuous Testing isn't available yet">
                    Running tests from CI pipelines isn't available yet. Tests run on their schedule and with Run Test Now; their runs are in the Results Explorer.
                  </SyntheticsNotAvailable>
                }
              />
            </Route>
            <Route path="/synthetics/settings" element={<SyntheticsSettingsLayout />}>
              <Route index element={<Navigate to="default" replace />} />
              <Route path="default" element={<DefaultSettingsPage />} />
              <Route path="profiles" element={<SyntheticsProfilesSettings />} />
              <Route path="downtimes" element={<SyntheticsDowntimesSettings />} />
              <Route path="variables" element={<GlobalVariablesPage />} />
              <Route path="integrations" element={<SyntheticsIntegrationSettings />} />
              <Route path="private-locations" element={<SyntheticsPrivateLocations />} />
              <Route path="continuous-testing" element={<ContinuousTestingSettings />} />
              <Route path="mobile-applications" element={<MobileApplicationsSettings />} />
            </Route>
            <Route path="/synthetics/create" element={<SyntheticTestEditor />} />
            <Route path="/synthetics/tests/:id" element={<SyntheticTestPage />} />
            <Route path="/synthetics/tests/:id/edit" element={<SyntheticTestEditor />} />
            <Route path="/feature-flags" element={<FeatureFlagsLayout />}>
              <Route index element={<FlagsOverview />} />
              <Route path="history" element={<FlagHistoryPage />} />
            </Route>
            <Route path="/feature-flags/create" element={<CreateFlagPage />} />
            <Route path="/feature-flags/environments" element={<EnvironmentsPage />} />
            <Route path="/feature-flags/settings" element={<Navigate to="/feature-flags/environments" replace />} />
            <Route path="/feature-flags/:id" element={<FlagDetailPage />} />
            <Route path="/feature-flags/:id/edit" element={<CreateFlagPage />} />
            <Route path="/event" element={<EventsLayout />}>
              <Route index element={<Navigate to="explorer" replace />} />
              <Route path="explorer" element={<EventExplorer />} />
              <Route path="inbox" element={<TriageInbox />} />
              <Route path="summary" element={<EventsSummary />} />
            </Route>
            <Route path="/event/settings" element={<EventSettingsLayout />}>
              <Route index element={<Navigate to="correlators" replace />} />
              <Route path="projects" element={<EventProjectsPage />} />
              <Route path="correlators" element={<CorrelatorsPage />} />
              <Route path="intelligent-correlation" element={<IntelligentCorrelationPage />} />
              <Route path="attributes" element={<WorkItemAttributesPage />} />
              <Route path="maintenance-windows" element={<MaintenanceWindowsPage />} />
              <Route path="pipelines" element={<EventPipelinesPage />} />
              <Route path="generate-metrics" element={<EventGenerateMetricsPage />} />
            </Route>
            <Route path="/check/summary" element={<CheckSummary />} />
            <Route path="/watchdog" element={<WatchdogLayout />}>
              <Route index element={<WatchdogPage />} />
              <Route path="external-provider-status" element={<ExternalProviderStatus />} />
            </Route>
            <Route path="/portal" element={<PortalLayout />}>
              <Route index element={<Navigate to="/portal/home" replace />} />
              <Route path="home" element={<PortalHome />} />
              <Route path="scorecards" element={<ScorecardsPage />} />
              <Route path="surveys" element={<SurveysPage />} />
              <Route path="surveys/new" element={<SurveyEditorPage />} />
              <Route path="surveys/:id/edit" element={<SurveyEditorPage />} />
              <Route path="surveys/:id/results" element={<SurveyResultsPage />} />
              <Route path="actions" element={<ActionsPage />} />
              <Route path="actions/new" element={<ActionEditorPage />} />
              <Route path="actions/:id/edit" element={<ActionEditorPage />} />
              <Route path="actions/:id" element={<ActionRunPage />} />
              <Route path="catalog" element={<SoftwareCatalogPage />} />
              <Route path="catalog/new" element={<EntityFormPage />} />
              <Route path="catalog/edit/:id" element={<EntityFormPage />} />
              <Route path="reports" element={<ReportsLayout />}>
                <Route index element={<Navigate to="teams" replace />} />
                <Route path="teams" element={<TeamsPerformance />} />
                <Route path="reliability" element={<ReliabilityOverview />} />
                <Route path="catalog-health" element={<CatalogHealth />} />
                <Route path="scorecards" element={<ScorecardsPerformance />} />
              </Route>
            </Route>
            <Route path="/portal/settings" element={<PortalSettings />}>
              <Route index element={<Navigate to="sources" replace />} />
              <Route path="sources" element={<SettingsSources />} />
              <Route path="configurations" element={<SettingsConfigurations />} />
              <Route path=":page" element={<PortalSettingsSoon />} />
            </Route>
            <Route path="/portal/surveys/:id/respond" element={<SurveyRespondPage />} />
            <Route path="/automation" element={<AutomationLayout />}>
              <Route index element={<WorkflowsList />} />
              <Route path="intro" element={<WorkflowsIntro />} />
              <Route path="blueprints" element={<WorkflowsBlueprints />} />
              <Route path="analytics" element={<WorkflowsAnalytics />} />
              <Route path="actions" element={<ActionCatalogPage />} />
              <Route path="apps" element={<AppsList />} />
              <Route path="apps/intro" element={<AppsIntro />} />
              <Route path="apps/blueprints" element={<AppsBlueprints />} />
            </Route>
            <Route path="/automation/workflows/new" element={<NewWorkflowPage />} />
            <Route path="/automation/workflows/:id" element={<WorkflowEditorPage />} />
            <Route path="/automation/apps/new" element={<AppBuilderPage />} />
            <Route path="/automation/apps/:id/edit" element={<AppBuilderPage />} />
            <Route path="/automation/apps/:id" element={<AppViewPage />} />
            <Route path="/incidents" element={<IncidentsPage />} />
            <Route path="/incidents/:id" element={<IncidentPage />} />
            <Route path="/on-call" element={<OnCallLayout />}>
              <Route index element={<OnCallSummaryPage />} />
              <Route path="teams" element={<OnCallTeamsPage />} />
              <Route path="pages/:id?" element={<OnCallPagesPage />} />
              <Route path="schedules" element={<OnCallSchedulesPage />} />
              <Route path="escalation-policies" element={<OnCallPoliciesPage />} />
              <Route path="analytics" element={<OnCallAnalyticsPage />} />
              <Route path="profile" element={<OnCallProfilePage />} />
            </Route>
            <Route path="/on-call/schedules/new" element={<OnCallScheduleEditor />} />
            <Route path="/on-call/schedules/:id" element={<OnCallScheduleEditor />} />
            <Route path="/on-call/escalation-policies/new" element={<OnCallPolicyEditor />} />
            <Route path="/on-call/escalation-policies/:id" element={<OnCallPolicyEditor />} />
            <Route path="/status-pages" element={<StatusPagesPage />} />
            <Route path="/status-pages/new" element={<StatusPageEditorPage />} />
            <Route path="/status-pages/:id" element={<StatusPagePage />} />
            <Route path="/status-pages/:id/edit" element={<StatusPageEditorPage />} />
            <Route path="/account/profile" element={<PersonalSettings />} />
            <Route path="/teams" element={<TeamsPage />} />
            <Route path="/metrics" element={<MetricsLayout />}>
              <Route index element={<MetricsExplorer />} />
              <Route path="summary" element={<MetricsSummary />} />
              <Route path="volume" element={<MetricsVolume />} />
              <Route path="derived" element={<DerivedMetrics />} />
            </Route>
            <Route path="/metrics/settings" element={<MetricsSettingsLayout />}>
              <Route index element={<Navigate to="rules" replace />} />
              <Route path="rules" element={<MetricRulesPage />} />
              <Route path="filtered" element={<FilteredMetricsPage />} />
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
      {/* Read-only status pages: no sign-in for public ones (the API decides what a visitor may see). */}
      <Route path="/status/:slug" element={<PublicStatusPage />} />
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
