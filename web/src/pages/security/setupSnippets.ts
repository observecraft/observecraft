// What the security products' setup pages tell to configure: tracer and Agent variables, Helm values,
// and what the Agents report running. Pure, tested in scripts/security-setup.test.mjs.

/** Tracer languages with runtime code analysis (IAST). */
export const IAST_LANGUAGES = ['java', 'python', 'nodejs', 'dotnet'];

/** App & API Protection's tracer variables for a service's language. */
export function appsecEnv(language: string, on: { threats: boolean; sca: boolean; iast: boolean }): string[] {
  const out: string[] = [];
  if (on.threats) out.push('DD_APPSEC_ENABLED=true');
  if (on.sca) out.push('DD_APPSEC_SCA_ENABLED=true');
  if (on.iast && IAST_LANGUAGES.includes(language)) out.push('DD_IAST_ENABLED=true');
  return out;
}

/** Cloud Security on a host Agent: SBOMs of the host and its container images, compliance checks. */
export function cloudAgentEnv(): string[] {
  return [
    'DD_SBOM_ENABLED=true',
    'DD_SBOM_HOST_ENABLED=true',
    'DD_SBOM_CONTAINER_IMAGE_ENABLED=true',
    'DD_CONTAINER_IMAGE_ENABLED=true',
    'DD_COMPLIANCE_CONFIG_ENABLED=true',
    'DD_COMPLIANCE_CONFIG_HOST_BENCHMARKS_ENABLED=true',
  ];
}

/** Cloud Security in the datadog Helm chart's values. */
export function cloudHelmValues(): string {
  return ['datadog:', '  sbom:', '    containerImage:', '      enabled: true', '    host:', '      enabled: true', '  securityAgent:', '    compliance:', '      enabled: true', ''].join('\n');
}

/** Workload Protection (runtime security) on a host Agent: the security agent and system-probe. */
export function workloadAgentEnv(): string[] {
  return ['DD_RUNTIME_SECURITY_CONFIG_ENABLED=true', 'DD_RUNTIME_SECURITY_CONFIG_REMOTE_CONFIGURATION_ENABLED=true', 'DD_RUNTIME_SECURITY_CONFIG_FIM_ENABLED=true'];
}

/** Workload Protection in the datadog Helm chart's values. */
export function workloadHelmValues(): string {
  return ['datadog:', '  securityAgent:', '    runtime:', '      enabled: true', '      fimEnabled: true', ''].join('\n');
}

export interface AgentFlags {
  isCwsEnabled: boolean;
  isCsmVmHostsEnabled: boolean;
  isCsmVmContainersEnabled: boolean;
  isCspmEnabled: boolean;
}

/** Of the Agents, how many run runtime security, send SBOMs (hosts or images), run compliance checks. */
export function agentCoverage(agents: AgentFlags[]): { total: number; runtime: number; sbom: number; compliance: number } {
  return {
    total: agents.length,
    runtime: agents.filter((a) => a.isCwsEnabled).length,
    sbom: agents.filter((a) => a.isCsmVmHostsEnabled || a.isCsmVmContainersEnabled).length,
    compliance: agents.filter((a) => a.isCspmEnabled).length,
  };
}
