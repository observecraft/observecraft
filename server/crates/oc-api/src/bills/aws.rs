//! AWS Cost and Usage Reports: which manifest describes each billing month in the bucket, which files it
//! lists, and their line items aggregated per day into [`BillLine`]s (DuckDB over the downloaded files).
//!
//! - **CUR 2.0** (Data Exports): `<prefix>/<export>/metadata/BILLING_PERIOD=YYYY-MM/<export>-Manifest.json`
//!   (or `Manifest.json`), naming its `dataFiles` (`s3://bucket/key`) and the run (`executionId`).
//! - **Legacy CUR**: `<prefix>/<report>/<yyyymmdd-yyyymmdd>/<report>-Manifest.json`, naming its
//!   `reportKeys` and the run (`assemblyId`); copies under the run's own folder are ignored.
//! - Columns are matched however the format spells them (`lineItem/UnblendedCost`,
//!   `line_item_unblended_cost`): lower-cased without separators. Missing ones (no Savings Plans) are NULL.
//! - Costs: unblended → BilledCost; amortized → EffectiveCost (Savings Plan and Reserved Instance usage at
//!   its effective cost, the unused part of their recurring fees, upfront fees spread out by AWS itself);
//!   public on-demand → ListCost.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde_json::Value;

use super::{columns, ident, quote, reader, tags_from_json, BillLine};

/// The manifest of each billing month (`YYYY-MM`) of the report `report` under `prefix`.
pub fn month_manifests(keys: impl IntoIterator<Item = String>, prefix: &str, report: &str) -> BTreeMap<String, String> {
    let prefix = prefix.trim_matches('/');
    let base = if prefix.is_empty() { format!("{report}/") } else { format!("{prefix}/{report}/") };
    let named = format!("{report}-Manifest.json");
    let mut out = BTreeMap::new();
    for key in keys {
        let Some(rest) = key.strip_prefix(&base) else { continue };
        let parts: Vec<&str> = rest.split('/').collect();
        let month = match parts.as_slice() {
            ["metadata", period, file] if *file == named || *file == "Manifest.json" => period.strip_prefix("BILLING_PERIOD=").filter(|m| m.len() == 7).map(str::to_string),
            [range, file] if *file == named && range.len() == 17 && range.as_bytes()[8] == b'-' && range[..8].bytes().all(|b| b.is_ascii_digit()) => Some(format!("{}-{}", &range[..4], &range[4..6])),
            _ => None,
        };
        if let Some(month) = month {
            // The export-named manifest wins over a plain `Manifest.json` of the same month.
            if !out.contains_key(&month) || key.ends_with(&named) {
                out.insert(month, key);
            }
        }
    }
    out
}

/// A manifest's run (it changes when AWS rewrites the month) and data files (keys in the bucket).
#[derive(Debug, PartialEq)]
pub struct Manifest {
    pub run: String,
    pub files: Vec<String>,
}

pub fn parse_manifest(bytes: &[u8]) -> Result<Manifest, String> {
    let v: Value = serde_json::from_slice(bytes).map_err(|e| format!("the manifest isn't JSON: {e}"))?;
    let run = v["executionId"].as_str().or(v["assemblyId"].as_str()).ok_or("the manifest has no executionId or assemblyId")?.to_string();
    let files = v["dataFiles"].as_array().or(v["reportKeys"].as_array()).ok_or("the manifest lists no dataFiles or reportKeys")?;
    let files = files
        .iter()
        .filter_map(Value::as_str)
        .map(|f| match f.strip_prefix("s3://") {
            Some(rest) => rest.split_once('/').map_or("", |(_, key)| key).to_string(),
            None => f.to_string(),
        })
        .filter(|f| !f.is_empty())
        .collect();
    Ok(Manifest { run, files })
}

