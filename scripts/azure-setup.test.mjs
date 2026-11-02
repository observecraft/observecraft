import assert from 'node:assert/strict';
import test from 'node:test';
import { cliCommand, terraformConfig, validGuid } from '../web/src/pages/integrations/azure/azureSetup.ts';

const SUBS = ['00000000-0000-0000-0000-000000000001', '00000000-0000-0000-0000-000000000002'];

test('the CLI creates a service principal with Monitoring Reader on every subscription', () => {
  assert.equal(
    cliCommand(SUBS),
    'az ad sp create-for-rbac --name observecraft-integration --role "Monitoring Reader" --scopes /subscriptions/00000000-0000-0000-0000-000000000001 /subscriptions/00000000-0000-0000-0000-000000000002',
  );
  assert.match(cliCommand([]), /--scopes \/subscriptions\/<SUBSCRIPTION_ID>$/);
});

test('the Terraform configuration creates the app registration, its role assignments and the integration', () => {
  const hcl = terraformConfig({ subscriptions: SUBS, apiUrl: 'https://observecraft.example.com', hostFilters: 'env:prod' });
  assert.match(hcl, /source = "hashicorp\/azuread"/);
  assert.match(hcl, /source = "hashicorp\/azurerm"/);
  assert.match(hcl, /api_url = "https:\/\/observecraft\.example\.com"/);
  assert.match(hcl, /resource "azuread_application" "observecraft"/);
  assert.match(hcl, /resource "azuread_application_password" "observecraft"/);
  assert.match(hcl, /for_each\s+= toset\(\["00000000-0000-0000-0000-000000000001", "00000000-0000-0000-0000-000000000002"\]\)/);
  assert.match(hcl, /role_definition_name = "Monitoring Reader"/);
  assert.match(hcl, /resource "datadog_integration_azure" "this"/);
  assert.match(hcl, /client_secret\s+= azuread_application_password\.observecraft\.value/);
  assert.match(hcl, /host_filters\s+= "env:prod"/);
  assert.doesNotMatch(hcl, /api_key|app_key/);
});

test('tenant, client and subscription ids are GUIDs', () => {
  assert.equal(validGuid('00000000-0000-0000-0000-000000000001'), true);
  assert.equal(validGuid('contoso.onmicrosoft.com'), false);
});
