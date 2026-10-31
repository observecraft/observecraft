//! The metric rollup job (docs/adr/0002-storage-at-saas-scale.md).
//!
//! Every minute, per org: each closed 5-minute bucket (ended at least the lateness window ago,
//! `OC_METRICS_ROLLUP_LATENESS_SECONDS`, 120) is rolled up from `metrics_samples` /
//! `metrics_dist_samples` into `metrics_rollup_5m` / `metrics_dist_rollup_5m`, and each hour whose
//! 5-minute buckets are all rolled up into the 1-hour tables. Progress is the Postgres table
//! `metric_rollups` (per org and tier, everything before `done_until` is rolled up), so a restart
//! resumes where it stopped and catches up a gap in hour-long steps. Rollup tables are
//! ReplacingMergeTree: re-rolling a bucket (a retry, or `done_until` moved back for late data)
//! replaces its rows, and readers read them with FINAL.
//!
//! Several servers can run it: orgs are claimed with `FOR UPDATE SKIP LOCKED` and held with
//! `claimed_until` while they're rolled up. Queries read rollups before `done_until` and raw
//! samples after it ([`progress`]). How far behind the job is (`lag_seconds`) is logged.
//!
//! Each rollup row keeps the tier retention its samples carry (the org's choice when they were
//! written, 0 = the install's default), as the samples do.
//!
//! Arrival markers in ClickHouse record ingestion time as well as the sample's bucket. Before
//! advancing rollups, scan settled arrival minutes and rewind both tiers for late samples. The
//! scan cursor and rewinds commit together, so a crash cannot acknowledge a repair before it is
//! scheduled. Markers expire after 30 days; a longer worker outage requires an explicit rebuild.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use crate::error::ApiError;
use crate::state::AppState;

pub(crate) const FIVE_MINUTES_MS: i64 = 300_000;
pub(crate) const HOUR_MS: i64 = 3_600_000;
/// Most time one rollup query covers: a gap after downtime is caught up in steps of this.
const CHUNK_MS: i64 = HOUR_MS;
/// Steps per org and tier in one run (12 hours of catch-up a minute).
const CHUNKS_PER_RUN: usize = 12;
/// How long a claimed org is held before another server may take it over.
const CLAIM_SECONDS: f64 = 300.0;
const MINUTE_MS: i64 = 60_000;
const ARRIVAL_RETENTION_MS: i64 = 30 * 24 * HOUR_MS;
// Automatic repair covers the default Kafka replay window. Older samples need an explicit
// rebuild with verified raw coverage: blindly replacing aggregates after raw TTL expiry loses history.
const REPAIR_HORIZON_MS: i64 = 72 * HOUR_MS;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Tier {
    FiveMinutes,
    Hour,
}

impl Tier {
    fn name(self) -> &'static str {
        match self {
            Tier::FiveMinutes => "5m",
            Tier::Hour => "1h",
        }
    }

    fn bucket_ms(self) -> i64 {
        match self {
            Tier::FiveMinutes => FIVE_MINUTES_MS,
            Tier::Hour => HOUR_MS,
        }
    }
}

/// Where an org's rollups stand (epoch ms): every bucket before these is rolled up.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub(crate) struct Progress {
    pub five_minutes: i64,
    pub hour: i64,
}

fn floor(t: i64, step: i64) -> i64 {
    t.div_euclid(step) * step
}

/// The next range `[from, to)` of closed buckets of `tier` to roll up after `done` (at most
/// [`CHUNK_MS`]), or None. A 5-minute bucket is closed `lateness_ms` after it ends; an hour once
/// its 5-minute buckets are rolled up (`done_5m`).
pub(crate) fn due(tier: Tier, done: i64, now: i64, lateness_ms: i64, done_5m: i64) -> Option<(i64, i64)> {
    let from = floor(done, tier.bucket_ms());
    let closed = match tier {
        Tier::FiveMinutes => floor(now - lateness_ms, FIVE_MINUTES_MS),
        Tier::Hour => floor(done_5m, HOUR_MS),
    };
    let to = closed.min(from + CHUNK_MS);
    (to > from).then_some((from, to))
}

/// Where a new org's rollups start: `backfill_hours` before now.
pub(crate) fn initial(now: i64, backfill_hours: i64) -> i64 {
    floor(now - backfill_hours * HOUR_MS, HOUR_MS)
}

