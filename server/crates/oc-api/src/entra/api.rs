//! Microsoft Graph as the Entra ID logs poller reads it: the three streams (sign-ins, directory audits, risk
//! detections), their time-filtered list paths (pure, tested below) and [`GraphApi`], implemented by the Azure
//! integration's client (`azure::api::Rest::graph_get`: the app registration's client credentials, a Graph token).

use serde_json::Value;

use crate::azure::api::{AzureError, Credentials, Rest};

/// A log stream of the tenant, one poller job each (its scope).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stream {
    /// `auditLogs/signIns`: interactive user sign-ins (Entra ID P1 or P2).
    SignIns,
    /// `auditLogs/directoryAudits`: changes to the directory (any license).
    Audits,
    /// `identityProtection/riskDetections`: Identity Protection's detections (Entra ID P2).
    Risk,
}

pub const STREAMS: [Stream; 3] = [Stream::SignIns, Stream::Audits, Stream::Risk];

impl Stream {
    pub fn scope(self) -> &'static str {
        match self {
            Stream::SignIns => "signins",
            Stream::Audits => "audits",
            Stream::Risk => "risk",
        }
    }

    pub fn of_scope(scope: &str) -> Option<Stream> {
        STREAMS.into_iter().find(|s| s.scope() == scope)
    }

    fn path(self) -> &'static str {
        match self {
            Stream::SignIns => "/auditLogs/signIns",
            Stream::Audits => "/auditLogs/directoryAudits",
            Stream::Risk => "/identityProtection/riskDetections",
        }
    }

    /// The event's time, which the list filters on.
    pub fn time_field(self) -> &'static str {
        match self {
            Stream::SignIns => "createdDateTime",
            Stream::Audits => "activityDateTime",
            Stream::Risk => "detectedDateTime",
        }
    }

    /// The application permission (Microsoft Graph) reading it needs.
    pub fn permission(self) -> &'static str {
        match self {
            Stream::SignIns | Stream::Audits => "AuditLog.Read.All",
            Stream::Risk => "IdentityRiskEvent.Read.All",
        }
    }

    /// The license reading it needs, when one does.
    pub fn license(self) -> Option<&'static str> {
        match self {
            Stream::SignIns => Some("Microsoft Entra ID P1 or P2"),
            Stream::Audits => None,
            Stream::Risk => Some("Microsoft Entra ID P2"),
        }
    }

    /// Events per page: Graph's maximum.
    fn top(self) -> usize {
        match self {
            Stream::Risk => 500,
            _ => 1000,
        }
    }
}

fn iso(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms).unwrap_or_default().format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string()
}

/// The first page of a stream's events from `from_ms` to `to_ms` (both included; Graph pages on with
/// `@odata.nextLink`).
pub fn list_path(stream: Stream, from_ms: i64, to_ms: i64) -> String {
    let field = stream.time_field();
    format!("{}?$filter={field}%20ge%20{}%20and%20{field}%20le%20{}&$top={}", stream.path(), iso(from_ms), iso(to_ms), stream.top())
}

/// Microsoft Graph, as the poller and the connection test read it.
#[async_trait::async_trait]
pub trait GraphApi: Send + Sync {
    /// GET a path under `/v1.0`, or an `@odata.nextLink` Graph returned.
    async fn get(&self, c: &Credentials, target: &str) -> Result<Value, AzureError>;
}

#[async_trait::async_trait]
impl GraphApi for Rest {
    async fn get(&self, c: &Credentials, target: &str) -> Result<Value, AzureError> {
        self.graph_get(c, target).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn streams_list_by_their_time_with_graphs_largest_pages() {
        let (from, to) = (1_794_132_000_000, 1_794_132_300_500);
        assert_eq!(
            list_path(Stream::SignIns, from, to),
            "/auditLogs/signIns?$filter=createdDateTime%20ge%202026-11-08T10:00:00.000Z%20and%20createdDateTime%20le%202026-11-08T10:05:00.500Z&$top=1000"
        );
        assert!(list_path(Stream::Audits, from, to).starts_with("/auditLogs/directoryAudits?$filter=activityDateTime%20ge%20"));
        assert!(list_path(Stream::Risk, from, to).starts_with("/identityProtection/riskDetections?$filter=detectedDateTime%20ge%20") && list_path(Stream::Risk, from, to).ends_with("&$top=500"));
        assert_eq!(STREAMS.map(|s| Stream::of_scope(s.scope())), STREAMS.map(Some));
        assert_eq!((Stream::SignIns.permission(), Stream::Risk.permission(), Stream::Audits.license()), ("AuditLog.Read.All", "IdentityRiskEvent.Read.All", None));
    }
}