/// The report's line items, summed per day and kind (DuckDB over the downloaded files).
pub fn load(files: &[PathBuf]) -> Result<Vec<BillLine>, String> {
    let source = reader(files)?;
    let conn = super::connection()?;
    let cols = columns(&conn, &source)?;
    let col = |names: &[&str]| names.iter().find_map(|n| cols.get(*n)).map(|(name, _)| ident(name));
    let text = |names: &[&str]| col(names).map_or("''".to_string(), |c| format!("COALESCE(CAST({c} AS VARCHAR), '')"));
    let num = |names: &[&str]| col(names).map_or("CAST(NULL AS DOUBLE)".to_string(), |c| format!("TRY_CAST({c} AS DOUBLE)"));
    let map_value = |map: &str, key: &str| cols.get(map).filter(|(_, t)| t.starts_with("MAP")).map(|(name, _)| format!("COALESCE(map_extract({}, {})[1], '')", ident(name), quote(key)));
    // Tags: CUR 2.0's `resource_tags` map, or the legacy formats' one column per tag.
    let tags = match cols.get("resourcetags").filter(|(_, t)| t.starts_with("MAP")) {
        Some((name, _)) => format!("COALESCE(CAST(to_json({}) AS VARCHAR), '{{}}')", ident(name)),
        None => {
            let pairs: Vec<String> = cols
                .iter()
                .filter(|(c, _)| c.starts_with("resourcetags") && c.len() > "resourcetags".len())
                .map(|(_, (name, _))| {
                    let key = name.split_once('/').map(|(_, k)| k).or_else(|| name.strip_prefix("resource_tags_")).unwrap_or(name);
                    format!("{}, CAST({} AS VARCHAR)", quote(key), ident(name))
                })
                .collect();
            if pairs.is_empty() { "'{}'".to_string() } else { format!("CAST(json_object({}) AS VARCHAR)", pairs.join(", ")) }
        }
    };
    let instance_type = if cols.contains_key("productinstancetype") { text(&["productinstancetype"]) } else { map_value("product", "instance_type").unwrap_or("''".into()) };
    let sql = format!(
        "WITH src AS (SELECT left({start}, 10) AS day, {account} AS account, {account_name} AS account_name, {product} AS product, {region} AS region, \
           {zone} AS zone, {kind} AS kind, {description} AS description, {resource} AS resource, {unit} AS unit, {usage_type} AS usage_type, \
           {operation} AS operation, {instance_type} AS instance_type, {currency} AS currency, {sp_arn} AS sp_arn, {ri_arn} AS ri_arn, {tags} AS tags, \
           COALESCE({quantity}, 0) AS quantity, COALESCE({unblended}, 0) AS unblended, {on_demand} AS on_demand, {sp_effective} AS sp_effective, \
           {sp_commitment} AS sp_commitment, {sp_used} AS sp_used, {ri_effective} AS ri_effective, {ri_unused_upfront} AS ri_unused_upfront, \
           {ri_unused_recurring} AS ri_unused_recurring FROM {source}) \
         SELECT day, account, max(account_name), product, region, zone, kind, description, resource, unit, usage_type, operation, instance_type, currency, \
           CASE WHEN kind IN ('SavingsPlanCoveredUsage', 'SavingsPlanRecurringFee') THEN sp_arn WHEN kind IN ('DiscountedUsage', 'RIFee') THEN ri_arn ELSE '' END AS commitment, \
           tags, sum(quantity), sum(unblended), \
           sum(CASE kind \
             WHEN 'SavingsPlanCoveredUsage' THEN COALESCE(sp_effective, 0) \
             WHEN 'SavingsPlanRecurringFee' THEN COALESCE(sp_commitment, 0) - COALESCE(sp_used, 0) \
             WHEN 'SavingsPlanNegation' THEN 0 WHEN 'SavingsPlanUpfrontFee' THEN 0 \
             WHEN 'DiscountedUsage' THEN COALESCE(ri_effective, 0) \
             WHEN 'RIFee' THEN COALESCE(ri_unused_upfront, 0) + COALESCE(ri_unused_recurring, 0) \
             WHEN 'Fee' THEN CASE WHEN ri_arn <> '' THEN 0 ELSE unblended END \
             ELSE unblended END), \
           sum(COALESCE(on_demand, CASE WHEN kind IN ('Usage', 'DiscountedUsage', 'SavingsPlanCoveredUsage') THEN unblended ELSE 0 END)) \
         FROM src GROUP BY ALL",
        start = text(&["lineitemusagestartdate"]),
        account = text(&["lineitemusageaccountid"]),
        account_name = text(&["lineitemusageaccountname"]),
        product = text(&["lineitemproductcode"]),
        region = text(&["productregioncode", "productregion"]),
        zone = text(&["lineitemavailabilityzone"]),
        kind = text(&["lineitemlineitemtype"]),
        description = text(&["lineitemlineitemdescription"]),
        resource = text(&["lineitemresourceid"]),
        unit = text(&["pricingunit"]),
        usage_type = text(&["lineitemusagetype"]),
        operation = text(&["lineitemoperation"]),
        currency = text(&["lineitemcurrencycode"]),
        sp_arn = text(&["savingsplansavingsplanarn"]),
        ri_arn = text(&["reservationreservationarn"]),
        quantity = num(&["lineitemusageamount"]),
        unblended = num(&["lineitemunblendedcost"]),
        on_demand = num(&["pricingpublicondemandcost"]),
        sp_effective = num(&["savingsplansavingsplaneffectivecost"]),
        sp_commitment = num(&["savingsplantotalcommitmenttodate"]),
        sp_used = num(&["savingsplanusedcommitment"]),
        ri_effective = num(&["reservationeffectivecost"]),
        ri_unused_upfront = num(&["reservationunusedamortizedupfrontfeeforbillingperiod"]),
        ri_unused_recurring = num(&["reservationunusedrecurringfee"]),
    );
    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| {
            let s = |i: usize| r.get::<_, String>(i);
            let kind = s(6)?;
            let usage_type = s(10)?;
            let (account_id, product, instance_type, operation) = (s(1)?, s(3)?, s(12)?, s(11)?);
            let mut tags = tags_from_json(&s(15)?);
            for (key, value) in [("aws_account", &account_id), ("aws_product", &product), ("aws_instance_type", &instance_type), ("aws_usage_type", &usage_type), ("aws_operation", &operation)] {
                if !value.is_empty() {
                    tags.insert(key.to_string(), value.clone());
                }
            }
            let commitment_id = s(14)?;
            let account_name = s(2)?;
            Ok(BillLine {
                date: s(0)?,
                account_name: if account_name.is_empty() { account_id.clone() } else { account_name },
                account_id,
                service: product,
                region: s(4)?,
                zone: s(5)?,
                category: category(&kind).to_string(),
                description: s(7)?,
                resource_id: s(8)?,
                pricing_unit: s(9)?,
                quantity: r.get(16)?,
                billed: r.get(17)?,
                effective: r.get(18)?,
                list: r.get(19)?,
                pricing_category: pricing_category(&kind, &usage_type).to_string(),
                commitment_type: if commitment_id.is_empty() { String::new() } else if kind.starts_with("SavingsPlan") { "Savings Plan".into() } else { "Reserved Instance".into() },
                commitment_status: match (commitment_id.is_empty(), kind.as_str()) {
                    (true, _) => "",
                    (false, "SavingsPlanRecurringFee" | "RIFee") => "Unused",
                    _ => "Used",
                }
                .to_string(),
                commitment_id,
                currency: s(13)?,
                tags,
            })
        })
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())
}

