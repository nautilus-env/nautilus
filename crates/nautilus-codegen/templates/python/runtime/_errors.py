"""Exception classes for Nautilus client."""

from __future__ import annotations

from typing import Any


class NautilusError(Exception):
    """Base exception for all Nautilus errors."""

    def __init__(self, message: str, *, code: int | None = None, data: Any = None) -> None:
        super().__init__(message)
        self.code = code
        self.data = data


class ProtocolError(NautilusError):
    """Protocol-level error (JSON-RPC framing, parsing, etc.)."""
    pass


class HandshakeError(NautilusError):
    """Error during engine handshake (version mismatch, etc.)."""
    pass


class ValidationError(NautilusError):
    """Schema validation error (1000-1999)."""
    pass


class QueryError(NautilusError):
    """Query planning or execution error (2000-2999)."""
    pass


class DatabaseError(NautilusError):
    """Database execution error (3000-3999)."""
    pass


class ConnectionError(DatabaseError):
    """Database connection error (code 3001).

    Raised when the connection pool is exhausted or the database is unreachable.
    """
    pass


class ConstraintViolationError(DatabaseError):
    """Generic constraint violation (code 3002).

    Raised when a database constraint is violated and the specific type
    cannot be determined.
    """
    pass


class UniqueConstraintError(ConstraintViolationError):
    """Unique constraint violation (code 3005).

    Raised when an INSERT or UPDATE would create a duplicate value in a
    column (or set of columns) that requires uniqueness.
    """
    pass


class ForeignKeyConstraintError(ConstraintViolationError):
    """Foreign key constraint violation (code 3006).

    Raised when an INSERT or UPDATE references a value that does not exist
    in the related table, or a DELETE would leave orphaned child rows.
    """
    pass


class CheckConstraintError(ConstraintViolationError):
    """Check constraint violation (code 3007).

    Raised when a value does not satisfy a CHECK constraint defined on a
    column or table.
    """
    pass


class NullConstraintError(ConstraintViolationError):
    """NOT NULL constraint violation (code 3008).

    Raised when a NULL value is provided for a column that requires a
    non-null value.
    """
    pass


class DeadlockError(DatabaseError):
    """Deadlock detected (code 3009).

    Raised when the database detects a deadlock between two or more
    concurrent transactions. The safest response is to retry the operation.
    """
    pass


class SerializationError(DatabaseError):
    """Transaction serialization failure (code 3010).

    Raised when a transaction cannot be serialized due to concurrent
    modifications. Retry the whole transaction from the beginning.
    """
    pass


class QueryTimeoutError(DatabaseError):
    """Query execution timeout (code 3003).

    Raised when a database query exceeds the configured execution timeout.
    """
    pass


class NotFoundError(DatabaseError):
    """Record not found error (code 3004).

    Raised by ``find_unique_or_throw`` and ``find_first_or_throw``
    when no matching record exists.
    """
    pass


class InternalError(NautilusError):
    """Internal engine error (9000-9999)."""
    pass


class TransactionError(NautilusError):
    """Transaction error (4001-4004).

    Raised when a transaction cannot be started, committed, rolled back,
    or is otherwise invalid.
    """
    pass


class TransactionTimeoutError(TransactionError):
    """Transaction timed out (code 4002).

    Raised when the engine's per-transaction timeout (default 5 s) expires.
    """
    pass


def error_from_code(code: int, message: str, data: Any = None) -> NautilusError:
    """Create appropriate exception from JSON-RPC error code."""
    if 1000 <= code < 2000:
        return ValidationError(f"[{code}] {message}", code=code, data=data)
    elif 2000 <= code < 3000:
        return QueryError(f"[{code}] {message}", code=code, data=data)
    elif code == 3001:
        return ConnectionError(f"[{code}] {message}", code=code, data=data)
    elif code == 3002:
        return ConstraintViolationError(f"[{code}] {message}", code=code, data=data)
    elif code == 3003:
        return QueryTimeoutError(f"[{code}] {message}", code=code, data=data)
    elif code == 3004:
        return NotFoundError(f"[{code}] {message}", code=code, data=data)
    elif code == 3005:
        return UniqueConstraintError(f"[{code}] {message}", code=code, data=data)
    elif code == 3006:
        return ForeignKeyConstraintError(f"[{code}] {message}", code=code, data=data)
    elif code == 3007:
        return CheckConstraintError(f"[{code}] {message}", code=code, data=data)
    elif code == 3008:
        return NullConstraintError(f"[{code}] {message}", code=code, data=data)
    elif code == 3009:
        return DeadlockError(f"[{code}] {message}", code=code, data=data)
    elif code == 3010:
        return SerializationError(f"[{code}] {message}", code=code, data=data)
    elif 3000 <= code < 4000:
        return DatabaseError(f"[{code}] {message}", code=code, data=data)
    elif code == 4002:
        return TransactionTimeoutError(f"[{code}] {message}", code=code, data=data)
    elif 4001 <= code <= 4004:
        return TransactionError(f"[{code}] {message}", code=code, data=data)
    elif 9000 <= code < 10000:
        return InternalError(f"[{code}] {message}", code=code, data=data)
    else:
        return ProtocolError(f"[{code}] {message}", code=code, data=data)
