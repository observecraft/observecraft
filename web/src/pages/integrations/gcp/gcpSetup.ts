// What the add-service-account flows hand to Google Cloud: gcloud commands (or Terraform) creating a
// service account in the first project, granting it read roles on every project, and letting this
// install's service account impersonate it (Service Account Token Creator): no key leaves Google.

export const SERVICE_ACCOUNT = 'observecraft-integration';
/** Read roles the crawler needs: Cloud Monitoring and listing projects. */
export const ROLES = ['roles/monitoring.viewer', 'roles/browser'];

export const validProjectId = (p: string) => /^[a-z][a-z0-9-]{4,28}[a-z0-9]$/.test(p.trim());
export const serviceAccountEmail = (project: string) => `${SERVICE_ACCOUNT}@${project}.iam.gserviceaccount.com`;

interface Input {
  projects: string[];
  /** This install's service account. */
  principal: string;
}

export function gcloudCommands({ projects, principal }: Input): string {
  const home = projects[0] ?? '<PROJECT_ID>';
  const sa = serviceAccountEmail(home);
  return [
    `gcloud iam service-accounts create ${SERVICE_ACCOUNT} --project ${home} --display-name "ObserveCraft integration"`,
    ...(projects.length ? projects : [home]).flatMap((p) => ROLES.map((role) => `gcloud projects add-iam-policy-binding ${p} --member serviceAccount:${sa} --role ${role} --condition None`)),
    `gcloud iam service-accounts add-iam-policy-binding ${sa} --project ${home} --member serviceAccount:${principal} --role roles/iam.serviceAccountTokenCreator`,
  ].join('\n');
}

const str = (s: string) => JSON.stringify(s);

export function terraformConfig({ projects, principal, apiUrl }: Input & { apiUrl: string }): string {
  const all = projects.length ? projects : ['<PROJECT_ID>'];
  return `terraform {
  required_providers {
    google = {
      source = "hashicorp/google"
    }
    datadog = {
      source = "DataDog/datadog"
    }
  }
}

provider "google" {}

# Keys from DD_API_KEY (an ObserveCraft ingest key) and DD_APP_KEY (an application key).
provider "datadog" {
  api_url = ${str(apiUrl)}
}

resource "google_service_account" "observecraft" {
  project      = ${str(all[0])}
  account_id   = ${str(SERVICE_ACCOUNT)}
  display_name = "ObserveCraft integration"
}

locals {
  grants = { for pair in setproduct(toset([${all.map(str).join(', ')}]), toset([${ROLES.map(str).join(', ')}])) : "\${pair[0]} \${pair[1]}" => pair }
}

resource "google_project_iam_member" "observecraft" {
  for_each = local.grants
  project  = each.value[0]
  role     = each.value[1]
  member   = "serviceAccount:\${google_service_account.observecraft.email}"
}

# This install impersonates the service account: no key is created.
resource "google_service_account_iam_member" "observecraft_impersonation" {
  service_account_id = google_service_account.observecraft.name
  role               = "roles/iam.serviceAccountTokenCreator"
  member             = ${str(`serviceAccount:${principal}`)}
}

resource "datadog_integration_gcp_sts" "this" {
  client_email = google_service_account.observecraft.email

  depends_on = [google_project_iam_member.observecraft, google_service_account_iam_member.observecraft_impersonation]
}
`;
}
