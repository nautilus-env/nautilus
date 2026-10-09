import { Nautilus } from './jsclient/index.js';

const db = new Nautilus();
await db.order.create({ data: { id: 1, invoice: '<a/>', region: 'north' } });
await db.order.update({ data: { invoice: null } });
await db.order.findMany({ where: { invoice: { isNull: true, isNotNull: false } } });
await db.order.findMany({ distinct: ['region'], orderBy: { region: 'asc' } });
await db.order.groupBy({ by: ['region'], count: { invoice: true }, order: { region: 'asc' } });
// @ts-expect-error XML has no equality operator.
await db.order.findMany({ where: { invoice: '<a/>' } });
// @ts-expect-error XML has no string operators.
await db.order.findMany({ where: { invoice: { contains: 'a' } } });
// @ts-expect-error Null checks require booleans.
await db.order.findMany({ where: { invoice: { isNull: 'true' } } });
// @ts-expect-error XML has no ordering operator.
await db.order.findMany({ orderBy: { invoice: 'asc' } });
// @ts-expect-error XML cannot be a distinct key.
await db.order.findMany({ distinct: ['invoice'] });
// @ts-expect-error XML cannot be a grouping key.
await db.order.groupBy({ by: ['invoice'] });
// @ts-expect-error XML cannot order groups.
await db.order.groupBy({ by: ['region'], order: { invoice: 'asc' } });
// @ts-expect-error XML cannot be aggregated with min.
await db.order.aggregate({ min: { invoice: true } });
