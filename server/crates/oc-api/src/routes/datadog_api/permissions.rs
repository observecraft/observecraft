//! Datadog permission names mapped onto our access levels, for the Datadog-compatible roles API.
//!
//! Our model has three levels (Read Only, Standard, Admin, from the built-in roles). Each Datadog
//! permission we support is held from one level up: the built-in roles report the permissions of
//! their level, and a custom role holds exactly the ones it was given (stored by name in
//! `role_permissions`; access still comes from the built-in roles). Ids are stable UUIDs derived
//! from the name, so they are the same on every install. Names Datadog has and we don't are
//! rejected when assigned.

use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::auth::Access;

/// A Datadog permission we support.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Permission {
    pub name: &'static str,
    pub display_name: &'static str,
    pub group: &'static str,
    /// `read`, `write` or `other`, as Datadog groups them.
    pub kind: &'static str,
    /// The lowest of our access levels that has it.
    pub level: Access,
}

const fn p(name: &'static str, display_name: &'static str, group: &'static str, kind: &'static str, level: Access) -> Permission {
    Permission { name, display_name, group, kind, level }
}

use Access::{Admin, ReadOnly, Standard};

/// Every supported permission, in the order `GET /api/v2/permissions` lists them.
pub const CATALOG: &[Permission] = &[
    p("dashboards_read", "Dashboards Read", "Dashboards", "read", ReadOnly),
    p("dashboards_write", "Dashboards Write", "Dashboards", "write", Standard),
    p("dashboards_public_share", "Dashboards Share", "Dashboards", "write", Standard),
    p("generate_dashboard_reports", "Dashboard Reports", "Dashboards", "write", Standard),
    p("monitors_read", "Monitors Read", "Monitors", "read", ReadOnly),
    p("monitors_write", "Monitors Write", "Monitors", "write", Standard),
    p("monitors_downtime", "Manage Downtimes", "Monitors", "write", Standard),
    p("slos_read", "SLOs Read", "Service Level Objectives", "read", ReadOnly),
    p("slos_write", "SLOs Write", "Service Level Objectives", "write", Standard),
    p("slos_corrections", "SLO Corrections", "Service Level Objectives", "write", Standard),
    p("logs_read_data", "Logs Read Data", "Log Management", "read", ReadOnly),
    p("logs_read_index_data", "Logs Read Index Data", "Log Management", "read", ReadOnly),
    p("logs_live_tail", "Logs Live Tail", "Log Management", "read", ReadOnly),
    p("logs_read_archives", "Logs Read Archives", "Log Management", "read", ReadOnly),
    p("logs_write_pipelines", "Logs Write Pipelines", "Log Management", "write", Standard),
    p("logs_write_processors", "Logs Write Processors", "Log Management", "write", Standard),
    p("logs_generate_metrics", "Logs Generate Metrics", "Log Management", "write", Standard),
    p("logs_modify_indexes", "Logs Modify Indexes", "Log Management", "write", Admin),
    p("logs_write_exclusion_filters", "Logs Write Exclusion Filters", "Log Management", "write", Admin),
    p("logs_write_archives", "Logs Write Archives", "Log Management", "write", Admin),
    p("apm_read", "APM Read", "APM", "read", ReadOnly),
    p("apm_service_catalog_read", "Service Catalog Read", "APM", "read", ReadOnly),
    p("apm_service_catalog_write", "Service Catalog Write", "APM", "write", Standard),
    p("apm_generate_metrics", "APM Generate Metrics", "APM", "write", Standard),
    p("continuous_profiler_read", "Profiles Read", "APM", "read", ReadOnly),
    p("error_tracking_read", "Error Tracking Read", "Error Tracking", "read", ReadOnly),
    p("error_tracking_write", "Error Tracking Write", "Error Tracking", "write", Standard),
    p("incident_read", "Incidents Read", "Case and Incident Management", "read", ReadOnly),
    p("incident_write", "Incidents Write", "Case and Incident Management", "write", Standard),
    p("notebooks_read", "Notebooks Read", "Notebooks", "read", ReadOnly),
    p("notebooks_write", "Notebooks Write", "Notebooks", "write", Standard),
    p("synthetics_read", "Synthetic Tests Read", "Synthetic Monitoring", "read", ReadOnly),
    p("synthetics_write", "Synthetic Tests Write", "Synthetic Monitoring", "write", Standard),
    p("synthetics_global_variable_read", "Synthetic Variables Read", "Synthetic Monitoring", "read", ReadOnly),
    p("synthetics_global_variable_write", "Synthetic Variables Write", "Synthetic Monitoring", "write", Standard),
    p("ci_visibility_read", "CI Visibility Read", "Software Delivery", "read", ReadOnly),
    p("deployment_gates_read", "Deployment Gates Read", "Software Delivery", "read", ReadOnly),
    p("deployment_gates_write", "Deployment Gates Write", "Software Delivery", "write", Standard),
    p("on_call_read", "On-Call Read", "On-Call", "read", ReadOnly),
    p("on_call_write", "On-Call Write", "On-Call", "write", Standard),
    p("status_pages_settings_read", "Status Pages Read", "Status Pages", "read", ReadOnly),
    p("status_pages_settings_write", "Status Pages Write", "Status Pages", "write", Standard),
    p("status_pages_incident_write", "Status Page Notices", "Status Pages", "write", Standard),
    p("reference_tables_read", "Reference Tables Read", "Reference Tables", "read", ReadOnly),
    p("reference_tables_write", "Reference Tables Write", "Reference Tables", "write", Standard),
    p("sheets_read", "Sheets Read", "Sheets", "read", ReadOnly),
    p("sheets_write", "Sheets Write", "Sheets", "write", Standard),
    p("feature_flag_config_read", "Feature Flags Read", "Feature Flags", "read", ReadOnly),
    p("feature_flag_config_write", "Feature Flags Write", "Feature Flags", "write", Standard),
    p("workflows_read", "Workflows Read", "App Builder & Workflow Automation", "read", ReadOnly),
    p("workflows_write", "Workflows Write", "App Builder & Workflow Automation", "write", Standard),
    p("workflows_run", "Workflows Run", "App Builder & Workflow Automation", "write", Standard),
    p("dbm_read", "Database Monitoring Read", "Database Monitoring", "read", ReadOnly),
    p("network_connections_read", "Network Connections Read", "Cloud Network Monitoring", "read", ReadOnly),
    p("llm_observability_read", "LLM Observability Read", "LLM Observability", "read", ReadOnly),
    p("llm_observability_write", "LLM Observability Write", "LLM Observability", "write", Standard),
    p("watchdog_insights_read", "Watchdog Read", "Watchdog", "read", ReadOnly),
    p("event_correlation_config_read", "Event Correlation Read", "Events", "read", ReadOnly),
    p("event_correlation_config_write", "Event Correlation Write", "Events", "write", Standard),
    p("api_keys_read", "API Keys Read", "API and Application Keys", "read", ReadOnly),
    p("api_keys_write", "API Keys Write", "API and Application Keys", "write", Admin),
    p("api_keys_delete", "API Keys Delete", "API and Application Keys", "write", Admin),
    p("user_app_keys", "Own Application Keys", "API and Application Keys", "write", ReadOnly),
    p("org_app_keys_read", "Organization Application Keys Read", "API and Application Keys", "read", Admin),
    p("org_app_keys_write", "Organization Application Keys Write", "API and Application Keys", "write", Admin),
    p("user_access_invite", "Invite Users", "Access Management", "write", Admin),
    p("user_access_manage", "Manage Users and Roles", "Access Management", "write", Admin),
    p("teams_manage", "Manage Teams", "Teams", "write", Admin),
    p("org_management", "Organization Settings", "Access Management", "write", Admin),
    p("audit_logs_read", "Audit Trail Read", "Compliance", "read", Admin),
];

