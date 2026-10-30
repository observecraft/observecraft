-- Cloud Cost: one row per cost line item and day, in the columns of FOCUS (the FinOps Open Cost and
-- Usage Specification) that Datadog's Cloud Cost Management reads, from custom costs files uploaded
-- through Datadog's API (`PUT /api/v2/cost/custom_costs`) and from the demo seed
-- (routes/cloud_cost.rs). Queried as cost metrics (`all.cost`, `aws.cost.amortized`, …) and by the
-- Cloud Cost pages.
--
-- Rows of a file share its FileId. Deleting a file writes its rows again with IsDeleted = 1, so
-- readers read with FINAL (ReplacingMergeTree keeps the latest Version of each line).
CREATE TABLE IF NOT EXISTS otel.cloud_costs${LOCAL} ${ON_CLUSTER}
(
    `OrgId` LowCardinality(String),
    `FileId` String,
    `LineNo` UInt32,
    `ChargeDate` Date,
    -- aws, azure, gcp, or the provider a custom costs file names (`ProviderName`).
    `ProviderName` LowCardinality(String),
    `ServiceName` LowCardinality(String),
    `SubAccountName` LowCardinality(String),
    `RegionName` LowCardinality(String),
    `AvailabilityZone` LowCardinality(String),
    -- usage, purchase, tax, credit or adjustment.
    `ChargeCategory` LowCardinality(String),
    `ChargeDescription` String,
    `ResourceId` String,
    `PricingUnit` LowCardinality(String),
    `UsageQuantity` Float64,
    -- What the bill charged that day, and the cost with commitments spread over their term (amortized).
    `BilledCost` Float64,
    `EffectiveCost` Float64,
    -- The same usage at on-demand (list) prices: what commitments saved is ListCost - EffectiveCost.
    `ListCost` Float64,
    -- Standard (on-demand), Committed (covered by a commitment), Dynamic (spot) or Other.
    `PricingCategory` LowCardinality(String),
    -- The Savings Plan, Reserved Instance, Pre-purchase Plan or Committed Use Discount covering the
    -- usage, and whether a row is its Used or Unused part.
    `CommitmentDiscountId` String,
    `CommitmentDiscountType` LowCardinality(String),
    `CommitmentDiscountStatus` LowCardinality(String),
    `BillingCurrency` LowCardinality(String),
    `Tags` Map(LowCardinality(String), String),
    `Source` LowCardinality(String),
    `Version` UInt64,
    `IsDeleted` UInt8
)
ENGINE = ${REPLICATED}ReplacingMergeTree(Version, IsDeleted)
PARTITION BY toYYYYMM(ChargeDate)
ORDER BY (OrgId, ChargeDate, ProviderName, FileId, LineNo)
TTL ChargeDate + INTERVAL 25 MONTH;
${DISTRIBUTED otel.cloud_costs cityHash64(OrgId, FileId)};
