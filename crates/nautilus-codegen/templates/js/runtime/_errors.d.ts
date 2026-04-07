// Runtime file — do not edit manually.

export interface NautilusErrorDetails {
  code?: number;
  data?: unknown;
}

export declare class NautilusError extends Error {
  readonly code?: number;
  readonly data?: unknown;
  constructor(message: string, details?: NautilusErrorDetails);
}
export declare class ProtocolError extends NautilusError {
  constructor(message: string, details?: NautilusErrorDetails);
}
export declare class HandshakeError extends NautilusError {
  constructor(message: string, details?: NautilusErrorDetails);
}
export declare class ValidationError extends NautilusError {
  constructor(message: string, details?: NautilusErrorDetails);
}
export declare class QueryError extends NautilusError {
  constructor(message: string, details?: NautilusErrorDetails);
}
export declare class DatabaseError extends NautilusError {
  constructor(message: string, details?: NautilusErrorDetails);
}
export declare class ConnectionError extends DatabaseError {
  constructor(message: string, details?: NautilusErrorDetails);
}
export declare class ConstraintViolationError extends DatabaseError {
  constructor(message: string, details?: NautilusErrorDetails);
}
export declare class UniqueConstraintError extends ConstraintViolationError {
  constructor(message: string, details?: NautilusErrorDetails);
}
export declare class ForeignKeyConstraintError extends ConstraintViolationError {
  constructor(message: string, details?: NautilusErrorDetails);
}
export declare class CheckConstraintError extends ConstraintViolationError {
  constructor(message: string, details?: NautilusErrorDetails);
}
export declare class NullConstraintError extends ConstraintViolationError {
  constructor(message: string, details?: NautilusErrorDetails);
}
export declare class DeadlockError extends DatabaseError {
  constructor(message: string, details?: NautilusErrorDetails);
}
export declare class SerializationError extends DatabaseError {
  constructor(message: string, details?: NautilusErrorDetails);
}
export declare class QueryTimeoutError extends DatabaseError {
  constructor(message: string, details?: NautilusErrorDetails);
}
export declare class NotFoundError extends DatabaseError {
  constructor(message: string, details?: NautilusErrorDetails);
}
export declare class InternalError extends NautilusError {
  constructor(message: string, details?: NautilusErrorDetails);
}
export declare class TransactionError extends NautilusError {
  constructor(message: string, details?: NautilusErrorDetails);
}
export declare class TransactionTimeoutError extends TransactionError {
  constructor(message: string, details?: NautilusErrorDetails);
}
export declare function errorFromCode(
  code: number,
  message: string,
  data?: unknown,
): NautilusError;
