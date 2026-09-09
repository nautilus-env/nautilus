export type CrudOperation =
  | 'create'
  | 'createMany'
  | 'update'
  | 'updateMany'
  | 'delete'
  | 'deleteMany';
export type EventPhaseValue = EventPhase | `${EventPhase}`;

export interface EventPriorityOptions {
  priority?: number;
}

export interface EventRegistrationOptions extends EventPriorityOptions {
  phase?: EventPhaseValue;
}

export interface StopPropagationOptions {
  result?: unknown;
}

export interface CrudEventContext<
  TModel = ModelEventToken,
  TOperation extends CrudOperation = CrudOperation,
  TArgs extends object = Record<string, unknown>,
  TPayload extends object = Record<string, unknown>,
  TResult = unknown,
  TError = unknown,
  TState extends object = Record<string, unknown>,
> {
  readonly db: unknown;
  readonly model: TModel;
  readonly modelName: string;
  readonly model_name: string;
  readonly operation: TOperation;
  readonly phase: EventPhase;
  readonly args: Readonly<TArgs>;
  readonly payload: Readonly<TPayload>;
  readonly result?: TResult;
  readonly error?: TError;
  readonly transactionId?: string;
  readonly transaction_id?: string;
  readonly state: TState;
}

export type CrudEventHandler<TContext extends CrudEventContext = CrudEventContext> = (
  context: TContext,
) => unknown | Promise<unknown>;

export interface ModelEventRegistrar<TContext extends CrudEventContext = CrudEventContext> {
  (handler: CrudEventHandler<TContext>): CrudEventHandler<TContext>;
  (
    handler: CrudEventHandler<TContext>,
    options: EventPriorityOptions,
  ): CrudEventHandler<TContext>;
  (
    phase: EventPhaseValue,
    options?: EventPriorityOptions,
  ): (handler: CrudEventHandler<TContext>) => CrudEventHandler<TContext>;
  (
    options: EventRegistrationOptions,
  ): (handler: CrudEventHandler<TContext>) => CrudEventHandler<TContext>;
}

export interface ModelEventContexts {
  create: CrudEventContext<ModelEventToken, 'create', any, any, any, any, any>;
  createMany: CrudEventContext<ModelEventToken, 'createMany', any, any, any, any, any>;
  update: CrudEventContext<ModelEventToken, 'update', any, any, any, any, any>;
  updateMany: CrudEventContext<ModelEventToken, 'updateMany', any, any, any, any, any>;
  delete: CrudEventContext<ModelEventToken, 'delete', any, any, any, any, any>;
  deleteMany: CrudEventContext<ModelEventToken, 'deleteMany', any, any, any, any, any>;
}

export interface ModelEventToken<TContexts extends ModelEventContexts = ModelEventContexts> {
  readonly modelName: string;
  readonly onCreate: ModelEventRegistrar<TContexts['create']>;
  readonly onCreateMany: ModelEventRegistrar<TContexts['createMany']>;
  readonly onUpdate: ModelEventRegistrar<TContexts['update']>;
  readonly onUpdateMany: ModelEventRegistrar<TContexts['updateMany']>;
  readonly onDelete: ModelEventRegistrar<TContexts['delete']>;
  readonly onDeleteMany: ModelEventRegistrar<TContexts['deleteMany']>;
}

export enum EventPhase {
  Before = 'before',
  After = 'after',
  Error = 'error',
}
Object.freeze(EventPhase);

const NO_RESULT = Symbol('nautilus.stopPropagation.noResult');
const registry = new Map<string, Map<CrudOperation, Map<EventPhase, Array<{ handler: CrudEventHandler; priority: number }>>>>();

export class StopPropagation extends Error {
  declare readonly result: unknown;
  declare readonly hasResult: boolean;
  constructor(options: StopPropagationOptions | undefined = undefined) {
    super('CRUD event propagation stopped');
    this.name = 'StopPropagation';
    this.result = options && Object.prototype.hasOwnProperty.call(options, 'result')
      ? options.result
      : NO_RESULT;
    this.hasResult = this.result !== NO_RESULT;
  }
}

export function createModelEvents<TContexts extends ModelEventContexts = ModelEventContexts>(modelName: string): ModelEventToken<TContexts> {
  const token = {
    modelName,
    onCreate:     modelEventRegistrar(modelName, 'create'),
    onCreateMany: modelEventRegistrar(modelName, 'createMany'),
    onUpdate:     modelEventRegistrar(modelName, 'update'),
    onUpdateMany: modelEventRegistrar(modelName, 'updateMany'),
    onDelete:     modelEventRegistrar(modelName, 'delete'),
    onDeleteMany: modelEventRegistrar(modelName, 'deleteMany'),
  };
  return Object.freeze(token) as ModelEventToken<TContexts>;
}

