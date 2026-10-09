//! ltree's binary payload is a version byte followed by the UTF-8 path.

use sqlx::error::BoxDynError;
use sqlx::postgres::{PgTypeInfo, PgValueFormat, PgValueRef};
use sqlx::{Decode, Postgres, Type};

pub(super) struct PgLtree(pub(super) String);

impl Type<Postgres> for PgLtree {
    fn type_info() -> PgTypeInfo {
        PgTypeInfo::with_name("ltree")
    }
}

impl<'r> Decode<'r, Postgres> for PgLtree {
    fn decode(value: PgValueRef<'r>) -> std::result::Result<Self, BoxDynError> {
        decode_ltree(value.as_bytes()?, value.format()).map(Self)
    }
}

pub(super) fn decode_ltree(
    bytes: &[u8],
    format: PgValueFormat,
) -> std::result::Result<String, BoxDynError> {
    let text = match format {
        PgValueFormat::Text => bytes,
        PgValueFormat::Binary => match bytes.split_first() {
            Some((1, text)) => text,
            Some((version, _)) => {
                return Err(format!("Unsupported LTREE binary version: {version}").into());
            }
            None => return Err("Missing LTREE binary version".into()),
        },
    };
    // PostgreSQL has already validated the labels. SQLx's PgLTree restricts
    // them to ASCII and 256 bytes, excluding paths newer servers accept.
    Ok(std::str::from_utf8(text)?.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binary_paths_drop_only_the_checked_version_byte() {
        for path in ["".to_string(), "top.caffè".to_string(), "x".repeat(300)] {
            let mut bytes = vec![1];
            bytes.extend_from_slice(path.as_bytes());
            assert_eq!(decode_ltree(&bytes, PgValueFormat::Binary).unwrap(), path);
        }
    }

    #[test]
    fn text_paths_are_preserved() {
        for path in ["", "top.c", "top.caffè", "\u{1}top.c"] {
            assert_eq!(
                decode_ltree(path.as_bytes(), PgValueFormat::Text).unwrap(),
                path
            );
        }
    }

    #[test]
    fn malformed_binary_paths_are_errors() {
        for bytes in [&[][..], &[0], &[2, b'a'], &[1, 0xff]] {
            assert!(decode_ltree(bytes, PgValueFormat::Binary).is_err());
        }
    }

    #[tokio::test]
    #[ignore = "requires PostgreSQL; set NAUTILUS_TEST_POSTGRES_URL"]
    async fn postgres_ltree_binary_and_text() {
        use nautilus_core::Value;
        use sqlx::Row as _;

        let url = std::env::var("NAUTILUS_TEST_POSTGRES_URL").expect("PostgreSQL test URL");
        let pool = sqlx::PgPool::connect(&url).await.unwrap();
        sqlx::query("CREATE EXTENSION IF NOT EXISTS ltree")
            .execute(&pool)
            .await
            .unwrap();
        let query = "SELECT 'top.caffè'::ltree AS path, \
            ARRAY['top.c', '', NULL, repeat('x', 300)]::ltree[] AS paths, \
            NULL::ltree AS null_path, ARRAY[]::ltree[] AS empty_paths, \
            NULL::ltree[] AS null_paths, 'top.c'::text AS plain_text";
        let binary = sqlx::query(query).fetch_one(&pool).await.unwrap();
        let text = sqlx::raw_sql(query).fetch_one(&pool).await.unwrap();
        for (row, format) in [(binary, PgValueFormat::Binary), (text, PgValueFormat::Text)] {
            assert_eq!(row.try_get_raw("path").unwrap().format(), format);
            assert_eq!(
                row.try_get_unchecked::<PgLtree, _>("path").unwrap().0,
                "top.caffè"
            );
            let paths: Vec<Option<PgLtree>> = row.try_get_unchecked("paths").unwrap();
            assert_eq!(
                paths
                    .into_iter()
                    .map(|path| path.map(|path| path.0))
                    .collect::<Vec<_>>(),
                vec![
                    Some("top.c".into()),
                    Some(String::new()),
                    None,
                    Some("x".repeat(300))
                ]
            );
            if format == PgValueFormat::Text {
                continue;
            }
            let row = super::super::stream::decode_row_internal(row).unwrap();
            assert_eq!(row.get("path"), Some(&Value::String("top.caffè".into())));
            assert_eq!(
                row.get("paths"),
                Some(&Value::Array(vec![
                    Value::String("top.c".into()),
                    Value::String(String::new()),
                    Value::Null,
                    Value::String("x".repeat(300)),
                ]))
            );
            assert_eq!(row.get("null_path"), Some(&Value::Null));
            assert_eq!(row.get("empty_paths"), Some(&Value::Array(vec![])));
            assert_eq!(row.get("null_paths"), Some(&Value::Null));
            assert_eq!(row.get("plain_text"), Some(&Value::String("top.c".into())));
        }
        pool.close().await;
    }

    #[tokio::test]
    #[ignore = "requires PostgreSQL; set NAUTILUS_TEST_POSTGRES_URL"]
    async fn postgres_ltree_inside_composites() {
        use nautilus_core::Value;

        let url = std::env::var("NAUTILUS_TEST_POSTGRES_URL").expect("PostgreSQL test URL");
        let pool = sqlx::PgPool::connect(&url).await.unwrap();
        sqlx::query("CREATE EXTENSION IF NOT EXISTS ltree")
            .execute(&pool)
            .await
            .unwrap();
        let name = format!("route_{}", uuid::Uuid::new_v4().simple());
        sqlx::query(&format!(
            "CREATE TYPE \"{name}\" AS (path ltree, note text)"
        ))
        .execute(&pool)
        .await
        .unwrap();
        let row = sqlx::query(&format!(
            "SELECT ROW('top.caffè'::ltree, 'fragile')::\"{name}\" AS route"
        ))
        .fetch_one(&pool)
        .await
        .unwrap();
        let row = super::super::stream::decode_row_internal(row).unwrap();
        assert_eq!(
            row.get("route"),
            Some(&Value::String("(\"top.caffè\",\"fragile\")".into()))
        );
        sqlx::query(&format!("DROP TYPE \"{name}\""))
            .execute(&pool)
            .await
            .unwrap();
        pool.close().await;
    }
}
