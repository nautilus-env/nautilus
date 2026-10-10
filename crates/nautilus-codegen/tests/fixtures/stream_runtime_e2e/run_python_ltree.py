import asyncio

from pyclient import Nautilus


async def main() -> None:
    async with Nautilus() as db:
        await db.order.raw_query("CREATE EXTENSION IF NOT EXISTS ltree")
        await db.order.raw_query("CREATE EXTENSION IF NOT EXISTS citext")
        await db.order.raw_query(
            'CREATE TABLE "__TABLE__" (id INTEGER PRIMARY KEY, route_path LTREE, '
            "route_paths LTREE[] NOT NULL, contact_names CITEXT[] NOT NULL DEFAULT '{}')"
        )
        try:
            paths = ["top.caffè", "", "x" * 300]
            names = ["Alice", "", "O'Reilly", "alice"]
            full = {"routePaths": paths, "contactNames": names}
            empty = {"routePaths": [], "contactNames": []}
            row = await db.order.create({"id": 1, "routePath": "top.c", **full})
            assert row is not None and row.route_path.value == "top.c"
            assert [path.value for path in row.route_paths] == paths
            assert [name.value for name in row.contact_names] == names
            row = await db.order.find_unique(where={"id": 1})
            assert row is not None and row.route_path.value == "top.c"
            assert [path.value for path in row.route_paths] == paths
            rows = await db.order.find_many(where={"routePath": "top.c"})
            assert len(rows) == 1 and rows[0].route_path.value == "top.c"
            projected = await db.order.find_many(
                select={"routePath": True, "routePaths": True, "contactNames": True}
            )
            assert projected[0]["route_path"].value == "top.c"
            assert [path.value for path in projected[0]["route_paths"]] == paths
            assert [name.value for name in projected[0]["contact_names"]] == names
            async with db.transaction() as tx:
                rows = await tx.order.update(
                    where={"id": 1},
                    data={
                        "routePath": "top.updated",
                        "routePaths": ["top.tx"],
                        "contactNames": ["Bob"],
                    },
                )
                assert len(rows) == 1 and rows[0].route_path.value == "top.updated"
                assert [path.value for path in rows[0].route_paths] == ["top.tx"]
                assert [name.value for name in rows[0].contact_names] == ["Bob"]
            await db.order.raw_query(
                'INSERT INTO "__TABLE__" (id, route_path, route_paths) '
                "VALUES (2, 'top.sql', ARRAY['top.sql']::ltree[])"
            )
            row = await db.order.find_unique(where={"id": 2})
            assert row is not None and row.route_path.value == "top.sql"
            assert [path.value for path in row.route_paths] == ["top.sql"]
            batch = await db.order.create_many([{"id": 3, **full}, {"id": 4, **empty}])
            by_id = {row.id: row for row in batch}
            assert [path.value for path in by_id[3].route_paths] == paths
            assert [name.value for name in by_id[3].contact_names] == names
            assert by_id[4].route_paths == [] and by_id[4].contact_names == []
            for lists in [full, empty]:
                await db.order.update_many(where={"id": {"in_": [3, 4]}}, data=lists)
                for row_id in [3, 4]:
                    row = await db.order.find_unique(where={"id": row_id})
                    assert row is not None
                    assert [path.value for path in row.route_paths] == lists[
                        "routePaths"
                    ]
                    assert [name.value for name in row.contact_names] == lists[
                        "contactNames"
                    ]
            for row_id, create, update, expected in [
                (3, empty, full, full),
                (4, full, empty, empty),
                (5, full, empty, full),
                (6, empty, full, empty),
            ]:
                row = await db.order.upsert(
                    where={"id": row_id},
                    data={"create": {"id": row_id, **create}, "update": update},
                )
                assert row is not None
                assert [path.value for path in row.route_paths] == expected[
                    "routePaths"
                ]
                assert [name.value for name in row.contact_names] == expected[
                    "contactNames"
                ]
            rows = await db.order.update(
                where={"id": 1}, data={"routePath": "", **empty}
            )
            assert (
                len(rows) == 1
                and rows[0].route_path.value == ""
                and rows[0].route_paths == []
            )
            rows = await db.order.update(where={"id": 1}, data={"routePath": None})
            assert len(rows) == 1 and rows[0].route_path is None
            row = await db.order.find_unique(where={"id": 1})
            assert row is not None and row.route_path is None and row.route_paths == []
        finally:
            await db.order.raw_query('DROP TABLE "__TABLE__"')
    print("ltree=ok")


asyncio.run(main())
