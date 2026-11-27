//! The org's members as Vanta `user_account` resources: who has access to ObserveCraft, with their permission level,
//! status, MFA and how they sign in, for Vanta's access reviews and offboarding reminders (what Vanta reads of
//! Datadog's users). Invitations not yet accepted (`pending`) aren't accounts yet and aren't sent.

use serde_json::{json, Value};

use crate::auth::{access_from_roles, Access};
use crate::state::AppState;

/// A member as read for Vanta (times in epoch ms).
#[derive(sqlx::FromRow, Clone, Debug)]
pub struct Member {
    pub id: String,
    pub email: String,
    pub name: String,
    /// active | deactivated
    pub status: String,
    pub login_methods: Vec<String>,
    pub mfa_enabled: bool,
    pub created_at: i64,
    pub last_login_at: Option<i64>,
    pub modified_at: Option<i64>,
    pub role_ids: Vec<String>,
    pub role_names: Vec<String>,
}

fn rfc3339(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms).unwrap_or_default().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

/// A member as Vanta's `user_account` (`ui`: the install's public URL, where Vanta links to the members page).
pub fn user_account(org: &str, m: &Member, ui: &str) -> Value {
    let permission = match access_from_roles(org, &m.role_ids) {
        Access::Admin => "ADMIN",
        Access::Standard => "EDITOR",
        Access::ReadOnly => "BASE",
    };
    // Whoever can still sign in with a password does, whatever SSO they also have.
    let has = |method: &str| m.login_methods.iter().any(|l| l == method);
    let auth = if !has("password") && (has("saml") || has("oidc")) { "SSO" } else { "PASSWORD" };
    let deactivated = m.status == "deactivated";
    let name = if m.name.trim().is_empty() { &m.email } else { &m.name };
    let mut v = json!({
        "uniqueId": m.id,
        "displayName": name,
        "externalUrl": format!("{ui}/organization-settings/users"),
        "fullName": name,
        "accountName": m.email,
        "email": m.email,
        "permissionLevel": permission,
        "createdTimestamp": rfc3339(m.created_at),
        "status": if deactivated { "DEACTIVATED" } else { "ACTIVE" },
        "mfaEnabled": m.mfa_enabled,
        "mfaMethods": [if m.mfa_enabled { "OTP" } else { "DISABLED" }],
        "authMethod": auth,
    });
    if !m.role_names.is_empty() {
        v["roleDescription"] = json!(m.role_names.join(", "));
    }
    if let Some(at) = m.last_login_at {
        v["lastLoginTimestamp"] = json!(rfc3339(at));
    }
    if let (true, Some(at)) = (deactivated, m.modified_at) {
        v["deactivatedTimestamp"] = json!(rfc3339(at));
    }
    v
}

