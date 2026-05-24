// Runtime file — do not edit manually.

export const EventPhase = Object.freeze({
  Before: 'before',
  After:  'after',
  Error:  'error',
});

const NO_RESULT = Symbol('nautilus.stopPropagation.noResult');
const registry = new Map();

export class StopPropagation extends Error {
  constructor(options = undefined) {
    super('CRUD event propagation stopped');
    this.name = 'StopPropagation';
    this.result = options && Object.prototype.hasOwnProperty.call(options, 'result')
      ? options.result
      : NO_RESULT;
    this.hasResult = this.result !== NO_RESULT;
  }
}

export function createModelEvents(modelName) {
  const token = {
    modelName,
    onCreate:     modelEventRegistrar(modelName, 'create'),
    onCreateMany: modelEventRegistrar(modelName, 'createMany'),
    onUpdate:     modelEventRegistrar(modelName, 'update'),
    onDelete:     modelEventRegistrar(modelName, 'delete'),
    onDeleteMany: modelEventRegistrar(modelName, 'deleteMany'),
  };
  return Object.freeze(token);
}

export function createCrudEventContext(input) {
  const modelName = input.modelName ?? input.model_name;
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

export async function runCrudEvent(context, options = undefined) {
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

export function defaultCrudResult(operation, returnData = true) {
  if (operation === 'create' || operation === 'delete') return null;
  if (operation === 'createMany') return [];
  if (operation === 'update' || operation === 'deleteMany') return returnData ? [] : 0;
  return null;
}

export function resolveStopResult(stop, defaultResult) {
  return stop.hasResult ? stop.result : defaultResult;
}

function modelEventRegistrar(modelName, operation) {
  return function registerOrDecorate(phaseOrHandler = EventPhase.Before) {
    if (typeof phaseOrHandler === 'function') {
      registerEventHandler(modelName, operation, EventPhase.Before, phaseOrHandler);
      return phaseOrHandler;
    }

    const phase = normalizeEventPhase(phaseOrHandler);
    return (handler) => {
      registerEventHandler(modelName, operation, phase, handler);
      return handler;
    };
  };
}

function registerEventHandler(modelName, operation, phase, handler) {
  if (typeof handler !== 'function') {
    throw new TypeError('CRUD event handler must be a function');
  }
  const normalizedPhase = normalizeEventPhase(phase);
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
  handlers.push(handler);
}

function eventHandlers(modelName, operation, phase) {
  return [
    ...(registry.get(modelName)?.get(operation)?.get(normalizeEventPhase(phase)) ?? []),
  ];
}

function normalizeEventPhase(value) {
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

function transactionIdFromDb(db) {
  const txId = db?.transactionId ?? db?._transactionId;
  return typeof txId === 'string' ? txId : undefined;
}
