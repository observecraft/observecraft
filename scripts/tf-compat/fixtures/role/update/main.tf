# Renamed, one permission dropped and one added.
data "datadog_permissions" "all" {}

data "datadog_role" "read_only" {
  filter = "Datadog Read Only Role"
}

data "datadog_roles" "all" {}

resource "datadog_role" "release_managers" {
  name = "tf-compat release captains"

  permission {
    id = data.datadog_permissions.all.permissions["monitors_write"]
  }
  permission {
    id = data.datadog_permissions.all.permissions["slos_write"]
  }
}

output "role" {
  value = {
    id          = datadog_role.release_managers.id
    permissions = sort([for p in datadog_role.release_managers.permission : p.name])
    read_only   = data.datadog_role.read_only.name
  }
}
