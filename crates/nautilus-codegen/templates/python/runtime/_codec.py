"""Wire codec rules shared by every generated model.

Nothing here depends on a specific model: the parts that do — the extension
serializers of a field, the columns that hold object-like values, the
Python-to-column maps — are passed in by the model module that calls into it.
"""

from __future__ import annotations

import base64
from dataclasses import asdict, is_dataclass
from datetime import datetime
from decimal import Decimal
from enum import Enum
from typing import Any, Callable, Dict, FrozenSet, List, Mapping, Optional, Union
from uuid import UUID

JsonPrimitive = Union[str, int, float, bool, None]
JsonValue = Union[JsonPrimitive, Dict[str, Any], List[Any]]
JsonScalarOrArray = Union[JsonPrimitive, List[Any]]
HstoreValue = Dict[str, Optional[str]]

FILTER_OPERATOR_KEYS: frozenset = frozenset({
    "contains",
    "endsWith",
    "endswith",
    "eq",
    "equals",
    "gt",
    "gte",
    "in",
    "in_",
    "isNull",
    "isNotNull",
    "is_not_null",
    "is_null",
    "like",
    "lt",
    "lte",
    "not",
    "notIn",
    "not_",
    "not_in",
    "startsWith",
    "startswith",
})


FILTER_OPERATOR_PROTOCOL_NAMES: Dict[str, str] = {
    "equals": "eq",
    "eq": "eq",
    "in_": "in",
    "not_": "not",
    "notIn": "notIn",
    "not_in": "notIn",
    "startsWith": "startsWith",
    "startswith": "startsWith",
    "endsWith": "endsWith",
    "endswith": "endsWith",
    "isNull": "isNull",
    "isNotNull": "isNotNull",
    "is_not_null": "isNotNull",
    "is_null": "isNull",
}

MISSING = object()

AGGREGATE_KEYS: FrozenSet[str] = frozenset({"_count", "_avg", "_sum", "_min", "_max"})


def coerce_boolean_value(value: Any) -> Any:
    """Read the shapes a driver may use for a boolean column."""
    if isinstance(value, bool):
        return value
    if isinstance(value, int) and value in (0, 1):
        return bool(value)
    if isinstance(value, str):
        lowered = value.lower()
        if lowered in ("true", "1"):
            return True
        if lowered in ("false", "0"):
            return False
    return value


def coerce_bytes_value(value: Any) -> Any:
    """Decode base64 Bytes fields, including elements of array fields."""
    if isinstance(value, str):
        return base64.b64decode(value, validate=True)
    if isinstance(value, list):
        return [coerce_bytes_value(item) for item in value]
    return value


def coerce_datetime_value(value: Any) -> Any:
    """Decode DateTime fields, sent as ISO 8601 text with a ``T`` or a space
    before the time, including elements of array fields."""
    if isinstance(value, str):
        return datetime.fromisoformat(value.replace(" ", "T"))
    if isinstance(value, list):
        return [coerce_datetime_value(item) for item in value]
    return value


def coerce_decimal_value(value: Any) -> Any:
    """Decode Decimal fields, sent as text or as JSON numbers, including
    elements of array fields."""
    if isinstance(value, str):
        return Decimal(value)
    if isinstance(value, (int, float)) and not isinstance(value, bool):
        return Decimal(str(value))
    if isinstance(value, list):
        return [coerce_decimal_value(item) for item in value]
    return value


def coerce_uuid_value(value: Any) -> Any:
    """Decode Uuid fields, sent as text, including elements of array fields."""
    if isinstance(value, str):
        return UUID(value)
    if isinstance(value, list):
        return [coerce_uuid_value(item) for item in value]
    return value


def serialize_wire_value(value: Any) -> Any:
    """Convert generated client values into JSON-RPC-safe payloads."""
    if isinstance(value, Enum):
        return value.value
    if isinstance(value, (bytes, bytearray, memoryview)):
        return base64.b64encode(value).decode("ascii")
    to_wire = getattr(value, "to_wire", None)
    if callable(to_wire):
        return serialize_wire_value(to_wire())
    if is_dataclass(value):
        return {k: serialize_wire_value(v) for k, v in asdict(value).items()}
    if isinstance(value, list):
        return [serialize_wire_value(item) for item in value]
    if isinstance(value, dict):
        return {k: serialize_wire_value(v) for k, v in value.items()}
    return value


def looks_like_filter_operator_dict(value: Dict[str, Any]) -> bool:
    """Tell a filter such as ``{"gt": 1}`` from a plain object value."""
    return bool(value) and all(key in FILTER_OPERATOR_KEYS for key in value)


