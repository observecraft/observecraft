# A custom role with permissions picked by name from datadog_permissions, and the built-in roles
# found by Datadog's names (GET /api/v2/permissions, /api/v2/roles).
data "datadog_permissions" "all" {}

data "datadog_role" "read_only" {
  filter = "Datadog Read Only Role"
}

data "datadog_roles" "all" {}

resource "datadog_role" "release_managers" {
  name = "tf-compat release managers"

  permission {
    id = data.datadog_permissions.all.permissions["monitors_write"]
  }
  permission {
    id = data.datadog_permissions.all.permissions["monitors_downtime"]
  }
  permission {
    id = data.datadog_permissions.all.permissions["dashboards_write"]
  }
}

output "role" {
  value = {
    id          = datadog_role.release_managers.id
    permissions = sort([for p in datadog_role.release_managers.permission : p.name])
    read_only   = data.datadog_role.read_only.name
  }

  precondition {
    condition     = length(data.datadog_roles.all.roles) >= 3 && data.datadog_role.read_only.id != ""
    error_message = "datadog_roles should list the built-in roles"
  }
}
