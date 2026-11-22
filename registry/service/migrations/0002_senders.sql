-- One report per sender per device per UTC day counts toward a device's summary, so one network
-- can't settle a device alone. client is the same daily-rotating HMAC as clients.client, and is
-- cleared once the report's day is older than yesterday, as clients rows are deleted then.
-- counted is 0 for a report a later one from the same sender, day and device replaced; it stays
-- after client is cleared, so the dedupe outlives the HMAC. Reports from before this have no
-- sender and each counts.

ALTER TABLE reports ADD COLUMN client TEXT;
ALTER TABLE reports ADD COLUMN counted INTEGER NOT NULL DEFAULT 1;
CREATE INDEX reports_sender ON reports (day, client, suite, device_key);