/// The org's members to send: active and deactivated, oldest first.
pub async fn user_accounts(st: &AppState, org: &str) -> Result<Vec<Value>, sqlx::Error> {
    let members: Vec<Member> = sqlx::query_as(
        "SELECT u.id::text AS id, u.email, u.name, u.status, u.login_methods, u.mfa_enabled, \
         (extract(epoch FROM u.created_at) * 1000)::bigint AS created_at, (extract(epoch FROM u.last_login_at) * 1000)::bigint AS last_login_at, \
         (extract(epoch FROM u.modified_at) * 1000)::bigint AS modified_at, \
         COALESCE(array_agg(r.id ORDER BY r.name) FILTER (WHERE r.id IS NOT NULL), '{}') AS role_ids, \
         COALESCE(array_agg(r.name ORDER BY r.name) FILTER (WHERE r.id IS NOT NULL), '{}') AS role_names \
         FROM users u LEFT JOIN user_roles ur ON ur.user_id = u.id LEFT JOIN roles r ON r.id = ur.role_id AND r.org_id = u.org_id \
         WHERE u.org_id = $1 AND u.status IN ('active', 'deactivated') GROUP BY u.id ORDER BY u.created_at, u.id",
    )
    .bind(org)
    .fetch_all(&st.postgres)
    .await?;
    Ok(members.iter().map(|m| user_account(org, m, &st.config.public_url)).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn member() -> Member {
        Member {
            id: "00000000-0000-4000-8000-0000000000a1".into(),
            email: "ana@example.com".into(),
            name: "Ana Example".into(),
            status: "active".into(),
            login_methods: vec!["saml".into()],
            mfa_enabled: true,
            created_at: 1_790_000_000_000,
            last_login_at: Some(1_795_000_000_000),
            modified_at: None,
            role_ids: vec!["example-org-admin".into()],
            role_names: vec!["Admin".into()],
        }
    }

    #[test]
    fn an_sso_admin_is_an_active_admin_account() {
        let v = user_account("example-org", &member(), "https://observecraft.example.com");
        assert_eq!(
            v,
            json!({
                "uniqueId": "00000000-0000-4000-8000-0000000000a1",
                "displayName": "Ana Example",
                "externalUrl": "https://observecraft.example.com/organization-settings/users",
                "fullName": "Ana Example",
                "accountName": "ana@example.com",
                "email": "ana@example.com",
                "permissionLevel": "ADMIN",
                "createdTimestamp": "2026-09-21T14:13:20Z",
                "status": "ACTIVE",
                "mfaEnabled": true,
                "mfaMethods": ["OTP"],
                "authMethod": "SSO",
                "roleDescription": "Admin",
                "lastLoginTimestamp": "2026-11-18T11:06:40Z",
            })
        );
    }

    #[test]
    fn roles_status_and_sign_in_map_to_vantas_values() {
        let mut m = member();
        // Standard members edit, read-only ones are base accounts; a custom role alone is a base account.
        m.role_ids = vec!["example-org-standard".into()];
        assert_eq!(user_account("example-org", &m, "https://oc.example.com")["permissionLevel"], "EDITOR");
        m.role_ids = vec!["example-org-readonly".into()];
        assert_eq!(user_account("example-org", &m, "https://oc.example.com")["permissionLevel"], "BASE");
        // Anyone who can still sign in with a password signs in with a password, MFA or not.
        m.login_methods = vec!["saml".into(), "password".into()];
        m.mfa_enabled = false;
        let v = user_account("example-org", &m, "https://oc.example.com");
        assert_eq!((v["authMethod"].as_str(), v["mfaEnabled"].as_bool(), v["mfaMethods"].clone()), (Some("PASSWORD"), Some(false), json!(["DISABLED"])));
        // A deactivated member, with when; one who never signed in has no last login.
        m.status = "deactivated".into();
        m.modified_at = Some(1_795_100_000_000);
        m.last_login_at = None;
        let v = user_account("example-org", &m, "https://oc.example.com");
        assert_eq!((v["status"].as_str(), v["deactivatedTimestamp"].as_str()), (Some("DEACTIVATED"), Some("2026-11-19T14:53:20Z")));
        assert!(v.get("lastLoginTimestamp").is_none());
    }

    #[tokio::test]
    async fn only_the_orgs_accepted_members_are_sent() {
        let Some((_, states)) = crate::test_support::replicas(1).await else { return };
        let st = &states[0];
        let (org, other) = (crate::test_support::new_org(st, "vanta-users").await, crate::test_support::new_org(st, "vanta-users").await);
        sqlx::query("INSERT INTO roles (id, org_id, name, description, managed) VALUES ($1 || '-admin', $1, 'Admin', '', true)").bind(&org).execute(&st.postgres).await.unwrap();
        for (o, email, status) in [(&org, "ana@example.com", "active"), (&org, "ben@example.com", "deactivated"), (&org, "cleo@example.com", "pending"), (&other, "dan@example.com", "active")] {
            let (id,): (String,) = sqlx::query_as("INSERT INTO users (id, org_id, email, name, status) VALUES (gen_random_uuid(), $1, $2, $2, $3) RETURNING id::text").bind(o).bind(email).bind(status).fetch_one(&st.postgres).await.unwrap();
            if email == "ana@example.com" {
                sqlx::query("INSERT INTO user_roles (user_id, role_id) VALUES ($1::uuid, $2)").bind(&id).bind(format!("{org}-admin")).execute(&st.postgres).await.unwrap();
            }
        }
        let sent = user_accounts(st, &org).await.unwrap();
        let emails: Vec<&str> = sent.iter().map(|u| u["email"].as_str().unwrap()).collect();
        assert_eq!(emails, ["ana@example.com", "ben@example.com"]);
        assert_eq!((sent[0]["permissionLevel"].as_str(), sent[1]["status"].as_str()), (Some("ADMIN"), Some("DEACTIVATED")));
    }
}
