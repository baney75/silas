import { env } from "cloudflare:workers";
import { type Env, handler } from "../src/service";

export const ORIGIN = "https://silas-registry.example";

/** One request through the Worker's fetch handler, with the test bindings unless overridden. */
export function call(path: string, init: RequestInit = {}, bindings: Env = env): Promise<Response> {
  return handler.fetch(new Request(`${ORIGIN}${path}`, init), bindings);
}

export function post(body: BodyInit, ip = "198.51.100.1", headers: Record<string, string> = {}): Promise<Response> {
  return call("/v1/reports", {
    method: "POST",
    body,
    headers: { "Content-Type": "application/json", "CF-Connecting-IP": ip, ...headers },
  });
}
