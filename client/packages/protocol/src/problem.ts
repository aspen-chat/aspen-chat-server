import type { components } from "./generated/openapi";

/** RFC 9457 Problem Details as the server emits them. `code` is the field to branch on. */
export type Problem = components["schemas"]["Problem"];
export type ProblemCode = components["schemas"]["ProblemCode"];

export function isProblem(value: unknown): value is Problem {
  if (value === null || typeof value !== "object") {
    return false;
  }
  const v = value as Record<string, unknown>;
  return typeof v.code === "string" && typeof v.status === "number";
}

/**
 * Thrown by the convenience wrappers when the server answers with a Problem. Callers that use
 * the raw openapi-fetch client receive the Problem as `error` instead and never see this.
 */
export class ApiProblemError extends Error {
  readonly problem: Problem;

  constructor(problem: Problem) {
    super(problem.detail ?? problem.title);
    this.name = "ApiProblemError";
    this.problem = problem;
  }

  get code(): ProblemCode {
    return this.problem.code;
  }

  get status(): number {
    return this.problem.status;
  }
}

/**
 * Wraps a transport-level failure (the network was unreachable, the response was not JSON) in a
 * Problem so callers have one error shape to handle.
 */
export function transportProblem(detail: string, status = 0): Problem {
  return {
    code: "internal",
    title: "The server could not be reached.",
    status,
    detail,
  };
}
