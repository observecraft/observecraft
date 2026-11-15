//! The cloud resources the crawlers discover with their tags (AWS: the Resource Groups Tagging API; Azure: the
//! subscription's resource list), kept in Postgres `cloud_resources` for tagging policies and Cloud Security
//! (`routes::provenance`). Written at most once an hour per job; rows not seen for two days are dropped.

use crate::state::AppState;

/// A discovered resource.
#[derive(Debug, Clone, PartialEq)]
pub struct Discovered {
    pub id: String,
    pub name: String,
    /// `AwsS3Bucket`, `AwsEc2Instance`; Azure's `Microsoft.Compute/virtualMachines`.
    pub resource_type: String,
    pub region: String,
    pub account: String,
    /// `key:value`
    pub tags: Vec<String>,
}

fn pascal(s: &str) -> String {
    s.split(['-', '_', '.']).filter(|w| !w.is_empty()).map(|w| w[..1].to_ascii_uppercase() + &w[1..]).collect()
}

/// An AWS resource from its ARN (`arn:partition:service:region:account:type/id`, `…:type:id` or `…:id`): its type
/// named as the account scans name theirs (`AwsS3Bucket`, `AwsEc2Instance`), its name the ARN's last part.
pub fn from_arn(arn: &str, tags: &[String], account: &str) -> Option<Discovered> {
    let mut parts = arn.splitn(6, ':');
    let (_arn, _partition, service, region, owner, resource) = (parts.next()?, parts.next()?, parts.next()?, parts.next()?, parts.next()?, parts.next()?);
    if service.is_empty() || resource.is_empty() {
        return None;
    }
    let (kind, name) = match resource.split_once(['/', ':']) {
        // The last part, unless it's a generated id after the name (`loadbalancer/app/storefront/50dc6c495c0c9188`).
        Some((kind, rest)) => {
            let parts: Vec<&str> = rest.split(['/', ':']).collect();
            let last = parts[parts.len() - 1];
            let generated = parts.len() >= 2 && last.len() >= 16 && last.chars().all(|c| c.is_ascii_hexdigit());
            (kind, if generated { parts[parts.len() - 2] } else { last })
        }
        // S3 buckets (`arn:aws:s3:::bucket`), SNS topics, SQS queues: the service's one kind.
        None => (if service == "s3" { "bucket" } else { "" }, resource),
    };
    Some(Discovered {
        id: arn.to_string(),
        name: name.to_string(),
        resource_type: format!("Aws{}{}", pascal(service), pascal(kind)),
        region: region.to_string(),
        account: if owner.is_empty() { account.to_string() } else { owner.to_string() },
        tags: tags.to_vec(),
    })
}

/// Stores a crawl's resources (ids as given; Azure's lower-cased by the caller) and drops the account's rows unseen
/// for two days.
pub async fn save(st: &AppState, org: &str, provider: &str, account: &str, rows: &[Discovered]) -> Result<(), sqlx::Error> {
    let col = |f: fn(&Discovered) -> String| rows.iter().map(f).collect::<Vec<String>>();
    let tags: Vec<serde_json::Value> = rows.iter().map(|r| serde_json::json!(r.tags)).collect();
    let mut tx = st.postgres.begin().await?;
    sqlx::query(
        "INSERT INTO cloud_resources (org_id, provider, resource_id, account_id, name, resource_type, region, tags) \
         SELECT $1, $2, * FROM unnest($3::text[], $4::text[], $5::text[], $6::text[], $7::text[], $8::jsonb[]) \
         ON CONFLICT (org_id, provider, resource_id) DO UPDATE SET account_id = EXCLUDED.account_id, name = EXCLUDED.name, resource_type = EXCLUDED.resource_type, \
         region = EXCLUDED.region, tags = EXCLUDED.tags, last_seen = now()",
    )
    .bind(org)
    .bind(provider)
    .bind(col(|r| r.id.clone()))
    .bind(col(|r| r.account.clone()))
    .bind(col(|r| r.name.clone()))
    .bind(col(|r| r.resource_type.clone()))
    .bind(col(|r| r.region.clone()))
    .bind(&tags)
    .execute(&mut *tx)
    .await?;
    sqlx::query("DELETE FROM cloud_resources WHERE org_id = $1 AND provider = $2 AND account_id = $3 AND last_seen < now() - interval '2 days'")
        .bind(org)
        .bind(provider)
        .bind(account)
        .execute(&mut *tx)
        .await?;
    tx.commit().await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arns_name_the_resource_and_its_type() {
        let t = vec!["team:storefront".to_string()];
        let r = |arn: &str| from_arn(arn, &t, "123456789012").map(|d| (d.resource_type, d.name, d.region, d.account));
        let s = |a: &str, b: &str, c: &str, d: &str| Some((a.to_string(), b.to_string(), c.to_string(), d.to_string()));
        assert_eq!(r("arn:aws:s3:::example-shop-assets"), s("AwsS3Bucket", "example-shop-assets", "", "123456789012"));
        assert_eq!(r("arn:aws:ec2:eu-west-1:123456789012:instance/i-0a1b2c3d4e5f60001"), s("AwsEc2Instance", "i-0a1b2c3d4e5f60001", "eu-west-1", "123456789012"));
        assert_eq!(r("arn:aws:lambda:eu-west-1:123456789012:function:order-events"), s("AwsLambdaFunction", "order-events", "eu-west-1", "123456789012"));
        assert_eq!(r("arn:aws:elasticloadbalancing:eu-west-1:123456789012:loadbalancer/app/storefront/50dc6c495c0c9188"), s("AwsElasticloadbalancingLoadbalancer", "storefront", "eu-west-1", "123456789012"));
        assert_eq!(r("arn:aws:sqs:eu-west-1:123456789012:order-events"), s("AwsSqs", "order-events", "eu-west-1", "123456789012"));
        assert_eq!(r("not-an-arn"), None);
    }

    /// Crawled resources are cloud resources of tagging policies. Live: skipped without the test databases.
    #[tokio::test]
    async fn saved_resources_are_evaluated_by_tagging_policies() {
        use crate::routes::provenance::{inventory, TagTarget};
        let Some((_, states)) = crate::test_support::replicas(1).await else { return };
        let st = &states[0];
        let org = crate::test_support::new_org(st, "cloud-res").await;
        let bucket = from_arn("arn:aws:s3:::example-shop-assets", &["team:storefront".to_string()], "123456789012").unwrap();
        save(st, &org, "aws", "123456789012", std::slice::from_ref(&bucket)).await.unwrap();
        save(st, &org, "aws", "123456789012", &[Discovered { tags: vec!["team:storefront".into(), "env:prod".into()], ..bucket }]).await.unwrap();
        let cloud = inventory(st, &org, &[TagTarget::Cloud]).await.unwrap();
        assert_eq!(cloud.iter().map(|r| (r.name.as_str(), r.account.as_deref(), r.env.as_deref())).collect::<Vec<_>>(), vec![("example-shop-assets", Some("123456789012"), Some("prod"))]);
    }
}
