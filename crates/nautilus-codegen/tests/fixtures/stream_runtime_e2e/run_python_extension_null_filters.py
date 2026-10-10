import asyncio

from pyclient import Nautilus
from pyclient.errors import QueryError


async def main():
    async with Nautilus() as db:
        for extension in ('citext', 'ltree', 'hstore'):
            await db.order.raw_query(f'CREATE EXTENSION IF NOT EXISTS {extension}')
        await db.order.raw_query(
            'CREATE TABLE "__TABLE__" (id INTEGER PRIMARY KEY, email_value CITEXT, '
            'route_value LTREE, attribute_value HSTORE)'
        )
        try:
            await db.order.create({'id': 1, 'emailAddress': 'A@example.com',
                                   'routePath': 'top.c', 'attributeMap': {'size': 'M'}})
            await db.order.create({'id': 2})
            for field in ('emailAddress', 'email_address', 'routePath', 'route_path',
                          'attributeMap', 'attribute_map'):
                for predicate, expected in [({'is_null': True}, 2), ({'is_null': False}, 1),
                                            ({'is_not_null': True}, 1), ({'is_not_null': False}, 2)]:
                    rows = await db.order.find_many(where={field: predicate})
                    assert [row.id for row in rows] == [expected], (field, predicate, rows)
                    assert await db.order.count(where={field: predicate}) == 1
                rows = await db.order.find_many(where={
                    'AND': [{field: {'is_null': False}},
                            {'NOT': {field: {'is_not_null': False}}}]
                })
                assert [row.id for row in rows] == [1]
                for operator in ('is_null', 'is_not_null'):
                    try:
                        await db.order.find_many(where={field: {operator: 'true'}})
                    except QueryError as error:
                        assert 'value must be a boolean' in str(error)
                    else:
                        raise AssertionError(f'invalid null operand accepted: {field}, {operator}')
            async with db.transaction() as tx:
                assert await tx.order.count(where={'routePath': {'is_not_null': True}}) == 1
        finally:
            await db.order.raw_query('DROP TABLE "__TABLE__"')
    print('extension_null_filters=ok')


asyncio.run(main())
