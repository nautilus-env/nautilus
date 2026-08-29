// Runtime file — do not edit manually.

export declare enum EventPhase {
  Before = 'before',
  After  = 'after',
  Error  = 'error',
}

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

export declare class StopPropagation extends Error {
  readonly result: unknown;
  readonly hasResult: boolean;
  constructor(options?: StopPropagationOptions);
}

export declare function createModelEvents<TContexts extends ModelEventContexts = ModelEventContexts>(
  modelName: string,
): ModelEventToken<TContexts>;

export declare function createCrudEventContext(input: {
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
}): CrudEventContext;

export declare function runCrudEvent(
  context: CrudEventContext,
  options?: { handleStopPropagation?: boolean },
): Promise<StopPropagation | null>;

export declare function defaultCrudResult(
  operation: CrudOperation,
  returnData?: boolean,
): unknown;

export declare function resolveStopResult(
  stop: StopPropagation,
  defaultResult: unknown,
): unknown;
