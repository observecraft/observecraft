//! Cross-replica quota admission. The transaction commits BEFORE storage: an uncertain write
//! keeps its slot and its identity, so replay can finish without over-admitting another event.
//! Only quota-bearing organizations enter this path; no telemetry payloads are stored here.

use super::*;
use std::collections::{BTreeSet, HashSet};

/// Caller holds the process-local org locks. Database advisory locks coordinate every replica.
/// Lock order is stable across mixed-org batches; no lock is held during metric outbox enqueue.
pub(super) async fn admit(
    st: &AppState,
    ix: &Indexer,
    rows: Vec<Value>,
    existing: &mut Admissions,
    reservations: &mut Reservations<'_>,
    now: i64,
    rules: &HashMap<String, Arc<Vec<IndexRule>>>,
) -> Result<(Vec<u8>, usize, usize), String> {
    let mut orgs = BTreeSet::new();
    for row in &rows {
        if let Some(org) = row
            .get("org_id")
            .and_then(Value::as_str)
            .filter(|org| rules.get(*org).is_some_and(|rs| rs.iter().any(|r| r.daily_quota.is_some())))
        {
            orgs.insert(org.to_owned());
        }
    }
    if orgs.is_empty() {
        return Ok(admit_batch(ix, rows, existing, reservations, now, rules));
    }
    let mut tx = st.postgres.begin().await.map_err(|e| e.to_string())?;
    let mut known = HashSet::new();
    sqlx::query("SET LOCAL synchronous_commit = on").execute(&mut *tx).await.map_err(|e| e.to_string())?;
    sqlx::query("SET LOCAL statement_timeout = '15s'").execute(&mut *tx).await.map_err(|e| e.to_string())?;
    sqlx::query("SET LOCAL lock_timeout = '5s'").execute(&mut *tx).await.map_err(|e| e.to_string())?;
    for org in &orgs {
        lock(&mut tx, org).await?;
        let ids: Vec<&str> = rows
            .iter()
            .filter(|r| r.get("org_id").and_then(Value::as_str) == Some(org))
            .filter_map(|r| r.get("id").and_then(Value::as_str))
            .collect();
        let admissions: Vec<(String, String, i32)> =
            sqlx::query_as("SELECT event_id::text, index_name, retention_days FROM log_quota_admissions WHERE org_id = $1 AND event_id = ANY(SELECT unnest($2::text[])::uuid)")
                .bind(org)
                .bind(&ids)
                .fetch_all(&mut *tx)
                .await
                .map_err(|e| e.to_string())?;
        for (id, index, retention) in admissions {
            known.insert((org.clone(), id.clone()));
            existing.insert((org.clone(), id), (index, retention as u16));
        }

        let counters: Vec<(String, i64, i64)> = sqlx::query_as("SELECT index_name, day_start, n FROM log_quota_counters WHERE org_id = $1 AND day_start >= $2")
            .bind(org)
            .bind(now - DAY_MS)
            .fetch_all(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;
        let counters: HashMap<_, _> = counters.into_iter().map(|(index, day, n)| ((index, day), n as u64)).collect();
        for rule in rules[org].iter().filter(|r| r.daily_quota.is_some()) {
            let day = rule.day_start(now);
            let n = match counters.get(&(rule.name.clone(), day)) {
                Some(n) => *n,
                None => {
                    let first: Option<String> = sqlx::query_scalar("INSERT INTO log_quota_epochs (org_id, index_name, bootstrap_day) VALUES ($1, $2, $3) ON CONFLICT DO NOTHING RETURNING index_name")
                        .bind(org)
                        .bind(&rule.name)
                        .bind(day)
                        .fetch_optional(&mut *tx)
                        .await
                        .map_err(|e| e.to_string())?;
                    // One cutover bootstrap per index. Future days start at zero: event-time counts
                    // cannot reconstruct ingestion-day accounting for late/future logs.
                    let n = if first.is_some() {
                        st.clickhouse
                            .query("SELECT count() FROM logs_read WHERE org_id = ? AND index_name = ? AND timestamp >= fromUnixTimestamp64Milli(?) AND timestamp < fromUnixTimestamp64Milli(?)")
                            .bind(org)
                            .bind(&rule.name)
                            .bind(day)
                            .bind(day + DAY_MS)
                            .fetch_one::<u64>()
                            .await
                            .map_err(|e| e.to_string())?
                    } else {
                        0
                    };
                    sqlx::query("INSERT INTO log_quota_counters (org_id, index_name, day_start, n) VALUES ($1, $2, $3, $4)")
                        .bind(org)
                        .bind(&rule.name)
                        .bind(day)
                        .bind(n as i64)
                        .execute(&mut *tx)
                        .await
                        .map_err(|e| e.to_string())?;
                    n
                }
            };
            ix.counters.lock().unwrap().insert((org.clone(), rule.name.clone()), Counter { day_start: day, n });
        }
    }
    let result = admit_batch(ix, rows, existing, reservations, now, rules);
    for ((org, index, day), count) in &reservations.counts {
        sqlx::query("UPDATE log_quota_counters SET n = n + $4 WHERE org_id = $1 AND index_name = $2 AND day_start = $3")
            .bind(org)
            .bind(index)
            .bind(day)
            .bind(*count as i64)
            .execute(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;
    }
    for org in &orgs {
        let admitted: Vec<_> = reservations
            .admitted
            .iter()
            .filter(|(key @ (o, _), (index, _))| o == org && !known.contains(*key) && rules[org].iter().any(|r| &r.name == index && r.daily_quota.is_some()))
            .collect();
        if admitted.is_empty() {
            continue;
        }
        let ids: Vec<&str> = admitted.iter().map(|((_, id), _)| id.as_str()).collect();
        let indexes: Vec<&str> = admitted.iter().map(|(_, (index, _))| index.as_str()).collect();
        let retention: Vec<i32> = admitted.iter().map(|(_, (_, days))| *days as i32).collect();
        sqlx::query("INSERT INTO log_quota_admissions (org_id, event_id, index_name, retention_days) SELECT $1, id::uuid, name, days FROM unnest($2::text[], $3::text[], $4::int[]) AS a(id, name, days) ON CONFLICT DO NOTHING")
            .bind(org).bind(ids).bind(indexes).bind(retention).execute(&mut *tx).await.map_err(|e| e.to_string())?;
    }
    tx.commit().await.map_err(|e| e.to_string())?;
    // These are durable reservations now, even if storage or outbox delivery fails next.
    reservations.counts.clear();
    Ok(result)
}

async fn lock(tx: &mut sqlx::Transaction<'_, sqlx::Postgres>, org: &str) -> Result<(), String> {
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 724915))")
        .bind(org)
        .execute(&mut **tx)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Bound receipt retention, independently of telemetry TTL. The seven-day guarantee exceeds
/// the bundled 72-hour Kafka retention; longer replay needs correspondingly longer receipts.
async fn cleanup(st: &AppState, org: &str) -> Result<u64, String> {
    let mut tx = st.postgres.begin().await.map_err(|e| e.to_string())?;
    sqlx::query("SET LOCAL statement_timeout = '15s'").execute(&mut *tx).await.map_err(|e| e.to_string())?;
    sqlx::query("SET LOCAL lock_timeout = '5s'").execute(&mut *tx).await.map_err(|e| e.to_string())?;
    lock(&mut tx, org).await?;
    let deleted = sqlx::query(
        "DELETE FROM log_quota_admissions WHERE org_id = $1 AND event_id IN (SELECT event_id FROM log_quota_admissions WHERE org_id = $1 AND expires_at < now() ORDER BY expires_at LIMIT 10000)",
    )
    .bind(org)
    .execute(&mut *tx)
    .await
    .map_err(|e| e.to_string())?
    .rows_affected();
    sqlx::query("DELETE FROM log_quota_counters WHERE org_id = $1 AND day_start < $2")
        .bind(org)
        .bind(now_ms() - 8 * DAY_MS)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;
    tx.commit().await.map_err(|e| e.to_string())?;
    Ok(deleted)
}

pub(super) async fn run(st: AppState) {
    let mut tick = tokio::time::interval(Duration::from_secs(10));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tick.tick().await;
        let orgs = sqlx::query_scalar::<_, String>("SELECT id FROM orgs WHERE EXISTS (SELECT 1 FROM log_quota_counters WHERE org_id = orgs.id AND day_start < $1) OR EXISTS (SELECT 1 FROM log_quota_admissions WHERE org_id = orgs.id AND expires_at < now()) ORDER BY id")
            .bind(now_ms() - 8 * DAY_MS).fetch_all(&st.postgres).await;
        match orgs {
            Ok(orgs) => {
                for org in orgs {
                    for _ in 0..16 {
                        match cleanup(&st, &org).await {
                            Ok(10000) => continue,
                            Ok(_) => break,
                            Err(e) => {
                                tracing::warn!(%org, error = %e, "cleaning quota metadata failed");
                                break;
                            }
                        }
                    }
                }
            }
            Err(e) => tracing::warn!(error = %e, "discovering quota metadata failed"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn indexer(org: &str, quota: u64) -> Arc<Indexer> {
        let ix = Arc::new(Indexer::default());
        ix.rules.write().unwrap().insert(
            org.into(),
            Arc::new(vec![IndexRule {
                name: "main".into(),
                terms: Vec::new(),
                retention_days: 7,
                daily_quota: Some(quota),
                reset_hour: 0,
                exclusions: Vec::new(),
            }]),
        );
        ix
    }

    fn event(org: &str, n: u64, ts: i64) -> Value {
        serde_json::json!({"org_id":org,"id":format!("00000000-0000-4000-8000-{n:012}"),"ts_ms":ts,
            "timestamp":chrono::DateTime::from_timestamp_millis(ts).unwrap().to_rfc3339(),"service":"order-service","status":"info","message":"quota probe"})
    }

    fn body(rows: &[Value]) -> Bytes {
        Bytes::from(rows.iter().map(|r| format!("{r}\n")).collect::<String>())
    }

    async fn count(st: &AppState, org: &str) -> i64 {
        sqlx::query_scalar("SELECT COALESCE(sum(n),0)::bigint FROM log_quota_counters WHERE org_id = $1")
            .bind(org)
            .fetch_one(&st.postgres)
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn independent_replicas_share_one_cap_and_replay_at_full_quota() {
        let Some((_, states)) = crate::test_support::replicas(4).await else { return };
        let org = crate::test_support::new_org(&states[0], "quota-global").await;
        let indexes: Vec<_> = (0..4).map(|_| indexer(&org, 37)).collect();
        let now = now_ms();
        let mut requests = Vec::new();
        for batch in 0..16 {
            let (st, ix) = (states[batch % 4].clone(), indexes[batch % 4].clone());
            let rows: Vec<_> = (1..=4).map(|n| event(&org, (batch * 4 + n) as u64, now)).collect();
            requests.push(tokio::spawn(async move { ingest(State((st, ix)), HeaderMap::new(), body(&rows)).await }));
        }
        for request in requests {
            assert_eq!(request.await.unwrap(), StatusCode::OK);
        }
        let st = &states[0];
        assert_eq!(count(st, &org).await, 37);
        assert_eq!(st.clickhouse.query("SELECT count() FROM logs_read WHERE org_id = ?").bind(&org).fetch_one::<u64>().await.unwrap(), 37);
        let rows: Vec<_> = (1..=64).rev().map(|n| event(&org, n, now)).collect();
        assert_eq!(ingest(State((st.clone(), indexer(&org, 37))), HeaderMap::new(), body(&rows)).await, StatusCode::OK);
        assert_eq!(count(st, &org).await, 37);
        let receipts: i64 = sqlx::query_scalar("SELECT count(*) FROM log_quota_admissions WHERE org_id = $1")
            .bind(&org)
            .fetch_one(&st.postgres)
            .await
            .unwrap();
        assert_eq!(receipts, 37);
        let other = crate::test_support::new_org(st, "quota-isolated").await;
        assert_eq!(ingest(State((st.clone(), indexer(&other, 1))), HeaderMap::new(), body(&[event(&other, 1, now)])).await, StatusCode::OK);
        assert_eq!(count(st, &other).await, 1);
        assert_eq!(count(st, &org).await, 37);
    }

    #[tokio::test]
    async fn mixed_organization_batches_lock_in_stable_order_without_blocking_other_tenants() {
        let Some((_, states)) = crate::test_support::replicas(2).await else { return };
        let (st, other_st) = (states[0].clone(), states[1].clone());
        let a = crate::test_support::new_org(&st, "quota-order-a").await;
        let b = crate::test_support::new_org(&st, "quota-order-b").await;
        let make = || {
            let ix = indexer(&a, 2);
            ix.rules.write().unwrap().insert(b.clone(), indexer(&b, 2).rules.read().unwrap()[&b].clone());
            ix
        };
        let now = now_ms();
        let (first, second) = (make(), make());
        let forward = body(&[event(&a, 1, now), event(&b, 1, now)]);
        let reverse = body(&[event(&b, 2, now), event(&a, 2, now)]);
        let results = tokio::time::timeout(Duration::from_secs(10), async {
            tokio::join!(
                ingest(State((st.clone(), first)), HeaderMap::new(), forward),
                ingest(State((other_st.clone(), second)), HeaderMap::new(), reverse)
            )
        })
        .await
        .unwrap();
        assert_eq!(results, (StatusCode::OK, StatusCode::OK));
        assert_eq!(count(&st, &a).await, 2);
        assert_eq!(count(&st, &b).await, 2);
        let mut blocked = st.postgres.begin().await.unwrap();
        lock(&mut blocked, &a).await.unwrap();
        let result = tokio::time::timeout(Duration::from_secs(3), ingest(State((other_st, make())), HeaderMap::new(), body(&[event(&b, 1, now)])))
            .await
            .unwrap();
        assert_eq!(result, StatusCode::OK, "an organization lock must not serialize unrelated tenants");
        blocked.rollback().await.unwrap();
    }

    #[tokio::test]
    async fn lost_storage_ack_and_stale_clickhouse_reads_cannot_release_or_double_charge_a_slot() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let Some((_, mut states)) = crate::test_support::replicas_with(2, |cfg| cfg.clickhouse_url = url.clone()).await else {
            return;
        };
        let mut stale = states.pop().unwrap();
        // RowBinary's empty stream simulates a replica that has not seen any indexed admissions.
        let lag = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let lag_url = format!("http://{}", lag.local_addr().unwrap());
        Arc::get_mut(&mut stale.0).unwrap().clickhouse = stale.clickhouse.clone().with_url(lag_url).with_compression(clickhouse::Compression::None);
        let lag_server = tokio::spawn(async move {
            axum::serve(lag, Router::new().route("/", axum::routing::any(|| async { "" }))).await.unwrap();
        });
        let st = states.pop().unwrap();
        let org = crate::test_support::new_org(&st, "quota-lost-ack").await;
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let handled = calls.clone();
        let real_url = std::env::var("OC_TEST_CLICKHOUSE_URL").unwrap();
        let server = tokio::spawn(async move {
            axum::serve(
                listener,
                Router::new().route(
                    "/",
                    post(move |axum::extract::RawQuery(query): axum::extract::RawQuery, rows: Bytes| {
                        let (url, calls) = (real_url.clone(), handled.clone());
                        async move {
                            let response = reqwest::Client::new().post(format!("{url}?{}", query.unwrap_or_default())).body(rows).send().await.unwrap();
                            assert!(response.status().is_success());
                            assert!(response.text().await.unwrap().is_empty());
                            if calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0 {
                                StatusCode::SERVICE_UNAVAILABLE
                            } else {
                                StatusCode::OK
                            }
                        }
                    }),
                ),
            )
            .await
            .unwrap();
        });
        let now = now_ms();
        assert_eq!(
            ingest(State((st.clone(), indexer(&org, 1))), HeaderMap::new(), body(&[event(&org, 1, now)])).await,
            StatusCode::SERVICE_UNAVAILABLE
        );
        assert_eq!(count(&st, &org).await, 1);
        assert_eq!(
            ingest(State((stale.clone(), indexer(&org, 1))), HeaderMap::new(), body(&[event(&org, 2, now), event(&org, 1, now)])).await,
            StatusCode::OK
        );
        assert_eq!(count(&st, &org).await, 1);
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 2);
        assert_eq!(st.clickhouse.query("SELECT count() FROM logs_read WHERE org_id = ?").bind(&org).fetch_one::<u64>().await.unwrap(), 1);
        server.abort();
        lag_server.abort();
    }

    #[tokio::test]
    async fn rollover_accounts_by_admission_day_and_reuses_previous_day_receipts() {
        let Some((_, states)) = crate::test_support::replicas(1).await else { return };
        let st = &states[0];
        let org = crate::test_support::new_org(st, "quota-midnight").await;
        let ix = indexer(&org, 1);
        let rules = ix.rules.read().unwrap().clone();
        let now = now_ms();
        let day = rules[&org][0].day_start(now);
        for (clock, ids, expected) in [(now, vec![1], 1), (day + DAY_MS + 1, vec![2], 1), (day + DAY_MS + 2, vec![1, 2, 3], 2)] {
            let rows = ids.into_iter().map(|n| event(&org, n, now - 10 * DAY_MS)).collect();
            let mut reservations = Reservations::new(&ix);
            let (rows, _, kept) = admit(st, &ix, rows, &mut Admissions::new(), &mut reservations, clock, &rules).await.unwrap();
            assert_eq!(kept, expected);
            insert_rows(st, "oc.logs_dedup", rows).await.unwrap();
        }
        let counts: Vec<i64> = sqlx::query_scalar("SELECT n FROM log_quota_counters WHERE org_id = $1 ORDER BY day_start")
            .bind(&org)
            .fetch_all(&st.postgres)
            .await
            .unwrap();
        assert_eq!(counts, vec![1, 1], "old event timestamps do not spend the following day's slots twice");
    }

    #[tokio::test]
    async fn cutover_bootstraps_existing_volume_once_and_adopts_legacy_ids_without_new_slots() {
        let Some((_, states)) = crate::test_support::replicas(1).await else { return };
        let st = &states[0];
        let org = crate::test_support::new_org(st, "quota-bootstrap").await;
        let now = now_ms();
        let mut legacy = event(&org, 1, now);
        legacy["index_name"] = "main".into();
        legacy["retention_days"] = 7.into();
        insert_rows(st, "oc.logs_dedup", body(&[legacy]).to_vec()).await.unwrap();
        let ix = indexer(&org, 1);
        assert_eq!(ingest(State((st.clone(), ix.clone())), HeaderMap::new(), body(&[event(&org, 2, now)])).await, StatusCode::OK);
        assert_eq!(count(st, &org).await, 1);
        assert_eq!(ingest(State((st.clone(), ix)), HeaderMap::new(), body(&[event(&org, 1, now)])).await, StatusCode::OK);
        let receipts: i64 = sqlx::query_scalar("SELECT count(*) FROM log_quota_admissions WHERE org_id = $1")
            .bind(&org)
            .fetch_one(&st.postgres)
            .await
            .unwrap();
        assert_eq!(receipts, 1);
        assert_eq!(count(st, &org).await, 1);
    }

    #[tokio::test]
    async fn cleanup_is_bounded_tenant_scoped_and_keeps_live_receipts_and_current_counters() {
        let Some((_, states)) = crate::test_support::replicas(1).await else { return };
        let st = &states[0];
        let org = crate::test_support::new_org(st, "quota-cleanup").await;
        let now = now_ms();
        assert_eq!(ingest(State((st.clone(), indexer(&org, 1))), HeaderMap::new(), body(&[event(&org, 1, now)])).await, StatusCode::OK);
        sqlx::query("INSERT INTO log_quota_admissions (org_id,event_id,index_name,retention_days,expires_at) SELECT $1, ('00000000-0000-4000-8000-' || lpad(n::text,12,'0'))::uuid, 'main',7,now()-INTERVAL '1 hour' FROM generate_series(2,10006) AS n")
            .bind(&org).execute(&st.postgres).await.unwrap();
        sqlx::query("INSERT INTO log_quota_counters (org_id,index_name,day_start,n) VALUES ($1,'main',$2,1)")
            .bind(&org)
            .bind(now - 9 * DAY_MS)
            .execute(&st.postgres)
            .await
            .unwrap();
        assert_eq!(cleanup(st, &org).await.unwrap(), 10000);
        assert_eq!(cleanup(st, &org).await.unwrap(), 5);
        assert_eq!(cleanup(st, &org).await.unwrap(), 0);
        assert_eq!(count(st, &org).await, 1);
        let receipts: i64 = sqlx::query_scalar("SELECT count(*) FROM log_quota_admissions WHERE org_id = $1")
            .bind(&org)
            .fetch_one(&st.postgres)
            .await
            .unwrap();
        assert_eq!(receipts, 1);
        sqlx::query("DELETE FROM orgs WHERE id = $1").bind(&org).execute(&st.postgres).await.unwrap();
        assert_eq!(count(st, &org).await, 0);
        let epochs: i64 = sqlx::query_scalar("SELECT count(*) FROM log_quota_epochs WHERE org_id = $1")
            .bind(&org)
            .fetch_one(&st.postgres)
            .await
            .unwrap();
        assert_eq!(epochs, 0);
    }
}
