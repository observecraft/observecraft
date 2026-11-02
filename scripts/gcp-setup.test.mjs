import assert from 'node:assert/strict';
import test from 'node:test';
import { gcloudCommands, serviceAccountEmail, terraformConfig, validProjectId } from '../web/src/pages/integrations/gcp/gcpSetup.ts';

const PRINCIPAL = 'observecraft@example-host.iam.gserviceaccount.com';

test('the service account is named in the first project', () => {
  assert.equal(serviceAccountEmail('example-storefront'), 'observecraft-integration@example-storefront.iam.gserviceaccount.com');
  assert.equal(validProjectId('example-storefront'), true);
  assert.equal(validProjectId('Ex'), false);
});

test('gcloud creates the service account, grants read roles on every project and lets this install impersonate it', () => {
  const script = gcloudCommands({ projects: ['example-storefront', 'example-catalog'], principal: PRINCIPAL });
  assert.match(script, /gcloud iam service-accounts create observecraft-integration --project example-storefront/);
  assert.match(
    script,
    /gcloud projects add-iam-policy-binding example-catalog --member serviceAccount:observecraft-integration@example-storefront\.iam\.gserviceaccount\.com --role roles\/monitoring\.viewer/,
  );
  assert.match(script, /--role roles\/browser/);
  assert.match(
    script,
    /gcloud iam service-accounts add-iam-policy-binding observecraft-integration@example-storefront\.iam\.gserviceaccount\.com --project example-storefront --member serviceAccount:observecraft@example-host\.iam\.gserviceaccount\.com --role roles\/iam\.serviceAccountTokenCreator/,
  );
});

test('Terraform creates the same and the integration against this install', () => {
  const hcl = terraformConfig({ projects: ['example-storefront', 'example-catalog'], principal: PRINCIPAL, apiUrl: 'https://observecraft.example.com' });
  assert.match(hcl, /source = "hashicorp\/google"/);
  assert.match(hcl, /api_url = "https:\/\/observecraft\.example\.com"/);
  assert.match(hcl, /resource "google_service_account" "observecraft"/);
  assert.match(hcl, /setproduct\(toset\(\["example-storefront", "example-catalog"\]\), toset\(\["roles\/monitoring\.viewer", "roles\/browser"\]\)\)/);
  assert.match(hcl, /role\s+= "roles\/iam\.serviceAccountTokenCreator"/);
  assert.match(hcl, /member\s+= "serviceAccount:observecraft@example-host\.iam\.gserviceaccount\.com"/);
  assert.match(hcl, /resource "datadog_integration_gcp_sts" "this"/);
  assert.match(hcl, /client_email = google_service_account\.observecraft\.email/);
});
