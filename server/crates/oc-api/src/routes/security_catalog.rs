//! Cloud SIEM content packs: per log source, the query that finds its logs and the default detection
//! rules ObserveCraft ships for it. The rules are our own, written for the attributes those sources
//! send (CloudTrail's `evt.name` / `userIdentity.*`, Okta's `evt.outcome`, …); every org gets them
//! as `is_default` rules ([`super::security_rules::ensure_defaults`]), on while their pack is on.

/// One default rule: a single log query, its group-by fields and threshold cases.
pub struct DefaultRule {
    /// Stable catalog id (`def-xxx-nnn`), also the rule's id in every org.
    pub id: &'static str,
    /// Bumped when the definition changes, so orgs pick up the new one.
    pub version: i32,
    pub name: &'static str,
    pub message: &'static str,
    pub query: &'static str,
    pub group_by: &'static [&'static str],
    /// `(condition, severity, name)`, most severe first.
    pub cases: &'static [(&'static str, &'static str, &'static str)],
    /// Seconds.
    pub window: i64,
    pub tags: &'static [&'static str],
}

pub struct ContentPack {
    pub id: &'static str,
    pub name: &'static str,
    /// Cloud Audit, Identity, Collaboration, Network, Web, Endpoint, …
    pub category: &'static str,
    /// The log sources table groups packs by platform.
    pub platform: &'static str,
    /// Brand logo shown on the card (the UI falls back to an icon).
    pub logo: &'static str,
    pub description: &'static str,
    /// Finds the pack's logs.
    pub query: &'static str,
    /// What to set up so the logs arrive.
    pub setup: &'static str,
    pub rules: &'static [DefaultRule],
}

const BRUTE_FORCE: &[&str] = &["tactic:TA0006-credential-access", "technique:T1110-brute-force"];