/// INSERT … SELECT statements rolling up `[from, to)` (epoch ms, the two params of each) for the
/// org of the client (`{oc_org:String}`).
pub(crate) fn rollup_sql(tier: Tier) -> [&'static str; 2] {
    match tier {
        Tier::FiveMinutes => [
            "INSERT INTO otel.metrics_rollup_5m (OrgId, MetricName, SeriesId, Bucket, Min, Max, Sum, Count, First, Last, Start, End, RetentionDays, Retention1hDays) \
             SELECT OrgId, MetricName, SeriesId, toStartOfFiveMinutes(TimeUnix) AS b, min(Value), max(Value), sum(Value), count(), \
                    argMin(Value, TimeUnix), argMax(Value, TimeUnix), min(TimeUnix), max(TimeUnix), Retention5mDays, max(Retention1hDays) \
             FROM otel.metrics_samples \
             WHERE OrgId = {oc_org:String} AND TimeUnix >= fromUnixTimestamp64Milli(?) AND TimeUnix < fromUnixTimestamp64Milli(?) \
             GROUP BY OrgId, MetricName, SeriesId, b, Retention5mDays",
            "INSERT INTO otel.metrics_dist_rollup_5m (OrgId, MetricName, SeriesId, Bucket, CountFirst, CountLast, CountTotal, SumFirst, SumLast, SumTotal, Min, Max, \
                    BucketsFirst, BucketsLast, BucketsTotal, ExplicitBounds, Scale, PositiveOffset, Quantiles, QuantileTotals, Points, Start, End, RetentionDays, Retention1hDays) \
             SELECT OrgId, MetricName, SeriesId, toStartOfFiveMinutes(TimeUnix) AS b, \
                    argMin(Count, TimeUnix), argMax(Count, TimeUnix), sum(Count), argMin(Sum, TimeUnix), argMax(Sum, TimeUnix), sum(Sum), min(Min), max(Max), \
                    argMin(if(empty(BucketCounts), PositiveBucketCounts, BucketCounts), TimeUnix), argMax(if(empty(BucketCounts), PositiveBucketCounts, BucketCounts), TimeUnix), \
                    sumForEach(if(empty(BucketCounts), PositiveBucketCounts, BucketCounts)), \
                    argMax(ExplicitBounds, TimeUnix), argMax(Scale, TimeUnix), argMax(PositiveOffset, TimeUnix), \
                    argMax(`ValueAtQuantiles.Quantile`, TimeUnix), sumForEach(`ValueAtQuantiles.Value`), \
                    count(), min(TimeUnix), max(TimeUnix), Retention5mDays, max(Retention1hDays) \
             FROM otel.metrics_dist_samples \
             WHERE OrgId = {oc_org:String} AND TimeUnix >= fromUnixTimestamp64Milli(?) AND TimeUnix < fromUnixTimestamp64Milli(?) \
             GROUP BY OrgId, MetricName, SeriesId, b, Retention5mDays",
        ],
        Tier::Hour => [
            "INSERT INTO otel.metrics_rollup_1h (OrgId, MetricName, SeriesId, Bucket, Min, Max, Sum, Count, First, Last, Start, End, RetentionDays) \
             SELECT OrgId, MetricName, SeriesId, toStartOfHour(Bucket) AS h, min(Min), max(Max), sum(Sum), sum(Count), argMin(First, Start), argMax(Last, End), \
                    min(Start), max(End), Retention1hDays \
             FROM otel.metrics_rollup_5m FINAL \
             WHERE OrgId = {oc_org:String} AND Bucket >= toDateTime(intDiv(?, 1000)) AND Bucket < toDateTime(intDiv(?, 1000)) \
             GROUP BY OrgId, MetricName, SeriesId, h, Retention1hDays",
            "INSERT INTO otel.metrics_dist_rollup_1h (OrgId, MetricName, SeriesId, Bucket, CountFirst, CountLast, CountTotal, SumFirst, SumLast, SumTotal, Min, Max, \
                    BucketsFirst, BucketsLast, BucketsTotal, ExplicitBounds, Scale, PositiveOffset, Quantiles, QuantileTotals, Points, Start, End, RetentionDays) \
             SELECT OrgId, MetricName, SeriesId, toStartOfHour(Bucket) AS h, \
                    argMin(CountFirst, Start), argMax(CountLast, End), sum(CountTotal), argMin(SumFirst, Start), argMax(SumLast, End), sum(SumTotal), min(Min), max(Max), \
                    argMin(BucketsFirst, Start), argMax(BucketsLast, End), sumForEach(BucketsTotal), \
                    argMax(ExplicitBounds, End), argMax(Scale, End), argMax(PositiveOffset, End), argMax(Quantiles, End), sumForEach(QuantileTotals), \
                    sum(Points), min(Start), max(End), Retention1hDays \
             FROM otel.metrics_dist_rollup_5m FINAL \
             WHERE OrgId = {oc_org:String} AND Bucket >= toDateTime(intDiv(?, 1000)) AND Bucket < toDateTime(intDiv(?, 1000)) \
             GROUP BY OrgId, MetricName, SeriesId, h, Retention1hDays",
        ],
    }
}

/// Rolls up `[from, to)` of `tier` with `ch`, an org's client (`AppState::ch`). In a cluster each
/// shard rolls up its own series (series, samples and rollups share the shard).
pub(crate) async fn roll(ch: &clickhouse::Client, cluster: bool, tier: Tier, from: i64, to: i64) -> Result<(), clickhouse::error::Error> {
    let ch = if cluster { ch.clone().with_option("parallel_distributed_insert_select", "2") } else { ch.clone() };
    for sql in rollup_sql(tier) {
        ch.query(sql).bind(from).bind(to).execute().await?;
    }
    Ok(())
}

/// Whether a cluster's replicas, at most `delay_secs` behind, hold every point of the buckets
/// closed `lateness_secs` ago. Each shard's part of a rollup reads one replica of the shard (the
/// server's own when it is one, whatever its delay), so a lagging replica would roll up a bucket
/// from incomplete samples, and nothing re-rolls it later.
pub(crate) fn replicas_caught_up(delay_secs: u64, lateness_secs: i64) -> bool {
    (delay_secs as i64).saturating_mul(2) <= lateness_secs.max(0)
}

/// Seconds the most behind replica of the metric tables lags (`system.replicas.absolute_delay`).
pub(crate) async fn replica_delay(ch: &clickhouse::Client, cluster: &str) -> Result<u64, clickhouse::error::Error> {
    ch.query(
        "SELECT toUInt64(max(absolute_delay)) FROM clusterAllReplicas({cluster:String}, system.replicas) \
         WHERE database = 'otel' AND table IN ('metrics_samples_local', 'metrics_dist_samples_local', 'metrics_rollup_5m_local', 'metrics_dist_rollup_5m_local', 'metrics_arrivals_local')",
    )
    .with_option("param_cluster", cluster)
    .fetch_one()
    .await
}

