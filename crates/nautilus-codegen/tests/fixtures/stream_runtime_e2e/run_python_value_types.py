import asyncio
import sqlite3
from datetime import datetime, timezone
from decimal import Decimal
from pathlib import Path
from typing import Any
from uuid import UUID

from pyclient import Nautilus


def same(actual: Any, expected: Any) -> bool:
    """Equal values of the same types, recursively: ``12.5 == Decimal("12.5")``
    and ``"12.5" != Decimal("12.5")`` alone would not tell a decoded value from
    a raw one."""
    if type(actual) is not type(expected):
        return False
    if isinstance(expected, dict):
        return actual.keys() == expected.keys() and all(
            same(actual[key], value) for key, value in expected.items()
        )
    if isinstance(expected, list):
        return len(actual) == len(expected) and all(
            same(item, value) for item, value in zip(actual, expected)
        )
    return actual == expected


async def main() -> None:
    db_path = Path(__file__).parent / "value_types.sqlite"
    with sqlite3.connect(db_path) as connection:
        connection.execute(
            "CREATE TABLE products (id INTEGER PRIMARY KEY, sku CHAR(36) NOT NULL, "
            "unit_price DECIMAL(10, 2) NOT NULL, discount DECIMAL(10, 2), "
            "supplier_id CHAR(36), price_history TEXT NOT NULL, related_skus TEXT NOT NULL, "
            "restocked_at TEXT NOT NULL)"
        )

    first_sku = UUID("1b4e28ba-2fa1-41d2-883f-0016d3cca427")
    second_sku = UUID("6fa459ea-ee8a-4ca4-894e-db77e160355e")
    third_sku = UUID("9c5b94b1-35ad-49bb-b118-8e8fc24abf80")
    supplier = UUID("16fd2706-8baf-433b-82eb-8c7fada847da")
    restocks = [
        datetime(2026, 1, 2, 3, 4, 5, tzinfo=timezone.utc),
        datetime(2026, 2, 3, 4, 5, 6, tzinfo=timezone.utc),
    ]

    db = Nautilus()
    await db.connect()
    try:
        await db.product.create(
            {
                "id": 1,
                "sku": first_sku,
                "unitPrice": Decimal("12.5"),
                "discount": Decimal("1.25"),
                "supplierId": supplier,
                "priceHistory": [Decimal("10"), Decimal("12.5")],
                "relatedSkus": [second_sku, third_sku],
                "restockedAt": restocks,
            }
        )
        await db.product.create(
            {
                "id": 2,
                "sku": second_sku,
                "unitPrice": Decimal("12.5"),
                "priceHistory": [],
                "relatedSkus": [],
                "restockedAt": [],
            }
        )
        await db.product.create(
            {
                "id": 3,
                "sku": third_sku,
                "unitPrice": Decimal("7.25"),
                "supplierId": supplier,
                "priceHistory": [Decimal("7.25")],
                "relatedSkus": [first_sku],
                "restockedAt": restocks[:1],
            }
        )

        expected = {
            "sku": first_sku,
            "unit_price": Decimal("12.5"),
            "discount": Decimal("1.25"),
            "supplier_id": supplier,
            "price_history": [Decimal("10"), Decimal("12.5")],
            "related_skus": [second_sku, third_sku],
            "restocked_at": restocks,
        }

        row = await db.product.find_unique(where={"id": 1})
        read = {name: getattr(row, name) for name in expected}
        assert same(read, expected), read

        projection = await db.product.find_unique(
            where={"id": 1},
            select={
                "sku": True,
                "unitPrice": True,
                "discount": True,
                "supplier_id": True,
                "priceHistory": True,
                "related_skus": True,
                "restockedAt": True,
            },
        )
        # The engine adds the primary key to every projection.
        assert same(projection, {"id": 1, **expected}), projection

        projections = await db.product.find_many(
            select={"id": True, "discount": True, "supplierId": True},
            order_by={"id": "asc"},
        )
        assert same(
            projections,
            [
                {"id": 1, "discount": Decimal("1.25"), "supplier_id": supplier},
                {"id": 2, "discount": None, "supplier_id": None},
                {"id": 3, "discount": None, "supplier_id": supplier},
            ],
        ), projections

        async with db.transaction() as tx:
            in_transaction = await tx.product.find_first(
                where={"id": 3}, select={"unit_price": True, "sku": True}
            )
        assert same(
            in_transaction, {"id": 3, "unit_price": Decimal("7.25"), "sku": third_sku}
        ), in_transaction

        groups = await db.product.group_by(
            ["unit_price", "supplier_id"],
            count=True,
            min={"sku": True},
            max={"unitPrice": True},
            order={"unit_price": "asc", "supplier_id": "asc"},
        )
        assert same(
            groups,
            [
                {
                    "unit_price": Decimal("7.25"),
                    "supplier_id": supplier,
                    "_count": {"_all": 1},
                    "_min": {"sku": third_sku},
                    "_max": {"unitPrice": Decimal("7.25")},
                },
                {
                    "unit_price": Decimal("12.5"),
                    "supplier_id": None,
                    "_count": {"_all": 1},
                    "_min": {"sku": second_sku},
                    "_max": {"unitPrice": Decimal("12.5")},
                },
                {
                    "unit_price": Decimal("12.5"),
                    "supplier_id": supplier,
                    "_count": {"_all": 1},
                    "_min": {"sku": first_sku},
                    "_max": {"unitPrice": Decimal("12.5")},
                },
            ],
        ), groups

        totals = await db.product.aggregate(
            min={"unit_price": True, "discount": True},
            max={"sku": True, "supplierId": True},
        )
        assert same(
            totals,
            {
                "_min": {"unit_price": Decimal("7.25"), "discount": Decimal("1.25")},
                "_max": {"sku": third_sku, "supplierId": supplier},
            },
        ), totals
        print("value_types=ok")
    finally:
        await db.disconnect()


asyncio.run(main())
