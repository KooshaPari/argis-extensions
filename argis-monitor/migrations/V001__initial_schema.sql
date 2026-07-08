-- V001: initial schema for argis-monitor state store.
-- Matches the inline SCHEMA constant that previously lived in state_store.rs.

CREATE TABLE IF NOT EXISTS alert_state (
    key             TEXT PRIMARY KEY,
    state           TEXT NOT NULL,
    since_unix      INTEGER NOT NULL,
    last_fired_unix INTEGER NOT NULL DEFAULT 0,
    sustained_secs  INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS idx_alert_state_key ON alert_state(key);

CREATE TABLE IF NOT EXISTS alert_history (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    key             TEXT NOT NULL,
    event           TEXT NOT NULL,
    severity        TEXT NOT NULL,
    burn_rate       REAL NOT NULL,
    threshold       REAL NOT NULL,
    payload_json    TEXT NOT NULL,
    fired_at_unix   INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_alert_history_key ON alert_history(key);
CREATE INDEX IF NOT EXISTS idx_alert_history_fired_at ON alert_history(fired_at_unix);
