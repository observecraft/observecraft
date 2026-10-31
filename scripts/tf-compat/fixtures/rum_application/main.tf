# A browser RUM application (POST /api/v2/rum/applications) and the application data source.
resource "datadog_rum_application" "storefront" {
  name = "tf-compat-storefront-web"
  type = "browser"
}

data "datadog_rum_application" "storefront" {
  id = datadog_rum_application.storefront.id
}

output "rum_application" {
  value = {
    name                = data.datadog_rum_application.storefront.name
    client_token_prefix = substr(datadog_rum_application.storefront.client_token, 0, 3)
  }
}
