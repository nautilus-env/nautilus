//! Type-safe row decoding.

use crate::Row;
use nautilus_core::{Column, FromValue, Result, SelectColumns};

/// Trait for decoding database rows into Rust types.
///
/// This trait enables type-safe conversion from `Row` to tuples or structs.
/// Implementations use positional decoding (`get_by_pos`) for performance.
/// The return type uses [`nautilus_core::Result`] so that struct impls can
/// return type / missing-field errors without a connector dependency.
pub trait FromRow: Sized {
    /// Decode a row into this type.
    ///
    /// Returns an error if:
    /// - A column is missing at the expected position
    /// - A value has an unexpected type
    /// - A NULL value is found for a non-nullable field
    fn from_row(row: &Row) -> Result<Self>;
}

/// Generates a [`FromRow`] implementation for a tuple of `FromValue` types.
///
/// Each implementation builds a temporary `SelectColumns` value from anonymous
/// `Column` descriptors (table name and column name are empty because decode uses
/// positional access) and delegates to `SelectColumns::decode`.
macro_rules! impl_from_row {
    ($($T:ident),+) => {
        impl<$($T: FromValue),+> FromRow for ($($T,)+) {
            fn from_row(row: &Row) -> Result<Self> {
                let columns = ($(Column::<$T>::new("", ""),)+);
                columns.decode(row)
            }
        }
    };
}

impl_from_row!(T1);
impl_from_row!(T1, T2);
impl_from_row!(T1, T2, T3);
impl_from_row!(T1, T2, T3, T4);
impl_from_row!(T1, T2, T3, T4, T5);
impl_from_row!(T1, T2, T3, T4, T5, T6);
impl_from_row!(T1, T2, T3, T4, T5, T6, T7);
impl_from_row!(T1, T2, T3, T4, T5, T6, T7, T8);

#[cfg(test)]
mod tests {
    use core::f64;

    use super::*;
    use crate::Row;
    use nautilus_core::Value;

    fn row(values: Vec<Value>) -> Row {
        Row::new(
            values
                .into_iter()
                .enumerate()
                .map(|(i, v)| (format!("c{}", i), v))
                .collect(),
        )
    }

    #[test]
    fn test_from_row_1_tuple() {
        let r = row(vec![Value::I64(42)]);
        let (a,): (i64,) = FromRow::from_row(&r).unwrap();
        assert_eq!(a, 42);
    }

    #[test]
    fn test_from_row_2_tuple() {
        let r = row(vec![Value::I64(1), Value::String("hello".to_string())]);
        let (a, b): (i64, String) = FromRow::from_row(&r).unwrap();
        assert_eq!(a, 1);
        assert_eq!(b, "hello");
    }

    #[test]
    fn test_from_row_3_tuple() {
        let r = row(vec![
            Value::I64(7),
            Value::Bool(true),
            Value::F64(f64::consts::PI),
        ]);
        let (a, b, c): (i64, bool, f64) = FromRow::from_row(&r).unwrap();
        assert_eq!(a, 7);
        assert!(b);
        assert!((c - f64::consts::PI).abs() < f64::EPSILON);
    }

    #[test]
    fn test_from_row_missing_column_returns_error() {
        let r = row(vec![Value::I64(1)]);
        let result: Result<(i64, i64)> = FromRow::from_row(&r);
        assert!(result.is_err());
    }
}