def object_equality_requires_explicit_equals(field_name: str) -> None:
    """Refuse an ambiguous object filter on a JSON or HSTORE field."""
    raise TypeError(
        f"Field '{field_name}' stores object-like JSON/HSTORE values. "
        "Use {'equals': ...} for object equality filters."
    )


def first_data_row(result: Dict[str, Any]) -> Optional[Dict[str, Any]]:
    """Return the first row of a response, or ``None`` when there is none."""
    data = result.get("data")
    if not isinstance(data, list) or not data:
        return None
    row = data[0]
    if not isinstance(row, dict):
        return None
    return row


def get_wire_value(row: Dict[str, Any], *keys: str) -> Any:
    """Read the first key a row carries, or ``MISSING`` when it carries none."""
    for key in keys:
        if key in row:
            return row[key]
    return MISSING


def logical_field_keys(spec: Any, py_to_logical: Mapping[str, str]) -> Any:
    """Name the fields of a ``count``/``avg``/``sum``/``min``/``max`` input
    as the schema does; ``True`` and ``_all`` pass unchanged."""
    if not isinstance(spec, dict):
        return spec
    return {py_to_logical.get(key, key): value for key, value in spec.items()}


def logical_aggregate_keys(
    spec: Mapping[Any, Any], py_to_logical: Mapping[str, str]
) -> Dict[str, Any]:
    """Name the fields of a ``having`` filter or a ``group_by`` order item as
    the schema does: its own keys, and those inside ``_count``, ``_min``, …"""
    result: Dict[str, Any] = {}
    for key, value in spec.items():
        if key in AGGREGATE_KEYS:
            result[key] = logical_field_keys(value, py_to_logical)
        else:
            result[py_to_logical.get(key, key)] = value
    return result


def logical_aggregate_args(
    args: Dict[str, Any], py_to_logical: Mapping[str, str]
) -> Dict[str, Dict[str, str]]:
    """Name the fields of a ``group_by`` or ``aggregate`` request as the schema
    does, in place.

    Returns, for each aggregate whose fields the request names, the key each
    field gets in the result: the spelling the request used, so
    ``sum={"unitPrice": True}`` reads back as ``row["_sum"]["unitPrice"]``.
    """
    if "by" in args:
        args["by"] = [py_to_logical.get(field, field) for field in args["by"]]
    if "having" in args:
        args["having"] = logical_aggregate_keys(args["having"], py_to_logical)
    if "orderBy" in args:
        args["orderBy"] = [
            logical_aggregate_keys(item, py_to_logical) for item in args["orderBy"]
        ]
    requested: Dict[str, Dict[str, str]] = {}
    for key in ("count", "avg", "sum", "min", "max"):
        spec = args.get(key)
        if isinstance(spec, dict):
            requested["_" + key] = {py_to_logical.get(field, field): field for field in spec}
            args[key] = logical_field_keys(spec, py_to_logical)
    return requested


def aggregate_row_from_wire(
    row: Dict[str, Any],
    logical_to_py: Mapping[str, str],
    coerce: Callable[[str, Any], Any],
    requested: Mapping[str, Mapping[str, str]],
) -> Dict[str, Any]:
    """Name the keys of a ``group_by`` or ``aggregate`` row for Python.

    The engine names grouped columns, and the fields inside ``_count``,
    ``_min`` and the other aggregates, as the schema does. Grouped columns get
    their Python names, as the result types declare. A field inside an
    aggregate gets the spelling the request used (``requested``, from
    :func:`logical_aggregate_args`), or its Python name when the request asked
    for every field with ``True``. Grouped values and the ``_min``/``_max`` of
    a field are decoded as in a ``select`` projection.
    """
    result: Dict[str, Any] = {}
    for key, value in row.items():
        if key in AGGREGATE_KEYS and isinstance(value, dict):
            names = requested.get(key, {})
            decode = key in ("_min", "_max")
            fields: Dict[str, Any] = {}
            for field, field_value in value.items():
                py_field = logical_to_py.get(field, field)
                if decode and field_value is not None:
                    field_value = coerce(py_field, field_value)
                fields[names.get(field, py_field)] = field_value
            result[key] = fields
        else:
            py_key = logical_to_py.get(key, key)
            result[py_key] = None if value is None else coerce(py_key, value)
    return result


def process_select_fields(
    select: Dict[str, bool], py_to_logical: Mapping[str, str]
) -> Dict[str, bool]:
    """Convert Python field names to logical field names for query projection."""
    result = {}
    for key, value in select.items():
        logical_key = py_to_logical.get(key, key)
        result[logical_key] = value
    return result


