import asyncio
import sqlite3
from datetime import datetime, timezone
from pathlib import Path

from pyclient import Nautilus
from pyclient.enums import Status


async def main() -> None:
    db_path = Path(__file__).parent / "group_by.sqlite"
    with sqlite3.connect(db_path) as connection:
        connection.execute(
            "CREATE TABLE orders (id INTEGER PRIMARY KEY, customer_id INTEGER NOT NULL, "
            'order_status TEXT NOT NULL, "createdAt" TEXT NOT NULL)'
        )

    db = Nautilus()
    await db.connect()
    try:
        early = datetime(2026, 1, 1, tzinfo=timezone.utc)
        late = datetime(2026, 1, 2, tzinfo=timezone.utc)
        latest = datetime(2026, 1, 3, tzinfo=timezone.utc)
        for order_id, customer_id, status, created_at in [
            (1, 0, Status.SHIPPED, early),
            (2, 0, Status.PENDING, early),
            (3, 0, Status.PENDING, late),
            (4, 1, Status.PENDING, late),
            (5, 2, Status.PENDING, latest),
        ]:
            await db.order.create(
                {
                    "id": order_id,
                    "customerId": customer_id,
                    "orderStatus": status,
                    "createdAt": created_at,
                }
            )
        first = (await db.order.find_unique(where={"id": 1})).created_at
        last = (await db.order.find_unique(where={"id": 4})).created_at
        newest = (await db.order.find_unique(where={"id": 5})).created_at

        groups = await db.order.group_by(
            ["customer_id", "order_status"],
            count=True,
            order={"customer_id": "asc", "order_status": "asc"},
        )
        assert groups == [
            {"customer_id": 0, "order_status": Status.PENDING, "_count": {"_all": 2}},
            {"customer_id": 0, "order_status": Status.SHIPPED, "_count": {"_all": 1}},
            {"customer_id": 1, "order_status": Status.PENDING, "_count": {"_all": 1}},
            {"customer_id": 2, "order_status": Status.PENDING, "_count": {"_all": 1}},
        ], groups
        assert all(type(group["order_status"]) is Status for group in groups), groups

        # `createdAt` has no @map: its column is not the Python name. A field inside an
        # aggregate comes back under the spelling the request used.
        async with db.transaction() as tx:
            by_time = await tx.order.group_by(
                ["created_at"],
                count={"_all": True, "created_at": True},
                min={"customerId": True},
                max={"orderStatus": True},
                having={"_count": {"created_at": {"gte": 2}}},
                order=[{"_max": {"customer_id": "desc"}}, {"created_at": "asc"}],
            )
        assert by_time == [
            {
                "created_at": last,
                "_count": {"_all": 2, "created_at": 2},
                "_min": {"customerId": 0},
                "_max": {"orderStatus": Status.PENDING},
            },
            {
                "created_at": first,
                "_count": {"_all": 2, "created_at": 2},
                "_min": {"customerId": 0},
                "_max": {"orderStatus": Status.SHIPPED},
            },
        ], by_time

        # `True` asks for every field: they come back under their Python names.
        totals = await db.order.aggregate(
            where={"orderStatus": Status.PENDING},
            count={"created_at": True},
            sum={"customerId": True},
            max=True,
        )
        assert totals == {
            "_count": {"created_at": 4},
            "_sum": {"customerId": 3},
            "_max": {
                "id": 5,
                "customer_id": 2,
                "order_status": Status.PENDING,
                "created_at": newest,
            },
        }, totals
        print("group_by_names=ok")
    finally:
        await db.disconnect()


asyncio.run(main())
