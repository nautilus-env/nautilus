// Runtime file — do not edit manually.

import type { NautilusClient } from './_client.js';

export declare enum IsolationLevel {
  ReadUncommitted = 'readUncommitted',
  ReadCommitted   = 'readCommitted',
  RepeatableRead  = 'repeatableRead',
  Serializable    = 'serializable',
}

export declare class TransactionClient {
  _delegates: Record<string, unknown>;
  constructor(parent: NautilusClient, transactionId: string);
  _rpc(method: string, params: Record<string, unknown>): Promise<unknown>;
}
