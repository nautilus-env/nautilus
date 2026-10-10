import asyncio
import copy
import inspect
import json

from pyclient.models import order


class CapturedClient:
    def __init__(self):
        self.payload = None

    def _sync_rpc(self, method, payload):
        assert method == 'query.findMany'
        self.payload = json.loads(json.dumps(payload))
        return {'data': []}

    async def _rpc(self, method, payload):
        return self._sync_rpc(method, payload)


async def main():
    client = CapturedClient()
    delegate = order.OrderDelegate(client)

    async def query(where, expected):
        original = copy.deepcopy(where)
        result = delegate.find_many(where=where)
        if inspect.isawaitable(result):
            result = await result
        assert result == []
        assert where == original, (where, original)
        assert client.payload['args']['where'] == expected, client.payload

    fields = [
        ('emailAddress', 'email_address', 'email_value', order.Citext('A@example.com'), 'A@example.com'),
        ('routePath', 'route_path', 'route_value', order.Ltree('top.c'), 'top.c'),
        ('attributeMap', 'attribute_map', 'attribute_value', order.Hstore({'size': 'M'}), {'size': 'M'}),
        ('footprint', 'footprint', 'footprint_value', order.Geometry.point(1, 2), 'POINT(1.0 2.0)'),
        ('serviceArea', 'service_area', 'service_value', order.Geography.point(1, 2), 'POINT(1.0 2.0)'),
        ('embedding', 'embedding', 'embedding_value', order.Vector.of(1, 2, 3), [1.0, 2.0, 3.0]),
    ]
    for logical, python, column, value, wire in fields:
        for name in {logical, python}:
            for alias, operator in [('is_null', 'isNull'), ('is_not_null', 'isNotNull'),
                                    ('isNull', 'isNull'), ('isNotNull', 'isNotNull')]:
                for operand in (True, False):
                    await query({name: {alias: operand}}, {column: {operator: operand}})
                    assert type(client.payload['args']['where'][column][operator]) is bool
            await query(
                {'AND': [{name: {'is_null': False}},
                         {'OR': [{name: {'is_not_null': True}},
                                 {'NOT': {name: {'is_null': True}}}]}]},
                {'AND': [{column: {'isNull': False}},
                         {'OR': [{column: {'isNotNull': True}},
                                 {'NOT': {column: {'isNull': True}}}]}]},
            )
            await query({name: {'is_null': 'true'}}, {column: {'isNull': 'true'}})
            await query({name: {'equals': value, 'is_not_null': True}},
                        {column: {'eq': wire, 'isNotNull': True}})
            await query({name: {'in_': [value, None], 'not_in': [value]}},
                        {column: {'in': [wire, None], 'notIn': [wire]}})
            data = order._process_create_data({name: value}, order._Order_py_to_db)
            assert data == {column: wire}
    print('extension_null_filters=ok')


asyncio.run(main())
