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
    if (signal.reason instanceof Error && signal.reason.name === "TimeoutError") throw signal.reason;
    throw new Cancelled();
  }
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : "Operation failed";
}

export function failureDetail(failures: readonly unknown[]): string {
  const causes = (error: unknown): unknown[] => error instanceof AggregateError ? error.errors.flatMap(causes) : [error];
  return failures.flatMap(causes).map(errorMessage).join("; ");
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

export function abortableDelay(milliseconds: number, signal?: AbortSignal): Promise<void> {
  assertActive(signal);
  return new Promise((resolve, reject) => {
    const onAbort = (): void => {
      clearTimeout(timer);
      signal?.removeEventListener("abort", onAbort);
      reject(new Cancelled());
    };
    const timer = setTimeout(() => {
      signal?.removeEventListener("abort", onAbort);
      resolve();
    }, milliseconds);
    signal?.addEventListener("abort", onAbort, { once: true });
  });
}
