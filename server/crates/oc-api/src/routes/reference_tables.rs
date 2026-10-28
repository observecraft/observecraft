//! Reference tables (Integrations → Reference Tables): CSV files users upload to look values up
//! by a primary key column. The browser parses the file and sends its header and rows; the
//! server checks them again, stores one row per key and replaces every row on a new upload.
//! Anyone in the org reads them; standard users and admins create, replace and delete them.

use std::collections::{HashMap, HashSet};

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use utoipa::{IntoParams, ToSchema};

use crate::auth::{AuthContext, Writer};
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

const MAX_TABLES: i64 = 100;
const MAX_NAME_CHARS: usize = 100;
const MAX_DESCRIPTION_CHARS: usize = 500;
const MAX_FILE_NAME_CHARS: usize = 255;
const MAX_COLUMN_CHARS: usize = 100;
const MAX_COLUMNS: usize = 50;
const MAX_ROWS: usize = 100_000;
/// Every value of the file together (the browser checks the file's own size first).
const MAX_FILE_BYTES: usize = 10 * 1024 * 1024;
/// Request bodies of uploads: the rows as JSON arrays take more room than the CSV did.
pub const MAX_BODY_BYTES: usize = 4 * MAX_FILE_BYTES;
const DEFAULT_PAGE: i64 = 50;
const MAX_PAGE: i64 = 500;

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ReferenceTable {
    /// Lowercase letters, digits and underscores; it identifies the table and can't be changed.
    pub name: String,
    pub description: String,
    /// Where the rows come from: `local_file` (a CSV uploaded from the browser).
    pub source: String,
    /// Name of the last uploaded file.
    pub file_name: String,
    /// The file's header, in file order.
    pub columns: Vec<String>,
    /// The column whose values identify rows (unique, never empty).
    pub primary_key: String,
    pub row_count: i64,
    /// Names of the users who created it and last uploaded a file.
    pub created_by: Option<String>,
    pub updated_by: Option<String>,
    /// Epoch milliseconds.
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ReferenceTableInput {
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub primary_key: String,
    #[serde(default)]
    pub file_name: String,
    /// The CSV header.
    pub columns: Vec<String>,
    /// The CSV rows after the header, each with one value per column.
    pub rows: Vec<Vec<String>>,
}

/// A new file for an existing table; its primary key column stays the same.
#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ReferenceTableFile {
    #[serde(default)]
    pub file_name: String,
    pub columns: Vec<String>,
    pub rows: Vec<Vec<String>>,
}

