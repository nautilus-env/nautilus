import asyncio
import sqlite3
from pathlib import Path

from pyclient import Nautilus
from pyclient._internal.client import _read_message

ROWS = 24
BODY = "x" * 1_000_000


async def check_reader_limit() -> None:
    # A newline already buffered past the limit.
    reader = asyncio.StreamReader(limit=8)
    reader.feed_data(b'{"id": 1, "result": "' + b"y" * 40 + b'"}\n{"id": 2}\n')
    assert await _read_message(reader) == b'{"id": 1, "result": "' + b"y" * 40 + b'"}\n'
    assert await _read_message(reader) == b'{"id": 2}\n'

    # A line that outgrows the limit before its newline arrives.
    reader = asyncio.StreamReader(limit=8)
    reader.feed_data(b"a" * 20)
    pending = asyncio.ensure_future(_read_message(reader))
    await asyncio.sleep(0)
    reader.feed_data(b"b" * 20 + b"\ntail")
    reader.feed_eof()
    assert await pending == b"a" * 20 + b"b" * 20 + b"\n"
    assert await _read_message(reader) == b"tail"
    assert await _read_message(reader) == b""


async def main() -> None:
    await check_reader_limit()

    db_path = Path(__file__).parent / "large_response.sqlite"
    with sqlite3.connect(db_path) as connection:
        connection.execute("CREATE TABLE notes (id INTEGER PRIMARY KEY, body TEXT NOT NULL)")
        connection.executemany(
            "INSERT INTO notes VALUES (?, ?)", [(i, BODY) for i in range(1, ROWS + 1)]
        )

    db = Nautilus()
    await db.connect()
    try:
        # One response line of about 24 MB, past the reader's 16 MB buffer.
        notes = await db.note.find_many()
        assert sorted(note.id for note in notes) == list(range(1, ROWS + 1))
        assert all(note.body == BODY for note in notes)
        # The client still answers after it.
        assert await db.note.count() == ROWS
        assert (await db.note.find_unique(where={"id": 1})).body == BODY
        print("large_response=ok")
    finally:
        await db.disconnect()


asyncio.run(main())