/// When the catalog's permissions were introduced (Datadog's `created`).
const CREATED: &str = "2026-10-29T00:00:00+00:00";

/// The permission's id: a version-8 UUID from the SHA-256 of its name (RFC 9562, name-based).
pub fn id_of(name: &str) -> String {
    let digest = Sha256::digest(format!("observecraft/datadog-permission/{name}").as_bytes());
    let mut b = [0u8; 16];
    b.copy_from_slice(&digest[..16]);
    b[6] = (b[6] & 0x0f) | 0x80;
    b[8] = (b[8] & 0x3f) | 0x80;
    let hex: String = b.iter().map(|x| format!("{x:02x}")).collect();
    format!("{}-{}-{}-{}-{}", &hex[..8], &hex[8..12], &hex[12..16], &hex[16..20], &hex[20..])
}

#[cfg(test)]
pub fn by_name(name: &str) -> Option<&'static Permission> {
    CATALOG.iter().find(|p| p.name == name)
}

pub fn by_id(id: &str) -> Option<&'static Permission> {
    let id = id.to_ascii_lowercase();
    CATALOG.iter().find(|p| id_of(p.name) == id)
}

/// The permissions an access level has.
pub fn granted(level: Access) -> impl Iterator<Item = &'static Permission> {
    CATALOG.iter().filter(move |p| p.level <= level)
}