/// FOCUS's charge category of a line item type.
fn category(kind: &str) -> &'static str {
    match kind {
        "Tax" => "tax",
        "Credit" | "Refund" | "BundledDiscount" | "EdpDiscount" | "PrivateRateDiscount" | "SppDiscount" | "Discount" | "DistributorDiscount" => "credit",
        "Fee" | "RIFee" | "SavingsPlanUpfrontFee" | "SavingsPlanRecurringFee" => "purchase",
        "SavingsPlanNegation" => "adjustment",
        _ => "usage",
    }
}

fn pricing_category(kind: &str, usage_type: &str) -> &'static str {
    match kind {
        "SavingsPlanCoveredUsage" | "DiscountedUsage" | "SavingsPlanRecurringFee" | "RIFee" => "Committed",
        "Usage" if usage_type.contains("SpotUsage") => "Dynamic",
        "Usage" => "Standard",
        _ => "",
    }
}

/// The config's `account_filters`: the member accounts whose costs are kept (all without filters).
pub fn filter_accounts(lines: Vec<BillLine>, filters: &Value) -> Vec<BillLine> {
    if filters.is_null() {
        return lines;
    }
    let list = |k: &str| -> Vec<&str> { filters[k].as_array().map(|a| a.iter().filter_map(Value::as_str).collect()).unwrap_or_default() };
    let (included, excluded) = (list("included_accounts"), list("excluded_accounts"));
    let include_new = filters["include_new_accounts"].as_bool().unwrap_or(true);
    lines.into_iter().filter(|l| !excluded.contains(&l.account_id.as_str()) && (include_new || included.contains(&l.account_id.as_str()))).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("oc-bills-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn each_month_has_its_latest_manifest() {
        let keys = [
            "cur/costs/metadata/BILLING_PERIOD=2026-10/costs-Manifest.json",
            "cur/costs/metadata/BILLING_PERIOD=2026-10/20261102T010203Z-e1/costs-Manifest.json",
            "cur/costs/data/BILLING_PERIOD=2026-10/costs-00001.snappy.parquet",
            "cur/costs/metadata/BILLING_PERIOD=2026-11/Manifest.json",
            "cur/legacy/20260901-20261001/legacy-Manifest.json",
            "cur/legacy/20260901-20261001/a1b2/legacy-Manifest.json",
            "cur/other/metadata/BILLING_PERIOD=2026-10/other-Manifest.json",
        ];
        let found = month_manifests(keys.iter().map(|k| k.to_string()), "cur", "costs");
        assert_eq!(found, BTreeMap::from([("2026-10".to_string(), keys[0].to_string()), ("2026-11".to_string(), keys[3].to_string())]));
        let found = month_manifests(keys.iter().map(|k| k.to_string()), "cur", "legacy");
        assert_eq!(found, BTreeMap::from([("2026-09".to_string(), keys[4].to_string())]));
    }

    #[test]
    fn manifests_name_their_run_and_files() {
        let cur2 = r#"{"executionId":"e1","dataFiles":["s3://example-org-cur/cur/costs/data/BILLING_PERIOD=2026-10/costs-00001.snappy.parquet"]}"#;
        assert_eq!(parse_manifest(cur2.as_bytes()).unwrap(), Manifest { run: "e1".into(), files: vec!["cur/costs/data/BILLING_PERIOD=2026-10/costs-00001.snappy.parquet".into()] });
        let legacy = r#"{"assemblyId":"a1b2","reportKeys":["cur/legacy/20260901-20261001/a1b2/legacy-1.csv.gz"]}"#;
        assert_eq!(parse_manifest(legacy.as_bytes()).unwrap(), Manifest { run: "a1b2".into(), files: vec!["cur/legacy/20260901-20261001/a1b2/legacy-1.csv.gz".into()] });
        assert!(parse_manifest(b"{}").is_err());
    }

    #[test]
    fn a_legacy_csv_report_is_aggregated_per_day_with_amortized_costs() {
        let d = dir("legacy");
        let header = "identity/LineItemId,lineItem/UsageStartDate,lineItem/UsageAccountId,lineItem/LineItemType,lineItem/ProductCode,lineItem/UsageType,lineItem/Operation,lineItem/ResourceId,lineItem/LineItemDescription,lineItem/UsageAmount,lineItem/UnblendedCost,lineItem/CurrencyCode,lineItem/AvailabilityZone,product/region,product/instanceType,pricing/unit,pricing/publicOnDemandCost,savingsPlan/SavingsPlanARN,savingsPlan/SavingsPlanEffectiveCost,savingsPlan/TotalCommitmentToDate,savingsPlan/UsedCommitment,resourceTags/user:team";
        let rows = [
            // Two hours of Savings Plan covered usage of one instance: billed 0, amortized 0.06 each.
            "1,2026-10-01T00:00:00Z,123456789012,SavingsPlanCoveredUsage,AmazonEC2,BoxUsage:m6i.xlarge,RunInstances,i-0a1b2c3d4e5f60001,usage,1,0.192,USD,us-east-1a,us-east-1,m6i.xlarge,Hrs,0.192,arn:aws:savingsplans::123456789012:savingsplan/sp-1,0.12,,,checkout",
            "2,2026-10-01T01:00:00Z,123456789012,SavingsPlanCoveredUsage,AmazonEC2,BoxUsage:m6i.xlarge,RunInstances,i-0a1b2c3d4e5f60001,usage,1,0.192,USD,us-east-1a,us-east-1,m6i.xlarge,Hrs,0.192,arn:aws:savingsplans::123456789012:savingsplan/sp-1,0.12,,,checkout",
            "3,2026-10-01T00:00:00Z,123456789012,SavingsPlanNegation,AmazonEC2,BoxUsage:m6i.xlarge,RunInstances,i-0a1b2c3d4e5f60001,negation,0,-0.384,USD,us-east-1a,us-east-1,m6i.xlarge,Hrs,,arn:aws:savingsplans::123456789012:savingsplan/sp-1,,,,checkout",
            // The plan's fee: 0.30 committed, 0.24 used → 0.06 unused.
            "4,2026-10-01T00:00:00Z,123456789012,SavingsPlanRecurringFee,ComputeSavingsPlans,ComputeSP:1yrNoUpfront,,arn:aws:savingsplans::123456789012:savingsplan/sp-1,fee,1,0.30,USD,,,,,,arn:aws:savingsplans::123456789012:savingsplan/sp-1,,0.30,0.24,",
            "5,2026-10-02T03:00:00Z,210987654321,Usage,AmazonS3,TimedStorage-ByteHrs,StandardStorage,example-assets,storage,10,0.23,USD,,us-east-1,,GB-Mo,0.23,,,,,",
            "6,2026-10-02T00:00:00Z,123456789012,Tax,AWSSupportBusiness,Tax,,,tax,1,1.5,USD,,,,,,,,,,",
        ];
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        writeln!(gz, "{header}").unwrap();
        for r in rows {
            writeln!(gz, "{r}").unwrap();
        }
        let file = d.join("legacy-1.csv.gz");
        std::fs::write(&file, gz.finish().unwrap()).unwrap();
        let lines = load(&[file]).unwrap();
        let ec2: Vec<&BillLine> = lines.iter().filter(|l| l.service == "AmazonEC2").collect();
        let covered = ec2.iter().find(|l| l.commitment_status == "Used").unwrap();
        assert_eq!((covered.date.as_str(), covered.quantity, covered.pricing_category.as_str()), ("2026-10-01", 2.0, "Committed"));
        assert!((covered.billed - 0.384).abs() < 1e-9 && (covered.effective - 0.24).abs() < 1e-9 && (covered.list - 0.384).abs() < 1e-9, "{covered:?}");
        assert_eq!((covered.commitment_type.as_str(), covered.commitment_id.as_str()), ("Savings Plan", "arn:aws:savingsplans::123456789012:savingsplan/sp-1"));
        assert_eq!(covered.tags.get("team").map(String::as_str), Some("checkout"));
        assert_eq!(covered.tags.get("aws_instance_type").map(String::as_str), Some("m6i.xlarge"));
        assert_eq!((covered.region.as_str(), covered.zone.as_str(), covered.resource_id.as_str()), ("us-east-1", "us-east-1a", "i-0a1b2c3d4e5f60001"));
        // The negation cancels the billed on-demand charge; it amortizes to nothing.
        let negation = ec2.iter().find(|l| l.category == "adjustment").unwrap();
        assert!((negation.billed + 0.384).abs() < 1e-9 && negation.effective == 0.0);
        let fee = lines.iter().find(|l| l.service == "ComputeSavingsPlans").unwrap();
        assert_eq!((fee.category.as_str(), fee.commitment_status.as_str()), ("purchase", "Unused"));
        assert!((fee.billed - 0.30).abs() < 1e-9 && (fee.effective - 0.06).abs() < 1e-9, "{fee:?}");
        // Over the month: billed 0.30 + 0.23 + 1.5 (the covered usage and its negation cancel), amortized
        // 0.24 used + 0.06 unused of the plan + 0.23 + 1.5.
        let billed: f64 = lines.iter().map(|l| l.billed).sum();
        let effective: f64 = lines.iter().map(|l| l.effective).sum();
        assert!((billed - 2.03).abs() < 1e-9 && (effective - 2.03).abs() < 1e-9, "{billed} {effective}");
        let s3 = lines.iter().find(|l| l.service == "AmazonS3").unwrap();
        assert_eq!((s3.account_id.as_str(), s3.account_name.as_str(), s3.pricing_category.as_str(), s3.date.as_str()), ("210987654321", "210987654321", "Standard", "2026-10-02"));
        assert_eq!(lines.iter().find(|l| l.category == "tax").map(|l| l.service.as_str()), Some("AWSSupportBusiness"));
    }

    #[test]
    fn a_cur2_parquet_export_reads_its_maps_and_reserved_instances() {
        let d = dir("cur2");
        let file = d.join("costs-00001.snappy.parquet");
        let conn = duckdb::Connection::open_in_memory().unwrap();
        conn.execute_batch(&format!(
            "COPY (SELECT * FROM (VALUES
               (TIMESTAMP '2026-10-03 05:00:00', '123456789012', 'ecommerce-prod', 'DiscountedUsage', 'AmazonRDS', 'InstanceUsage:db.r6g.large', 'CreateDBInstance', 'arn:aws:rds:us-east-1:123456789012:db:orders', 'db', 1.0, 0.0, 0.26, 'USD', 'us-east-1', 'Hrs', 'arn:aws:rds:us-east-1:123456789012:ri:ri-1', 0.15, NULL::DOUBLE, NULL::DOUBLE, MAP {{'user_team': 'orders', 'aws_created_by': 'x'}}, MAP {{'instance_type': 'db.r6g.large'}}),
               (TIMESTAMP '2026-10-03 00:00:00', '123456789012', 'ecommerce-prod', 'RIFee', 'AmazonRDS', 'HeavyUsage:db.r6g.large', '', 'arn:aws:rds:us-east-1:123456789012:ri:ri-1', 'fee', 24.0, 3.6, 0.0, 'USD', 'us-east-1', 'Hrs', 'arn:aws:rds:us-east-1:123456789012:ri:ri-1', NULL::DOUBLE, 0.5, 0.7, MAP {{}}::MAP(VARCHAR, VARCHAR), MAP {{}}::MAP(VARCHAR, VARCHAR)),
               (TIMESTAMP '2026-10-03 07:00:00', '123456789012', 'ecommerce-prod', 'Usage', 'AmazonEC2', 'SpotUsage:c6i.large', 'RunInstances:SV001', 'i-0a1b2c3d4e5f60002', 'spot', 1.0, 0.03, 0.085, 'USD', 'us-east-1', 'Hrs', NULL, NULL::DOUBLE, NULL::DOUBLE, NULL::DOUBLE, MAP {{'user_team': 'catalog'}}, MAP {{'instance_type': 'c6i.large'}})
             ) AS t(line_item_usage_start_date, line_item_usage_account_id, line_item_usage_account_name, line_item_line_item_type, line_item_product_code, line_item_usage_type, line_item_operation, line_item_resource_id, line_item_line_item_description, line_item_usage_amount, line_item_unblended_cost, pricing_public_on_demand_cost, line_item_currency_code, product_region_code, pricing_unit, reservation_reservation_a_r_n, reservation_effective_cost, reservation_unused_amortized_upfront_fee_for_billing_period, reservation_unused_recurring_fee, resource_tags, product))
             TO '{}' (FORMAT parquet)",
            file.display()
        ))
        .unwrap();
        let lines = load(&[file]).unwrap();
        let used = lines.iter().find(|l| l.commitment_status == "Used").unwrap();
        assert_eq!((used.commitment_type.as_str(), used.account_name.as_str(), used.service.as_str()), ("Reserved Instance", "ecommerce-prod", "AmazonRDS"));
        assert!((used.effective - 0.15).abs() < 1e-9 && used.billed == 0.0 && (used.list - 0.26).abs() < 1e-9);
        assert_eq!(used.tags.get("team").map(String::as_str), Some("orders"));
        assert_eq!(used.tags.get("aws_instance_type").map(String::as_str), Some("db.r6g.large"));
        assert!(!used.tags.contains_key("aws_created_by") || used.tags["aws_created_by"] == "x");
        let fee = lines.iter().find(|l| l.commitment_status == "Unused").unwrap();
        assert!((fee.billed - 3.6).abs() < 1e-9 && (fee.effective - 1.2).abs() < 1e-9, "{fee:?}");
        let spot = lines.iter().find(|l| l.service == "AmazonEC2").unwrap();
        assert_eq!((spot.pricing_category.as_str(), spot.date.as_str()), ("Dynamic", "2026-10-03"));
    }

    #[test]
    fn account_filters_keep_the_accounts_they_name() {
        let line = |account: &str| BillLine { account_id: account.into(), ..BillLine::default() };
        let lines = vec![line("111111111111"), line("222222222222"), line("333333333333")];
        let keep = |filters: serde_json::Value| filter_accounts(lines.clone(), &filters).into_iter().map(|l| l.account_id).collect::<Vec<_>>();
        assert_eq!(keep(serde_json::Value::Null).len(), 3);
        assert_eq!(keep(serde_json::json!({ "include_new_accounts": false, "included_accounts": ["111111111111"], "excluded_accounts": [] })), ["111111111111"]);
        assert_eq!(keep(serde_json::json!({ "include_new_accounts": true, "included_accounts": [], "excluded_accounts": ["222222222222"] })), ["111111111111", "333333333333"]);
    }
}