/// Each org's rollup progress, read by queries (cached briefly: a stale value only means more
/// raw samples are read).
pub(crate) async fn progress(st: &AppState, org: &str) -> Result<Progress, ApiError> {
    type Cache = Mutex<HashMap<String, (Progress, Instant)>>;
    static CACHE: OnceLock<Cache> = OnceLock::new();
    let cache = CACHE.get_or_init(Default::default);
    if let Some((p, at)) = cache.lock().unwrap().get(org) {
        if at.elapsed() < Duration::from_secs(15) {
            return Ok(*p);
        }
    }
    let p = read_progress(&st.postgres, org).await?;
    cache.lock().unwrap().insert(org.to_string(), (p, Instant::now()));
    Ok(p)
}

async fn read_progress(pg: &sqlx::PgPool, org: &str) -> Result<Progress, sqlx::Error> {
    let rows: Vec<(String, i64)> = sqlx::query_as("SELECT tier, (extract(epoch FROM done_until) * 1000)::bigint FROM metric_rollups WHERE org_id = $1")
        .bind(org)
        .fetch_all(pg)
        .await?;
    let get = |tier: Tier| rows.iter().find(|(t, _)| t == tier.name()).map_or(0, |(_, ms)| *ms);
    Ok(Progress {
        five_minutes: get(Tier::FiveMinutes),
        hour: get(Tier::Hour),
    })
}

/// Adds progress rows for active orgs that have none, starting at `start` (epoch ms).
async fn seed(pg: &sqlx::PgPool, start: i64) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO metric_rollups (org_id, tier, done_until) SELECT o.id, t.tier, to_timestamp($1::double precision / 1000) \
         FROM orgs o CROSS JOIN (VALUES ('5m'), ('1h')) AS t(tier) WHERE o.status = 'active' ON CONFLICT (org_id, tier) DO NOTHING",
    )
    .bind(start)
    .execute(pg)
    .await?;
    Ok(())
}

/// Claims up to `limit` orgs with buckets to roll up and holds them for [`CLAIM_SECONDS`].
async fn claim(pg: &sqlx::PgPool, limit: i64, lateness_ms: i64) -> Result<Vec<(String, Progress)>, sqlx::Error> {
    let mut tx = pg.begin().await?;
    let rows: Vec<(String, i64, i64)> = sqlx::query_as(
        "SELECT f.org_id, (extract(epoch FROM f.done_until) * 1000)::bigint, (extract(epoch FROM h.done_until) * 1000)::bigint \
         FROM metric_rollups f JOIN metric_rollups h ON h.org_id = f.org_id AND h.tier = '1h' JOIN orgs o ON o.id = f.org_id \
         WHERE f.tier = '5m' AND o.status = 'active' AND (f.claimed_until IS NULL OR f.claimed_until < now()) \
           AND (f.done_until <= now() - make_interval(secs => $2::double precision / 1000) - interval '5 minutes' OR h.done_until <= f.done_until - interval '1 hour') \
         ORDER BY f.done_until LIMIT $1 FOR UPDATE OF f SKIP LOCKED",
    )
    .bind(limit)
    .bind(lateness_ms)
    .fetch_all(&mut *tx)
    .await?;
    for (org, ..) in &rows {
        sqlx::query("UPDATE metric_rollups SET claimed_until = now() + make_interval(secs => $2) WHERE org_id = $1 AND tier = '5m'")
            .bind(org)
            .bind(CLAIM_SECONDS)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(rows.into_iter().map(|(org, five_minutes, hour)| (org, Progress { five_minutes, hour })).collect())
}

/// Records that `tier` of `org` is rolled up to `to`, unless its progress was moved since `from`
/// (someone asked for a re-roll: that wins).
async fn advance(pg: &sqlx::PgPool, org: &str, tier: Tier, from: i64, to: i64) -> Result<bool, sqlx::Error> {
    let done = sqlx::query(
        "UPDATE metric_rollups SET done_until = to_timestamp($3::double precision / 1000), updated_at = now() \
         WHERE org_id = $1 AND tier = $2 AND (extract(epoch FROM done_until) * 1000)::bigint = $4",
    )
    .bind(org)
    .bind(tier.name())
    .bind(to)
    .bind(from)
    .execute(pg)
    .await?;
    Ok(done.rows_affected() == 1)
}

/// Seconds between now and the 5-minute progress of the most behind active org.
async fn lag_seconds(pg: &sqlx::PgPool) -> Result<i64, sqlx::Error> {
    let (lag,): (i64,) =
        sqlx::query_as("SELECT COALESCE(max(EXTRACT(EPOCH FROM now() - r.done_until)), 0)::bigint FROM metric_rollups r JOIN orgs o ON o.id = r.org_id WHERE r.tier = '5m' AND o.status = 'active'")
            .fetch_one(pg)
            .await?;
    Ok(lag)
}

async fn release(pg: &sqlx::PgPool, org: &str) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE metric_rollups SET claimed_until = NULL WHERE org_id = $1 AND tier = '5m'")
        .bind(org)
        .execute(pg)
        .await?;
    Ok(())
}