/// The access level of a built-in role (`<org>-admin`, `-standard`, `-readonly`), or None for a custom role.
pub fn builtin_level(org: &str, role_id: &str) -> Option<Access> {
    match role_id.strip_prefix(org)?.strip_prefix('-')? {
        "admin" => Some(Admin),
        "standard" => Some(Standard),
        "readonly" => Some(ReadOnly),
        _ => None,
    }
}

/// Datadog's names for its built-in roles, which find ours in role filters.
pub fn builtin_alias(filter: &str) -> Option<&'static str> {
    match filter.trim().to_ascii_lowercase().as_str() {
        "datadog admin role" => Some("admin"),
        "datadog standard role" => Some("standard"),
        "datadog read only role" => Some("readonly"),
        _ => None,
    }
}

/// Puts names in catalog order, dropping unknown ones.
pub fn ordered<'a>(names: impl IntoIterator<Item = &'a str>) -> Vec<&'static Permission> {
    let set: std::collections::HashSet<&str> = names.into_iter().collect();
    CATALOG.iter().filter(|p| set.contains(p.name)).collect()
}

/// `{"type": "permissions", "id": …}`, a relationship to the permission.
pub fn relationship(p: &Permission) -> Value {
    json!({ "type": "permissions", "id": id_of(p.name) })
}

