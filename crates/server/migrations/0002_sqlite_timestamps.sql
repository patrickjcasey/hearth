-- received_at used to be RFC 3339 ("2026-01-02T03:04:05.123+00:00"), which
-- doesn't compare correctly with SQLite's datetime() ("2026-01-02 03:04:05").
-- Store SQLite's UTC format instead and convert the existing rows.
UPDATE usage_events SET received_at = datetime(received_at) WHERE received_at LIKE '%T%';