pub const PACKS: &[ContentPack] = &[
    ContentPack {
        id: "aws-cloudtrail",
        name: "AWS CloudTrail",
        category: "Cloud Audit",
        platform: "AWS",
        logo: "aws",
        description: "Watch who does what in your AWS accounts: console sign-ins, root usage, public buckets and tampering with the audit trail.",
        query: "source:cloudtrail",
        setup: "Send CloudTrail to the Datadog Forwarder (or an S3 → Lambda forwarder) with source `cloudtrail` and point it at this ObserveCraft intake.",
        rules: &[
            DefaultRule {
                id: "def-ctl-001",
                version: 1,
                name: "Brute force on AWS console sign-ins",
                message: "Many failed AWS console sign-ins for one identity. Check the source IPs and whether a sign-in finally succeeded.",
                query: "source:cloudtrail @evt.name:ConsoleLogin @evt.outcome:failure",
                group_by: &["@userIdentity.arn"],
                cases: &[("a > 25", "high", ""), ("a > 5", "medium", "")],
                window: 900,
                tags: BRUTE_FORCE,
            },
            DefaultRule {
                id: "def-ctl-002",
                version: 1,
                name: "AWS console sign-in without MFA",
                message: "An IAM user signed in to the AWS console without multi-factor authentication.",
                query: "source:cloudtrail @evt.name:ConsoleLogin @evt.outcome:success @additionalEventData.MFAUsed:No",
                group_by: &["@userIdentity.arn"],
                cases: &[("a > 0", "medium", "")],
                window: 300,
                tags: &["tactic:TA0001-initial-access", "technique:T1078-valid-accounts"],
            },
            DefaultRule {
                id: "def-ctl-003",
                version: 1,
                name: "AWS root account activity",
                message: "The account's root user made API calls. Root should only be used for the few tasks that require it.",
                query: "source:cloudtrail @userIdentity.type:Root",
                group_by: &["@userIdentity.accountId"],
                cases: &[("a > 0", "high", "")],
                window: 300,
                tags: &["tactic:TA0004-privilege-escalation", "technique:T1078.004-cloud-accounts"],
            },
            DefaultRule {
                id: "def-ctl-004",
                version: 1,
                name: "S3 bucket policy allows public access",
                message: "A bucket policy now grants access to everyone. Confirm the bucket is meant to be public.",
                query: "source:cloudtrail @evt.name:PutBucketPolicy @requestParameters.public:true",
                group_by: &["@requestParameters.bucketName"],
                cases: &[("a > 0", "high", "")],
                window: 300,
                tags: &["tactic:TA0010-exfiltration", "technique:T1530-data-from-cloud-storage"],
            },
            DefaultRule {
                id: "def-ctl-005",
                version: 1,
                name: "CloudTrail logging stopped or deleted",
                message: "A trail was stopped or deleted, which hides later activity from the audit log.",
                query: "source:cloudtrail @evt.name:(StopLogging OR DeleteTrail)",
                group_by: &["@userIdentity.arn"],
                cases: &[("a > 0", "high", "")],
                window: 300,
                tags: &["tactic:TA0005-defense-evasion", "technique:T1562.008-disable-cloud-logs"],
            },
        ],
    },
    ContentPack {
        id: "okta",
        name: "Okta",
        category: "Authentication",
        platform: "Identity",
        logo: "okta",
        description: "Spot attacks on your identity provider: MFA fatigue, credential stuffing and new administrators.",
        query: "source:okta",
        setup: "Enable the Okta system log integration (or stream the system log) with source `okta`.",
        rules: &[
            DefaultRule {
                id: "def-okt-001",
                version: 1,
                name: "Okta MFA brute force",
                message: "Repeated failed MFA challenges for one user: a push-fatigue or stolen-password attempt.",
                query: "source:okta @evt.name:user.authentication.auth_via_mfa @evt.outcome:FAILURE",
                group_by: &["@usr.email"],
                cases: &[("a > 10", "high", ""), ("a > 4", "medium", "")],
                window: 900,
                tags: BRUTE_FORCE,
            },
            DefaultRule {
                id: "def-okt-002",
                version: 1,
                name: "Okta administrator role granted",
                message: "A user was given an administrator role in Okta.",
                query: "source:okta @evt.name:user.account.privilege.grant",
                group_by: &["@target.email"],
                cases: &[("a > 0", "medium", "")],
                window: 300,
                tags: &["tactic:TA0003-persistence", "technique:T1098-account-manipulation"],
            },
        ],
    },
    ContentPack {
        id: "github",
        name: "GitHub",
        category: "Collaboration",
        platform: "SaaS",
        logo: "github",
        description: "Protect your source code: repositories made public, branch protections removed and new deploy keys.",
        query: "source:github",
        setup: "Stream the GitHub organization audit log with source `github`.",
        rules: &[
            DefaultRule {
                id: "def-ghb-001",
                version: 1,
                name: "GitHub repository made public",
                message: "A private repository became public. Check that no secrets or internal code are exposed.",
                query: "source:github @evt.name:repo.access @visibility:public",
                group_by: &["@repo"],
                cases: &[("a > 0", "high", "")],
                window: 300,
                tags: &["tactic:TA0010-exfiltration", "technique:T1567-exfiltration-over-web-service"],
            },
            DefaultRule {
                id: "def-ghb-002",
                version: 1,
                name: "GitHub branch protection removed",
                message: "The protection of a branch was removed, so changes can land without review.",
                query: "source:github @evt.name:protected_branch.destroy",
                group_by: &["@repo"],
                cases: &[("a > 0", "medium", "")],
                window: 300,
                tags: &["tactic:TA0005-defense-evasion", "technique:T1562-impair-defenses"],
            },
        ],
    },
    ContentPack {
        id: "kubernetes-audit",
        name: "Kubernetes Audit",
        category: "Containers",
        platform: "Kubernetes",
        logo: "kubernetes",
        description: "Follow interactive access to your clusters and requests that should never be allowed.",
        query: "source:kubernetes.audit",
        setup: "Collect the API server audit log with the Datadog Agent (`logs_config` on the audit file) with source `kubernetes.audit`.",
        rules: &[
            DefaultRule {
                id: "def-k8s-001",
                version: 1,
                name: "Interactive exec into a Kubernetes pod",
                message: "Someone opened a shell in a running pod.",
                query: "source:kubernetes.audit @objectRef.subresource:exec",
                group_by: &["@usr.name", "@objectRef.namespace"],
                cases: &[("a > 0", "low", "")],
                window: 300,
                tags: &["tactic:TA0002-execution", "technique:T1609-container-administration-command"],
            },
            DefaultRule {
                id: "def-k8s-002",
                version: 1,
                name: "Anonymous request allowed by the Kubernetes API",
                message: "The API server allowed a request from an unauthenticated user.",
                query: "source:kubernetes.audit @usr.name:\"system:anonymous\" @http.status_code:200",
                group_by: &["@network.client.ip"],
                cases: &[("a > 0", "high", "")],
                window: 300,
                tags: &["tactic:TA0001-initial-access", "technique:T1190-exploit-public-facing-application"],
            },
        ],
    },
    ContentPack {
        id: "web-attacks",
        name: "Web Server Attacks",
        category: "Web",
        platform: "Web",
        logo: "nginx",
        description: "Find attacks against your web servers in their access logs: injection, path traversal and scanners.",
        query: "source:nginx",
        setup: "Collect nginx access logs with the Datadog Agent's nginx integration (source `nginx`).",
        rules: &[
            DefaultRule {
                id: "def-web-001",
                version: 1,
                name: "Attack attempts on a web service",
                message: "A burst of requests carrying attack payloads hit the service. Look at the attacking IPs and whether any request succeeded.",
                query: "source:nginx @security.attack:true",
                group_by: &["service"],
                cases: &[("a > 1000", "medium", "Condition 2"), ("a > 100", "info", "Condition 1")],
                window: 900,
                tags: &["tactic:TA0043-reconnaissance", "technique:T1595-active-scanning"],
            },
            DefaultRule {
                id: "def-web-002",
                version: 1,
                name: "SQL injection attempts from one IP",
                message: "One client sent many SQL injection payloads.",
                query: "source:nginx @security.attack_type:sql_injection",
                group_by: &["@network.client.ip"],
                cases: &[("a > 10", "low", "")],
                window: 600,
                tags: &["tactic:TA0001-initial-access", "technique:T1190-exploit-public-facing-application"],
            },
        ],
    },
    ContentPack {
        id: "azure-activity",
        name: "Azure Activity Logs",
        category: "Cloud Audit",
        platform: "Azure",
        logo: "azure",
        description: "Monitor changes to your Azure subscriptions: new role assignments and deleted diagnostic settings.",
        query: "source:azure.activity_logs",
        setup: "Forward Azure activity logs (Event Hub → Datadog forwarder) with source `azure.activity_logs`.",
        rules: &[DefaultRule {
            id: "def-azr-001",
            version: 1,
            name: "Azure owner role assigned",
            message: "A principal was given the Owner role on a subscription or resource group.",
            query: "source:azure.activity_logs @evt.name:\"MICROSOFT.AUTHORIZATION/ROLEASSIGNMENTS/WRITE\" @properties.roleName:Owner",
            group_by: &["@identity.claims.name"],
            cases: &[("a > 0", "medium", "")],
            window: 300,
            tags: &["tactic:TA0004-privilege-escalation", "technique:T1098-account-manipulation"],
        }],
    },
    ContentPack {
        id: "gcp-audit",
        name: "Google Cloud Audit Logs",
        category: "Cloud Audit",
        platform: "GCP",
        logo: "googlecloud",
        description: "Watch your Google Cloud projects for new service account keys and IAM policy changes.",
        query: "source:gcp.audit",
        setup: "Export Cloud Audit Logs through Pub/Sub to the intake with source `gcp.audit`.",
        rules: &[DefaultRule {
            id: "def-gcp-001",
            version: 1,
            name: "Google Cloud service account key created",
            message: "A key was created for a service account. Long-lived keys are a common way in.",
            query: "source:gcp.audit @evt.name:google.iam.admin.v1.CreateServiceAccountKey",
            group_by: &["@usr.email"],
            cases: &[("a > 0", "low", "")],
            window: 300,
            tags: &["tactic:TA0003-persistence", "technique:T1098.001-additional-cloud-credentials"],
        }],
    },
    ContentPack {
        id: "cloudflare",
        name: "Cloudflare",
        category: "Network",
        platform: "Web",
        logo: "cloudflare",
        description: "Follow firewall events at your edge: blocked attacks and challenges.",
        query: "source:cloudflare",
        setup: "Send Cloudflare Logpush (firewall events) with source `cloudflare`.",
        rules: &[DefaultRule {
            id: "def-cfl-001",
            version: 1,
            name: "Cloudflare firewall blocking many requests from one IP",
            message: "The firewall blocked a burst of requests from one client.",
            query: "source:cloudflare @Action:block",
            group_by: &["@network.client.ip"],
            cases: &[("a > 200", "low", "")],
            window: 600,
            tags: &["tactic:TA0043-reconnaissance", "technique:T1595-active-scanning"],
        }],
    },
    ContentPack {
        id: "slack-audit",
        name: "Slack Audit Logs",
        category: "Collaboration",
        platform: "SaaS",
        logo: "slack",
        description: "Notice risky changes in your Slack workspace, such as new apps with broad access.",
        query: "source:slack",
        setup: "Stream the Slack audit logs API with source `slack`.",
        rules: &[DefaultRule {
            id: "def-slk-001",
            version: 1,
            name: "Slack app installed with broad scopes",
            message: "An app was installed with access to every channel or user data.",
            query: "source:slack @evt.name:app_installed @details.broad_scopes:true",
            group_by: &["@usr.email"],
            cases: &[("a > 0", "low", "")],
            window: 300,
            tags: &["tactic:TA0003-persistence", "technique:T1098-account-manipulation"],
        }],
    },
];

pub fn pack(id: &str) -> Option<&'static ContentPack> {
    PACKS.iter().find(|p| p.id == id)
}

/// Every default rule with its pack.
pub fn default_rules() -> impl Iterator<Item = (&'static ContentPack, &'static DefaultRule)> {
    PACKS.iter().flat_map(|p| p.rules.iter().map(move |r| (p, r)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn catalog_ids_are_unique_and_rules_valid() {
        let mut ids = HashSet::new();
        for (p, r) in default_rules() {
            assert!(ids.insert(r.id), "duplicate rule id {}", r.id);
            assert!(r.query.contains(p.query), "{}: the rule must search the pack's logs", r.id);
            assert!(!r.cases.is_empty());
            for (cond, sev, _) in r.cases {
                assert!(super::super::security_rules::Condition::parse(cond).is_ok(), "{}: {cond}", r.id);
                assert!(super::super::security_rules::SEVERITIES.contains(sev), "{}: {sev}", r.id);
            }
        }
        let packs: HashSet<_> = PACKS.iter().map(|p| p.id).collect();
        assert_eq!(packs.len(), PACKS.len());
    }
}
