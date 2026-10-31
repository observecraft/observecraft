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
import { MetricsCatalog } from './pages/metrics/MetricsCatalog';
import { MetricsOverview } from './pages/metrics/MetricsOverview';
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
import { InstrumentPage } from './pages/apm/InstrumentPage';
import { CodeSecurityLayout } from './pages/codeSecurity/CodeSecurityLayout';
import { SummaryPage as CodeSecuritySummary } from './pages/codeSecurity/SummaryPage';
import { VulnerabilitiesPage as CodeSecurityVulnerabilities } from './pages/codeSecurity/VulnerabilitiesPage';
import { DetectionCoveragePage, InventoryPage as CodeSecurityInventory, PrSecurityPage, RepositoriesPage as CodeSecurityRepositories } from './pages/codeSecurity/CoveragePages';
import { SettingsPage as CodeSecuritySettings } from './pages/codeSecurity/SettingsPage';
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
import { RumLayout } from './pages/rum/RumLayout';
import { RumSummaryPage } from './pages/rum/RumSummaryPage';
import { RumOperationsPage } from './pages/rum/RumOperationsPage';
import { RumExplorerPage } from './pages/rum/RumExplorerPage';
import { RumApplicationsPage } from './pages/rum/RumApplicationsPage';
import { RumApplicationPage } from './pages/rum/RumApplicationPage';
import { ProductAnalyticsLayout, ProductAnalyticsPage } from './pages/rum/ProductAnalyticsPage';
import { SessionReplayLayout } from './pages/rum/replay/SessionReplayLayout';
import { SecurityLayout, SiemLayout } from './pages/security/SecurityLayout';
import { SecurityOverviewPage } from './pages/security/OverviewPage';
import { InboxPage } from './pages/security/InboxPage';
import { AppsecSignalsPage, SignalsPage, SiemSignalsPage } from './pages/security/SignalsPage';
import { FeedPage } from './pages/security/FeedPage';
import { SiemOverviewPage } from './pages/security/SiemOverviewPage';
import { ContentPacksPage } from './pages/security/ContentPacksPage';
import { RuleEditorPage, RulesPage } from './pages/security/RulesPage';
import { SiemSettingsPage } from './pages/security/SiemSettingsPage';
import { CloudSecurityLayout } from './pages/security/cloud/CloudSecurityLayout';
import { AppsecLayout } from './pages/security/appsec/AppsecLayout';
import { WorkloadLayout } from './pages/security/workload/WorkloadLayout';
import { WorkloadOverviewPage } from './pages/security/workload/OverviewPage';
import { WorkloadSignalsPage } from './pages/security/workload/SignalsPage';
import { AgentEventsPage } from './pages/security/workload/InvestigatePage';
import { WorkloadFindingsPage } from './pages/security/workload/FindingsPage';
import { CoveragePage, HostsContainersPage, ServerlessPage as WpServerlessPage } from './pages/security/workload/InventoryPages';
import { AgentConfigurationPage, FindingRulesPage, WorkloadDetectionRulesPage } from './pages/security/workload/PoliciesPages';
import { AiGuardLayout } from './pages/security/aiGuard/AiGuardLayout';
import { AiGuardInvestigatePage } from './pages/security/aiGuard/InvestigatePage';
import { AiGuardSignalsPage } from './pages/security/aiGuard/SignalsPage';
import { AiGuardDiscoverPage } from './pages/security/aiGuard/DiscoverPage';
import { AiGuardPlaygroundPage } from './pages/security/aiGuard/PlaygroundPage';
import { AiGuardSettingsPage } from './pages/security/aiGuard/SettingsPage';
import { AppsecOverviewPage } from './pages/security/appsec/OverviewPage';
import { ApiFindingsPage, CompliancePage } from './pages/security/appsec/FindingsPages';
import { AppsecUsersPage, AttackersPage, SecurityTracesPage } from './pages/security/appsec/InvestigatePages';
import { AppsecServicesPage, EndpointsPage } from './pages/security/appsec/InventoryPages';
import { AppsecRulesPage, BusinessLogicPage, DenylistPage, InAppWafPage, PasslistPage, SensitiveDataPage } from './pages/security/appsec/PoliciesPages';
import { CloudSummaryPage } from './pages/security/cloud/SummaryPage';
import { InfraVulnerabilitiesPage, MisconfigurationsPage } from './pages/security/cloud/FindingsPages';
import { CloudInboxPage, CveExplorerPage, PackagesPage, ResourcesPage } from './pages/security/cloud/InventoryPages';
import { RecordingsPage } from './pages/rum/replay/RecordingsPage';
import { ReplayPlayerPage } from './pages/rum/replay/ReplayPlayerPage';
import { PlaylistPage, PlaylistsPage } from './pages/rum/replay/PlaylistsPage';
import { HeatmapsPage } from './pages/rum/replay/HeatmapsPage';
import { SyntheticTestEditor } from './pages/synthetics/SyntheticTestEditor';
import { ApiTemplatesPage } from './pages/synthetics/NewTest';
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
import { ResourceChangesPage } from './pages/infrastructure/ResourceChangesPage';
import { ResourceCatalogPage } from './pages/infrastructure/ResourceCatalogPage';
import { ResourcesLayout } from './pages/infrastructure/ResourcesLayout';
import { CloudCostComingSoon, CloudCostLayout } from './pages/cloudCost/CloudCostLayout';
import { OverviewPage as CloudCostOverviewPage } from './pages/cloudCost/OverviewPage';
import { ContainersCostPage } from './pages/cloudCost/ContainersCostPage';
import { CostBoardPage } from './pages/cloudCost/CostBoardPage';
import { AiCostsPage } from './pages/cloudCost/AiCostsPage';
import { CostMapPage } from './pages/cloudCost/CostMapPage';
import { CostExplorerPage } from './pages/cloudCost/CostExplorerPage';
import { CostTagsPage } from './pages/cloudCost/CostTagsPage';
import { BudgetsPage } from './pages/cloudCost/BudgetsPage';
import { BudgetDetailPage } from './pages/cloudCost/BudgetDetailPage';
import { BudgetEditorPage } from './pages/cloudCost/BudgetEditorPage';
import { CommitmentsPage } from './pages/cloudCost/CommitmentsPage';
import { UnitCostsPage } from './pages/cloudCost/UnitCostsPage';
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
import { DataStreamsLayout } from './pages/dataStreams/DataStreamsLayout';
import { KafkaConsolePage } from './pages/dataStreams/KafkaConsolePage';
import { MapPage as DataStreamsMap } from './pages/dataStreams/MapPage';
import { MeasurePage as DataStreamsMeasure } from './pages/dataStreams/MeasurePage';
import { PipelinesPage as DataStreamsPipelines } from './pages/dataStreams/PipelinesPage';
import { SetupPage as DataStreamsSetup } from './pages/dataStreams/SetupPage';
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
              <Route path="endpoints" element={<Navigate to="/portal/catalog?section=endpoint&view=performance" replace />} />
              <Route path="instrument" element={<InstrumentPage />} />
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
              <Route path="/infrastructure/changes" element={<ResourceChangesPage />} />
              <Route path="/infrastructure/change-tracking" element={<ChangesPage />} />
              <Route path="/infrastructure/policies" element={<PoliciesPage />} />
              <Route path="/infrastructure/policies/new" element={<PolicyEditorPage />} />
              <Route path="/infrastructure/policies/:id" element={<PolicyDetailPage />} />
              <Route path="/infrastructure/policies/:id/edit" element={<PolicyEditorPage />} />
              <Route path="/cloudcraft" element={<CloudcraftPage />} />
            </Route>
            <Route path="/code-security" element={<CodeSecurityLayout />}>
              <Route index element={<CodeSecuritySummary />} />
              <Route path="vulnerabilities" element={<Navigate to="/code-security/vulnerabilities/libraries" replace />} />
              <Route path="vulnerabilities/:type" element={<CodeSecurityVulnerabilities />} />
              <Route path="repositories" element={<CodeSecurityRepositories />} />
              <Route path="detection-coverage" element={<Navigate to="/code-security/detection-coverage/cves" replace />} />
              <Route path="detection-coverage/:tab" element={<DetectionCoveragePage />} />
              <Route path="inventory" element={<Navigate to="/code-security/inventory/libraries" replace />} />
              <Route path="inventory/:tab" element={<CodeSecurityInventory />} />
              <Route path="pr-security" element={<PrSecurityPage />} />
              <Route path="settings" element={<CodeSecuritySettings />} />
            </Route>
            <Route path="/gpu-monitoring" element={<GpuMonitoringPage view="summary" />} />
            <Route path="/gpu-monitoring/explorer" element={<GpuMonitoringPage view="explorer" />} />
            <Route path="/serverless" element={<ServerlessPage />} />
            <Route path="/ddsql" element={<SqlEditorPage />} />
            <Route path="/data" element={<DatabaseMonitoringPage view="overview" />} />
            <Route path="/data/setup" element={<DatabaseMonitoringPage view="setup" />} />
            <Route path="/data/jobs" element={<DataJobsPage />} />
            <Route path="/data-streams" element={<DataStreamsLayout />}>
              <Route index element={<DataStreamsMap />} />
              <Route path="pipelines" element={<DataStreamsPipelines />} />
              <Route path="measure" element={<DataStreamsMeasure />} />
              <Route path="kafka" element={<KafkaConsolePage />} />
              <Route path="setup" element={<DataStreamsSetup />} />
            </Route>
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
            <Route path="/cloud-cost/budgets/new" element={<BudgetEditorPage />} />
            <Route path="/cloud-cost/budgets/:id/edit" element={<BudgetEditorPage />} />
            <Route path="/cloud-cost" element={<CloudCostLayout />}>
              <Route index element={<CloudCostOverviewPage />} />
              <Route path="containers" element={<ContainersCostPage />} />
              <Route path="gpu" element={<CostBoardPage key="gpu" tab="gpu" />} />
              <Route path="networking" element={<CostBoardPage key="networking" tab="networking" />} />
              <Route path="storage" element={<CostBoardPage key="storage" tab="storage" />} />
              <Route path="saas" element={<CostBoardPage key="saas" tab="saas" />} />
              <Route path="ai" element={<AiCostsPage />} />
              <Route path="map" element={<CostMapPage />} />
              <Route path="explorer" element={<CostExplorerPage />} />
              <Route path="tags" element={<CostTagsPage />} />
              <Route path="budgets" element={<BudgetsPage />} />
              <Route path="budgets/:id" element={<BudgetDetailPage />} />
              <Route path="commitments" element={<CommitmentsPage />} />
              <Route path="unit-costs" element={<UnitCostsPage />} />
              <Route path="*" element={<CloudCostComingSoon />} />
            </Route>
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
            <Route path="/rum" element={<RumLayout />}>
              <Route index element={<RumSummaryPage />} />
              <Route path="operations" element={<RumOperationsPage />} />
              <Route path="explorer" element={<RumExplorerPage />} />
            </Route>
            <Route path="/product-analytics" element={<ProductAnalyticsLayout />}>
              <Route index element={<ProductAnalyticsPage />} />
              {['users', 'devices', 'traffic', 'engagement', 'performance'].map((p) => (
                <Route key={p} path={p} element={<ProductAnalyticsPage />} />
              ))}
            </Route>
            <Route path="/security" element={<SecurityLayout />}>
              <Route index element={<SecurityOverviewPage />} />
              <Route path="inbox" element={<InboxPage />} />
              <Route path="signals" element={<SignalsPage />} />
              <Route path="feed" element={<FeedPage />} />
              <Route path="settings" element={<Navigate to="/security/siem/settings" replace />} />
            </Route>
            <Route path="/security/appsec" element={<AppsecLayout />}>
              <Route index element={<AppsecOverviewPage />} />
              <Route path="signals" element={<AppsecSignalsPage />} />
              <Route path="findings" element={<ApiFindingsPage />} />
              <Route path="findings/compliance" element={<CompliancePage />} />
              <Route path="investigate" element={<SecurityTracesPage />} />
              <Route path="investigate/users" element={<AppsecUsersPage />} />
              <Route path="investigate/attackers" element={<AttackersPage />} />
              <Route path="inventory" element={<EndpointsPage />} />
              <Route path="inventory/services" element={<AppsecServicesPage />} />
              <Route path="policies" element={<InAppWafPage />} />
              <Route path="policies/business-logic" element={<BusinessLogicPage />} />
              <Route path="policies/rules" element={<AppsecRulesPage />} />
              <Route path="policies/denylist" element={<DenylistPage />} />
              <Route path="policies/passlist" element={<PasslistPage />} />
              <Route path="policies/sensitive-data" element={<SensitiveDataPage />} />
            </Route>
            <Route path="/security/workload-protection" element={<WorkloadLayout />}>
              <Route index element={<WorkloadOverviewPage />} />
              <Route path="signals" element={<WorkloadSignalsPage />} />
              <Route path="investigate" element={<AgentEventsPage />} />
              <Route path="findings" element={<WorkloadFindingsPage />} />
              <Route path="inventory" element={<HostsContainersPage />} />
              <Route path="inventory/serverless" element={<WpServerlessPage />} />
              <Route path="inventory/coverage" element={<CoveragePage />} />
              <Route path="policies" element={<AgentConfigurationPage />} />
              <Route path="policies/detection-rules" element={<WorkloadDetectionRulesPage />} />
              <Route path="policies/finding-rules" element={<FindingRulesPage />} />
            </Route>
            <Route path="/security/ai-guard" element={<AiGuardLayout />}>
              <Route index element={<AiGuardInvestigatePage />} />
              <Route path="signals" element={<AiGuardSignalsPage />} />
              <Route path="discover" element={<AiGuardDiscoverPage />} />
              <Route path="playground" element={<AiGuardPlaygroundPage />} />
              <Route path="settings" element={<AiGuardSettingsPage />} />
            </Route>
            <Route path="/security/cloud" element={<CloudSecurityLayout />}>
              <Route index element={<CloudSummaryPage />} />
              <Route path="inbox" element={<CloudInboxPage />} />
              <Route path="findings" element={<MisconfigurationsPage />} />
              <Route path="vulnerabilities" element={<InfraVulnerabilitiesPage />} />
              <Route path="inventory" element={<ResourcesPage />} />
              <Route path="inventory/packages" element={<PackagesPage />} />
              <Route path="inventory/cves" element={<CveExplorerPage />} />
            </Route>
            <Route path="/security/siem" element={<SiemLayout />}>
              <Route index element={<SiemOverviewPage />} />
              <Route path="content-packs" element={<ContentPacksPage />} />
              <Route path="signals" element={<SiemSignalsPage />} />
              <Route path="rules" element={<RulesPage />} />
              <Route path="rules/:id" element={<RuleEditorPage />} />
              <Route path="settings" element={<SiemSettingsPage />} />
            </Route>
            <Route path="/rum/replay" element={<SessionReplayLayout />}>
              <Route index element={<Navigate to="sessions" replace />} />
              <Route path="sessions" element={<RecordingsPage />} />
              <Route path="playlists" element={<PlaylistsPage />} />
              <Route path="playlists/:id" element={<PlaylistPage />} />
              <Route path="heatmaps" element={<HeatmapsPage />} />
            </Route>
            <Route path="/rum/replay/sessions/:id" element={<ReplayPlayerPage />} />
            <Route path="/rum/applications" element={<RumApplicationsPage />} />
            <Route path="/rum/applications/:id" element={<RumApplicationPage />} />
            <Route path="/synthetics/create" element={<SyntheticTestEditor />} />
            <Route path="/synthetics/templates" element={<ApiTemplatesPage />} />
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
              <Route path="overview" element={<MetricsOverview />} />
              <Route path="catalog" element={<MetricsCatalog />} />
              <Route path="summary" element={<Navigate to="/metrics/catalog" replace />} />
              <Route path="volume" element={<Navigate to="/metrics/catalog" replace />} />
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
