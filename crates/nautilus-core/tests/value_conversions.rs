//! Integration tests for nautilus-core Value conversions and equality.
//!
//! Tests From<T> impls, Option<T> via Into, and null/equality checks.

use core::f64;

use nautilus_core::{Geography, Geometry, Value};

#[test]
fn from_bool() {
    assert_eq!(Value::from(true), Value::Bool(true));
    assert_eq!(Value::from(false), Value::Bool(false));
}

#[test]
fn from_i32() {
    assert_eq!(Value::from(0i32), Value::I32(0));
    assert_eq!(Value::from(-1i32), Value::I32(-1));
    assert_eq!(Value::from(i32::MAX), Value::I32(i32::MAX));
}

#[test]
fn from_i64() {
    assert_eq!(Value::from(0i64), Value::I64(0));
    assert_eq!(Value::from(i64::MIN), Value::I64(i64::MIN));
    assert_eq!(Value::from(i64::MAX), Value::I64(i64::MAX));
}

#[test]
fn from_f64() {
    let v = Value::from(f64::consts::PI);
    assert_eq!(v, Value::F64(f64::consts::PI));
}

#[test]
fn from_str_ref() {
    let v = Value::from("hello");
    assert_eq!(v, Value::String("hello".to_string()));
}

#[test]
fn from_string() {
    let v = Value::from("world".to_string());
    assert_eq!(v, Value::String("world".to_string()));
}

#[test]
fn from_bytes() {
    let v = Value::from(vec![1u8, 2, 3]);
    assert_eq!(v, Value::Bytes(vec![1, 2, 3]));
}

#[test]
fn from_json_value() {
    let json = serde_json::json!({"key": "value"});
    let v = Value::from(json.clone());
    assert_eq!(v, Value::Json(json));
}

#[test]
fn from_postgis_spatial_newtypes() {
    assert_eq!(
        Value::from(Geometry::from("POINT(1 2)")),
        Value::Geometry("POINT(1 2)".to_string())
    );
    assert_eq!(
        Value::from(Geography::from("SRID=4326;POINT(1 2)")),
        Value::Geography("SRID=4326;POINT(1 2)".to_string())
    );
}

#[test]
fn null_is_null() {
    assert_eq!(Value::Null, Value::Null);
}

#[test]
fn null_not_equal_to_zero() {
    assert_ne!(Value::Null, Value::I64(0));
}

#[test]
fn null_not_equal_to_empty_string() {
    assert_ne!(Value::Null, Value::String(String::new()));
}

#[test]
fn array_equality() {
    let a = Value::Array(vec![Value::I64(1), Value::I64(2), Value::I64(3)]);
    let b = Value::Array(vec![Value::I64(1), Value::I64(2), Value::I64(3)]);
    assert_eq!(a, b);
}

#[test]
fn array_different_lengths_not_equal() {
    let a = Value::Array(vec![Value::I64(1), Value::I64(2)]);
    let b = Value::Array(vec![Value::I64(1)]);
    assert_ne!(a, b);
}

#[test]
fn array_2d() {
    let v = Value::Array2D(vec![
        vec![Value::I64(1), Value::I64(2)],
        vec![Value::I64(3), Value::I64(4)],
    ]);
    if let Value::Array2D(rows) = &v {
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].len(), 2);
    } else {
        panic!("expected Array2D");
    }
}

#[test]
fn enum_value_equality() {
    let a = Value::Enum {
        value: "ADMIN".to_string(),
        type_name: "role".to_string(),
    };
    let b = Value::Enum {
        value: "ADMIN".to_string(),
        type_name: "role".to_string(),
    };
    let c = Value::Enum {
        value: "USER".to_string(),
        type_name: "role".to_string(),
    };
    assert_eq!(a, b);
    assert_ne!(a, c);
}

#[test]
fn value_clone() {
    let original = Value::String("test".to_string());
    let cloned = original.clone();
    assert_eq!(original, cloned);
}

#[test]
fn array_clone() {
    let original = Value::Array(vec![Value::I64(1), Value::Bool(false)]);
    let cloned = original.clone();
    assert_eq!(original, cloned);
}
