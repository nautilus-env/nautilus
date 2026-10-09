import asyncio
import json
import sqlite3
from pathlib import Path

from pyclient import Nautilus
from pyclient.errors import InternalError, QueryError


async def main() -> None:
    db_path = Path(__file__).with_name("list_inputs.sqlite")
    with sqlite3.connect(db_path) as connection:
        connection.execute(
            "CREATE TABLE orders (id INTEGER PRIMARY KEY, weights TEXT NOT NULL, "
            "notes TEXT NOT NULL, metadata TEXT NOT NULL)"
        )
    db = Nautilus()
    await db.connect()
    try:
        data = {"id": 1, "weights": [1, 2.5], "notes": ["fragile"], "metadata": {"increment": 1}}
        row = await db.order.create(data)
        assert row.weights == [1.0, 2.5]
        assert row.metadata == {"increment": 1}
        for invalid in ({"increment": 1}, {"set": 1.5}, {"set": [4.5]}, {}, 1.5):
            try:
                await db.order.update(where={"id": 1}, data={"weights": invalid})
            except InternalError as error:
                assert error.code == 9003
                assert "takes an array value" in str(error)
            else:
                raise AssertionError(f"accepted invalid list: {invalid}")
            row = await db.order.find_unique(where={"id": 1})
            assert row.weights == [1.0, 2.5]
        for invalid in (2.5, {"equals": 2.5}, {"in_": [2.5]}, {"gt": 2.5}):
            try:
                await db.order.find_many(where={"weights": invalid})
            except QueryError as error:
                assert error.code == 2001
            else:
                raise AssertionError(f"accepted invalid filter: {invalid}")
        await db.order.update_many(data={"weights": [], "notes": []})
        row = await db.order.find_unique(where={"id": 1})
        assert row.weights == [] and row.notes == []
        async with db.transaction() as tx:
            await tx.order.update(where={"id": 1}, data={"weights": [4.5]})
        row = await db.order.find_unique(where={"id": 1})
        assert row.weights == [4.5]
        projection = await db.order.find_many(select={"weights": True})
        assert projection == [{"id": 1, "weights": [4.5]}]
    finally:
        await db.disconnect()
    with sqlite3.connect(db_path) as connection:
        stored = connection.execute("SELECT weights FROM orders").fetchone()[0]
        assert json.loads(stored) == [4.5]
    print("list_inputs=ok")


asyncio.run(main())
