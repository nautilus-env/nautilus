import {
  EventPhase, IsolationLevel, Nautilus, StopPropagation, User,
  type EngineMetrics, type UserCreateEventContext,
} from './jsclient/index.js';
import { EngineProcess } from './jsclient/_internal/_engine.js';
import { errorFromCode, NautilusError } from './jsclient/_internal/_errors.js';
import type { JsonRpcRequest, JsonRpcResponse } from './jsclient/_internal/_protocol.js';

const db = new Nautilus({ pool: { maxConnections: 2, idleTimeoutMs: null } });
const metrics: EngineMetrics = await db.$metrics({ reset: true });
const calls: number = metrics.methods[0].calls;

User.onCreate((context: UserCreateEventContext) => {
  const name: string = context.args.data.name;
  context.state.name = name;
  // @ts-expect-error Event arguments are read-only.
  context.args.data = { name: 'changed' };
  // @ts-expect-error This model has no email field.
  context.args.data.email;
}, { priority: 2 });
User.onCreate(EventPhase.After)((context) => {
  const name: string | undefined = context.result?.name;
  throw new StopPropagation({ result: name });
});
User.onUpdate({ phase: 'before', priority: 1 })((context) => {
  const operation: 'update' = context.operation;
});

const selected = await db.user.findMany({ select: { id: true } });
const selectedId: number = selected[0].id;
// @ts-expect-error Fields outside the selection are absent.
selected[0].name;
// @ts-expect-error A name must be a string.
await db.user.create({ data: { name: 123 } });
for await (const row of db.user.streamMany({ select: { id: true } })) {
  const id: number = row.id;
  // @ts-expect-error Streaming preserves the selection.
  row.name;
}

const result: string = await db.$transaction(async (tx) => {
  await tx._rpc('query.count', { model: 'User' });
  return 'committed';
}, { isolationLevel: IsolationLevel.Serializable });
const batch: unknown[] = await db.$transactionBatch([
  { method: 'query.count', params: { model: 'User' } },
]);

const engine = new EngineProcess(undefined, false, { statementTimeoutMs: 1000 });
const input: NodeJS.WritableStream | null = engine.stdin;
const error: NautilusError = errorFromCode(3004, 'missing');
const request: JsonRpcRequest = { jsonrpc: '2.0', method: 'engine.metrics' };
const response: JsonRpcResponse = { jsonrpc: '2.0', result: metrics, partial: false };
