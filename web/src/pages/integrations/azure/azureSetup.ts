// What the add-app-registration flows hand to Azure: the Azure CLI command creating a service principal
// with Monitoring Reader on the chosen subscriptions, and a Terraform configuration creating the app
// registration, its secret and role assignments, and the integration (Datadog's provider against this install).

export const validGuid = (s: string) => /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(s.trim());

export function cliCommand(subscriptions: string[]): string {
  const scopes = (subscriptions.length ? subscriptions : ['<SUBSCRIPTION_ID>']).map((s) => `/subscriptions/${s}`).join(' ');
  return `az ad sp create-for-rbac --name observecraft-integration --role "Monitoring Reader" --scopes ${scopes}`;
}

const str = (s: string) => JSON.stringify(s);

interface TerraformInput {
  subscriptions: string[];
  apiUrl: string;
  hostFilters: string;
}

/** Terraform: the app registration (azuread), Monitoring Reader on each subscription (azurerm), the integration (Datadog's provider). */
export function terraformConfig({ subscriptions, apiUrl, hostFilters }: TerraformInput): string {
  const subs = subscriptions.length ? subscriptions : ['<SUBSCRIPTION_ID>'];
  return `terraform {
  required_providers {
    azuread = {
      source = "hashicorp/azuread"
    }
    azurerm = {
      source = "hashicorp/azurerm"
    }
    datadog = {
      source = "DataDog/datadog"
    }
  }
}

provider "azuread" {}

provider "azurerm" {
  features {}
}

# Keys from DD_API_KEY (an ObserveCraft ingest key) and DD_APP_KEY (an application key).
provider "datadog" {
  api_url = ${str(apiUrl)}
}

data "azuread_client_config" "current" {}

resource "azuread_application" "observecraft" {
  display_name = "observecraft-integration"
}

resource "azuread_service_principal" "observecraft" {
  client_id = azuread_application.observecraft.client_id
}

resource "azuread_application_password" "observecraft" {
  application_id = azuread_application.observecraft.id
}

resource "azurerm_role_assignment" "observecraft" {
  for_each             = toset([${subs.map(str).join(', ')}])
  scope                = "/subscriptions/\${each.value}"
  role_definition_name = "Monitoring Reader"
  principal_id         = azuread_service_principal.observecraft.object_id
}

resource "datadog_integration_azure" "this" {
  tenant_name   = data.azuread_client_config.current.tenant_id
  client_id     = azuread_application.observecraft.client_id
  client_secret = azuread_application_password.observecraft.value
  host_filters  = ${str(hostFilters)}

  depends_on = [azurerm_role_assignment.observecraft]
}
`;
}
