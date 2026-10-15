/** The web app's health response, served at /health through the proxy. */
export function healthResponse(): Response {
  return Response.json({ status: "ok" });
}
