//! Azure cost exports (Cost Management "cost and usage details", improved exports): which run of each
//! billing month an export last delivered, which files its manifest lists, and their rows summed per day
//! into [`BillLine`]s (DuckDB over the downloaded files).
//!
//! - **Layout**: `<container>/<path>/<export>/<yyyymmdd-yyyymmdd>/[<yyyyMMddHHmm>/]<runId>/manifest.json`
//!   (the 2023-07 API adds the run's time folder) and the files it lists (`blobs[].blobName`); without
//!   overwrite, every daily run of a month is a new `runId` folder,
//!   and the latest one (by its manifest's last modification) has the month to date. Exports of the classic
//!   API have no manifest: each run is one more file of the month's folder, the latest one is read.
//! - **Columns** are matched however the dataset version spells them (`CostInBillingCurrency`,
//!   `costInBillingCurrency`, `cost_in_billing_currency`): Enterprise Agreement, Microsoft Customer
//!   Agreement and pay-as-you-go details.
//! - **Datasets**: the actual cost export gives BilledCost, the amortized one EffectiveCost and ListCost
//!   (pay-as-you-go price × quantity); a config's two exports are read separately and their rows add up.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde_json::Value;

use super::{columns, ident, reader, tags_from_json, BillLine};

/// The latest run of each billing month (`YYYY-MM`) of the export `export` under `path`: its manifest, or
/// for exports without one (the classic exports API) its latest data file, and the run's id. `blobs` are
/// the names under the export with their last modification (epoch ms).
pub fn month_runs(blobs: Vec<(String, i64)>, path: &str, export: &str) -> BTreeMap<String, (String, String)> {
    let path = path.trim_matches('/');
    let base = if path.is_empty() { format!("{export}/") } else { format!("{path}/{export}/") };
    let is_range = |r: &str| r.len() == 17 && r.as_bytes()[8] == b'-' && r[..8].bytes().all(|b| b.is_ascii_digit());
    // month → (has a manifest, last modified, blob, run)
    let mut latest: BTreeMap<String, (bool, i64, String, String)> = BTreeMap::new();
    for (name, modified) in blobs {
        let Some(rest) = name.strip_prefix(&base) else { continue };
        let parts: Vec<&str> = rest.split('/').collect();
        let (range, manifest, run) = match parts.as_slice() {
            [range, run, "manifest.json"] => (*range, true, run.to_string()),
            // The 2023-07 API adds a folder named for the run's time (yyyyMMddHHmm) before the run id.
            [range, stamp, run, "manifest.json"] if stamp.len() == 12 && stamp.bytes().all(|b| b.is_ascii_digit()) => (*range, true, run.to_string()),
            // The file can change under the same name (read while the export was still writing it).
            [range, file] if [".csv", ".csv.gz", ".parquet"].iter().any(|x| file.ends_with(x)) => (*range, false, format!("{file}@{modified}")),
            _ => continue,
        };
        if !is_range(range) {
            continue;
        }
        let month = format!("{}-{}", &range[..4], &range[4..6]);
        if latest.get(&month).is_none_or(|(m, at, _, _)| (manifest, modified) > (*m, *at)) {
            latest.insert(month, (manifest, modified, name.clone(), run));
        }
    }
    latest.into_iter().map(|(month, (_, _, name, run))| (month, (name, run))).collect()
}

/// Why an export that delivered files yields no month: none of them is laid out as [`month_runs`] reads,
/// so the import would otherwise succeed with nothing (named after one of the files, to show the layout).
pub fn unread_layout(blobs: &[(String, i64)], path: &str, export: &str) -> Option<String> {
    let first = blobs.first()?;
    if !month_runs(blobs.to_vec(), path, export).is_empty() {
        return None;
    }
    Some(format!(
        "The export {export} has {} file(s), none in a layout ObserveCraft reads (<export>/<yyyymmdd-yyyymmdd>/[<yyyyMMddHHmm>/]<runId>/manifest.json), e.g. {}",
        blobs.len(),
        first.0
    ))
}

/// The blobs a run's manifest lists.
pub fn manifest_blobs(bytes: &[u8]) -> Result<Vec<String>, String> {
    let v: Value = serde_json::from_slice(bytes).map_err(|e| format!("the manifest isn't JSON: {e}"))?;
    let blobs = v["blobs"].as_array().ok_or("the manifest lists no blobs")?;
    Ok(blobs.iter().filter_map(|b| b["blobName"].as_str()).map(|b| b.trim_start_matches('/').to_string()).collect())
}

