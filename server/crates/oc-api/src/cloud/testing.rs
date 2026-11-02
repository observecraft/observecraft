//! Test doubles shared by the cloud crawlers' tests.

use std::sync::Mutex as StdMutex;

use serde_json::Value;

use super::jobs::MetricSink;

/// A published record: org, headers, body.
pub type Published = (String, Vec<(String, String)>, Value);
/// A published point: name, host, attributes.
pub type PublishedPoint = (String, Option<String>, Vec<(String, String)>);

#[derive(Default)]
pub struct FakeSink {
    pub records: StdMutex<Vec<Published>>,
    pub fail: bool,
}

#[async_trait::async_trait]
impl MetricSink for FakeSink {
    async fn publish(&self, org: &str, headers: &[(String, String)], body: Vec<u8>) -> Result<(), String> {
        if self.fail {
            return Err("broker unavailable".into());
        }
        self.records.lock().unwrap().push((org.to_string(), headers.to_vec(), serde_json::from_slice(&body).unwrap()));
        Ok(())
    }
}

/// Every published point: (name, host, attributes).
pub fn published(sink: &FakeSink) -> Vec<PublishedPoint> {
    let mut out = vec![];
    for (_, _, doc) in sink.records.lock().unwrap().iter() {
        for rm in doc["resourceMetrics"].as_array().unwrap() {
            let host = rm["resource"]["attributes"].as_array().unwrap().iter().find(|a| a["key"] == "host.name").map(|a| a["value"]["stringValue"].as_str().unwrap().to_string());
            for m in rm["scopeMetrics"][0]["metrics"].as_array().unwrap() {
                for dp in m["gauge"]["dataPoints"].as_array().unwrap() {
                    let attrs = dp["attributes"].as_array().unwrap().iter().map(|a| (a["key"].as_str().unwrap().to_string(), a["value"]["stringValue"].as_str().unwrap().to_string())).collect();
                    out.push((m["name"].as_str().unwrap().to_string(), host.clone(), attrs));
                }
            }
        }
    }
    out
}