#[derive(Deserialize, IntoParams)]
pub struct ReferenceTableRowsParams {
    /// Case-insensitive text the primary key or a value contains.
    pub query: Option<String>,
    /// Search only this column (default: every column).
    pub column: Option<String>,
    /// Default 50, at most 500.
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

#[derive(Serialize, ToSchema)]
pub struct ReferenceTableRows {
    pub table: ReferenceTable,
    /// The matching rows in file order, each with one value per column of `table.columns`.
    pub rows: Vec<Vec<String>>,
    /// How many rows match.
    pub total: i64,
}

// ================================================================ validation

fn check_name(name: &str) -> Result<(), String> {
    let valid = (1..=MAX_NAME_CHARS).contains(&name.len()) && name.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_');
    if valid {
        Ok(())
    } else {
        Err(format!("The name must be 1–{MAX_NAME_CHARS} characters: lowercase letters, digits and underscores"))
    }
}

/// Trims the column names and checks the file: a header of unique names, the primary key among
/// them, at least one row, every row as wide as the header, and unique, non-empty keys.
fn check_file(columns: &mut [String], rows: &[Vec<String>], primary_key: &str) -> Result<(), String> {
    for c in columns.iter_mut() {
        *c = c.trim().to_string();
    }
    if columns.is_empty() {
        return Err("The file has no columns: its first line must name them".into());
    }
    if columns.len() > MAX_COLUMNS {
        return Err(format!("A reference table has at most {MAX_COLUMNS} columns"));
    }
    let mut seen = HashSet::new();
    for (i, c) in columns.iter().enumerate() {
        if c.is_empty() {
            return Err(format!("Column {} has no name", i + 1));
        }
        if c.chars().count() > MAX_COLUMN_CHARS {
            return Err(format!("Column names are at most {MAX_COLUMN_CHARS} characters"));
        }
        if !seen.insert(c.as_str()) {
            return Err(format!("Column '{c}' appears twice"));
        }
    }
    let Some(pk) = columns.iter().position(|c| c == primary_key) else {
        return Err(format!("Primary key '{primary_key}' is not a column of the file"));
    };
    if rows.is_empty() {
        return Err("The file has no rows after its header".into());
    }
    if rows.len() > MAX_ROWS {
        return Err("A reference table has at most 100,000 rows".into());
    }
    let mut bytes = 0;
    let mut keys: HashMap<&str, usize> = HashMap::with_capacity(rows.len());
    for (i, row) in rows.iter().enumerate() {
        if row.len() != columns.len() {
            return Err(format!("Row {} has {} values, expected {}", i + 1, row.len(), columns.len()));
        }
        bytes += row.iter().map(String::len).sum::<usize>();
        if bytes > MAX_FILE_BYTES {
            return Err("The file is larger than 10 MiB".into());
        }
        let key = row[pk].as_str();
        if key.trim().is_empty() {
            return Err(format!("Row {} has an empty primary key ({primary_key})", i + 1));
        }
        if let Some(first) = keys.insert(key, i) {
            return Err(format!("Primary key value '{key}' appears twice (rows {} and {})", first + 1, i + 1));
        }
    }
    Ok(())
}

/// Trims and bounds the description and the file name.
fn check_text(description: &mut String, file_name: &mut String) -> Result<(), String> {
    *description = description.trim().to_string();
    *file_name = file_name.trim().to_string();
    if description.chars().count() > MAX_DESCRIPTION_CHARS {
        return Err(format!("The description is at most {MAX_DESCRIPTION_CHARS} characters"));
    }
    if file_name.chars().count() > MAX_FILE_NAME_CHARS {
        return Err(format!("The file name is at most {MAX_FILE_NAME_CHARS} characters"));
    }
    Ok(())
}

/// An ILIKE pattern matching values that contain the search literally; None for no search.
fn like_pattern(query: &str) -> Option<String> {
    let q = query.trim();
    if q.is_empty() {
        return None;
    }
    let mut out = String::with_capacity(q.len() + 2);
    out.push('%');
    for c in q.chars() {
        if matches!(c, '%' | '_' | '\\') {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('%');
    Some(out)
}

/// (limit, offset) of a rows page.
fn page(limit: Option<i64>, offset: Option<i64>) -> (i64, i64) {
    (limit.unwrap_or(DEFAULT_PAGE).clamp(1, MAX_PAGE), offset.unwrap_or(0).max(0))
}

/// A stored row's values in column order ('' for a column it lacks).
fn row_values(columns: &[String], data: &Value) -> Vec<String> {
    columns.iter().map(|c| data.get(c).and_then(Value::as_str).unwrap_or_default().to_string()).collect()
}

/// A file row as stored: its values by column name.
fn row_object(columns: &[String], row: &[String]) -> Value {
    Value::Object(columns.iter().cloned().zip(row.iter().map(|v| Value::String(v.clone()))).collect::<Map<_, _>>())
}

// ================================================================ storage

#[derive(sqlx::FromRow)]
struct Row {
    id: String,
    name: String,
    description: String,
    file_name: String,
    columns: Value,
    primary_key: String,
    row_count: i32,
    created_by: Option<String>,
    updated_by: Option<String>,
    created_at: i64,
    updated_at: i64,
}

impl From<Row> for ReferenceTable {
    fn from(r: Row) -> Self {
        ReferenceTable {
            name: r.name,
            description: r.description,
            source: "local_file".into(),
            file_name: r.file_name,
            columns: serde_json::from_value(r.columns).unwrap_or_default(),
            primary_key: r.primary_key,
            row_count: r.row_count.into(),
            created_by: r.created_by,
            updated_by: r.updated_by,
            created_at: r.created_at,
            updated_at: r.updated_at,
        }
    }
}

const SELECT: &str = "SELECT t.id::text AS id, t.name, t.description, t.file_name, t.columns, t.primary_key, t.row_count, \
    c.name AS created_by, u.name AS updated_by, \
    (extract(epoch FROM t.created_at) * 1000)::bigint AS created_at, (extract(epoch FROM t.updated_at) * 1000)::bigint AS updated_at \
    FROM reference_tables t LEFT JOIN users c ON c.id = t.created_by LEFT JOIN users u ON u.id = t.updated_by";

async fn get_table(st: &AppState, org: &str, name: &str) -> Result<Row, ApiError> {
    let row: Option<Row> = sqlx::query_as(&format!("{SELECT} WHERE t.org_id = $1 AND t.name = $2")).bind(org).bind(name).fetch_optional(&st.postgres).await?;
    row.ok_or_else(|| ApiError::NotFound(format!("Reference table '{name}' not found")))
}

/// Writes every row of a file: one statement over a JSON array of the row objects.
async fn insert_rows(tx: &mut sqlx::Transaction<'_, sqlx::Postgres>, org: &str, id: &str, columns: &[String], rows: &[Vec<String>], primary_key: &str) -> Result<(), ApiError> {
    let objects = Value::Array(rows.iter().map(|r| row_object(columns, r)).collect());
    sqlx::query(
        "INSERT INTO reference_table_rows (org_id, table_id, key, position, data) \
         SELECT $1, $2::uuid, e.value->>$3, (e.ord - 1)::int, e.value FROM jsonb_array_elements($4::jsonb) WITH ORDINALITY AS e(value, ord)",
    )
    .bind(org)
    .bind(id)
    .bind(primary_key)
    .bind(objects)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

fn unique(e: sqlx::Error, name: &str) -> ApiError {
    match &e {
        sqlx::Error::Database(d) if d.is_unique_violation() => ApiError::BadRequest(format!("A reference table named '{name}' already exists")),
        _ => e.into(),
    }
}

/// Rows matching the search: `$3` is the ILIKE pattern (NULL: every row), `$4` the one column to search (NULL: all).
const ROWS_WHERE: &str = "WHERE r.org_id = $1 AND r.table_id = $2::uuid AND ($3::text IS NULL \
    OR ($4::text IS NULL AND (r.key ILIKE $3 OR EXISTS (SELECT 1 FROM jsonb_each_text(r.data) v WHERE v.value ILIKE $3))) \
    OR ($4::text IS NOT NULL AND r.data->>$4 ILIKE $3))";

// ================================================================ handlers

#[utoipa::path(get, path = "/api/v1/reference-tables", operation_id = "list_reference_tables", responses((status = 200, body = Vec<ReferenceTable>)), tag = "integrations")]
pub async fn list(State(st): State<AppState>, ctx: AuthContext) -> ApiResult<Vec<ReferenceTable>> {
    let rows: Vec<Row> = sqlx::query_as(&format!("{SELECT} WHERE t.org_id = $1 ORDER BY t.name")).bind(&ctx.org_id).fetch_all(&st.postgres).await?;
    Ok(Json(rows.into_iter().map(Into::into).collect()))
}

/// Creates a table from an uploaded file.
#[utoipa::path(post, path = "/api/v1/reference-tables", operation_id = "create_reference_table", request_body = ReferenceTableInput,
    responses((status = 200, body = ReferenceTable), (status = 400, body = crate::error::ErrorBody)), tag = "integrations")]
pub async fn create(State(st): State<AppState>, Writer(ctx): Writer, Json(mut input): Json<ReferenceTableInput>) -> ApiResult<ReferenceTable> {
    input.name = input.name.trim().to_string();
    input.primary_key = input.primary_key.trim().to_string();
    check_name(&input.name).map_err(ApiError::BadRequest)?;
    check_text(&mut input.description, &mut input.file_name).map_err(ApiError::BadRequest)?;
    check_file(&mut input.columns, &input.rows, &input.primary_key).map_err(ApiError::BadRequest)?;
    let (count,): (i64,) = sqlx::query_as("SELECT count(*) FROM reference_tables WHERE org_id = $1").bind(&ctx.org_id).fetch_one(&st.postgres).await?;
    if count >= MAX_TABLES {
        return Err(ApiError::BadRequest(format!("An organization can have at most {MAX_TABLES} reference tables")));
    }
    let mut tx = st.postgres.begin().await?;
    let (id,): (String,) = sqlx::query_as(
        "INSERT INTO reference_tables (org_id, name, description, columns, primary_key, file_name, row_count, created_by, updated_by) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8::uuid, $8::uuid) RETURNING id::text",
    )
    .bind(&ctx.org_id)
    .bind(&input.name)
    .bind(&input.description)
    .bind(serde_json::to_value(&input.columns).unwrap_or_default())
    .bind(&input.primary_key)
    .bind(&input.file_name)
    .bind(input.rows.len() as i32)
    .bind(&ctx.user_id)
    .fetch_one(&mut *tx)
    .await
    .map_err(|e| unique(e, &input.name))?;
    insert_rows(&mut tx, &ctx.org_id, &id, &input.columns, &input.rows, &input.primary_key).await?;
    tx.commit().await?;
    ctx.audit(&st, "reference_table.created", &input.name, &format!("{} rows from {}", input.rows.len(), input.file_name)).await;
    Ok(Json(get_table(&st, &ctx.org_id, &input.name).await?.into()))
}

/// The table and a page of its rows (in file order), optionally searched.
#[utoipa::path(get, path = "/api/v1/reference-tables/{name}", operation_id = "get_reference_table", params(("name" = String, Path), ReferenceTableRowsParams),
    responses((status = 200, body = ReferenceTableRows), (status = 400, body = crate::error::ErrorBody), (status = 404, body = crate::error::ErrorBody)), tag = "integrations")]
