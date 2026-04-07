"""Transaction support for the Nautilus client.

Provides both **interactive** transactions (context-manager / closure style)
and **batch** transactions (send a list of operations atomically).
"""

from __future__ import annotations

import copy
from enum import Enum
from typing import Any, Callable, Dict, List, Optional, TypeVar

T = TypeVar("T")


class IsolationLevel(str, Enum):
    """SQL transaction isolation levels."""

    READ_UNCOMMITTED = "readUncommitted"
    READ_COMMITTED = "readCommitted"
    REPEATABLE_READ = "repeatableRead"
    SERIALIZABLE = "serializable"


class TransactionClient:
    """Thin wrapper around a :class:`NautilusClient` that injects a
    ``transactionId`` into every JSON-RPC request sent through it.

    Delegates (e.g. ``tx.user``, ``tx.post``) are clones of the parent
    client's delegates re-bound to this wrapper so that every query they
    execute participates in the same database transaction.
    """

    def __init__(self, parent: Any, transaction_id: str) -> None:
        self._parent = parent
        self._transaction_id = transaction_id
        # Clone delegates: each delegate gets a reference to *this* TX client
        # so its _rpc / _sync_rpc calls go through us.
        for name, delegate in parent._delegates.items():
            clone = copy.copy(delegate)
            clone._client = self  # re-bind to TX client
            setattr(self, name, clone)

    async def _rpc(self, method: str, params: Dict[str, Any]) -> Any:
        """Async RPC call with transaction ID injected."""
        params = {**params, "transactionId": self._transaction_id}
        return await self._parent._rpc(method, params)

    def _sync_rpc(self, method: str, params: Dict[str, Any]) -> Any:
        """Sync RPC call with transaction ID injected."""
        params = {**params, "transactionId": self._transaction_id}
        return self._parent._sync_rpc(method, params)

    def get_delegate(self, name: str) -> Any:
        """Return a delegate bound to this transaction."""
        return getattr(self, name)

    def register_delegate(self, name: str, delegate: Any) -> None:
        """No-op — delegates are cloned at TX creation time."""
        pass
