use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// A database migration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Migration {
    /// Migration name (should be unique and timestamp-based)
    pub name: String,

    /// SQL statements for this migration (up direction)
    pub up_sql: Vec<String>,

    /// SQL statements to rollback (down direction)
    pub down_sql: Vec<String>,

    /// SHA256 checksum of the migration content
    pub checksum: String,

    /// When the migration was created
    pub created_at: DateTime<Utc>,
}

impl Migration {
    /// Create a new migration
    pub fn new(name: String, up_sql: Vec<String>, down_sql: Vec<String>) -> Self {
        let checksum = Self::calculate_checksum(&up_sql, &down_sql);
        Self {
            name,
            up_sql,
            down_sql,
            checksum,
            created_at: Utc::now(),
        }
    }

    /// Calculate SHA256 checksum of migration content
    fn calculate_checksum(up_sql: &[String], down_sql: &[String]) -> String {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();

        for stmt in up_sql {
            hasher.update(stmt.as_bytes());
            hasher.update(b"\n");
        }
        hasher.update(b"---DOWN---\n");
        for stmt in down_sql {
            hasher.update(stmt.as_bytes());
            hasher.update(b"\n");
        }

        hasher
            .finalize()
            .iter()
            .map(|b| format!("{:02x}", b))
            .collect()
    }

    /// Verify checksum matches
    pub fn verify_checksum(&self) -> bool {
        self.checksum == Self::calculate_checksum(&self.up_sql, &self.down_sql)
    }
}

/// Migration direction
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MigrationDirection {
    /// Apply migration (up)
    Up,
    /// Rollback migration (down)
    Down,
}

/// Status of a migration
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MigrationStatus {
    /// Migration is pending (not applied)
    Pending,
    /// Migration is applied
    Applied,
    /// Migration failed during application
    Failed,
}

impl std::fmt::Display for MigrationStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MigrationStatus::Pending => write!(f, "pending"),
            MigrationStatus::Applied => write!(f, "applied"),
            MigrationStatus::Failed => write!(f, "failed"),
        }
    }
}

/// Record of an applied migration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MigrationRecord {
    /// Migration name
    pub name: String,
    /// Checksum at time of application
    pub checksum: String,
    /// When it was applied
    pub applied_at: DateTime<Utc>,
    /// How long it took to apply (milliseconds)
    pub execution_time_ms: i64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checksum_is_deterministic() {
        let up = vec!["CREATE TABLE t (id INT)".to_string()];
        let down = vec!["DROP TABLE t".to_string()];
        let m1 = Migration::new("m1".into(), up.clone(), down.clone());
        let m2 = Migration::new("m2".into(), up, down);

        assert_eq!(m1.checksum, m2.checksum);
    }

    #[test]
    fn different_sql_produces_different_checksum() {
        let m1 = Migration::new(
            "a".into(),
            vec!["CREATE TABLE a (id INT)".to_string()],
            vec![],
        );
        let m2 = Migration::new(
            "b".into(),
            vec!["CREATE TABLE b (id INT)".to_string()],
            vec![],
        );
        assert_ne!(m1.checksum, m2.checksum);
    }

    #[test]
    fn verify_checksum_passes_for_fresh_migration() {
        let m = Migration::new(
            "init".into(),
            vec!["SELECT 1".to_string()],
            vec!["SELECT 2".to_string()],
        );
        assert!(m.verify_checksum());
    }

    #[test]
    fn verify_checksum_fails_after_tampering() {
        let mut m = Migration::new(
            "init".into(),
            vec!["SELECT 1".to_string()],
            vec!["SELECT 2".to_string()],
        );
        m.up_sql.push("EXTRA".to_string());
        assert!(!m.verify_checksum());
    }
}