/// Find late samples without scanning the samples themselves. Wait at least two minutes so the
/// current arrival minute is closed and replicated; the cluster delay check in `run` also applies.
async fn repair_arrivals(pg: &sqlx::PgPool, ch: &clickhouse::Client, org: &str, now: i64, lateness_ms: i64) -> anyhow::Result<Progress> {
    #[derive(clickhouse::Row, serde::Deserialize)]
    struct Arrival {
        bucket: Option<u32>,
    }
    let (from,): (i64,) = sqlx::query_as("SELECT (extract(epoch FROM arrivals_until) * 1000)::bigint FROM metric_rollups WHERE org_id = $1 AND tier = '5m'")
        .bind(org)
        .fetch_one(pg)
        .await?;
    let to = floor(now - lateness_ms.max(2 * MINUTE_MS), MINUTE_MS);
    if from >= to {
        return Ok(read_progress(pg, org).await?);
    }
    anyhow::ensure!(
        from >= now - ARRIVAL_RETENTION_MS,
        "metric arrival cursor is older than 30 days; rebuild rollups before resuming this organization"
    );
    let p = read_progress(pg, org).await?;
    let Arrival { bucket: late } = ch
        .query(
            "SELECT minOrNull(toUnixTimestamp(Bucket)) AS bucket FROM otel.metrics_arrivals \
             WHERE OrgId = {oc_org:String} AND ArrivalMinute >= toDateTime(intDiv(?, 1000)) \
             AND ArrivalMinute < toDateTime(intDiv(?, 1000)) AND Bucket < toDateTime(intDiv(?, 1000)) \
             AND ArrivalMinute >= Bucket + INTERVAL 5 MINUTE + toIntervalSecond(?)",
        )
        .bind(from)
        .bind(to)
        .bind(p.five_minutes)
        // Arrival time is rounded down to a minute; keep a minute of overlap at the deadline.
        .bind((lateness_ms - MINUTE_MS).max(0) / 1000)
        .fetch_one()
        .await?;
    let mut tx = pg.begin().await?;
    if let Some(bucket) = late {
        let five_minutes = i64::from(bucket) * 1000;
        anyhow::ensure!(five_minutes >= now - REPAIR_HORIZON_MS, "late metric bucket is older than 72 hours; verify raw coverage and explicitly rebuild rollups");
        let hour = floor(five_minutes, HOUR_MS);
        sqlx::query(
            "UPDATE metric_rollups SET done_until = LEAST(done_until, to_timestamp(\
             (CASE WHEN tier = '5m' THEN $2::bigint ELSE $3::bigint END)::double precision / 1000)), updated_at = now() WHERE org_id = $1",
        )
        .bind(org)
        .bind(five_minutes)
        .bind(hour)
        .execute(&mut *tx)
        .await?;
    }
    sqlx::query("UPDATE metric_rollups SET arrivals_until = to_timestamp($2::double precision / 1000) WHERE org_id = $1 AND tier = '5m'")
        .bind(org)
        .bind(to)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(read_progress(pg, org).await?)
}

/// Rolls up what is due for one claimed org; returns its progress after.
async fn run_org(st: &AppState, org: &str, now: i64) -> anyhow::Result<Progress> {
    let lateness = st.config.metrics_rollup_lateness_secs * 1000;
    let ch = st.ch(org);
    let cluster = st.tables.cluster.is_some();
    let mut p = repair_arrivals(&st.postgres, &ch, org, now, lateness).await?;
    for tier in [Tier::FiveMinutes, Tier::Hour] {
        for _ in 0..CHUNKS_PER_RUN {
            let done = if tier == Tier::FiveMinutes { p.five_minutes } else { p.hour };
            let Some((from, to)) = due(tier, done, now, lateness, p.five_minutes) else {
                break;
            };
            roll(&ch, cluster, tier, from, to).await?;
            if !advance(&st.postgres, org, tier, done, to).await? {
                // Moved back meanwhile: the next run starts from there.
                break;
            }
            match tier {
                Tier::FiveMinutes => p.five_minutes = to,
                Tier::Hour => p.hour = to,
            }
        }
    }
    Ok(p)
}

