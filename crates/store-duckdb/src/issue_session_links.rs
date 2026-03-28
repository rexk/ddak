use duckdb::{Connection, OptionalExt, params};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkKind {
    Primary,
    Secondary,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueSessionLink {
    pub issue_id: String,
    pub session_id: String,
    pub is_primary: bool,
}

pub struct IssueSessionLinksStore<'a> {
    conn: &'a Connection,
}

impl<'a> IssueSessionLinksStore<'a> {
    pub fn new(conn: &'a Connection) -> Self {
        Self { conn }
    }

    pub fn create_link(
        &mut self,
        issue_id: &str,
        session_id: &str,
        kind: LinkKind,
    ) -> duckdb::Result<IssueSessionLink> {
        let tx = self.conn.unchecked_transaction()?;
        let is_primary = matches!(kind, LinkKind::Primary);

        if is_primary && issue_is_in_progress(&tx, issue_id)? {
            tx.execute(
                "UPDATE issue_session_links SET is_primary = FALSE WHERE issue_id = ? AND session_id <> ?",
                params![issue_id, session_id],
            )?;
        }

        tx.execute(
            "INSERT INTO issue_session_links(issue_id, session_id, is_primary)
             VALUES (?, ?, ?)
             ON CONFLICT(issue_id, session_id)
             DO UPDATE SET is_primary = excluded.is_primary",
            params![issue_id, session_id, is_primary],
        )?;

        tx.commit()?;

        Ok(IssueSessionLink {
            issue_id: issue_id.to_string(),
            session_id: session_id.to_string(),
            is_primary,
        })
    }

    pub fn list_by_issue(&self, issue_id: &str) -> duckdb::Result<Vec<IssueSessionLink>> {
        let mut stmt = self.conn.prepare(
            "SELECT issue_id, session_id, is_primary
             FROM issue_session_links
             WHERE issue_id = ?
             ORDER BY is_primary DESC, created_at ASC",
        )?;

        let rows = stmt.query_map(params![issue_id], |row| {
            Ok(IssueSessionLink {
                issue_id: row.get(0)?,
                session_id: row.get(1)?,
                is_primary: row.get(2)?,
            })
        })?;

        rows.collect()
    }

    pub fn primary_for_issue(&self, issue_id: &str) -> duckdb::Result<Option<IssueSessionLink>> {
        self.conn
            .query_row(
                "SELECT issue_id, session_id, is_primary
                 FROM issue_session_links
                 WHERE issue_id = ? AND is_primary = TRUE
                 ORDER BY created_at ASC
                 LIMIT 1",
                params![issue_id],
                |row| {
                    Ok(IssueSessionLink {
                        issue_id: row.get(0)?,
                        session_id: row.get(1)?,
                        is_primary: row.get(2)?,
                    })
                },
            )
            .optional()
    }
}

fn issue_is_in_progress(conn: &Connection, issue_id: &str) -> duckdb::Result<bool> {
    conn.query_row(
        "SELECT status = 'in_progress' FROM issues WHERE id = ?",
        params![issue_id],
        |row| row.get(0),
    )
}

#[cfg(test)]
mod tests {
    use super::{IssueSessionLinksStore, LinkKind};
    use crate::Migrator;
    use duckdb::{Connection, params};

    fn seed_issue(conn: &Connection, issue_id: &str, status: &str) {
        conn.execute(
            "INSERT INTO issues(id, board_id, project_id, title, status)
             VALUES (?, 'board-1', 'project-1', 'Example issue', ?)",
            params![issue_id, status],
        )
        .expect("issue should seed");
    }

    fn seed_session(conn: &Connection, session_id: &str) {
        conn.execute(
            "INSERT INTO sessions(id, status) VALUES (?, 'running')",
            params![session_id],
        )
        .expect("session should seed");
    }

    #[test]
    fn create_link_supports_secondary_sessions() {
        let conn = Connection::open_in_memory().expect("in-memory db should open");
        Migrator::apply_all(&conn).expect("migrations should apply");

        seed_issue(&conn, "issue-1", "in_progress");
        seed_session(&conn, "sess-1");

        let mut store = IssueSessionLinksStore::new(&conn);
        store
            .create_link("issue-1", "sess-1", LinkKind::Secondary)
            .expect("secondary link should be created");

        let links = store
            .list_by_issue("issue-1")
            .expect("issue links query should succeed");

        assert_eq!(links.len(), 1);
        assert_eq!(links[0].session_id, "sess-1");
        assert!(!links[0].is_primary);
        assert!(
            store
                .primary_for_issue("issue-1")
                .expect("primary lookup should succeed")
                .is_none()
        );
    }

    #[test]
    fn in_progress_issue_keeps_single_primary_session() {
        let conn = Connection::open_in_memory().expect("in-memory db should open");
        Migrator::apply_all(&conn).expect("migrations should apply");

        seed_issue(&conn, "issue-1", "in_progress");
        seed_session(&conn, "sess-1");
        seed_session(&conn, "sess-2");

        let mut store = IssueSessionLinksStore::new(&conn);
        store
            .create_link("issue-1", "sess-1", LinkKind::Primary)
            .expect("first primary link should be created");
        store
            .create_link("issue-1", "sess-2", LinkKind::Primary)
            .expect("second primary link should replace first");

        let links = store
            .list_by_issue("issue-1")
            .expect("issue links query should succeed");
        let primary_count = links.iter().filter(|link| link.is_primary).count();

        assert_eq!(primary_count, 1);

        let primary = store
            .primary_for_issue("issue-1")
            .expect("primary lookup should succeed")
            .expect("a primary should exist");

        assert_eq!(primary.session_id, "sess-2");
    }
}
