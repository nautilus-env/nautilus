// Runtime file — do not edit manually.

export interface NautilusErrorDetails {
  code?: number;
  data?: unknown;
}

export class NautilusError extends Error {
  readonly code?: number;
  readonly data?: unknown;

  constructor(message: string, details?: NautilusErrorDetails) {
    super(message);
    this.name = 'NautilusError';
    this.code = details?.code;
    this.data = details?.data;
    // Restore prototype chain for instanceof checks in transpiled code.
    Object.setPrototypeOf(this, new.target.prototype);
  }
}

export class ProtocolError extends NautilusError {
  constructor(message: string, details?: NautilusErrorDetails) {
    super(message, details);
    this.name = 'ProtocolError';
    Object.setPrototypeOf(this, new.target.prototype);
  }
}

export class HandshakeError extends NautilusError {
  constructor(message: string, details?: NautilusErrorDetails) {
    super(message, details);
    this.name = 'HandshakeError';
    Object.setPrototypeOf(this, new.target.prototype);
  }
}

export class ValidationError extends NautilusError {
  constructor(message: string, details?: NautilusErrorDetails) {
    super(message, details);
    this.name = 'ValidationError';
    Object.setPrototypeOf(this, new.target.prototype);
  }
}

export class QueryError extends NautilusError {
  constructor(message: string, details?: NautilusErrorDetails) {
    super(message, details);
    this.name = 'QueryError';
    Object.setPrototypeOf(this, new.target.prototype);
  }
}

export class DatabaseError extends NautilusError {
  constructor(message: string, details?: NautilusErrorDetails) {
    super(message, details);
    this.name = 'DatabaseError';
    Object.setPrototypeOf(this, new.target.prototype);
  }
}

export class ConnectionError extends DatabaseError {
  constructor(message: string, details?: NautilusErrorDetails) {
    super(message, details);
    this.name = 'ConnectionError';
    Object.setPrototypeOf(this, new.target.prototype);
  }
}

export class ConstraintViolationError extends DatabaseError {
  constructor(message: string, details?: NautilusErrorDetails) {
    super(message, details);
    this.name = 'ConstraintViolationError';
    Object.setPrototypeOf(this, new.target.prototype);
  }
}

export class UniqueConstraintError extends ConstraintViolationError {
  constructor(message: string, details?: NautilusErrorDetails) {
    super(message, details);
    this.name = 'UniqueConstraintError';
    Object.setPrototypeOf(this, new.target.prototype);
  }
}

export class ForeignKeyConstraintError extends ConstraintViolationError {
  constructor(message: string, details?: NautilusErrorDetails) {
    super(message, details);
    this.name = 'ForeignKeyConstraintError';
    Object.setPrototypeOf(this, new.target.prototype);
  }
}

export class CheckConstraintError extends ConstraintViolationError {
  constructor(message: string, details?: NautilusErrorDetails) {
    super(message, details);
    this.name = 'CheckConstraintError';
    Object.setPrototypeOf(this, new.target.prototype);
  }
}

export class NullConstraintError extends ConstraintViolationError {
  constructor(message: string, details?: NautilusErrorDetails) {
    super(message, details);
    this.name = 'NullConstraintError';
    Object.setPrototypeOf(this, new.target.prototype);
  }
}

export class DeadlockError extends DatabaseError {
  constructor(message: string, details?: NautilusErrorDetails) {
    super(message, details);
    this.name = 'DeadlockError';
    Object.setPrototypeOf(this, new.target.prototype);
  }
}

export class SerializationError extends DatabaseError {
  constructor(message: string, details?: NautilusErrorDetails) {
    super(message, details);
    this.name = 'SerializationError';
    Object.setPrototypeOf(this, new.target.prototype);
  }
}

export class QueryTimeoutError extends DatabaseError {
  constructor(message: string, details?: NautilusErrorDetails) {
    super(message, details);
    this.name = 'QueryTimeoutError';
    Object.setPrototypeOf(this, new.target.prototype);
  }
}

export class NotFoundError extends DatabaseError {
  constructor(message: string, details?: NautilusErrorDetails) {
    super(message, details);
    this.name = 'NotFoundError';
    Object.setPrototypeOf(this, new.target.prototype);
  }
}

export class InternalError extends NautilusError {
  constructor(message: string, details?: NautilusErrorDetails) {
    super(message, details);
    this.name = 'InternalError';
    Object.setPrototypeOf(this, new.target.prototype);
  }
}

export class TransactionError extends NautilusError {
  constructor(message: string, details?: NautilusErrorDetails) {
    super(message, details);
    this.name = 'TransactionError';
    Object.setPrototypeOf(this, new.target.prototype);
  }
}

export class TransactionTimeoutError extends TransactionError {
  constructor(message: string, details?: NautilusErrorDetails) {
    super(message, details);
    this.name = 'TransactionTimeoutError';
    Object.setPrototypeOf(this, new.target.prototype);
  }
}

/**
 * Map a numeric error code from the engine to the correct error subclass.
 *
 * Code ranges (mirrors the Python implementation):
 *  1000–1999  Validation errors
 *  2000–2999  Query errors
 *  3000–3999  Database errors
 *    3001  ConnectionError
 *    3002  ConstraintViolationError
 *    3003  QueryTimeoutError
 *    3004  NotFoundError
 *    3005  UniqueConstraintError
 *    3006  ForeignKeyConstraintError
 *    3007  CheckConstraintError
 *  4001–4004  Transaction errors (4002 = timeout)
 *  9000–9999  Internal errors
 */
export function errorFromCode(code: number, message: string, data?: unknown): NautilusError {
  const details = { code, data };
  if (code >= 1000 && code < 2000) return new ValidationError(`[${code}] ${message}`, details);
  if (code >= 2000 && code < 3000) return new QueryError(`[${code}] ${message}`, details);
  if (code === 3001)               return new ConnectionError(`[${code}] ${message}`, details);
  if (code === 3002)               return new ConstraintViolationError(`[${code}] ${message}`, details);
  if (code === 3003)               return new QueryTimeoutError(`[${code}] ${message}`, details);
  if (code === 3004)               return new NotFoundError(`[${code}] ${message}`, details);
  if (code === 3005)               return new UniqueConstraintError(`[${code}] ${message}`, details);
  if (code === 3006)               return new ForeignKeyConstraintError(`[${code}] ${message}`, details);
  if (code === 3007)               return new CheckConstraintError(`[${code}] ${message}`, details);
  if (code === 3008)               return new NullConstraintError(`[${code}] ${message}`, details);
  if (code === 3009)               return new DeadlockError(`[${code}] ${message}`, details);
  if (code === 3010)               return new SerializationError(`[${code}] ${message}`, details);
  if (code >= 3000 && code < 4000) return new DatabaseError(`[${code}] ${message}`, details);
  if (code === 4002)               return new TransactionTimeoutError(`[${code}] ${message}`, details);
  if (code >= 4001 && code <= 4004) return new TransactionError(`[${code}] ${message}`, details);
  if (code >= 9000 && code < 10000) return new InternalError(`[${code}] ${message}`, details);
  return new ProtocolError(`[${code}] ${message}`, details);
}
