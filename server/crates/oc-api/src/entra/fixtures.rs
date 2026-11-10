//! Microsoft Graph's sign-ins, directory audits and risk detections as its API returns them (fictional tenant and
//! people), and a fake Graph for the poller's tests.

use std::sync::Mutex;

use serde_json::{json, Value};

use super::api::GraphApi;
use crate::azure::api::{AzureError, Credentials};

pub const TENANT: &str = "6b3c1f0e-2a4d-4c8e-9f10-123456789012";

/// An interactive sign-in (`signIn`): `error_code` 0 is a success.
pub fn sign_in(id: &str, at: &str, upn: &str, error_code: i64) -> Value {
    let failure = (error_code != 0).then_some("Invalid username or password or Invalid on-premise username or password.");
    json!({
        "id": id,
        "createdDateTime": at,
        "userDisplayName": "Jordan Patel",
        "userPrincipalName": upn,
        "userId": "0f3a7c52-6d1e-4b7a-9c2d-5e8f1a2b3c4d",
        "appId": "c44b4083-3bb0-49c1-b47d-974e53cbdf3c",
        "appDisplayName": "Azure Portal",
        "ipAddress": "203.0.113.24",
        "clientAppUsed": "Browser",
        "correlationId": "9d1e2f3a-4b5c-4d6e-8f70-0a1b2c3d4e5f",
        "conditionalAccessStatus": "notApplied",
        "isInteractive": true,
        "riskDetail": "none",
        "riskLevelAggregated": "none",
        "riskLevelDuringSignIn": "none",
        "riskState": "none",
        "riskEventTypes_v2": [],
        "resourceDisplayName": "Windows Azure Service Management API",
        "resourceId": "797f4846-ba00-4fd7-ba43-dac1f8f63013",
        "status": { "errorCode": error_code, "failureReason": failure.unwrap_or("Other."), "additionalDetails": null },
        "deviceDetail": { "deviceId": "", "displayName": null, "operatingSystem": "Windows10", "browser": "Edge 130.0.0", "isCompliant": false, "isManaged": false, "trustType": null },
        "location": { "city": "Bucharest", "state": "Bucuresti", "countryOrRegion": "RO", "geoCoordinates": { "altitude": null, "latitude": 44.4268, "longitude": 26.1025 } },
        "appliedConditionalAccessPolicies": []
    })
}

/// A directory audit (`directoryAudit`) by an admin.
pub fn audit(id: &str, at: &str, activity: &str, category: &str, targets: Value) -> Value {
    json!({
        "id": id,
        "category": category,
        "correlationId": "5d3c3340-5c5e-419c-9ae9-193b1669f84e",
        "result": "success",
        "resultReason": "",
        "activityDisplayName": activity,
        "activityDateTime": at,
        "loggedByService": "Core Directory",
        "operationType": "Assign",
        "initiatedBy": { "app": null, "user": { "id": "9ff6ab06-49d6-49e8-9974-e979522283c1", "displayName": null, "userPrincipalName": "sam.lee@example.com", "ipAddress": "198.51.100.7", "userType": null, "homeTenantId": null, "homeTenantName": null } },
        "targetResources": targets,
        "additionalDetails": []
    })
}

/// "Add member to role": `role` given to a user.
pub fn role_assigned(id: &str, at: &str, role: &str) -> Value {
    audit(
        id,
        at,
        "Add member to role",
        "RoleManagement",
        json!([{
            "id": "febf6383-8bed-4efb-8244-c9a456a1f172",
            "displayName": null,
            "type": "User",
            "userPrincipalName": "riley.chen@example.com",
            "groupType": null,
            "modifiedProperties": [
                { "displayName": "Role.ObjectID", "oldValue": null, "newValue": "\"62e90394-69f5-4237-9190-012177145e10\"" },
                { "displayName": "Role.DisplayName", "oldValue": null, "newValue": format!("\"{role}\"") },
                { "displayName": "Role.TemplateId", "oldValue": null, "newValue": "\"62e90394-69f5-4237-9190-012177145e10\"" }
            ]
        }, { "id": "62e90394-69f5-4237-9190-012177145e10", "displayName": null, "type": "Role", "userPrincipalName": null, "groupType": null, "modifiedProperties": [] }]),
    )
}

