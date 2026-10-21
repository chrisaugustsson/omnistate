import type { z } from "zod";
import { ErrorBodySchema } from "./generated";

/** An answer of the API that is not a success: its status and stable error code. */
export class ApiError extends Error {
  constructor(
    readonly status: number,
    readonly code: string,
  ) {
    super(`${status} ${code}`);
    this.name = "ApiError";
  }
}

/**
 * Calls the API on the page's own origin (the proxy serves both), with the
 * session cookie. `body` is sent as JSON and the answer is checked against
 * `response`, a Zod schema of src/api/generated.ts; a response without a body
 * (204) is `undefined`. Anything but a 2xx throws an ApiError.
 */
export async function callApi<T = undefined>(
  method: string,
  path: string,
  options: { body?: unknown; response?: z.ZodType<T> } = {},
): Promise<T> {
  const headers: Record<string, string> = { accept: "application/json" };
  let body: string | undefined;
  if (options.body !== undefined) {
    headers["content-type"] = "application/json";
    body = JSON.stringify(options.body);
  }
  const response = await fetch(path, { method, headers, body, credentials: "same-origin" });
  if (!response.ok) {
    const parsed = ErrorBodySchema.safeParse(await response.json().catch(() => null));
    throw new ApiError(response.status, parsed.success ? parsed.data.error : "unknown");
  }
  if (!options.response) return undefined as T;
  return options.response.parse(await response.json());
}
