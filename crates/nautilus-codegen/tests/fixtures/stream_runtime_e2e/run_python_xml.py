import asyncio

from pyclient import Nautilus
from pyclient.errors import QueryError


async def main() -> None:
    async with Nautilus() as db:
        await db.order.raw_query(
            'CREATE TABLE "__TABLE__" (id INTEGER PRIMARY KEY, invoice_xml XML, region_code TEXT NOT NULL)'
        )
        try:
            document = '<invoice>caffè &amp; tea</invoice>'
            row = await db.order.create({'id': 1, 'invoice': document, 'regionName': 'north'})
            assert row is not None and row.invoice == document
            row = await db.order.find_unique(where={'id': 1})
            assert row is not None and row.invoice == document
            rows = await db.order.find_many(select={'invoice': True})
            assert rows[0]['invoice'] == document
            async with db.transaction() as tx:
                rows = await tx.order.update(where={'id': 1}, data={'invoice': '<invoice>changed</invoice>'})
                assert len(rows) == 1 and rows[0].invoice == '<invoice>changed</invoice>'
            await db.order.create({'id': 2, 'invoice': None, 'regionName': 'north'})
            for predicate, expected in [({'is_null': True}, 2), ({'is_null': False}, 1),
                                        ({'is_not_null': True}, 1), ({'is_not_null': False}, 2)]:
                rows = await db.order.find_many(where={'invoice': predicate})
                assert len(rows) == 1 and rows[0].id == expected
            rows = await db.order.find_many(distinct=['region_name'])
            assert len(rows) == 1
            result = await db.order.aggregate(count={'invoice': True})
            assert result['_count']['invoice'] == 1
            rows = await db.order.group_by(['region_name'], count={'invoice': True})
            assert rows[0]['_count']['invoice'] == 1
            for args in [{'where': {'invoice': document}}, {'order_by': {'invoice': 'asc'}},
                         {'distinct': ['invoice']}]:
                try:
                    await db.order.find_many(**args)
                except QueryError as error:
                    assert 'Xml' in str(error)
                else:
                    raise AssertionError(f'unsupported XML operation succeeded: {args}')
            rows = await db.order.update(where={'id': 1}, data={'invoice': None})
            assert len(rows) == 1 and rows[0].invoice is None
        finally:
            await db.order.raw_query('DROP TABLE "__TABLE__"')
    print('xml=ok')


asyncio.run(main())
