-- Development user (the identity every request acts as until authentication lands; see CurrentUser).
INSERT INTO users (id, org_id, email, name)
VALUES ('00000000-0000-0000-0000-000000000001', 'default', 'alex.morgan@example.com', 'Alex Morgan')
ON CONFLICT DO NOTHING;

INSERT INTO teams (org_id, handle, name) VALUES
    ('default', 'platform', 'Platform'),
    ('default', 'storefront', 'Storefront'),
    ('default', 'checkout', 'Checkout'),
    ('default', 'billing', 'Billing'),
    ('default', 'payments', 'Payments')
ON CONFLICT DO NOTHING;

-- Popularity is derived from how often a dashboard is opened.
ALTER TABLE dashboards ADD COLUMN view_count INTEGER NOT NULL DEFAULT 0;
CREATE INDEX dashboard_lists_org ON dashboard_lists (org_id);