pub async fn get_one(State(st): State<AppState>, ctx: AuthContext, Path(name): Path<String>, Query(p): Query<ReferenceTableRowsParams>) -> ApiResult<ReferenceTableRows> {
    let row = get_table(&st, &ctx.org_id, &name).await?;
    let id = row.id.clone();
    let table: ReferenceTable = row.into();
    let column = p.column.map(|c| c.trim().to_string()).filter(|c| !c.is_empty());
    if let Some(c) = &column {
        if !table.columns.contains(c) {
            return Err(ApiError::BadRequest(format!("The table has no column '{c}'")));
        }
    }
    let pattern = p.query.as_deref().and_then(like_pattern);
    let (limit, offset) = page(p.limit, p.offset);
    let (total,): (i64,) =
        sqlx::query_as(&format!("SELECT count(*) FROM reference_table_rows r {ROWS_WHERE}")).bind(&ctx.org_id).bind(&id).bind(&pattern).bind(&column).fetch_one(&st.postgres).await?;
    let data: Vec<(Value,)> = sqlx::query_as(&format!("SELECT r.data FROM reference_table_rows r {ROWS_WHERE} ORDER BY r.position LIMIT $5 OFFSET $6"))
        .bind(&ctx.org_id)
        .bind(&id)
        .bind(&pattern)
        .bind(&column)
        .bind(limit)
        .bind(offset)
        .fetch_all(&st.postgres)
        .await?;
    let rows = data.iter().map(|(d,)| row_values(&table.columns, d)).collect();
    Ok(Json(ReferenceTableRows { table, rows, total }))
}

