-- Reports → Scorecards Performance: each rule's results per day and team (historical trends by
-- scorecard and by rule), written by the scorecard evaluations (the last evaluation of a day wins).
-- Skipped services count in neither column.
CREATE TABLE portal_scorecard_rule_days (
    org_id  TEXT NOT NULL,
    day     DATE NOT NULL,
    rule_id UUID NOT NULL,
    team    TEXT NOT NULL DEFAULT '',
    pass    INTEGER NOT NULL,
    fail    INTEGER NOT NULL,
    PRIMARY KEY (org_id, day, rule_id, team),
    FOREIGN KEY (org_id, rule_id) REFERENCES scorecard_rules(org_id, id) ON DELETE CASCADE
);
