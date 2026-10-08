import { Status } from './jsclient/enums.js';
import type { TaskWhereInput } from './jsclient/models/task.js';

const filters: TaskWhereInput[] = [
  { status: Status.DONE },
  { status: { in: [Status.PENDING, Status.DONE] } },
  { status: { notIn: [Status.DONE], not: Status.PENDING } },
  { previous: null },
  { previous: { isNull: true } },
  { OR: [{ previous: { in: [Status.DONE] } }, { previous: null }] },
];

// @ts-expect-error An enum filter takes the enum's values.
const unknownValue: TaskWhereInput = { status: { in: ['LATER'] } };
// @ts-expect-error An enum filter has no string operators.
const stringOperator: TaskWhereInput = { status: { contains: 'DO' } };
// @ts-expect-error A required enum is never null.
const nullStatus: TaskWhereInput = { status: null };