/// An export's rows of `dataset` (`actual` or `amortized`), summed per day and kind.
pub fn load(files: &[PathBuf], dataset: &str) -> Result<Vec<BillLine>, String> {
    let source = reader(files)?;
    let conn = super::connection()?;
    let cols = columns(&conn, &source)?;
    let col = |names: &[&str]| names.iter().find_map(|n| cols.get(*n)).map(|(name, _)| ident(name));
    let text = |names: &[&str]| col(names).map_or("''".to_string(), |c| format!("COALESCE(CAST({c} AS VARCHAR), '')"));
    let num = |names: &[&str]| col(names).map_or("CAST(NULL AS DOUBLE)".to_string(), |c| format!("TRY_CAST({c} AS DOUBLE)"));
    let date = text(&["date", "usagedatetime", "usagedate"]);
    let sql = format!(
        "WITH src AS (SELECT CASE WHEN {date} LIKE '__/__/____' THEN strftime(strptime({date}, '%m/%d/%Y'), '%Y-%m-%d') ELSE left({date}, 10) END AS day, \
           {account} AS account, {account_name} AS account_name, lower({group}) AS rg, lower(replace({region}, ' ', '')) AS region, {resource} AS resource, \
           {service} AS service, {subcategory} AS subcategory, {meter} AS meter, {unit} AS unit, {currency} AS currency, {charge} AS charge, {model} AS model, \
           {benefit} AS benefit, {tags} AS tags, COALESCE({quantity}, 0) AS quantity, COALESCE({cost}, 0) AS cost, {payg} AS payg FROM {source}) \
         SELECT day, account, max(account_name), rg, region, resource, service, subcategory, meter, unit, currency, charge, model, benefit, tags, \
           sum(quantity), sum(cost), sum(CASE WHEN payg IS NOT NULL THEN payg * quantity WHEN charge = 'Usage' THEN cost ELSE 0 END) \
         FROM src GROUP BY ALL",
        account = text(&["subscriptionid", "subscriptionguid"]),
        account_name = text(&["subscriptionname"]),
        group = text(&["resourcegroupname", "resourcegroup"]),
        region = text(&["resourcelocation"]),
        resource = text(&["resourceid", "instanceid"]),
        service = text(&["metercategory"]),
        subcategory = text(&["metersubcategory"]),
        meter = text(&["metername"]),
        unit = text(&["unitofmeasure"]),
        currency = text(&["billingcurrency", "billingcurrencycode", "currency"]),
        charge = text(&["chargetype"]),
        model = text(&["pricingmodel"]),
        benefit = text(&["benefitid", "reservationid"]),
        tags = text(&["tags"]),
        quantity = num(&["quantity"]),
        cost = num(&["costinbillingcurrency", "cost", "pretaxcost"]),
        payg = num(&["paygprice"]),
    );
    let amortized = dataset == "amortized";
    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| {
            let s = |i: usize| r.get::<_, String>(i);
            let (account_id, account_name, group, service, subcategory) = (s(1)?, s(2)?, s(3)?, s(6)?, s(7)?);
            let (charge, model, benefit) = (s(11)?, s(12)?, s(13)?);
            let mut tags = tags_from_json(&tags_object(&s(14)?));
            for (key, value) in [("resource_group", &group), ("metercategory", &service), ("metersubcategory", &subcategory)] {
                if !value.is_empty() {
                    tags.insert(key.to_string(), value.clone());
                }
            }
            let (cost, list): (f64, f64) = (r.get(16)?, r.get(17)?);
            let committed = matches!(model.as_str(), "Reservation" | "SavingsPlan") && !benefit.is_empty();
            let status = match charge.as_str() {
                "UnusedReservation" | "UnusedSavingsPlan" => "Unused",
                "Usage" if committed => "Used",
                _ => "",
            };
            Ok(BillLine {
                date: s(0)?,
                account_name: if account_name.is_empty() { account_id.clone() } else { account_name },
                account_id,
                service,
                region: s(4)?,
                zone: String::new(),
                category: category(&charge).to_string(),
                description: s(8)?,
                resource_id: s(5)?,
                pricing_unit: s(9)?,
                quantity: r.get(15)?,
                billed: if amortized { 0.0 } else { cost },
                effective: if amortized { cost } else { 0.0 },
                list: if amortized { list } else { 0.0 },
                pricing_category: match (status, model.as_str()) {
                    ("", "OnDemand") => "Standard",
                    ("", "Spot") => "Dynamic",
                    ("", _) => "",
                    _ => "Committed",
                }
                .to_string(),
                commitment_type: match (status, model.as_str()) {
                    ("", _) => "",
                    (_, "SavingsPlan") => "Savings Plan",
                    _ => "Reserved Instance",
                }
                .to_string(),
                commitment_id: if status.is_empty() { String::new() } else { benefit },
                commitment_status: status.to_string(),
                currency: s(10)?,
                tags,
            })
        })
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())
}