/// An Identity Protection risk detection (`riskDetection`).
pub fn risk(id: &str, at: &str, level: &str) -> Value {
    json!({
        "id": id,
        "requestId": "1f2e3d4c-5b6a-4978-8a9b-0c1d2e3f4a5b",
        "correlationId": "9d1e2f3a-4b5c-4d6e-8f70-0a1b2c3d4e5f",
        "riskEventType": "unfamiliarFeatures",
        "riskState": "atRisk",
        "riskLevel": level,
        "riskDetail": "none",
        "source": "IdentityProtection",
        "detectionTimingType": "realtime",
        "activity": "signin",
        "ipAddress": "198.51.100.200",
        "activityDateTime": at,
        "detectedDateTime": at,
        "lastUpdatedDateTime": at,
        "userId": "0f3a7c52-6d1e-4b7a-9c2d-5e8f1a2b3c4d",
        "userDisplayName": "Jordan Patel",
        "userPrincipalName": "jordan.patel@example.com",
        "location": { "city": "Lagos", "state": "Lagos", "countryOrRegion": "NG", "geoCoordinates": { "latitude": 6.45, "longitude": 3.39 } }
    })
}

/// Graph's lists: each collection's events (newest first, as Graph returns sign-ins), read by their time filter
/// in pages of `page` and the requests made.
pub struct FakeGraph {
    pub events: Mutex<Vec<(String, Vec<Value>)>>,
    pub page: usize,
    pub requests: Mutex<Vec<String>>,
    pub fail: Option<AzureError>,
}

impl Default for FakeGraph {
    fn default() -> Self {
        FakeGraph { events: Mutex::default(), page: 1000, requests: Mutex::default(), fail: None }
    }
}

impl FakeGraph {
    pub fn set(&self, collection: &str, events: Vec<Value>) {
        let mut all = self.events.lock().unwrap();
        all.retain(|(c, _)| c != collection);
        all.push((collection.to_string(), events));
    }

    pub fn requested(&self) -> Vec<String> {
        self.requests.lock().unwrap().clone()
    }
}

fn ms(t: &str) -> i64 {
    chrono::DateTime::parse_from_rfc3339(t).map(|t| t.timestamp_millis()).unwrap_or(0)
}

#[async_trait::async_trait]
impl GraphApi for FakeGraph {
    async fn get(&self, _c: &Credentials, target: &str) -> Result<Value, AzureError> {
        self.requests.lock().unwrap().push(target.to_string());
        if let Some(e) = &self.fail {
            return Err(e.clone());
        }
        // `/{collection}?$filter={field} ge {from} and {field} le {to}&$top=…[&$skiptoken={offset}]`
        let (path, query) = target.split_once('?').unwrap_or((target, ""));
        let filter = query.split('&').find_map(|p| p.strip_prefix("$filter=")).unwrap_or_default().replace("%20", " ");
        let words: Vec<&str> = filter.split(' ').collect();
        let (field, from, to) = (words[0], ms(words[2]), ms(words[6]));
        let skip: usize = query.split('&').find_map(|p| p.strip_prefix("$skiptoken=")).and_then(|s| s.parse().ok()).unwrap_or(0);
        let all = self.events.lock().unwrap();
        let events = all.iter().find(|(c, _)| path.ends_with(c.as_str())).map(|(_, e)| e.clone()).unwrap_or_default();
        let mut window: Vec<Value> = events.into_iter().filter(|e| (from..=to).contains(&ms(e[field].as_str().unwrap_or_default()))).collect();
        window.sort_by_key(|e| std::cmp::Reverse(ms(e[field].as_str().unwrap_or_default())));
        let page: Vec<Value> = window.iter().skip(skip).take(self.page).cloned().collect();
        let mut out = json!({ "value": page });
        if skip + self.page < window.len() {
            let base = target.split("&$skiptoken=").next().unwrap_or(target);
            out["@odata.nextLink"] = json!(format!("https://graph.microsoft.com/v1.0{base}&$skiptoken={}", skip + self.page));
        }
        Ok(out)
    }
}
