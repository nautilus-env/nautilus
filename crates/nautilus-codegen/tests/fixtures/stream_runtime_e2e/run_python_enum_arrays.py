import asyncio
import sqlite3
from pathlib import Path
from typing import Any

from pyclient import Nautilus
from pyclient.enums import OrderStatus


def same(actual: Any, expected: Any) -> bool:
    """Equal values of the same types, recursively: ``"PENDING" ==
    OrderStatus.PENDING`` alone would not tell a decoded value from a raw one."""
    if type(actual) is not type(expected):
        return False
    if isinstance(expected, dict):
        return actual.keys() == expected.keys() and all(
            same(actual[key], value) for key, value in expected.items()
        )
    if isinstance(expected, list):
        return len(actual) == len(expected) and all(
            same(item, value) for item, value in zip(actual, expected)
        )
    return actual == expected


def fields(row: Any) -> dict:
    return {"id": row.id, "status": row.status, "history": row.history}


async def main() -> None:
    db_path = Path(__file__).parent / "enum_arrays.sqlite"
    with sqlite3.connect(db_path) as connection:
        connection.execute(
            "CREATE TABLE orders (id INTEGER PRIMARY KEY, status TEXT NOT NULL, "
            "history TEXT NOT NULL)"
        )

    pending, shipped = OrderStatus.PENDING, OrderStatus.SHIPPED
    first = {"id": 1, "status": shipped, "history": [pending, shipped]}
    second = {"id": 2, "status": pending, "history": []}

    db = Nautilus()
    await db.connect()
    try:
        created = await db.order.create(first)
        assert same(fields(created), first), fields(created)
        created = await db.order.create(second)
        assert same(fields(created), second), fields(created)

        row = await db.order.find_unique(where={"id": 1})
        assert same(fields(row), first), fields(row)

        rows = await db.order.find_many(order_by={"id": "asc"})
        assert same([fields(row) for row in rows], [first, second]), rows

        projections = await db.order.find_many(
            select={"history": True}, order_by={"id": "asc"}
        )
        assert same(
            projections,
            [{"id": 1, "history": [pending, shipped]}, {"id": 2, "history": []}],
        ), projections

        delivered = OrderStatus.DELIVERED
        updated = await db.order.update(
            where={"id": 2}, data={"history": [pending, shipped, delivered]}
        )
        assert same(
            [fields(row) for row in updated],
            [{**second, "history": [pending, shipped, delivered]}],
        ), updated

        async with db.transaction() as tx:
            in_transaction = await tx.order.find_unique(
                where={"id": 2}, select={"status": True, "history": True}
            )
        assert same(
            in_transaction,
            {"id": 2, "status": pending, "history": [pending, shipped, delivered]},
        ), in_transaction
        print("enum_arrays=ok")
    finally:
        await db.disconnect()


asyncio.run(main())