export function createCrudEventContext(input: {
  db: unknown;
  model: ModelEventToken;
  modelName?: string;
  model_name?: string;
  operation: CrudOperation;
  phase: EventPhase | `${EventPhase}`;
  args: Record<string, unknown>;
  payload: Record<string, unknown>;
  state: Record<string, unknown>;
  result?: unknown;
  error?: unknown;
  transactionId?: string;
  transaction_id?: string;
}): CrudEventContext {
  const modelName = (input.modelName ?? input.model_name) as string;
  const transactionId = input.transactionId ?? input.transaction_id ?? transactionIdFromDb(input.db);
  return Object.freeze({
    db:            input.db,
    model:         input.model,
    modelName,
    model_name:    modelName,
    operation:     input.operation,
    phase:         normalizeEventPhase(input.phase),
    args:          Object.freeze({ ...(input.args ?? {}) }),
    payload:       Object.freeze({ ...(input.payload ?? {}) }),
    result:        input.result,
    error:         input.error,
    transactionId,
    transaction_id: transactionId,
    state:         input.state ?? {},
  });
}

export async function runCrudEvent(context: CrudEventContext, options: { handleStopPropagation?: boolean } | undefined = undefined): Promise<StopPropagation | null> {
  const handleStopPropagation = options?.handleStopPropagation ?? true;
  for (const handler of eventHandlers(context.modelName, context.operation, context.phase)) {
    try {
      await handler(context);
    } catch (error) {
      if (error instanceof StopPropagation && handleStopPropagation) {
        return error;
      }
      throw error;
    }
  }
  return null;
}

export function defaultCrudResult(operation: CrudOperation, returnData = true): unknown {
  if (operation === 'create' || operation === 'delete') return null;
  if (operation === 'createMany') return [];
  if (operation === 'update' || operation === 'deleteMany') return returnData ? [] : 0;
  if (operation === 'updateMany') return 0;
  return null;
}

export function resolveStopResult(stop: StopPropagation, defaultResult: unknown): unknown {
  return stop.hasResult ? stop.result : defaultResult;
}

function modelEventRegistrar(modelName: string, operation: CrudOperation): ModelEventRegistrar {
  return function registerOrDecorate(phaseOrHandler: EventPhaseValue | EventRegistrationOptions | CrudEventHandler = EventPhase.Before, options: EventPriorityOptions | undefined = undefined) {
    if (typeof phaseOrHandler === 'function') {
      registerEventHandler(
        modelName,
        operation,
        EventPhase.Before,
        phaseOrHandler,
        eventPriority(options),
      );
      return phaseOrHandler;
    }

    const spec = eventRegistrationSpec(phaseOrHandler, options);
    return (handler: CrudEventHandler) => {
      registerEventHandler(modelName, operation, spec.phase, handler, spec.priority);
      return handler;
    };
  } as ModelEventRegistrar;
}

function registerEventHandler(modelName: string, operation: CrudOperation, phase: EventPhaseValue, handler: CrudEventHandler, priority = 0): void {
  if (typeof handler !== 'function') {
    throw new TypeError('CRUD event handler must be a function');
  }
  const normalizedPhase = normalizeEventPhase(phase);
  const normalizedPriority = normalizeEventPriority(priority);
  let byModel = registry.get(modelName);
  if (!byModel) {
    byModel = new Map();
    registry.set(modelName, byModel);
  }
  let byOperation = byModel.get(operation);
  if (!byOperation) {
    byOperation = new Map();
    byModel.set(operation, byOperation);
  }
  let handlers = byOperation.get(normalizedPhase);
  if (!handlers) {
    handlers = [];
    byOperation.set(normalizedPhase, handlers);
  }
  handlers.push({ handler, priority: normalizedPriority });
  handlers.sort((left, right) => right.priority - left.priority);
}

function eventHandlers(modelName: string, operation: CrudOperation, phase: EventPhaseValue): CrudEventHandler[] {
  return [
    ...(registry.get(modelName)?.get(operation)?.get(normalizeEventPhase(phase)) ?? []),
  ].map((registered) => registered.handler);
}

function eventRegistrationSpec(phaseOrSpec: EventPhaseValue | EventRegistrationOptions, options?: EventPriorityOptions): { phase: EventPhase; priority: number } {
  if (phaseOrSpec && typeof phaseOrSpec === 'object' && !Array.isArray(phaseOrSpec)) {
    return {
      phase: normalizeEventPhase(phaseOrSpec.phase ?? EventPhase.Before),
      priority: eventPriority(phaseOrSpec),
    };
  }
  return {
    phase: normalizeEventPhase(phaseOrSpec),
    priority: eventPriority(options),
  };
}

function eventPriority(options?: EventPriorityOptions): number {
  return normalizeEventPriority(options?.priority ?? 0);
}

function normalizeEventPriority(value: number): number {
  if (!Number.isInteger(value) || value < 0 || value > 255) {
    throw new TypeError('CRUD event handler priority must be an integer between 0 and 255');
  }
  return value;
}

function normalizeEventPhase(value: unknown): EventPhase {
  if (value === EventPhase.Before || value === 'before' || value === 'Before') {
    return EventPhase.Before;
  }
  if (value === EventPhase.After || value === 'after' || value === 'After') {
    return EventPhase.After;
  }
  if (value === EventPhase.Error || value === 'error' || value === 'Error') {
    return EventPhase.Error;
  }
  throw new Error(`Unknown event phase: ${String(value)}`);
}

function transactionIdFromDb(db: unknown): string | undefined {
  const txId = (db as { transactionId?: unknown } | undefined)?.transactionId ?? (db as { _transactionId?: unknown } | undefined)?._transactionId;
  return typeof txId === 'string' ? txId : undefined;
}
