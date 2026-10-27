-- Error Tracking's issue key of each error log, computed at insert so grouping reads two small columns
-- instead of every message and attribute map. Older parts compute them on read until they expire.
--   error_kind: the error.kind / error.type / exception.type attribute
--   error_key:  the message's first 200 characters with each run of hex characters containing a digit
--               as 0, cut to 80 characters (oc-api error_tracking::shape_key)
ALTER TABLE oc.logs${LOCAL} ${ON_CLUSTER}
    ADD COLUMN IF NOT EXISTS `error_kind` LowCardinality(String) MATERIALIZED if(status = 'error', multiIf(attr['error.kind'] != '', attr['error.kind'], attr['error.type'] != '', attr['error.type'], attr['exception.type']), '') CODEC(ZSTD(1)),
    ADD COLUMN IF NOT EXISTS `error_key` String MATERIALIZED if(status = 'error', substringUTF8(replaceRegexpAll(substringUTF8(message, 1, 200), '[0-9A-Fa-f]*[0-9][0-9A-Fa-f]*', '0'), 1, 80), '') CODEC(ZSTD(1));
-- In a cluster the Distributed table needs the columns too (on a single node this is the same table).
ALTER TABLE oc.logs ${ON_CLUSTER}
    ADD COLUMN IF NOT EXISTS `error_kind` LowCardinality(String) MATERIALIZED if(status = 'error', multiIf(attr['error.kind'] != '', attr['error.kind'], attr['error.type'] != '', attr['error.type'], attr['exception.type']), ''),
    ADD COLUMN IF NOT EXISTS `error_key` String MATERIALIZED if(status = 'error', substringUTF8(replaceRegexpAll(substringUTF8(message, 1, 200), '[0-9A-Fa-f]*[0-9][0-9A-Fa-f]*', '0'), 1, 80), '');