/// Replaces every row with those of a new file, which must have the table's primary key column.
#[utoipa::path(put, path = "/api/v1/reference-tables/{name}", operation_id = "replace_reference_table", params(("name" = String, Path)), request_body = ReferenceTableFile,
    responses((status = 200, body = ReferenceTable), (status = 400, body = crate::error::ErrorBody), (status = 404, body = crate::error::ErrorBody)), tag = "integrations")]
pub async fn replace(State(st): State<AppState>, Writer(ctx): Writer, Path(name): Path<String>, Json(mut file): Json<ReferenceTableFile>) -> ApiResult<ReferenceTable> {
    let mut tx = st.postgres.begin().await?;
    let current: Option<(String, String)> =
        sqlx::query_as("SELECT id::text, primary_key FROM reference_tables WHERE org_id = $1 AND name = $2 FOR UPDATE").bind(&ctx.org_id).bind(&name).fetch_optional(&mut *tx).await?;
    let Some((id, primary_key)) = current else { return Err(ApiError::NotFound(format!("Reference table '{name}' not found"))) };
    check_text(&mut String::new(), &mut file.file_name).map_err(ApiError::BadRequest)?;
    check_file(&mut file.columns, &file.rows, &primary_key).map_err(ApiError::BadRequest)?;
    sqlx::query("DELETE FROM reference_table_rows WHERE org_id = $1 AND table_id = $2::uuid").bind(&ctx.org_id).bind(&id).execute(&mut *tx).await?;
    insert_rows(&mut tx, &ctx.org_id, &id, &file.columns, &file.rows, &primary_key).await?;
    sqlx::query("UPDATE reference_tables SET columns = $3, file_name = $4, row_count = $5, updated_by = $6::uuid, updated_at = now() WHERE org_id = $1 AND id = $2::uuid")
        .bind(&ctx.org_id)
        .bind(&id)
        .bind(serde_json::to_value(&file.columns).unwrap_or_default())
        .bind(&file.file_name)
        .bind(file.rows.len() as i32)
        .bind(&ctx.user_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    ctx.audit(&st, "reference_table.replaced", &name, &format!("{} rows from {}", file.rows.len(), file.file_name)).await;
    Ok(Json(get_table(&st, &ctx.org_id, &name).await?.into()))
}

#[utoipa::path(delete, path = "/api/v1/reference-tables/{name}", operation_id = "delete_reference_table", params(("name" = String, Path)),
    responses((status = 204), (status = 404, body = crate::error::ErrorBody)), tag = "integrations")]