/// The `Tags` column as a JSON object: older datasets leave out the braces (`"env": "prod","team": "x"`).
fn tags_object(tags: &str) -> String {
    let t = tags.trim();
    if t.is_empty() || t.starts_with('{') { t.to_string() } else { format!("{{{t}}}") }
}

/// FOCUS's charge category of a charge type.
fn category(charge: &str) -> &'static str {
    match charge {
        "Purchase" => "purchase",
        "Refund" => "credit",
        "Tax" => "tax",
        "RoundingAdjustment" => "adjustment",
        _ => "usage",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("oc-bills-azure-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn each_month_has_its_latest_run() {
        let blobs = vec![
            ("cost-exports/observecraft-actual/20261001-20261031/run-1/manifest.json".to_string(), 100),
            ("cost-exports/observecraft-actual/20261001-20261031/run-2/manifest.json".to_string(), 200),
            ("cost-exports/observecraft-actual/20261001-20261031/run-2/part_0_0001.csv.gz".to_string(), 200),
            ("cost-exports/observecraft-actual/20261101-20261130/run-3/manifest.json".to_string(), 300),
            ("cost-exports/observecraft-amortized/20261101-20261130/run-4/manifest.json".to_string(), 400),
        ];
        // The path as Datadog's API takes it: with or without the leading slash.
        for path in ["cost-exports", "/cost-exports/"] {
            assert_eq!(
                month_runs(blobs.clone(), path, "observecraft-actual"),
                BTreeMap::from([
                    ("2026-10".to_string(), ("cost-exports/observecraft-actual/20261001-20261031/run-2/manifest.json".to_string(), "run-2".to_string())),
                    ("2026-11".to_string(), ("cost-exports/observecraft-actual/20261101-20261130/run-3/manifest.json".to_string(), "run-3".to_string())),
                ])
            );
        }
    }

    #[test]
    fn files_in_a_layout_we_dont_read_are_reported() {
        let unknown = vec![("p/observecraft-actual/2026/11/run/part_0_0001.csv".to_string(), 1)];
        let message = unread_layout(&unknown, "p", "observecraft-actual").expect("reported");
        assert!(message.contains("p/observecraft-actual/2026/11/run/part_0_0001.csv"), "{message}");
        // Nothing delivered yet, or a layout we read: nothing to report.
        assert_eq!(unread_layout(&[], "p", "observecraft-actual"), None);
        let known = vec![("p/observecraft-actual/20261101-20261130/run-1/manifest.json".to_string(), 1)];
        assert_eq!(unread_layout(&known, "p", "observecraft-actual"), None);
    }

    #[test]
    fn runs_under_a_timestamp_folder_are_read() {
        // Exports of the 2023-07 API put each run under a folder named for its time (yyyyMMddHHmm) inside the month's.
        let base = "actual/ed33eded-52f5-4336-af3e-662fd012d9ab/observecraft-actual/20261101-20261130";
        let blobs = vec![
            (format!("{base}/202611100445/afae5b51-7058-43c0-ae48-52cc296f698f/manifest.json"), 100),
            (format!("{base}/202611100445/afae5b51-7058-43c0-ae48-52cc296f698f/part_0_0001.csv"), 100),
            (format!("{base}/202611110444/e1beaf65-40ce-4563-9a5a-434aeb9c8b44/manifest.json"), 200),
            (format!("{base}/202611110444/e1beaf65-40ce-4563-9a5a-434aeb9c8b44/part_0_0001.csv"), 200),
        ];
        assert_eq!(
            month_runs(blobs, "actual/ed33eded-52f5-4336-af3e-662fd012d9ab", "observecraft-actual"),
            BTreeMap::from([(
                "2026-11".to_string(),
                (format!("{base}/202611110444/e1beaf65-40ce-4563-9a5a-434aeb9c8b44/manifest.json"), "e1beaf65-40ce-4563-9a5a-434aeb9c8b44".to_string())
            )])
        );
    }

    #[test]
    fn exports_without_a_manifest_have_their_latest_file() {
        // The classic exports API: each daily run writes the month to date as one more file of the month's folder.
        let blobs = vec![
            ("cost-exports/observecraft-actual/20261001-20261031/observecraft-actual_0a1b.csv".to_string(), 100),
            ("cost-exports/observecraft-actual/20261001-20261031/observecraft-actual_2c3d.csv".to_string(), 200),
        ];
        assert_eq!(
            month_runs(blobs.clone(), "cost-exports", "observecraft-actual"),
            BTreeMap::from([(
                "2026-10".to_string(),
                ("cost-exports/observecraft-actual/20261001-20261031/observecraft-actual_2c3d.csv".to_string(), "observecraft-actual_2c3d.csv@200".to_string())
            )])
        );
        // A file read while the export was still writing it is read again once it changed: the run is the file
        // and its last modification.
        let mut later = blobs;
        later[1].1 = 300;
        assert_eq!(month_runs(later, "cost-exports", "observecraft-actual")["2026-10"].1, "observecraft-actual_2c3d.csv@300");
    }

    #[test]
    fn manifests_list_their_blobs() {
        let m = r#"{"manifestVersion":"2024-04-01","blobCount":1,"blobs":[{"blobName":"cost-exports/observecraft-actual/20261001-20261031/run-2/part_0_0001.csv.gz","byteCount":10}],"runInfo":{"runId":"run-2"}}"#;
        assert_eq!(manifest_blobs(m.as_bytes()).unwrap(), ["cost-exports/observecraft-actual/20261001-20261031/run-2/part_0_0001.csv.gz"]);
        assert!(manifest_blobs(b"{}").is_err());
    }

    fn csv(name: &str, header: &str, rows: &[&str]) -> PathBuf {
        let path = dir(name).join("part_0_0001.csv");
        std::fs::write(&path, format!("{header}\n{}\n", rows.join("\n"))).unwrap();
        path
    }

    #[test]
    fn the_actual_and_amortized_exports_add_up() {
        // Microsoft Customer Agreement details (camelCase), with a reservation bought on the 1st.
        let header = "date,subscriptionId,subscriptionName,resourceGroupName,resourceLocation,resourceId,meterCategory,meterSubCategory,meterName,quantity,unitOfMeasure,costInBillingCurrency,billingCurrency,payGPrice,chargeType,pricingModel,benefitId,benefitName,tags";
        let actual = csv(
            "actual",
            header,
            &[
                r#"10/01/2026,sub-1,storefront-prod,Storefront-RG,EastUS,/subscriptions/sub-1/resourceGroups/storefront-rg/providers/Microsoft.Compute/virtualMachines/web-1,Virtual Machines,Dv5 Series,D4s v5,24,1 Hour,4.8,USD,0.2,Usage,OnDemand,,,"{""env"": ""prod"", ""team"": ""checkout""}""#,
                r#"10/01/2026,sub-1,storefront-prod,,,,Virtual Machines,Reservation-Base VM,D4s v5,1,1 Each,300,USD,,Purchase,Reservation,/providers/Microsoft.Capacity/reservationOrders/ro-1,vm-reservation,"#,
                r#"10/01/2026,sub-1,storefront-prod,Storefront-RG,EastUS,/subscriptions/sub-1/resourceGroups/storefront-rg/providers/Microsoft.Compute/virtualMachines/web-2,Virtual Machines,Dv5 Series,D4s v5,24,1 Hour,0,USD,0.2,Usage,Reservation,/providers/Microsoft.Capacity/reservationOrders/ro-1,vm-reservation,"""env"": ""prod""""#,
            ],
        );
        let amortized = csv(
            "amortized",
            header,
            &[
                r#"10/01/2026,sub-1,storefront-prod,Storefront-RG,EastUS,/subscriptions/sub-1/resourceGroups/storefront-rg/providers/Microsoft.Compute/virtualMachines/web-1,Virtual Machines,Dv5 Series,D4s v5,24,1 Hour,4.8,USD,0.2,Usage,OnDemand,,,"{""env"": ""prod"", ""team"": ""checkout""}""#,
                r#"10/01/2026,sub-1,storefront-prod,Storefront-RG,EastUS,/subscriptions/sub-1/resourceGroups/storefront-rg/providers/Microsoft.Compute/virtualMachines/web-2,Virtual Machines,Dv5 Series,D4s v5,24,1 Hour,2.9,USD,0.2,Usage,Reservation,/providers/Microsoft.Capacity/reservationOrders/ro-1,vm-reservation,"""env"": ""prod""""#,
                r#"10/01/2026,sub-1,storefront-prod,,,,Virtual Machines,Reservation-Base VM,D4s v5,4,1 Hour,0.6,USD,,UnusedReservation,Reservation,/providers/Microsoft.Capacity/reservationOrders/ro-1,vm-reservation,"#,
            ],
        );
        let mut lines = load(&[actual], "actual").unwrap();
        lines.extend(load(&[amortized], "amortized").unwrap());
        let billed: f64 = lines.iter().map(|l| l.billed).sum();
        let effective: f64 = lines.iter().map(|l| l.effective).sum();
        assert!((billed - 304.8).abs() < 1e-9 && (effective - 8.3).abs() < 1e-9, "{billed} {effective}");
        let web1: Vec<&BillLine> = lines.iter().filter(|l| l.resource_id.ends_with("web-1")).collect();
        assert_eq!(web1.len(), 2);
        let a = web1.iter().find(|l| l.billed > 0.0).unwrap();
        assert_eq!((a.date.as_str(), a.account_id.as_str(), a.account_name.as_str(), a.service.as_str(), a.region.as_str()), ("2026-10-01", "sub-1", "storefront-prod", "Virtual Machines", "eastus"));
        assert_eq!((a.pricing_category.as_str(), a.category.as_str(), a.effective, a.list), ("Standard", "usage", 0.0, 0.0));
        assert_eq!(a.tags.get("team").map(String::as_str), Some("checkout"));
        assert_eq!(a.tags.get("resource_group").map(String::as_str), Some("storefront-rg"));
        let m = web1.iter().find(|l| l.effective > 0.0).unwrap();
        assert!((m.list - 4.8).abs() < 1e-9 && m.billed == 0.0);
        let covered = lines.iter().find(|l| l.resource_id.ends_with("web-2") && l.effective > 0.0).unwrap();
        assert_eq!((covered.commitment_type.as_str(), covered.commitment_status.as_str(), covered.pricing_category.as_str()), ("Reserved Instance", "Used", "Committed"));
        assert_eq!(covered.tags.get("env").map(String::as_str), Some("prod"));
        let unused = lines.iter().find(|l| l.commitment_status == "Unused").unwrap();
        assert!((unused.effective - 0.6).abs() < 1e-9);
        assert_eq!(lines.iter().find(|l| l.category == "purchase").map(|l| l.billed), Some(300.0));
    }

    #[test]
    fn enterprise_agreement_details_spell_their_columns_otherwise() {
        let path = csv(
            "ea",
            "Date,SubscriptionGuid,SubscriptionName,ResourceGroup,ResourceLocation,InstanceId,MeterCategory,Quantity,UnitOfMeasure,CostInBillingCurrency,BillingCurrencyCode,ChargeType,PricingModel,Tags",
            &[r#"2026-10-02,sub-2,catalog-dev,catalog,westeurope,/subscriptions/sub-2/x,Storage,10,1 GB/Month,1.25,USD,Usage,Spot,"#],
        );
        let lines = load(&[path], "actual").unwrap();
        assert_eq!(lines.len(), 1);
        let l = &lines[0];
        assert_eq!((l.date.as_str(), l.account_id.as_str(), l.service.as_str(), l.region.as_str(), l.pricing_category.as_str(), l.billed), ("2026-10-02", "sub-2", "Storage", "westeurope", "Dynamic", 1.25));
    }
}