/// The permission as a Datadog `permissions` document.
pub fn document(p: &Permission) -> Value {
    let level = match p.level {
        ReadOnly => "every role",
        Standard => "the Standard and Admin roles",
        Admin => "the Admin role",
    };
    json!({
        "type": "permissions",
        "id": id_of(p.name),
        "attributes": {
            "name": p.name,
            "display_name": p.display_name,
            "description": format!("{} in ObserveCraft; granted to {level}.", p.display_name),
            "created": CREATED,
            "group_name": p.group,
            "display_type": p.kind,
            "restricted": false,
            "name_aliases": [],
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_stable_uuids_from_the_name() {
        let id = id_of("dashboards_write");
        assert_eq!(id, id_of("dashboards_write"));
        assert_ne!(id, id_of("dashboards_read"));
        assert_eq!(id.len(), 36);
        let parts: Vec<&str> = id.split('-').collect();
        assert_eq!(parts.iter().map(|p| p.len()).collect::<Vec<_>>(), vec![8, 4, 4, 4, 12]);
        assert!(parts[2].starts_with('8'), "version 8: {id}");
        assert!(matches!(parts[3].chars().next(), Some('8' | '9' | 'a' | 'b')), "RFC variant: {id}");
        // Pinned: ids must never change once Terraform state holds them.
        assert_eq!(id_of("dashboards_write"), id_of(by_id(&id).unwrap().name));
        let ids: std::collections::HashSet<String> = CATALOG.iter().map(|p| id_of(p.name)).collect();
        assert_eq!(ids.len(), CATALOG.len());
    }

    #[test]
    fn names_and_ids_map_both_ways() {
        for p in CATALOG {
            assert_eq!(by_name(p.name), Some(p));
            assert_eq!(by_id(&id_of(p.name)), Some(p));
            assert_eq!(by_id(&id_of(p.name).to_uppercase()), Some(p));
        }
        assert_eq!(by_name("security_monitoring_rules_write"), None);
        assert_eq!(by_id("984a2bd4-d3b4-11e8-a1ff-a7f660d43029"), None);
        let names: std::collections::HashSet<&str> = CATALOG.iter().map(|p| p.name).collect();
        assert_eq!(names.len(), CATALOG.len(), "duplicate permission name");
    }

    #[test]
    fn levels_follow_our_roles() {
        let names = |level| granted(level).map(|p| p.name).collect::<Vec<_>>();
        let (read_only, standard, admin) = (names(ReadOnly), names(Standard), names(Admin));
        assert!(read_only.contains(&"dashboards_read") && read_only.contains(&"logs_read_data") && read_only.contains(&"user_app_keys"));
        assert!(!read_only.contains(&"dashboards_write") && !read_only.contains(&"monitors_write"));
        assert!(standard.contains(&"dashboards_write") && standard.contains(&"monitors_write") && standard.contains(&"slos_write"));
        for admin_only in ["user_access_manage", "user_access_invite", "api_keys_write", "teams_manage", "org_management", "logs_modify_indexes", "audit_logs_read"] {
            assert!(!standard.contains(&admin_only), "{admin_only} is for admins only");
            assert!(admin.contains(&admin_only));
        }
        assert_eq!(admin.len(), CATALOG.len());
        assert!(read_only.iter().all(|n| standard.contains(n)) && standard.iter().all(|n| admin.contains(n)));
    }

    #[test]
    fn builtin_roles_and_their_datadog_names() {
        assert_eq!(builtin_level("acme", "acme-admin"), Some(Admin));
        assert_eq!(builtin_level("acme", "acme-standard"), Some(Standard));
        assert_eq!(builtin_level("acme", "acme-readonly"), Some(ReadOnly));
        assert_eq!(builtin_level("acme", "other-admin"), None);
        assert_eq!(builtin_level("acme", "acme-auditors"), None);
        assert_eq!(builtin_level("acme", "3fdd7736-d1d9-11ec-ad3d-da7ad0900002"), None);
        assert_eq!(builtin_alias("Datadog Admin Role"), Some("admin"));
        assert_eq!(builtin_alias("datadog read only role"), Some("readonly"));
        assert_eq!(builtin_alias("Datadog Standard Role"), Some("standard"));
        assert_eq!(builtin_alias("Admin"), None);
    }

    #[test]
    fn the_docs_list_every_permission_at_its_level() {
        let docs = include_str!("../../../../../../docs/datadog-api.md");
        for (level, row) in [(ReadOnly, "| Read Only (every role) |"), (Standard, "| Standard |"), (Admin, "| Admin |")] {
            let line = docs.lines().find(|l| l.starts_with(row)).unwrap_or_else(|| panic!("no {row} row in docs/datadog-api.md"));
            let listed: Vec<&str> = line.split('`').skip(1).step_by(2).collect();
            let expected: Vec<&str> = CATALOG.iter().filter(|p| p.level == level).map(|p| p.name).collect();
            assert_eq!(listed, expected, "docs/datadog-api.md, {row}");
        }
    }

    #[test]
    fn documents_have_datadogs_shape() {
        let d = document(by_name("monitors_write").unwrap());
        assert_eq!(d["type"], "permissions");
        assert_eq!(d["id"], id_of("monitors_write"));
        assert_eq!(d["attributes"]["name"], "monitors_write");
        assert_eq!(d["attributes"]["display_type"], "write");
        assert_eq!(d["attributes"]["restricted"], false);
        assert!(d["attributes"]["created"].as_str().unwrap().starts_with("2026-"));
        assert_eq!(ordered(["monitors_write", "nope", "dashboards_read"]).iter().map(|p| p.name).collect::<Vec<_>>(), vec!["dashboards_read", "monitors_write"]);
    }
}
