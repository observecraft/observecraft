//! Minimal S3 client for log archives: presigned requests (rusty-s3, SigV4) sent with reqwest.
//! Works with AWS S3 and S3-compatible stores (RustFS, Ceph, R2, …).

use std::time::Duration;

use rusty_s3::actions::{ListObjectsV2, S3Action};
use rusty_s3::{Bucket, Credentials, UrlStyle};

use crate::error::ApiError;

/// Presigned URLs live just long enough for one request.
const SIGN_TTL: Duration = Duration::from_secs(600);

pub struct Store {
    bucket: Bucket,
    creds: Credentials,
    http: reqwest::Client,
}

#[derive(Debug, Clone)]
pub struct Object {
    pub key: String,
    pub size: u64,
}

fn s3_error(what: &str, e: impl std::fmt::Display) -> ApiError {
    ApiError::BadRequest(format!("{what}: {e}"))
}

async fn check(res: reqwest::Response, what: &str) -> Result<reqwest::Response, ApiError> {
    if res.status().is_success() {
        return Ok(res);
    }
    let status = res.status();
    let body = res.text().await.unwrap_or_default();
    // S3 errors are XML: <Error><Code>NoSuchBucket</Code><Message>…</Message></Error>.
    let code = body.split("<Code>").nth(1).and_then(|s| s.split("</Code>").next()).unwrap_or("");
    let message = body.split("<Message>").nth(1).and_then(|s| s.split("</Message>").next()).unwrap_or("");
    Err(ApiError::BadRequest(format!("{what} failed ({status}{}{})", if code.is_empty() { "" } else { ", " }, [code, message].iter().filter(|s| !s.is_empty()).copied().collect::<Vec<_>>().join(": "))))
}

impl Store {
    pub fn new(endpoint: &str, region: &str, bucket: &str, path_style: bool, access_key: &str, secret_key: &str) -> Result<Store, ApiError> {
        let url = endpoint.parse().map_err(|e| s3_error("Invalid endpoint URL", e))?;
        let style = if path_style { UrlStyle::Path } else { UrlStyle::VirtualHost };
        let bucket = Bucket::new(url, style, bucket.to_string(), region.to_string()).map_err(|e| s3_error("Invalid bucket", e))?;
        let http = reqwest::Client::builder().timeout(Duration::from_secs(120)).build().map_err(|e| ApiError::Internal(e.into()))?;
        Ok(Store { bucket, creds: Credentials::new(access_key, secret_key), http })
    }

    pub async fn put(&self, key: &str, body: Vec<u8>, content_type: &str) -> Result<(), ApiError> {
        let url = self.bucket.put_object(Some(&self.creds), key).sign(SIGN_TTL);
        let res = self.http.put(url).header(reqwest::header::CONTENT_TYPE, content_type).body(body).send().await.map_err(|e| s3_error("Upload", e))?;
        check(res, "Upload").await.map(|_| ())
    }

    pub async fn get(&self, key: &str) -> Result<Vec<u8>, ApiError> {
        let url = self.bucket.get_object(Some(&self.creds), key).sign(SIGN_TTL);
        let res = self.http.get(url).send().await.map_err(|e| s3_error("Download", e))?;
        Ok(check(res, "Download").await?.bytes().await.map_err(|e| s3_error("Download", e))?.to_vec())
    }

    pub async fn delete(&self, key: &str) -> Result<(), ApiError> {
        let url = self.bucket.delete_object(Some(&self.creds), key).sign(SIGN_TTL);
        let res = self.http.delete(url).send().await.map_err(|e| s3_error("Delete", e))?;
        check(res, "Delete").await.map(|_| ())
    }

    /// Every object under `prefix` (follows continuation tokens).
    pub async fn list(&self, prefix: &str) -> Result<Vec<Object>, ApiError> {
        let mut out = Vec::new();
        let mut token: Option<String> = None;
        loop {
            let mut action: ListObjectsV2<'_> = self.bucket.list_objects_v2(Some(&self.creds));
            action.with_prefix(prefix);
            if let Some(t) = &token {
                action.with_continuation_token(t.clone());
            }
            let url = action.sign(SIGN_TTL);
            let res = self.http.get(url).send().await.map_err(|e| s3_error("List", e))?;
            let text = check(res, "List").await?.text().await.map_err(|e| s3_error("List", e))?;
            let parsed = ListObjectsV2::parse_response(&text).map_err(|e| s3_error("List response", e))?;
            out.extend(parsed.contents.into_iter().map(|c| Object { key: c.key, size: c.size }));
            match parsed.next_continuation_token {
                Some(t) if !t.is_empty() => token = Some(t),
                _ => break,
            }
        }
        Ok(out)
    }

    /// Creates the bucket (development setup); an existing bucket owned by us is fine.
    pub async fn ensure_bucket(&self) -> Result<(), ApiError> {
        let head = self.bucket.head_bucket(Some(&self.creds)).sign(SIGN_TTL);
        if self.http.head(head).send().await.map(|r| r.status().is_success()).unwrap_or(false) {
            return Ok(());
        }
        let url = self.bucket.create_bucket(&self.creds).sign(SIGN_TTL);
        let res = self.http.put(url).send().await.map_err(|e| s3_error("Create bucket", e))?;
        check(res, "Create bucket").await.map(|_| ())
    }

    /// Proves read/write access: writes, reads back and deletes a marker object.
    pub async fn probe(&self, prefix: &str) -> Result<(), ApiError> {
        let key = format!("{}.observecraft-write-test-{}", prefix_dir(prefix), crate::auth::crypto::random_token(""));
        self.put(&key, b"observecraft archive access check".to_vec(), "text/plain").await?;
        let back = self.get(&key).await?;
        self.delete(&key).await?;
        if back != b"observecraft archive access check" {
            return Err(ApiError::BadRequest("The bucket returned different content than was written".into()));
        }
        Ok(())
    }
}

/// `logs/` style prefix (empty stays empty).
pub fn prefix_dir(prefix: &str) -> String {
    let p = prefix.trim_matches('/');
    if p.is_empty() {
        String::new()
    } else {
        format!("{p}/")
    }
}
