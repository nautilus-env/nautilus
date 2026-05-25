"""CRUD event runtime for generated Nautilus Python clients."""

from __future__ import annotations

import inspect
import threading
from dataclasses import dataclass, field
from enum import Enum
from types import MappingProxyType
from typing import Any, Callable, Dict, Generic, List, Mapping, MutableMapping, Optional, TypeVar


_MISSING = object()
ModelT = TypeVar("ModelT")
OperationT = TypeVar("OperationT", bound=str)
ArgsT = TypeVar("ArgsT", bound=Mapping[str, Any])
PayloadT = TypeVar("PayloadT", bound=Mapping[str, Any])
ResultT = TypeVar("ResultT")
ErrorT = TypeVar("ErrorT", bound=BaseException)
StateT = TypeVar("StateT", bound=MutableMapping[str, Any])


class EventPhase(str, Enum):
    """When a CRUD event handler runs."""

    BEFORE = "before"
    AFTER = "after"
    ERROR = "error"


class StopPropagation(Exception):
    """Stop the current CRUD event chain and return an optional result."""

    def __init__(self, result: Any = _MISSING) -> None:
        super().__init__("CRUD event propagation stopped")
        self.result = result
        self.has_result = result is not _MISSING


@dataclass(frozen=True)
class CrudEventContext(
    Generic[ModelT, OperationT, ArgsT, PayloadT, ResultT, ErrorT, StateT]
):
    """Context passed to generated CRUD event handlers."""

    db: Any
    model: ModelT
    model_name: str
    operation: OperationT
    phase: EventPhase
    args: ArgsT
    payload: PayloadT
    result: ResultT = None
    error: Optional[ErrorT] = None
    transaction_id: Optional[str] = None
    state: StateT = field(default_factory=dict)

    def __post_init__(self) -> None:
        object.__setattr__(self, "phase", normalize_event_phase(self.phase))
        object.__setattr__(self, "args", MappingProxyType(dict(self.args)))
        object.__setattr__(self, "payload", MappingProxyType(dict(self.payload)))
        if self.transaction_id is None:
            object.__setattr__(self, "transaction_id", _transaction_id_from_db(self.db))


CrudEventHandler = Callable[[CrudEventContext], Any]
EventHandler = CrudEventHandler


@dataclass(frozen=True)
class _RegisteredEventHandler:
    priority: int
    handler: EventHandler


_REGISTRY: Dict[str, Dict[str, Dict[EventPhase, List[_RegisteredEventHandler]]]] = {}
_REGISTRY_LOCK = threading.RLock()


def normalize_event_phase(value: Any) -> EventPhase:
    if isinstance(value, EventPhase):
        return value
    if isinstance(value, str):
        normalized = value.lower()
        for phase in EventPhase:
            if phase.value == normalized or phase.name.lower() == normalized:
                return phase
    raise ValueError(f"Unknown event phase: {value!r}")


def model_name_from_token(model: Any) -> str:
    name = getattr(model, "__nautilus_model_name__", None)
    if isinstance(name, str) and name:
        return name
    name = getattr(model, "model_name", None)
    if isinstance(name, str) and name:
        return name
    name = getattr(model, "__name__", None)
    if isinstance(name, str) and name:
        return name
    raise TypeError("Event model must be a generated Nautilus model")


def register_event_handler(
    model: Any,
    operation: str,
    phase: Any,
    handler: EventHandler,
    priority: int = 0,
) -> EventHandler:
    if not callable(handler):
        raise TypeError("CRUD event handler must be callable")
    model_name = model_name_from_token(model)
    normalized_phase = normalize_event_phase(phase)
    normalized_priority = normalize_event_priority(priority)
    with _REGISTRY_LOCK:
        handlers = _REGISTRY.setdefault(model_name, {}).setdefault(operation, {}).setdefault(
            normalized_phase,
            [],
        )
        handlers.append(_RegisteredEventHandler(normalized_priority, handler))
        handlers.sort(key=lambda registered: registered.priority, reverse=True)
    return handler


def model_event_decorator(
    model: Any,
    operation: str,
    phase_or_handler: Any = EventPhase.BEFORE,
    *,
    priority: int = 0,
) -> Any:
    if callable(phase_or_handler):
        return register_event_handler(
            model,
            operation,
            EventPhase.BEFORE,
            phase_or_handler,
            priority,
        )

    phase = normalize_event_phase(phase_or_handler)

    def decorator(handler: EventHandler) -> EventHandler:
        return register_event_handler(model, operation, phase, handler, priority)

    return decorator


def event_handlers(model_name: str, operation: str, phase: EventPhase) -> List[EventHandler]:
    with _REGISTRY_LOCK:
        return [
            registered.handler
            for registered in _REGISTRY.get(model_name, {}).get(operation, {}).get(phase, ())
        ]


def normalize_event_priority(value: Any) -> int:
    if isinstance(value, bool) or not isinstance(value, int):
        raise TypeError("CRUD event handler priority must be an int between 0 and 255")
    if value < 0 or value > 255:
        raise ValueError("CRUD event handler priority must be between 0 and 255")
    return value


async def run_crud_event(
    context: CrudEventContext,
    *,
    handle_stop_propagation: bool = True,
) -> Optional[StopPropagation]:
    for handler in event_handlers(context.model_name, context.operation, context.phase):
        try:
            result = handler(context)
            if inspect.isawaitable(result):
                result = await result
        except StopPropagation as stop:
            if handle_stop_propagation:
                return stop
            raise
    return None


def run_crud_event_sync(
    context: CrudEventContext,
    *,
    handle_stop_propagation: bool = True,
) -> Optional[StopPropagation]:
    for handler in event_handlers(context.model_name, context.operation, context.phase):
        try:
            result = handler(context)
            if inspect.isawaitable(result):
                close = getattr(result, "close", None)
                if callable(close):
                    close()
                raise RuntimeError(
                    "Async CRUD event handlers require an async Nautilus client"
                )
        except StopPropagation as stop:
            if handle_stop_propagation:
                return stop
            raise
    return None


def default_cud_result(operation: str, return_data: bool = True) -> Any:
    if operation in ("create", "delete"):
        return None
    if operation == "createMany":
        return []
    if operation in ("update", "deleteMany"):
        return [] if return_data else 0
    return None


def resolve_stop_result(stop: StopPropagation, default_result: Any) -> Any:
    return stop.result if stop.has_result else default_result


def _transaction_id_from_db(db: Any) -> Optional[str]:
    tx_id = getattr(db, "_transaction_id", None)
    return tx_id if isinstance(tx_id, str) else None
