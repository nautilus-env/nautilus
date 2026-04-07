use nautilus_core::{RowAccess, Value};

pub struct StandardUserExpectation<'a> {
    pub id: i64,
    pub name: &'a str,
    pub email: &'a str,
    pub age: i64,
    pub score: f64,
    pub active: bool,
    pub data: &'a [u8],
}

pub fn assert_standard_user_row<'row, R>(row: &'row R, expected: StandardUserExpectation<'_>)
where
    R: RowAccess<'row>,
{
    assert_eq!(row.get("id"), Some(&Value::I64(expected.id)));
    assert_eq!(
        row.get("name"),
        Some(&Value::String(expected.name.to_string()))
    );
    assert_eq!(
        row.get("email"),
        Some(&Value::String(expected.email.to_string()))
    );
    assert_eq!(row.get("age"), Some(&Value::I64(expected.age)));
    assert_eq!(row.get("score"), Some(&Value::F64(expected.score)));
    assert_eq!(row.get("active"), Some(&Value::Bool(expected.active)));
    assert_eq!(row.get("data"), Some(&Value::Bytes(expected.data.to_vec())));
}

pub fn assert_positional_projection<'row, R>(row: &'row R, id: i64, name: &str)
where
    R: RowAccess<'row>,
{
    assert_eq!(row.get_by_pos(0), Some(&Value::I64(id)));
    assert_eq!(row.get_by_pos(1), Some(&Value::String(name.to_string())));
    assert_eq!(row.get_by_pos(2), Some(&Value::Null));
    assert_eq!(row.get_by_pos(3), None);
}

pub fn assert_null_user_row<'row, R>(row: &'row R, id: i64, name: &str)
where
    R: RowAccess<'row>,
{
    assert_eq!(row.get("id"), Some(&Value::I64(id)));
    assert_eq!(row.get("name"), Some(&Value::String(name.to_string())));
    assert_eq!(row.get("email"), Some(&Value::Null));
    assert_eq!(row.get("age"), Some(&Value::Null));
    assert_eq!(row.get("score"), Some(&Value::Null));
    assert_eq!(row.get("active"), Some(&Value::Null));
    assert_eq!(row.get("data"), Some(&Value::Null));
}

pub fn assert_duplicate_projection<'row, R>(row: &'row R, id: i64, name: &str)
where
    R: RowAccess<'row>,
{
    assert_eq!(row.get("id"), Some(&Value::I64(id)));
    assert_eq!(row.len(), 3);
    assert_eq!(row.get_by_pos(0), Some(&Value::I64(id)));
    assert_eq!(row.get_by_pos(1), Some(&Value::String(name.to_string())));
    assert_eq!(row.get_by_pos(2), Some(&Value::I64(id)));
}

pub fn assert_sequential_user_rows<'row, R>(rows: &'row [R], start: i64)
where
    R: RowAccess<'row>,
{
    for (i, row) in rows.iter().enumerate() {
        let expected_id = start + i as i64;
        assert_eq!(row.get("id"), Some(&Value::I64(expected_id)));
        assert_eq!(
            row.get("name"),
            Some(&Value::String(format!("User{}", expected_id)))
        );
    }
}
