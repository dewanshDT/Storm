export function jsonResponse(body: unknown, status: number, headers: Record<string, string> = {}): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": "application/json", ...headers },
  });
}

export function notFound(): Response {
  return jsonResponse({ error: "not found" }, 404);
}

export function unauthorized(): Response {
  return jsonResponse({ error: "unauthorized" }, 401);
}

export function methodNotAllowed(): Response {
  return jsonResponse({ error: "method not allowed" }, 405);
}
