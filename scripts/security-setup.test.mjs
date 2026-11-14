import assert from 'node:assert/strict';
import test from 'node:test';
import { agentCoverage, appsecEnv, cloudAgentEnv, cloudHelmValues, workloadAgentEnv, workloadHelmValues } from '../web/src/pages/security/setupSnippets.ts';

test('App & API Protection: tracer variables per language, IAST only where the tracer has it', () => {
  assert.deepEqual(appsecEnv('java', { threats: true, sca: true, iast: true }), ['DD_APPSEC_ENABLED=true', 'DD_APPSEC_SCA_ENABLED=true', 'DD_IAST_ENABLED=true']);
  assert.deepEqual(appsecEnv('go', { threats: true, sca: true, iast: true }), ['DD_APPSEC_ENABLED=true', 'DD_APPSEC_SCA_ENABLED=true']);
  assert.deepEqual(appsecEnv('python', { threats: false, sca: true, iast: false }), ['DD_APPSEC_SCA_ENABLED=true']);
});

test('Cloud Security: Agent variables and Helm values for SBOMs and compliance', () => {
  const env = cloudAgentEnv();
  for (const v of ['DD_SBOM_ENABLED=true', 'DD_SBOM_HOST_ENABLED=true', 'DD_SBOM_CONTAINER_IMAGE_ENABLED=true', 'DD_CONTAINER_IMAGE_ENABLED=true', 'DD_COMPLIANCE_CONFIG_ENABLED=true'])
    assert.ok(env.includes(v), v);
  const helm = cloudHelmValues();
  assert.match(helm, /sbom:\n\s+containerImage:\n\s+enabled: true\n\s+host:\n\s+enabled: true/);
  assert.match(helm, /securityAgent:\n\s+compliance:\n\s+enabled: true/);
});

test('Workload Protection: runtime security on the Agent', () => {
  assert.match(workloadHelmValues(), /securityAgent:\n\s+runtime:\n\s+enabled: true\n\s+fimEnabled: true/);
  assert.ok(workloadAgentEnv().includes('DD_RUNTIME_SECURITY_CONFIG_ENABLED=true'));
});

test('agent coverage counts what each Agent runs', () => {
  const a = (cws, sbom, cspm) => ({ isCwsEnabled: cws, isCsmVmHostsEnabled: sbom, isCsmVmContainersEnabled: false, isCspmEnabled: cspm });
  assert.deepEqual(agentCoverage([a(true, true, false), a(false, true, true), a(false, false, false)]), { total: 3, runtime: 1, sbom: 2, compliance: 1 });
  assert.deepEqual(agentCoverage([]), { total: 0, runtime: 0, sbom: 0, compliance: 0 });
});