pub async fn delete(State(st): State<AppState>, Writer(ctx): Writer, Path(name): Path<String>) -> Result<StatusCode, ApiError> {
    let row: Option<(i32,)> = sqlx::query_as("DELETE FROM reference_tables WHERE org_id = $1 AND name = $2 RETURNING row_count").bind(&ctx.org_id).bind(&name).fetch_optional(&st.postgres).await?;
    let Some((rows,)) = row else { return Err(ApiError::NotFound(format!("Reference table '{name}' not found"))) };
    ctx.audit(&st, "reference_table.deleted", &name, &format!("{rows} rows")).await;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cols(names: &[&str]) -> Vec<String> {
        names.iter().map(|c| c.to_string()).collect()
    }

    fn rows(rows: &[&[&str]]) -> Vec<Vec<String>> {
        rows.iter().map(|r| cols(r)).collect()
    }

    #[test]
    fn names_are_lowercase_letters_digits_and_underscores() {
        for ok in ["products", "service_owners_2", "9lives"] {
            assert_eq!(check_name(ok), Ok(()), "{ok}");
        }
        for bad in ["", "Products", "with-dash", "with space", "é", &"x".repeat(MAX_NAME_CHARS + 1)] {
            assert!(check_name(bad).unwrap_err().contains("lowercase letters, digits and underscores"), "{bad}");
        }
    }

    #[test]
    fn a_valid_file_passes_and_column_names_are_trimmed() {
        let mut c = cols(&[" sku ", "name"]);
        assert_eq!(check_file(&mut c, &rows(&[&["SKU-1", "Mug"], &["SKU-2", "Plate"]]), "sku"), Ok(()));
        assert_eq!(c, cols(&["sku", "name"]));
    }

    #[test]
    fn files_are_rejected_with_clear_errors() {
        let ok = rows(&[&["SKU-1", "Mug"], &["SKU-2", "Plate"]]);
        let check = |c: &[&str], r: &[Vec<String>], pk: &str| check_file(&mut cols(c), r, pk).unwrap_err();
        assert!(check(&[], &[], "sku").contains("no columns"));
        assert!(check(&["sku", "name"], &[], "sku").contains("no rows"));
        assert!(check(&["sku", " "], &ok, "sku").contains("Column 2 has no name"));
        assert!(check(&["sku", "sku"], &ok, "sku").contains("Column 'sku' appears twice"));
        assert!(check(&["sku", "name"], &ok, "id").contains("Primary key 'id' is not a column"));
        assert!(check(&["sku", "name"], &rows(&[&["SKU-1", "Mug"], &["SKU-2"]]), "sku").contains("Row 2 has 1 values, expected 2"));
        assert!(check(&["sku", "name"], &rows(&[&["SKU-1", "Mug"], &[" ", "Plate"]]), "sku").contains("Row 2 has an empty primary key (sku)"));
        let dup = rows(&[&["SKU-1", "Mug"], &["SKU-2", "Cup"], &["SKU-1", "Plate"]]);
        assert!(check(&["sku", "name"], &dup, "sku").contains("Primary key value 'SKU-1' appears twice (rows 1 and 3)"));
        let wide: Vec<String> = (0..=MAX_COLUMNS).map(|i| format!("c{i}")).collect();
        let wide: Vec<&str> = wide.iter().map(String::as_str).collect();
        assert!(check(&wide, &[], "c0").contains("at most 50 columns"));
        let long: Vec<Vec<String>> = (0..=MAX_ROWS).map(|i| vec![format!("k{i}"), "v".into()]).collect();
        assert!(check(&["sku", "name"], &long, "sku").contains("at most 100,000 rows"));
        let big = vec![vec!["k".to_string(), "v".repeat(MAX_FILE_BYTES)]];
        assert!(check(&["sku", "name"], &big, "sku").contains("larger than 10 MiB"));
    }

    #[test]
    fn searches_match_literally() {
        assert_eq!(like_pattern("  "), None);
        assert_eq!(like_pattern(" sku-1 "), Some("%sku-1%".into()));
        assert_eq!(like_pattern(r"50%_off\"), Some(r"%50\%\_off\\%".into()));
    }

    #[test]
    fn pages_are_bounded() {
        assert_eq!(page(None, None), (DEFAULT_PAGE, 0));
        assert_eq!(page(Some(0), Some(-5)), (1, 0));
        assert_eq!(page(Some(10_000), Some(20)), (MAX_PAGE, 20));
    }

    #[test]
    fn rows_follow_the_column_order() {
        let data = serde_json::json!({ "name": "Mug", "sku": "SKU-1" });
        assert_eq!(row_values(&cols(&["sku", "name", "missing"]), &data), cols(&["SKU-1", "Mug", ""]));
        let c = cols(&["sku", "name"]);
        assert_eq!(row_object(&c, &cols(&["SKU-1", "Mug"])), data);
    }
}
