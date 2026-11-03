-- AI Credits Management: an organization's limits on Assist's use of its model (1 credit = 1,000 tokens,
-- input and output) and the token prices used to estimate cost on the Usage page.
CREATE TABLE IF NOT EXISTS assist_credits (
    org_id               TEXT PRIMARY KEY REFERENCES orgs(id) ON DELETE CASCADE,
    -- Credits per calendar month (UTC) for the whole org; none: unlimited.
    monthly_limit        BIGINT,
    -- Credits per member per day (UTC); none: unlimited.
    user_daily_limit     BIGINT,
    -- USD per million tokens; none: cost isn't estimated.
    input_price_per_mtok  DOUBLE PRECISION,
    output_price_per_mtok DOUBLE PRECISION,
    updated_by           TEXT NOT NULL DEFAULT '',
    updated_at           TIMESTAMPTZ NOT NULL DEFAULT now()
);
