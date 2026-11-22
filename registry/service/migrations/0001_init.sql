-- Silas device registry. Only validated report fields and UTC days are stored: no IP
-- addresses, user agents or headers. clients holds a daily-rotating HMAC of the IP, deleted
-- once it is older than yesterday.

CREATE TABLE reports (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  day TEXT NOT NULL,
  suite TEXT NOT NULL,
  device_key TEXT NOT NULL,
  report TEXT NOT NULL
);
CREATE INDEX reports_suite_device ON reports (suite, device_key);

-- One precomputed aggregate per device and benchmark set, rewritten on each accepted report.
CREATE TABLE devices (
  suite TEXT NOT NULL,
  device_key TEXT NOT NULL,
  summary TEXT NOT NULL,
  PRIMARY KEY (suite, device_key)
);

-- Accepted reports per UTC day, for the global cap.
CREATE TABLE daily (
  day TEXT PRIMARY KEY,
  accepted INTEGER NOT NULL
);

-- POSTs per client per UTC day; client is hex HMAC-SHA256(HMAC-SHA256(RATE_KEY, day), ip).
CREATE TABLE clients (
  day TEXT NOT NULL,
  client TEXT NOT NULL,
  posts INTEGER NOT NULL,
  PRIMARY KEY (day, client)
);
