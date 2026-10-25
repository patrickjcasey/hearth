-- One row per counter per opt-in usage report. Nothing here identifies a
-- person: install_id is random and changes whenever stats are re-enabled.
CREATE TABLE usage_events (
    id           INTEGER PRIMARY KEY,
    install_id   TEXT NOT NULL,
    app_version  TEXT NOT NULL,
    platform     TEXT NOT NULL,
    event        TEXT NOT NULL,
    count        INTEGER NOT NULL,
    received_at  TEXT NOT NULL
);

CREATE INDEX usage_events_install ON usage_events (install_id, received_at);
CREATE INDEX usage_events_event ON usage_events (event, received_at);
