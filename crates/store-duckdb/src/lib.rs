use duckdb::Connection;

pub mod issue_session_links;

pub const CRATE_NAME: &str = "store-duckdb";
const LATEST_SCHEMA_VERSION: i64 = 3;

const MIGRATION_001: &str = r#"
CREATE TABLE IF NOT EXISTS schema_migrations (
  version BIGINT PRIMARY KEY,
  applied_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS workspaces (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS teams (
  id TEXT PRIMARY KEY,
  workspace_id TEXT NOT NULL,
  name TEXT NOT NULL,
  created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS projects (
  id TEXT PRIMARY KEY,
  workspace_id TEXT NOT NULL,
  team_id TEXT,
  name TEXT NOT NULL,
  created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS boards (
  id TEXT PRIMARY KEY,
  project_id TEXT NOT NULL,
  name TEXT NOT NULL,
  created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS board_columns (
  id TEXT PRIMARY KEY,
  board_id TEXT NOT NULL,
  status TEXT NOT NULL,
  position INTEGER NOT NULL,
  created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS issues (
  id TEXT PRIMARY KEY,
  board_id TEXT NOT NULL,
  column_id TEXT,
  project_id TEXT NOT NULL,
  team_id TEXT,
  identifier TEXT,
  title TEXT NOT NULL,
  description TEXT,
  status TEXT NOT NULL,
  priority TEXT,
  assignee_id TEXT,
  labels TEXT,
  estimate INTEGER,
  due_date TIMESTAMP,
  position INTEGER NOT NULL DEFAULT 0,
  created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
  updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
  completed_at TIMESTAMP
);

CREATE TABLE IF NOT EXISTS sessions (
  id TEXT PRIMARY KEY,
  issue_id TEXT,
  project_id TEXT,
  status TEXT NOT NULL,
  adapter TEXT,
  adapter_session_ref TEXT,
  runtime_pid BIGINT,
  created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
  updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS session_events (
  event_id TEXT PRIMARY KEY,
  session_id TEXT NOT NULL,
  event_type TEXT NOT NULL,
  session_seq BIGINT NOT NULL,
  correlation_id TEXT,
  payload_json TEXT,
  ts TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS issue_session_links (
  issue_id TEXT NOT NULL,
  session_id TEXT NOT NULL,
  is_primary BOOLEAN NOT NULL DEFAULT FALSE,
  created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
  PRIMARY KEY (issue_id, session_id)
);

CREATE TABLE IF NOT EXISTS integrations (
  id TEXT PRIMARY KEY,
  workspace_id TEXT NOT NULL,
  integration_type TEXT NOT NULL,
  config_json TEXT,
  created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
  updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS integration_mappings (
  id TEXT PRIMARY KEY,
  integration_id TEXT NOT NULL,
  project_id TEXT,
  external_id TEXT NOT NULL,
  object_type TEXT NOT NULL,
  local_id TEXT NOT NULL,
  mapping_version BIGINT DEFAULT 1,
  last_validated_at TIMESTAMP,
  created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
  updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS sync_state (
  integration_id TEXT PRIMARY KEY,
  last_cursor TEXT,
  last_synced_at TIMESTAMP,
  status TEXT,
  error_message TEXT
);

CREATE INDEX IF NOT EXISTS idx_sessions_project_status ON sessions(project_id, status);
CREATE INDEX IF NOT EXISTS idx_session_events_session_ts ON session_events(session_id, ts);
CREATE INDEX IF NOT EXISTS idx_issues_board_column_position ON issues(board_id, column_id, position);
CREATE INDEX IF NOT EXISTS idx_integration_mappings_external_id ON integration_mappings(external_id);

INSERT INTO schema_migrations(version)
SELECT 1
WHERE NOT EXISTS (SELECT 1 FROM schema_migrations WHERE version = 1);
"#;

const MIGRATION_002: &str = r#"
ALTER TABLE projects ADD COLUMN IF NOT EXISTS identifier TEXT;
ALTER TABLE projects ADD COLUMN IF NOT EXISTS description TEXT;
ALTER TABLE projects ADD COLUMN IF NOT EXISTS status TEXT;
ALTER TABLE projects ADD COLUMN IF NOT EXISTS lead_id TEXT;
ALTER TABLE projects ADD COLUMN IF NOT EXISTS updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP;
ALTER TABLE projects ADD COLUMN IF NOT EXISTS archived_at TIMESTAMP;
ALTER TABLE projects ADD COLUMN IF NOT EXISTS custom_fields_json TEXT;

ALTER TABLE issues ADD COLUMN IF NOT EXISTS custom_fields_json TEXT;

INSERT INTO schema_migrations(version)
SELECT 2
WHERE NOT EXISTS (SELECT 1 FROM schema_migrations WHERE version = 2);
"#;

const MIGRATION_003: &str = r#"
CREATE UNIQUE INDEX IF NOT EXISTS uq_projects_workspace_identifier
ON projects(workspace_id, identifier);

CREATE UNIQUE INDEX IF NOT EXISTS uq_issues_project_identifier
ON issues(project_id, identifier);

INSERT INTO schema_migrations(version)
SELECT 3
WHERE NOT EXISTS (SELECT 1 FROM schema_migrations WHERE version = 3);
"#;

pub struct Migrator;

impl Migrator {
    pub fn apply_all(conn: &Connection) -> duckdb::Result<()> {
        conn.execute_batch(MIGRATION_001)?;
        conn.execute_batch(MIGRATION_002)?;
        conn.execute_batch(MIGRATION_003)?;
        Ok(())
    }

    pub fn latest_applied_version(conn: &Connection) -> duckdb::Result<i64> {
        conn.query_row(
            "SELECT COALESCE(MAX(version), 0) FROM schema_migrations",
            [],
            |row| row.get(0),
        )
    }
}

pub fn open_and_migrate(path: &str) -> duckdb::Result<Connection> {
    let conn = Connection::open(path)?;
    Migrator::apply_all(&conn)?;
    Ok(conn)
}

pub fn latest_schema_version() -> i64 {
    LATEST_SCHEMA_VERSION
}

#[cfg(test)]
mod tests {
    use super::*;
    use duckdb::params;

    fn table_exists(conn: &Connection, table_name: &str) -> duckdb::Result<bool> {
        conn.query_row(
            "SELECT COUNT(*) FROM information_schema.tables WHERE lower(table_name) = lower(?)",
            params![table_name],
            |row| row.get::<_, i64>(0),
        )
        .map(|count| count > 0)
    }

    fn index_exists(conn: &Connection, index_name: &str) -> duckdb::Result<bool> {
        conn.query_row(
            "SELECT COUNT(*) FROM duckdb_indexes() WHERE lower(index_name) = lower(?)",
            params![index_name],
            |row| row.get::<_, i64>(0),
        )
        .map(|count| count > 0)
    }

    fn column_exists(
        conn: &Connection,
        table_name: &str,
        column_name: &str,
    ) -> duckdb::Result<bool> {
        conn.query_row(
            "SELECT COUNT(*)
             FROM information_schema.columns
             WHERE lower(table_name) = lower(?)
               AND lower(column_name) = lower(?)",
            params![table_name, column_name],
            |row| row.get::<_, i64>(0),
        )
        .map(|count| count > 0)
    }

    #[test]
    fn applies_migrations_and_creates_expected_tables_and_indexes() {
        let conn = Connection::open_in_memory().expect("in-memory db should open");

        Migrator::apply_all(&conn).expect("migrations should apply");

        let tables = [
            "workspaces",
            "teams",
            "projects",
            "boards",
            "board_columns",
            "issues",
            "sessions",
            "session_events",
            "issue_session_links",
            "integrations",
            "integration_mappings",
            "sync_state",
        ];

        for table in tables {
            assert!(
                table_exists(&conn, table).unwrap(),
                "table missing: {table}"
            );
        }

        let indexes = [
            "idx_sessions_project_status",
            "idx_session_events_session_ts",
            "idx_issues_board_column_position",
            "idx_integration_mappings_external_id",
            "uq_projects_workspace_identifier",
            "uq_issues_project_identifier",
        ];

        for index in indexes {
            assert!(
                index_exists(&conn, index).unwrap(),
                "index missing: {index}"
            );
        }

        let project_columns = [
            "identifier",
            "description",
            "status",
            "lead_id",
            "updated_at",
            "archived_at",
            "custom_fields_json",
        ];
        for column in project_columns {
            assert!(
                column_exists(&conn, "projects", column).unwrap(),
                "projects column missing: {column}"
            );
        }

        assert!(
            column_exists(&conn, "issues", "custom_fields_json").unwrap(),
            "issues column missing: custom_fields_json"
        );
    }

    #[test]
    fn migrations_are_idempotent() {
        let conn = Connection::open_in_memory().expect("in-memory db should open");

        Migrator::apply_all(&conn).expect("first migration pass should succeed");
        Migrator::apply_all(&conn).expect("second migration pass should also succeed");

        let latest = Migrator::latest_applied_version(&conn).expect("query should succeed");
        assert_eq!(latest, latest_schema_version());

        let version_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM schema_migrations WHERE version IN (1, 2, 3)",
                [],
                |row| row.get(0),
            )
            .expect("version count query should succeed");

        assert_eq!(version_count, 3);
    }
}
