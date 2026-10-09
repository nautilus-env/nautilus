import asyncio

from pyclient import Nautilus


async def main() -> None:
    async with Nautilus() as db:
        await db.order.raw_query('CREATE EXTENSION IF NOT EXISTS ltree')
        await db.order.raw_query(
            'CREATE TABLE "__TABLE__" (id INTEGER PRIMARY KEY, route_path LTREE, route_paths LTREE[] NOT NULL)'
        )
        try:
            row = await db.order.create({'id': 1, 'routePath': 'top.c', 'routePaths': []})
            assert row is not None and row.route_path.value == 'top.c' and row.route_paths == []
            await db.order.raw_query(
                'UPDATE "__TABLE__" SET route_paths = ARRAY[\'top.caffè\', \'\', repeat(\'x\', 300)]::ltree[] WHERE id = 1'
            )
            paths = ['top.caffè', '', 'x' * 300]
            row = await db.order.find_unique(where={'id': 1})
            assert row is not None and row.route_path.value == 'top.c'
            assert [path.value for path in row.route_paths] == paths
            rows = await db.order.find_many(where={'routePath': 'top.c'})
            assert len(rows) == 1 and rows[0].route_path.value == 'top.c'
            projected = await db.order.find_many(select={'routePath': True, 'routePaths': True})
            assert projected[0]['route_path'].value == 'top.c'
            assert [path.value for path in projected[0]['route_paths']] == paths
            async with db.transaction() as tx:
                rows = await tx.order.update(where={'id': 1}, data={'routePath': 'top.updated'})
                assert len(rows) == 1 and rows[0].route_path.value == 'top.updated'
                assert [path.value for path in rows[0].route_paths] == paths
            await db.order.raw_query(
                'INSERT INTO "__TABLE__" VALUES (2, \'top.sql\', ARRAY[\'top.sql\']::ltree[])'
            )
            row = await db.order.find_unique(where={'id': 2})
            assert row is not None and row.route_path.value == 'top.sql'
            assert [path.value for path in row.route_paths] == ['top.sql']
            rows = await db.order.update(where={'id': 1}, data={'routePath': '', 'routePaths': []})
            assert len(rows) == 1 and rows[0].route_path.value == '' and rows[0].route_paths == []
            rows = await db.order.update(where={'id': 1}, data={'routePath': None})
            assert len(rows) == 1 and rows[0].route_path is None
            row = await db.order.find_unique(where={'id': 1})
            assert row is not None and row.route_path is None and row.route_paths == []
        finally:
            await db.order.raw_query('DROP TABLE "__TABLE__"')
    print('ltree=ok')


asyncio.run(main())
