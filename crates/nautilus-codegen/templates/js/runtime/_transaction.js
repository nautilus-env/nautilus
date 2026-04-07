// Runtime file — do not edit manually.

export var IsolationLevel;
(function (IsolationLevel) {
  IsolationLevel["ReadUncommitted"] = "readUncommitted";
  IsolationLevel["ReadCommitted"]   = "readCommitted";
  IsolationLevel["RepeatableRead"]  = "repeatableRead";
  IsolationLevel["Serializable"]    = "serializable";
})(IsolationLevel || (IsolationLevel = {}));

export class TransactionClient {
  constructor(parent, transactionId) {
    this._delegates = {};
    this.parent = parent;
    this.transactionId = transactionId;
    for (const [name, delegate] of Object.entries(parent._delegates)) {
      const proto  = Object.getPrototypeOf(delegate);
      const clone  = Object.create(proto);
      Object.assign(clone, delegate);
      clone['client'] = this;
      this[name] = clone;
      this._delegates[name] = clone;
    }
  }

  async _rpc(method, params) {
    return this.parent._rpc(method, { ...params, transactionId: this.transactionId });
  }
}
