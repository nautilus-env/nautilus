// Runtime file — do not edit manually.

import { Writable, Readable } from 'stream';

export declare class EngineProcess {
  constructor(enginePath?: string, migrate?: boolean);
  spawn(schemaPath: string): void;
  get stdin(): Writable | null;
  get stdout(): Readable | null;
  isRunning(): boolean;
  getStderrOutput(): string;
  terminate(): Promise<void>;
}
