//! Cloud Cost Management from the providers' bills: the AWS Cost and Usage Reports and Azure cost exports
//! the `cost_bill_configs` point at (`routes::datadog_api::cost_bills`), read into `otel.cloud_costs`.

pub mod aws;
pub mod azure;
pub mod import;

use std::collections::BTreeMap;

/// A bill's line items of one day and kind (account, service, region, resource, charge, commitment and
/// tags), summed: what becomes a FOCUS row of `otel.cloud_costs`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BillLine {
    /// `YYYY-MM-DD`
    pub date: String,
    pub account_id: String,
    /// The account's name, or its id when the bill doesn't name it.
    pub account_name: String,
    pub service: String,
    pub region: String,
    pub zone: String,
    /// usage, purchase, tax, credit or adjustment.
    pub category: String,
    pub description: String,
    pub resource_id: String,
    pub pricing_unit: String,
    pub quantity: f64,
    pub billed: f64,
    pub effective: f64,
    pub list: f64,
    /// Standard, Committed, Dynamic or empty.
    pub pricing_category: String,
    pub commitment_id: String,
    /// `Savings Plan`, `Reserved Instance`, …
    pub commitment_type: String,
    /// Used or Unused.
    pub commitment_status: String,
    pub currency: String,
    pub tags: BTreeMap<String, String>,
}

/// A column's name as every format of a bill spells it: lower case, letters and digits only
/// (`lineItem/UnblendedCost`, `line_item_unblended_cost` → `lineitemunblendedcost`).
pub fn canonical(name: &str) -> String {
    name.chars().filter(char::is_ascii_alphanumeric).map(|c| c.to_ascii_lowercase()).collect()
}

/// A SQL string literal.
pub fn quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

/// A SQL identifier.
pub fn ident(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "\"\""))
}

/// DuckDB's reader over downloaded bill files: Parquet, or CSV (gzip or not), every column of CSV as text.
pub fn reader(files: &[std::path::PathBuf]) -> Result<String, String> {
    let names: Vec<String> = files.iter().map(|f| f.to_string_lossy().to_string()).collect();
    if let Some(zip) = names.iter().find(|n| n.ends_with(".zip")) {
        return Err(format!("{zip}: ZIP files aren't read, deliver the report as GZIP CSV or Parquet"));
    }
    let list = format!("[{}]", names.iter().map(|n| quote(n)).collect::<Vec<_>>().join(", "));
    if names.iter().all(|n| n.ends_with(".parquet")) {
        Ok(format!("read_parquet({list}, union_by_name = true)"))
    } else {
        Ok(format!("read_csv({list}, union_by_name = true, header = true, all_varchar = true)"))
    }
}

/// The source's columns: canonical name → (name, type).
pub fn columns(conn: &duckdb::Connection, source: &str) -> Result<BTreeMap<String, (String, String)>, String> {
    let mut stmt = conn.prepare(&format!("DESCRIBE SELECT * FROM {source}")).map_err(|e| e.to_string())?;
    let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))).map_err(|e| e.to_string())?;
    let mut out = BTreeMap::new();
    for row in rows {
        let (name, kind) = row.map_err(|e| e.to_string())?;
        out.entry(canonical(&name)).or_insert((name, kind));
    }
    Ok(out)
}

/// A bill's tags as `{key: value}` JSON (keys as the bill names them), empty values dropped and the
/// `user:` / `user_` prefix of user-defined cost allocation tags removed.
pub fn tags_from_json(text: &str) -> BTreeMap<String, String> {
    let v: serde_json::Value = serde_json::from_str(text).unwrap_or_default();
    let Some(m) = v.as_object() else { return BTreeMap::new() };
    m.iter()
        .filter_map(|(k, v)| {
            let v = v.as_str()?.trim();
            let k = k.strip_prefix("user:").or_else(|| k.strip_prefix("user_")).unwrap_or(k).trim();
            (!v.is_empty() && !k.is_empty()).then(|| (k.to_ascii_lowercase(), v.to_string()))
        })
        .collect()
}