/// Runs the rollup job every minute (several servers can run it: orgs are claimed with SKIP LOCKED).
pub async fn run(st: AppState) {
    let mut tick = tokio::time::interval(Duration::from_secs(60));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tick.tick().await;
        let now = crate::routes::metrics::now_ms();
        if let Err(e) = seed(&st.postgres, initial(now, st.config.metrics_rollup_backfill_hours)).await {
            tracing::warn!(error = %e, "metric rollups: adding orgs failed");
            continue;
        }
        if let Some(cluster) = &st.tables.cluster {
            match replica_delay(&st.clickhouse, cluster).await {
                Ok(delay) if replicas_caught_up(delay, st.config.metrics_rollup_lateness_secs) => {}
                Ok(delay) => {
                    tracing::warn!(delay_seconds = delay, "metric rollups: waiting for lagging replicas");
                    continue;
                }
                Err(e) => {
                    tracing::warn!(error = %e, "metric rollups: reading the replicas' delay failed");
                    continue;
                }
            }
        }
        let claimed = match claim(&st.postgres, 50, st.config.metrics_rollup_lateness_secs * 1000).await {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(error = %e, "metric rollups: claiming orgs failed");
                continue;
            }
        };
        for (org, _) in claimed {
            let started = Instant::now();
            match run_org(&st, &org, now).await {
                Ok(after) => {
                    let behind = (now - after.five_minutes) / 1000;
                    tracing::debug!(org = %org, lag_seconds = behind, ms = started.elapsed().as_millis() as u64, "metric rollups");
                }
                Err(e) => tracing::warn!(org = %org, error = %e, "metric rollup failed"),
            }
            if let Err(e) = release(&st.postgres, &org).await {
                tracing::warn!(org = %org, error = %e, "metric rollups: releasing the org failed");
            }
        }
        // How far behind the most behind org's 5-minute tier is (lateness and the open bucket included).
        match lag_seconds(&st.postgres).await {
            Ok(lag) => tracing::info!(lag_seconds = lag, "metric rollups"),
            Err(e) => tracing::warn!(error = %e, "metric rollups: reading the lag failed"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const T: i64 = 1_793_095_200_000; // 2026-10-27 10:00:00 UTC
    const MIN: i64 = 60_000;
    const LATE: i64 = 2 * MIN;

    #[test]
    fn a_five_minute_bucket_is_rolled_up_once_closed_and_past_the_lateness_window() {
        // 10:06: 10:00–10:05 ended a minute ago, still within the window.
        assert_eq!(due(Tier::FiveMinutes, T, T + 6 * MIN, LATE, 0), None);
        // 10:07: it is closed.
        assert_eq!(due(Tier::FiveMinutes, T, T + 7 * MIN, LATE, 0), Some((T, T + 5 * MIN)));
        // 10:13: two buckets in one go; the open 10:10 bucket waits.
        assert_eq!(due(Tier::FiveMinutes, T, T + 13 * MIN, LATE, 0), Some((T, T + 10 * MIN)));
        // Done to 10:10: nothing more until 10:17.
        assert_eq!(due(Tier::FiveMinutes, T + 10 * MIN, T + 16 * MIN, LATE, 0), None);
    }

    #[test]
    fn a_gap_after_downtime_is_caught_up_an_hour_at_a_time() {
        let now = T + 5 * HOUR_MS + 30 * MIN;
        assert_eq!(due(Tier::FiveMinutes, T, now, LATE, 0), Some((T, T + HOUR_MS)));
        assert_eq!(due(Tier::FiveMinutes, T + 4 * HOUR_MS, now, LATE, 0), Some((T + 4 * HOUR_MS, T + 5 * HOUR_MS)));
        assert_eq!(due(Tier::FiveMinutes, T + 5 * HOUR_MS, now, LATE, 0), Some((T + 5 * HOUR_MS, T + 5 * HOUR_MS + 25 * MIN)));
    }

    #[test]
    fn an_hour_is_rolled_up_once_its_five_minute_buckets_are() {
        assert_eq!(due(Tier::Hour, T, T + 2 * HOUR_MS, LATE, T + 55 * MIN), None);
        assert_eq!(due(Tier::Hour, T, T + 2 * HOUR_MS, LATE, T + HOUR_MS), Some((T, T + HOUR_MS)));
        assert_eq!(due(Tier::Hour, T, T + 9 * HOUR_MS, LATE, T + 3 * HOUR_MS + 5 * MIN), Some((T, T + HOUR_MS)));
    }

    #[test]
    fn progress_starts_on_a_bucket_boundary() {
        // A progress mark moved back to any time re-rolls from its bucket's start.
        assert_eq!(due(Tier::FiveMinutes, T + 7 * MIN, T + 20 * MIN, LATE, 0), Some((T + 5 * MIN, T + 15 * MIN)));
        assert_eq!(initial(T + 25 * MIN, 24), T - 24 * HOUR_MS);
    }

    #[test]
    fn a_cluster_rolls_up_only_while_every_replica_has_the_closed_buckets() {
        // Lateness 120 s: a replica 60 s behind has every point of a bucket closed 120 s ago.
        assert!(replicas_caught_up(0, 120));
        assert!(replicas_caught_up(60, 120));
        // One further behind would roll up from incomplete samples (the shard's part of the
        // INSERT … SELECT may run on it), so the job waits.
        assert!(!replicas_caught_up(61, 120));
        assert!(!replicas_caught_up(1, 0));
    }

    /// The replicas' delay is readable on a cluster. Runs when OC_TEST_CLICKHOUSE_URL is a node of
    /// the cluster OC_TEST_CLICKHOUSE_CLUSTER.
    #[tokio::test]
    async fn the_replicas_delay_is_read_from_every_replica() {
        let Some(live) = crate::test_support::live_clickhouse().await else { return };
        let Some(cluster) = live.cfg.cluster.clone() else { return };
        let delay = replica_delay(&live.client, &cluster).await.expect("delay");
        assert!(delay < 60, "a replica of the test cluster is {delay} s behind");
    }

    #[test]
    fn rollup_queries_are_org_scoped_and_bound() {
        for tier in [Tier::FiveMinutes, Tier::Hour] {
            for sql in rollup_sql(tier) {
                assert!(sql.contains("OrgId = {oc_org:String}"), "{sql}");
                assert_eq!(sql.matches('?').count(), 2, "{sql}");
            }
        }
        // Hours are rolled up from the 5-minute rollups (deduplicated: re-rolled buckets replace).
        assert!(rollup_sql(Tier::Hour).iter().all(|s| s.contains("rollup_5m FINAL")));
    }

    #[derive(clickhouse::Row, serde::Deserialize, Debug, PartialEq)]
    struct Agg {
        bucket: i64,
        min: f64,
        max: f64,
        sum: f64,
        count: u64,
        first: f64,
        last: f64,
        start: i64,
        end: i64,
        retention: u16,
    }

    /// Replay repairs both tiers automatically; the arrival cursor must not skip unsettled
    /// minutes or move when the metadata query fails. Uses actual materialized-view markers.
    #[tokio::test]
    async fn arrivals_repair_closed_buckets_and_preserve_other_orgs() {
        let Some(live) = crate::test_support::live_clickhouse().await else { return };
        let Ok(url) = std::env::var("OC_TEST_POSTGRES_URL") else { return };
        let pg = oc_store::postgres(&url, Duration::from_secs(5)).await.expect("postgres");
        oc_store::migrate_postgres(&pg).await.expect("migrate");
        let now = crate::routes::metrics::now_ms();
        let sample_start = floor(now, HOUR_MS) - 2 * HOUR_MS;
        let org = format!("arrival-repair-{}-{now}", std::process::id());
        let other = format!("{org}-other");
        for id in [&org, &other] {
            sqlx::query("INSERT INTO orgs (id, name) VALUES ($1, $1)").bind(id).execute(&pg).await.unwrap();
            sqlx::query("INSERT INTO metric_rollups (org_id, tier, done_until, arrivals_until) SELECT $1, tier, to_timestamp($2::double precision / 1000), to_timestamp($3::double precision / 1000) FROM (VALUES ('5m'), ('1h')) t(tier)")
                .bind(id).bind(sample_start + HOUR_MS).bind(floor(now, MINUTE_MS)).execute(&pg).await.unwrap();
        }
        let ch = live.client.clone().with_option("param_oc_org", &org);
        let insert = |at, value| {
            ch.query("INSERT INTO otel.metrics_samples (OrgId, MetricName, SeriesId, TimeUnix, Value, RetentionDays, Retention5mDays, Retention1hDays) VALUES ({oc_org:String}, 'system.cpu.user', 1, fromUnixTimestamp64Milli(?), ?, 30, 90, 395)")
                .bind(at).bind(value)
        };
        insert(sample_start + MINUTE_MS, 10.0f64).execute().await.unwrap();
        roll(&ch, live.cluster(), Tier::FiveMinutes, sample_start, sample_start + HOUR_MS).await.unwrap();
        roll(&ch, live.cluster(), Tier::Hour, sample_start, sample_start + HOUR_MS).await.unwrap();
        // Kafka replay adds a point to a bucket the worker already closed.
        insert(sample_start + 2 * MINUTE_MS, 20.0f64).execute().await.unwrap();
        let before = read_progress(&pg, &org).await.unwrap();
        assert_eq!(repair_arrivals(&pg, &ch, &org, now, LATE).await.unwrap(), before, "open arrival minutes are not acknowledged");
        let broken = ch.clone().with_url("http://127.0.0.1:1");
        assert!(repair_arrivals(&pg, &broken, &org, now + 5 * MINUTE_MS, LATE).await.is_err());
        let (cursor,): (i64,) = sqlx::query_as("SELECT (extract(epoch FROM arrivals_until) * 1000)::bigint FROM metric_rollups WHERE org_id = $1 AND tier = '5m'")
            .bind(&org)
            .fetch_one(&pg)
            .await
            .unwrap();
        assert_eq!(cursor, floor(now, MINUTE_MS), "failed metadata reads do not advance the cursor");
        let p = repair_arrivals(&pg, &ch, &org, now + 5 * MINUTE_MS, LATE).await.unwrap();
        assert_eq!(p, Progress { five_minutes: sample_start, hour: sample_start });
        assert_eq!(read_progress(&pg, &other).await.unwrap(), before, "repair is tenant scoped");
        roll(&ch, live.cluster(), Tier::FiveMinutes, p.five_minutes, sample_start + HOUR_MS).await.unwrap();
        roll(&ch, live.cluster(), Tier::Hour, p.hour, sample_start + HOUR_MS).await.unwrap();
        for tier in [Tier::FiveMinutes, Tier::Hour] {
            assert!(advance(&pg, &org, tier, sample_start, sample_start + HOUR_MS).await.unwrap());
            let sum: f64 = ch
                .query(&format!("SELECT sum(Sum) FROM otel.metrics_rollup_{} FINAL WHERE OrgId = {{oc_org:String}}", tier.name()))
                .fetch_one()
                .await
                .unwrap();
            assert_eq!(sum, 30.0, "late points appear in {} exactly once", tier.name());
        }
        assert_eq!(
            repair_arrivals(&pg, &ch, &org, now + 5 * MINUTE_MS, LATE).await.unwrap(),
            before,
            "processed arrivals do not rewind again"
        );
        for id in [&org, &other] {
            sqlx::query("DELETE FROM orgs WHERE id = $1").bind(id).execute(&pg).await.unwrap();
        }
    }

    /// The job's rollups equal the same aggregation over the samples; re-rolling a bucket after a
    /// late point replaces it; rows keep the tiers' retention. Skipped unless
    /// OC_TEST_CLICKHOUSE_URL points at a ClickHouse server (user `default`, or
    /// OC_TEST_CLICKHOUSE_USER / OC_TEST_CLICKHOUSE_PASSWORD).
    #[tokio::test]
    async fn rollups_match_the_samples_and_reruns_replace_them() {
        let Some(live) = crate::test_support::live_clickhouse().await else { return };
        let org = format!("rollup-job-{}-{}", std::process::id(), crate::routes::metrics::now_ms());
        let ch = live.client.clone().with_option("param_oc_org", &org);
        // Two series, a point every 10 s for 2 hours; tiers 15 / 180 / 760 days.
        let insert = |from_s: i64, n: u64, value: &str| {
            format!(
                "INSERT INTO otel.metrics_ingest_gauge (ResourceAttributes, MetricName, Attributes, TimeUnix, Value) \
                 SELECT map('oc.org_id', '{org}', 'host.name', concat('i-0a1b2c3d4e5f6000', toString(number % 2)), 'oc.metrics_retention_days', '15', \
                            'oc.metrics_retention_5m_days', '180', 'oc.metrics_retention_1h_days', '760'), \
                        'system.cpu.utilization', map('state', 'user'), toDateTime64({from_s} + intDiv(number, 2) * 10, 3), {value} FROM numbers({n})"
            )
        };
        let start = T / 1000;
        ch.query(&insert(start, 1440, "(number % 17) / 17")).execute().await.expect("insert");
        for (from, to) in [(T, T + HOUR_MS), (T + HOUR_MS, T + 2 * HOUR_MS)] {
            roll(&ch, live.cluster(), Tier::FiveMinutes, from, to).await.expect("5m");
        }
        roll(&ch, live.cluster(), Tier::Hour, T, T + 2 * HOUR_MS).await.expect("1h");

        let raw = |step: i64| {
            format!(
                "SELECT toInt64(toUnixTimestamp(toStartOfInterval(TimeUnix, INTERVAL {step} SECOND))) AS bucket, min(Value) AS min, max(Value) AS max, sum(Value) AS sum, \
                 count() AS count, argMin(Value, TimeUnix) AS first, argMax(Value, TimeUnix) AS last, toInt64(toUnixTimestamp64Milli(min(TimeUnix))) AS start, \
                 toInt64(toUnixTimestamp64Milli(max(TimeUnix))) AS end, toUInt16({retention}) AS retention FROM otel.metrics_samples \
                 WHERE OrgId = {{oc_org:String}} AND MetricName = 'system.cpu.utilization' GROUP BY SeriesId, bucket ORDER BY SeriesId, bucket",
                retention = if step == 300 { 180 } else { 760 }
            )
        };
        let rolled = |table: &str| {
            format!(
                "SELECT toInt64(toUnixTimestamp(Bucket)) AS bucket, Min AS min, Max AS max, Sum AS sum, Count AS count, First AS first, Last AS last, \
                 toInt64(toUnixTimestamp64Milli(Start)) AS start, toInt64(toUnixTimestamp64Milli(End)) AS end, RetentionDays AS retention FROM otel.{table} FINAL \
                 WHERE OrgId = {{oc_org:String}} AND MetricName = 'system.cpu.utilization' ORDER BY SeriesId, Bucket"
            )
        };
        let fetch = |sql: String| {
            let ch = ch.clone();
            async move { ch.query(&sql).fetch_all::<Agg>().await.unwrap_or_else(|e| panic!("{sql}: {e}")) }
        };
        let close = |a: &[Agg], b: &[Agg], what: &str| {
            assert_eq!(a.len(), b.len(), "{what}: rows");
            for (x, y) in a.iter().zip(b) {
                assert!((x.sum - y.sum).abs() < 1e-9, "{what}: {x:?} vs {y:?}");
                assert_eq!(
                    (x.bucket, x.min, x.max, x.count, x.start, x.end, x.retention),
                    (y.bucket, y.min, y.max, y.count, y.start, y.end, y.retention),
                    "{what}"
                );
            }
        };
        let five = fetch(rolled("metrics_rollup_5m")).await;
        assert_eq!(five.len(), 2 * 24, "two series × 24 buckets");
        close(&five, &fetch(raw(300)).await, "5m");
        close(&fetch(rolled("metrics_rollup_1h")).await, &fetch(raw(3600)).await, "1h");

        // A late point in 10:00–10:05, then the bucket and its hour rolled up again: replaced, not added.
        ch.query(&insert(start + 30, 2, "1000")).execute().await.expect("late");
        roll(&ch, live.cluster(), Tier::FiveMinutes, T, T + FIVE_MINUTES_MS).await.expect("re-roll 5m");
        roll(&ch, live.cluster(), Tier::Hour, T, T + HOUR_MS).await.expect("re-roll 1h");
        let five = fetch(rolled("metrics_rollup_5m")).await;
        assert_eq!(five.len(), 2 * 24);
        close(&five, &fetch(raw(300)).await, "5m after a re-roll");
        assert_eq!(five[0].max, 1000.0);
        close(&fetch(rolled("metrics_rollup_1h")).await, &fetch(raw(3600)).await, "1h after a re-roll");

        // Another org's samples aren't touched.
        let other = ch.clone().with_option("param_oc_org", format!("{org}-other"));
        let n: u64 = other
            .query("SELECT count() FROM otel.metrics_rollup_5m WHERE OrgId = {oc_org:String}")
            .fetch_one()
            .await
            .expect("count");
        assert_eq!(n, 0);
    }

    /// Distribution rollups keep each bucket's first, last and total values, as the old per-insert
    /// views did (the query parity tests in routes/metrics.rs check the statistics).
    #[tokio::test]
    async fn distribution_rollups_keep_first_last_and_totals() {
        let Some(live) = crate::test_support::live_clickhouse().await else { return };
        let org = format!("rollup-dist-{}-{}", std::process::id(), crate::routes::metrics::now_ms());
        let ch = live.client.clone().with_option("param_oc_org", &org);
        // A cumulative histogram: 3 points in 10:00–10:05, one at 10:07.
        ch.query(&format!(
            "INSERT INTO otel.metrics_ingest_histogram (ResourceAttributes, MetricName, Attributes, TimeUnix, Count, Sum, BucketCounts, ExplicitBounds, Min, Max, AggregationTemporality) VALUES \
             (map('oc.org_id', '{org}'), 'http.server.duration', map(), '2026-10-27 10:00:10', 4, 40, [1, 3], [10.], 1, 9, 2), \
             (map('oc.org_id', '{org}'), 'http.server.duration', map(), '2026-10-27 10:01:10', 6, 70, [2, 4], [10.], 2, 30, 2), \
             (map('oc.org_id', '{org}'), 'http.server.duration', map(), '2026-10-27 10:02:10', 9, 100, [2, 7], [10.], 3, 8, 2), \
             (map('oc.org_id', '{org}'), 'http.server.duration', map(), '2026-10-27 10:07:10', 12, 150, [3, 9], [10.], 1, 12, 2)"
        ))
        .execute()
        .await
        .expect("insert");
        roll(&ch, live.cluster(), Tier::FiveMinutes, T, T + 10 * MIN).await.expect("5m");
        roll(&ch, live.cluster(), Tier::Hour, T, T + HOUR_MS).await.expect("1h");
        #[derive(clickhouse::Row, serde::Deserialize, Debug, PartialEq)]
        struct Dist {
            count_first: u64,
            count_last: u64,
            count_total: u64,
            sum_total: f64,
            min: f64,
            max: f64,
            buckets_first: Vec<u64>,
            buckets_last: Vec<u64>,
            buckets_total: Vec<u64>,
            bounds: Vec<f64>,
            points: u64,
        }
        let fetch = |table: &str| {
            let sql = format!(
                "SELECT CountFirst AS count_first, CountLast AS count_last, CountTotal AS count_total, SumTotal AS sum_total, Min AS min, Max AS max, \
                 BucketsFirst AS buckets_first, BucketsLast AS buckets_last, BucketsTotal AS buckets_total, ExplicitBounds AS bounds, Points AS points \
                 FROM otel.{table} FINAL WHERE OrgId = {{oc_org:String}} ORDER BY Bucket"
            );
            let ch = ch.clone();
            async move { ch.query(&sql).fetch_all::<Dist>().await.unwrap_or_else(|e| panic!("{sql}: {e}")) }
        };
        let five = fetch("metrics_dist_rollup_5m").await;
        assert_eq!(
            five,
            vec![
                Dist {
                    count_first: 4,
                    count_last: 9,
                    count_total: 19,
                    sum_total: 210.0,
                    min: 1.0,
                    max: 30.0,
                    buckets_first: vec![1, 3],
                    buckets_last: vec![2, 7],
                    buckets_total: vec![5, 14],
                    bounds: vec![10.0],
                    points: 3
                },
                Dist {
                    count_first: 12,
                    count_last: 12,
                    count_total: 12,
                    sum_total: 150.0,
                    min: 1.0,
                    max: 12.0,
                    buckets_first: vec![3, 9],
                    buckets_last: vec![3, 9],
                    buckets_total: vec![3, 9],
                    bounds: vec![10.0],
                    points: 1
                },
            ]
        );
        let hour = fetch("metrics_dist_rollup_1h").await;
        assert_eq!(
            hour,
            vec![Dist {
                count_first: 4,
                count_last: 12,
                count_total: 31,
                sum_total: 360.0,
                min: 1.0,
                max: 30.0,
                buckets_first: vec![1, 3],
                buckets_last: vec![3, 9],
                buckets_total: vec![8, 23],
                bounds: vec![10.0],
                points: 4
            }]
        );
    }

    /// Claiming, progress and re-rolls through Postgres. Skipped unless OC_TEST_POSTGRES_URL points
    /// at a Postgres database the migrations may be applied to.
    #[tokio::test]
    async fn progress_is_claimed_advanced_and_can_be_moved_back() {
        let Ok(url) = std::env::var("OC_TEST_POSTGRES_URL") else {
            eprintln!("OC_TEST_POSTGRES_URL unset: skipping");
            return;
        };
        let pg = oc_store::postgres(&url, Duration::from_secs(5)).await.expect("postgres");
        oc_store::migrate_postgres(&pg).await.expect("migrate");
        let org = format!("rollup-pg-{}", std::process::id());
        sqlx::query("INSERT INTO orgs (id, name) VALUES ($1, $1) ON CONFLICT DO NOTHING")
            .bind(&org)
            .execute(&pg)
            .await
            .expect("org");
        sqlx::query("DELETE FROM metric_rollups WHERE org_id = $1").bind(&org).execute(&pg).await.expect("reset");
        let now = crate::routes::metrics::now_ms();
        let start = initial(now, 1);
        seed(&pg, start).await.expect("seed");
        seed(&pg, start - HOUR_MS).await.expect("seeding again keeps progress");
        assert_eq!(read_progress(&pg, &org).await.unwrap(), Progress { five_minutes: start, hour: start });

        // Claimed once: a second server gets nothing for this org while it's held.
        let first = claim(&pg, 10_000, LATE).await.expect("claim");
        assert!(first.iter().any(|(o, p)| *o == org && p.five_minutes == start));
        let second = claim(&pg, 10_000, LATE).await.expect("claim again");
        assert!(!second.iter().any(|(o, _)| *o == org));

        // Progress moves forward from where it was…
        assert!(advance(&pg, &org, Tier::FiveMinutes, start, start + HOUR_MS).await.unwrap());
        // …but not when it was moved back meanwhile (a re-roll asked for: it wins).
        sqlx::query("UPDATE metric_rollups SET done_until = to_timestamp($2::double precision / 1000) WHERE org_id = $1 AND tier = '5m'")
            .bind(&org)
            .bind(start)
            .execute(&pg)
            .await
            .unwrap();
        assert!(!advance(&pg, &org, Tier::FiveMinutes, start + HOUR_MS, start + 2 * HOUR_MS).await.unwrap());
        assert_eq!(read_progress(&pg, &org).await.unwrap().five_minutes, start);

        assert!(lag_seconds(&pg).await.unwrap() >= 3600, "the org is an hour behind");
        // Released: claimable again (still behind).
        release(&pg, &org).await.unwrap();
        assert!(claim(&pg, 10_000, LATE).await.unwrap().iter().any(|(o, _)| *o == org));
        release(&pg, &org).await.unwrap();
        sqlx::query("DELETE FROM orgs WHERE id = $1").bind(&org).execute(&pg).await.unwrap();
    }
}
