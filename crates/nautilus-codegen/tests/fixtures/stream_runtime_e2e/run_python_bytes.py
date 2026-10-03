import asyncio
import base64
import sqlite3
from pathlib import Path

from pyclient import Nautilus
from pyclient._internal.client import _json_default


async def main() -> None:
    payload = bytes(range(256))
    db_path = Path(__file__).parent / "bytes.sqlite"
    with sqlite3.connect(db_path) as connection:
        connection.execute("CREATE TABLE blobs (id INTEGER PRIMARY KEY, binary_data BLOB NOT NULL, optional_data BLOB)")
        connection.execute("INSERT INTO blobs VALUES (?, ?, ?)", (1, payload, None))

    db = Nautilus()
    await db.connect()
    try:
        original = await db.blob.find_unique(where={"id": 1})
        assert isinstance(original.binary_data, bytes)
        assert original.binary_data == payload and original.optional_data is None
        created = await db.blob.create({"id": 2, "binaryData": payload, "optionalData": b"\x00\xff"})
        assert created.binary_data == payload and created.optional_data == b"\x00\xff"
        projection = await db.blob.find_unique(where={"id": 2}, select={"binaryData": True})
        assert projection["binary_data"] == payload
        found = await db.blob.find_many(where={"binaryData": {"in": [payload]}})
        assert {row.id for row in found} == {1, 2}
        updated = await db.blob.update(where={"id": 2}, data={"binaryData": b"", "optionalData": None})
        assert updated[0].binary_data == b"" and updated[0].optional_data is None
        async with db.transaction() as tx:
            row = await tx.blob.upsert(where={"id": 3}, data={
                "create": {"id": 3, "binaryData": payload},
                "update": {},
            })
            assert row.binary_data == payload
        assert (await db.blob.find_unique(where={"id": 3})).binary_data == payload
        assert _json_default(bytearray(payload)) == base64.b64encode(payload).decode("ascii")
        assert _json_default(memoryview(payload)) == base64.b64encode(payload).decode("ascii")
        print("bytes_round_trip=ok")
    finally:
        await db.disconnect()


asyncio.run(main())
