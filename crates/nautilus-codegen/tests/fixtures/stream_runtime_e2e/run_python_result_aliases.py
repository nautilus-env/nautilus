import asyncio

from pyclient import Nautilus


async def main() -> None:
    async with Nautilus() as db:
        await db.order.raw_query("CREATE EXTENSION IF NOT EXISTS citext")
        await db.order.raw_query(
            'CREATE TABLE "__TABLE__" (order_identifier INTEGER PRIMARY KEY, '
            "contact_names CITEXT[] NOT NULL, contact_namex CITEXT[] NOT NULL, contact_notes TEXT)"
        )
        try:
            full = {
                "contactNames": ["Alice", "", "O'Reilly"],
                "contactNamex": ["Bob"],
                "contactNotes": "caffè",
            }
            empty = {"contactNames": [], "contactNamex": [], "contactNotes": None}

            def check(row, row_id, data):
                assert row is not None and row.id == row_id
                assert [value.value for value in row.contact_names] == data[
                    "contactNames"
                ]
                assert [value.value for value in row.contact_namex] == data[
                    "contactNamex"
                ]
                assert row.contact_notes == data["contactNotes"]

            check(await db.order.create({"id": 1, **full}), 1, full)
            for _ in range(2):
                check(await db.order.find_unique(where={"id": 1}), 1, full)
                rows = await db.order.find_many(where={"id": 1})
                assert len(rows) == 1
                check(rows[0], 1, full)
            projection = await db.order.find_many(
                select={
                    "contactNames": True,
                    "contactNamex": True,
                    "contactNotes": True,
                }
            )
            assert [value.value for value in projection[0]["contact_names"]] == full[
                "contactNames"
            ]
            assert [value.value for value in projection[0]["contact_namex"]] == full[
                "contactNamex"
            ]
            assert projection[0]["contact_notes"] == "caffè"
            batch = await db.order.create_many([{"id": 2, **full}, {"id": 3, **empty}])
            by_id = {row.id: row for row in batch}
            check(by_id[2], 2, full)
            check(by_id[3], 3, empty)
            for row_id in [2, 4]:
                check(
                    await db.order.upsert(
                        where={"id": row_id},
                        data={"create": {"id": row_id, **full}, "update": full},
                    ),
                    row_id,
                    full,
                )
            async with db.transaction() as tx:
                rows = await tx.order.update(where={"id": 1}, data=empty)
                assert len(rows) == 1
                check(rows[0], 1, empty)
                check(await tx.order.find_unique(where={"id": 1}), 1, empty)
            await db.order.update_many(where={"id": 3}, data=full)
            check(await db.order.find_unique(where={"id": 3}), 3, full)
            streamed = [
                row
                async for row in db.order.stream_many(
                    order_by={"id": "asc"}, chunk_size=1
                )
            ]
            assert len(streamed) == 4
            for row in streamed:
                check(row, row.id, empty if row.id == 1 else full)
            grouped = await db.order.group_by(
                ["contact_notes"], count={"contact_notes": True}
            )
            assert any(
                row["contact_notes"] == "caffè" and row["_count"]["contact_notes"] == 3
                for row in grouped
            )
            deleted = await db.order.delete(where={"id": 4})
            check(deleted, 4, full)
            # Raw queries retain the aliases the caller supplied, including an
            # alias that resembles a generated projection name.
            raw = await db.order.raw_query('SELECT 7 AS "__nautilus_col_0"')
            assert raw == [{"__nautilus_col_0": 7}]
        finally:
            await db.order.raw_query('DROP TABLE "__TABLE__"')
    print("result_aliases=ok")


asyncio.run(main())
