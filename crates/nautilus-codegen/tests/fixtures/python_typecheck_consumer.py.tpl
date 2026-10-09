from typing import assert_type

from generated import Nautilus
from generated.events import EventPhase
from generated.models import Order, OrderCreateEventContext, OrderCreateInput, OrderProjection


@Order.onCreate
def before(context: OrderCreateEventContext) -> None:
    context.state["seen"] = True


@Order.onCreate(EventPhase.AFTER, priority=1)
def after(context: OrderCreateEventContext) -> None:
    assert_type(context.result, Order | None)


__ASYNC__def read(client: Nautilus) -> list[Order]:
    return __AWAIT__client.order.find_many(where={"id": {"gte": 1}})


__ASYNC__def exercise(client: Nautilus, data: OrderCreateInput) -> None:
    assert_type(__AWAIT__client.order.find_unique(where={"id": 1}), Order | None)
    assert_type(__AWAIT__client.order.find_many(select={"note": True}), list[OrderProjection])
    assert_type(__AWAIT__client.order.find_first(select={"id": True}), OrderProjection | None)
    assert_type(__AWAIT__client.order.find_unique_or_throw(where={"id": 1}), Order)
    assert_type(__AWAIT__client.order.create(data), Order | None)
    assert_type(__AWAIT__client.order.create_many([data]), list[Order])
    assert_type(__AWAIT__client.order.update(data={"note": "updated"}), list[Order] | int)
    assert_type(__AWAIT__client.order.delete_many(), list[Order] | int)
    assert_type(__AWAIT__client.order.update_many(data={"note": "updated"}), int)
    assert_type(__AWAIT__client.order.count(), int)
    __ASYNC__with client.__TRANSACTION__() as tx:
        assert_type(tx, Nautilus)
        assert_type(__AWAIT__tx.order.find_many(), list[Order])
    assert_type(__AWAIT__client.__TRANSACTION__(read), list[Order])


__ASYNC__def context_manager(data: OrderCreateInput) -> None:
    __ASYNC__with Nautilus() as client:
        assert_type(client, Nautilus)
        assert_type(__AWAIT__client.order.find_many(), list[Order])
        assert_type(__AWAIT__client.order.create(data), Order | None)
        __ASYNC__with client.__TRANSACTION__() as tx:
            assert_type(tx, Nautilus)
            assert_type(__AWAIT__tx.order.find_many(), list[Order])