def serialize_include_args(
    spec: Any,
    where_filters: Callable[[Dict[str, Any], Mapping[str, str]], Dict[str, Any]],
    py_to_db: Mapping[str, str],
    serialize_include: Callable[[Any], Any],
) -> Any:
    """Prepare one include node against the model it loads.

    The node has the shape of a read's arguments and gets the same
    preparation: its ``where`` goes through the serializers of the included
    model rather than the parent's, and ``order_by`` becomes the ``orderBy``
    list the engine expects.
    """
    if spec is None or isinstance(spec, bool):
        return spec
    if not isinstance(spec, dict):
        return spec
    node: Dict[str, Any] = {}
    for key, value in spec.items():
        if value is None:


            continue
        if key in ("order_by", "orderBy"):
            if isinstance(value, dict):
                node["orderBy"] = [{fk: fv} for fk, fv in value.items()]
            else:
                node["orderBy"] = value
        elif key == "where":
            node["where"] = (
                where_filters(value, py_to_db) if isinstance(value, dict) else value
            )
        elif key == "include":
            node["include"] = serialize_include(value)
        else:
            node[key] = value
    return node


class ModelInputCodec:
    """Applies one model's extension serializers to its inputs and filters."""

    def __init__(
        self,
        extension_input_serializers: Mapping[str, Any],
        object_value_db_fields: frozenset,
    ) -> None:
        """Bind the codec to the extension fields of a single model.

        Args:
            extension_input_serializers: Per-field serializer for extension
                scalars, keyed by both the logical and the Python field name.
            object_value_db_fields: Columns holding object-like JSON or HSTORE
                values, for which equality needs an explicit ``equals``.
        """
        self.extension_input_serializers = extension_input_serializers
        self.object_value_db_fields = object_value_db_fields

    def scalar_input(self, field_name: str, value: Any) -> Any:
        """Serialize one input value written to ``field_name``."""
        serializer = self.extension_input_serializers.get(field_name)
        if value is None or serializer is None:
            return serialize_wire_value(value)
        return serialize_wire_value(serializer(value))

    def filter_input(self, field_name: str, operator: str, value: Any) -> Any:
        """Serialize the operand of ``operator`` applied to ``field_name``."""
        serializer = self.extension_input_serializers.get(field_name)
        if value is None or serializer is None:
            return serialize_wire_value(value)
        if operator in ("in", "notIn") and isinstance(value, list):
            return [
                serialize_wire_value(item if item is None else serializer(item))
                for item in value
            ]
        return serialize_wire_value(serializer(value))

    def nearest_input(self, nearest: Dict[str, Any]) -> Dict[str, Any]:
        """Serialize the query vector of a nearest-neighbour search."""
        result = dict(nearest)
        field_name = result.get("field")
        if isinstance(field_name, str) and "query" in result:
            result["query"] = self.scalar_input(field_name, result["query"])
        return result

    def create_data(
        self, data: Dict[str, Any], py_to_db: Mapping[str, str]
    ) -> Dict[str, Any]:
        """Convert CreateInput/UpdateInput dict to DB column names."""
        result = {}
        for key, value in data.items():
            db_key = py_to_db.get(key, key)
            result[db_key] = self.scalar_input(key, value)
        return result

    def where_filters(
        self, where: Dict[str, Any], py_to_db: Mapping[str, str]
    ) -> Dict[str, Any]:
        """Convert WhereInput format to internal filter format."""
        result = {}
        for key, value in where.items():
            if key in ("AND", "OR", "NOT"):
                if isinstance(value, list):
                    result[key] = [self.where_filters(item, py_to_db) for item in value]
                else:
                    result[key] = self.where_filters(value, py_to_db)
            else:
                db_key = py_to_db.get(key, key)
                if isinstance(value, dict):
                    if (
                        key in self.extension_input_serializers
                        and db_key not in self.object_value_db_fields
                        and not looks_like_filter_operator_dict(value)
                    ):
                        result[db_key] = self.scalar_input(key, value)
                        continue
                    if (
                        db_key in self.object_value_db_fields
                        and not looks_like_filter_operator_dict(value)
                    ):
                        object_equality_requires_explicit_equals(key)


                    ops = {}
                    for op, op_value in value.items():
                        actual_op = FILTER_OPERATOR_PROTOCOL_NAMES.get(op, op)
                        ops[actual_op] = self.filter_input(key, actual_op, op_value)
                    if ops:
                        result[db_key] = ops
                else:
                    result[db_key] = self.scalar_input(key, value)
        return result
