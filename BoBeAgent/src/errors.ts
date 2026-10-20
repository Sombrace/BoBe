import { addAbortListener } from "node:events";

export class Cancelled extends Error {
  constructor(message = "Operation canceled") {
    super(message);
    this.name = "AbortError";
  }
}

export class WorkerFault extends Error {
  constructor(message: string, options?: ErrorOptions) {
    super(message, options);
    this.name = "WorkerFault";
  }
}

export function assertActive(signal?: AbortSignal): void {
  if (signal?.aborted) {
    if (Error.isError(signal.reason) && signal.reason.name === "TimeoutError") throw signal.reason;
    throw new Cancelled();
  }
}

export function onAbort(signal: AbortSignal | undefined, listener: () => void): Disposable | undefined {
  return signal ? addAbortListener(signal, listener) : undefined;
}

export function asError(error: unknown, fallback: string): Error {
  return Error.isError(error) ? error : new Error(fallback);
}

function errorMessage(error: unknown): string {
  if (error instanceof SuppressedError) return failureDetail([error]);
  return Error.isError(error) ? error.message : "Operation failed";
}

function causes(error: unknown): unknown[] {
  if (error instanceof AggregateError) return error.errors.flatMap(causes);
  if (error instanceof SuppressedError) return [...causes(error.suppressed), ...causes(error.error)];
  return [error];
}

export function failureDetail(failures: readonly unknown[]): string {
  return failures.flatMap(causes).map(errorMessage).join("; ");
}

export function rejections(outcomes: readonly PromiseSettledResult<unknown>[]): unknown[] {
  return outcomes.flatMap(outcome => outcome.status === "rejected" ? [outcome.reason] : []);
}

export async function settleEach(operations: readonly (() => unknown)[]): Promise<unknown[]> {
  const failures: unknown[] = [];
  for (const operation of operations) {
    try {
      await operation();
    } catch (error) {
      failures.push(error);
    }
  }
  return failures;
}

export class Redactor {
  private readonly secrets = new Set<string>();

  add(secret: string | null | undefined): void {
    if (secret) this.secrets.add(secret);
  }

  message(error: unknown): string {
    let message = errorMessage(error);
    for (const secret of this.secrets) message = message.replaceAll(secret, "[redacted]");
    return message
      .replace(/\bBearer\s+\S+/gi, "Bearer [redacted]")
      .replace(/\b(?:sk-|gh[opusr]_|github_pat_)[A-Za-z0-9_-]+/g, "[redacted]")
      .replace(/\beyJ[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+\b/g, "[redacted]")
      .slice(0, 2_048);
  }
}

export class Serial {
  private tail: Promise<void> = Promise.resolve();

  run<T>(operation: () => Promise<T>): Promise<T> {
    const next = this.tail.then(operation);
    this.tail = next.then(() => {}, () => {});
    return next;
  }
}
