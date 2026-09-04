PRAGMA user_version = 0;
CREATE TABLE upgrade_fixture_sentinel (
    id INTEGER PRIMARY KEY,
    value TEXT NOT NULL
);
INSERT INTO upgrade_fixture_sentinel(id, value) VALUES (1, 'preserve-me');