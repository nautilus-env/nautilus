import type { NautilusClient } from './_client.js';

export enum IsolationLevel {
  ReadUncommitted = 'readUncommitted',
  ReadCommitted   = 'readCommitted',
  RepeatableRead  = 'repeatableRead',
  Serializable    = 'serializable',
}

export class TransactionClient {
  declare _delegates: Record<string, unknown>;
  /** @internal */
  declare private parent: NautilusClient;
  /** @internal */
  declare private transactionId: string;

  constructor(parent: NautilusClient, transactionId: string) {
    this._delegates = {};
    this.parent = parent;
    this.transactionId = transactionId;
    for (const [name, delegate] of Object.entries(parent._delegates)) {
      const proto  = Object.getPrototypeOf(delegate);
      const clone  = Object.create(proto);
      Object.assign(clone, delegate);
      clone['client'] = this;
      (this as unknown as Record<string, unknown>)[name] = clone;
      this._delegates[name] = clone;
    }
  }

  async _rpc(method: string, params: Record<string, unknown>): Promise<unknown> {
    return this.parent._rpc(method, { ...params, transactionId: this.transactionId });
  }
}
