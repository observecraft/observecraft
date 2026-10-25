-- 0006 also captured incident numbers ("Incident #97:") as #channels when converting old timeline entries.
UPDATE incident_notifications
SET recipients = ARRAY(SELECT r FROM unnest(recipients) AS r WHERE r !~ '^#[0-9]+$')
WHERE EXISTS (SELECT 1 FROM unnest(recipients) AS r WHERE r ~ '^#[0-9]+$');
