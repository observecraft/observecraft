//! The latest message of each service check per host (Monitoring → Check Summary shows it). The
//! status itself is the `service_check.status` gauge; messages would make that gauge a new series
//! per message, so they go to Postgres `service_check_messages` instead, written only when a
//! check's status or message changes (remembered here per org, check and host).

use std::collections::HashMap;
use std::sync::Mutex;

use sqlx::PgPool;

/// Most remembered (org, check, host) entries; past it the memory starts over (and rewrites once).
const MAX_REMEMBERED: usize = 200_000;
/// Longest message kept.
const MAX_MESSAGE_CHARS: usize = 2_000;

#[derive(Clone, Debug, PartialEq)]
pub struct CheckMessage {
    pub check: String,
    pub host: String,
    /// 0 OK, 1 WARNING, 2 CRITICAL, 3 UNKNOWN.
    pub status: u8,
    pub message: String,
}

/// (org, check, host) → the last status and message written.
type Last = HashMap<(String, String, String), (u8, String)>;

#[derive(Default)]
pub struct CheckMessages {
    last: Mutex<Last>,
}

impl CheckMessages {
    /// The messages whose status or text differs from the last one seen, remembering them.
    pub fn changed(&self, org: &str, messages: Vec<CheckMessage>) -> Vec<CheckMessage> {
        let mut last = self.last.lock().unwrap();
        if last.len() > MAX_REMEMBERED {
            last.clear();
        }
        let mut out: Vec<CheckMessage> = Vec::new();
        for mut m in messages {
            m.message = m.message.chars().take(MAX_MESSAGE_CHARS).collect();
            let key = (org.to_string(), m.check.clone(), m.host.clone());
            if last.get(&key).is_some_and(|(s, text)| *s == m.status && *text == m.message) {
                continue;
            }
            last.insert(key, (m.status, m.message.clone()));
            // A payload may carry several runs of one check: the last one wins.
            out.retain(|o| !(o.check == m.check && o.host == m.host));
            out.push(m);
        }
        out
    }

    /// Writes the changed messages (a failed write is logged; the check's status is unaffected).
    pub async fn record(&self, pg: &PgPool, org: &str, messages: Vec<CheckMessage>) {
        let changed = self.changed(org, messages);
        if changed.is_empty() {
            return;
        }
        let checks: Vec<&str> = changed.iter().map(|m| m.check.as_str()).collect();
        let hosts: Vec<&str> = changed.iter().map(|m| m.host.as_str()).collect();
        let statuses: Vec<i16> = changed.iter().map(|m| i16::from(m.status)).collect();
        let texts: Vec<&str> = changed.iter().map(|m| m.message.as_str()).collect();
        let done = sqlx::query(
            "INSERT INTO service_check_messages (org_id, check_name, host, status, message, updated_at) \
             SELECT $1, c, h, s, m, now() FROM UNNEST($2::text[], $3::text[], $4::smallint[], $5::text[]) AS t(c, h, s, m) \
             ON CONFLICT (org_id, check_name, host) DO UPDATE SET status = EXCLUDED.status, message = EXCLUDED.message, updated_at = now()",
        )
        .bind(org)
        .bind(&checks)
        .bind(&hosts)
        .bind(&statuses)
        .bind(&texts)
        .execute(pg)
        .await;
        if let Err(e) = done {
            tracing::warn!(error = %e, org, "recording service check messages failed");
            // Forget them so the next run tries again.
            let mut last = self.last.lock().unwrap();
            for m in &changed {
                last.remove(&(org.to_string(), m.check.clone(), m.host.clone()));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(check: &str, host: &str, status: u8, message: &str) -> CheckMessage {
        CheckMessage { check: check.into(), host: host.into(), status, message: message.into() }
    }

    #[test]
    fn only_changes_are_written() {
        let m = CheckMessages::default();
        assert_eq!(m.changed("org", vec![msg("ntp.in_sync", "web-1", 0, "")]).len(), 1, "first sight");
        assert!(m.changed("org", vec![msg("ntp.in_sync", "web-1", 0, "")]).is_empty(), "same status and text");
        assert_eq!(m.changed("org", vec![msg("ntp.in_sync", "web-1", 2, "offset 3s")]).len(), 1, "status changed");
        assert_eq!(m.changed("org", vec![msg("ntp.in_sync", "web-1", 2, "offset 4s")]).len(), 1, "text changed");
        assert_eq!(m.changed("other-org", vec![msg("ntp.in_sync", "web-1", 2, "offset 4s")]).len(), 1, "per org");
    }

    #[test]
    fn the_last_run_of_a_check_in_a_payload_wins_and_long_messages_are_cut() {
        let m = CheckMessages::default();
        let out = m.changed("org", vec![msg("http.can_connect", "web-1", 0, "ok"), msg("http.can_connect", "web-1", 2, "timeout"), msg("http.can_connect", "web-2", 0, "")]);
        assert_eq!(out, vec![msg("http.can_connect", "web-1", 2, "timeout"), msg("http.can_connect", "web-2", 0, "")]);
        let long = m.changed("org", vec![msg("x", "h", 1, &"a".repeat(MAX_MESSAGE_CHARS + 10))]);
        assert_eq!(long[0].message.chars().count(), MAX_MESSAGE_CHARS);
    }
}
