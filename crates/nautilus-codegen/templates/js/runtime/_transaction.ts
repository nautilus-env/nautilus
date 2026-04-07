// Runtime file — do not edit manually.

import type { NautilusClient } from './_client';

export enum IsolationLevel {
  ReadUncommitted = 'readUncommitted',
  ReadCommitted   = 'readCommitted',
  RepeatableRead  = 'repeatableRead',
  Serializable    = 'serializable',
}

/**
 * A thin client wrapper that routes all RPC calls through a server-side
 * transaction. Delegates are cloned from the parent client so the same
 * generated API is available on the transaction object.
 *
 * Usage (via `Nautilus.$transaction`):
 *
 *   const result = await db.$transaction(async (tx) => {
 *     const user = await tx.user.create({ data: { email: 'a@b.com' } });
 *     await tx.post.create({ data: { title: 'Hello', authorId: user!.id } });
 *     return user;
 *   });
 */
export class TransactionClient {
  /** Cloned delegates, each re-bound to this TransactionClient as their RPC target. */
  _delegates: Record<string, unknown> = {};

  constructor(
    private readonly parent: NautilusClient,
    private readonly transactionId: string,
  ) {
    // Clone every delegate from the parent and replace its `client` property so
    // that all RPC calls made through the delegate are automatically tagged with
    // the transaction ID by `TransactionClient._rpc`.
    for (const [name, delegate] of Object.entries(parent._delegates)) {
      const proto  = Object.getPrototypeOf(delegate);
      const clone  = Object.create(proto) as Record<string, unknown>;
      Object.assign(clone, delegate);
      clone['client'] = this;
      (this as Record<string, unknown>)[name] = clone;
      this._delegates[name] = clone;
    }
  }

  /**
   * Forward an RPC call to the parent client, injecting the transaction ID
   * into every request so the engine executes the query inside the transaction.
   */
  async _rpc(method: string, params: Record<string, unknown>): Promise<unknown> {
    return this.parent._rpc(method, { ...params, transactionId: this.transactionId });
  }
}
