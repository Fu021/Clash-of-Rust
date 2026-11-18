import { DurableObject } from "cloudflare:workers";

const visitorPattern = /^[a-f0-9]{8}-[a-f0-9]{4}-4[a-f0-9]{3}-[89ab][a-f0-9]{3}-[a-f0-9]{12}$/;
const headers = { "Cache-Control": "no-store", "X-Content-Type-Options": "nosniff" };

export class SiteVisitors extends DurableObject {
  #sql;

  constructor(context, environment) {
    super(context, environment);
    this.#sql = context.storage.sql;
    this.#sql.exec("CREATE TABLE IF NOT EXISTS visitors (sequence INTEGER PRIMARY KEY, digest TEXT NOT NULL UNIQUE)");
  }

  count(digest) {
    if (digest !== null) {
      this.#sql.exec("INSERT INTO visitors (digest) VALUES (?) ON CONFLICT (digest) DO NOTHING", digest);
    }
    return this.#sql.exec("SELECT COALESCE(MAX(sequence), 0) AS count FROM visitors").one().count;
  }
}

export default {
  async fetch(request, environment) {
    const url = new URL(request.url);
    if (url.pathname !== "/api/visitors") return environment.ASSETS.fetch(request);
    if (![environment.VISITOR_HOST, "localhost", "127.0.0.1"].includes(url.hostname)) {
      return new Response(null, { status: 404, headers });
    }
    if (request.method !== "GET" && request.method !== "POST") {
      return new Response(null, { status: 405, headers: { ...headers, Allow: "GET, POST" } });
    }
    let digest = null;
    if (request.method === "POST") {
      if (request.headers.get("Origin") !== url.origin) {
        return new Response(null, { status: 403, headers });
      }
      const identifier = request.headers.get("X-Cor-Visitor");
      if (!visitorPattern.test(identifier)) return new Response(null, { status: 400, headers });
      const buffer = await crypto.subtle.digest("SHA-256", new TextEncoder().encode(identifier));
      digest = new Uint8Array(buffer).toHex();
    }
    try {
      const count = await environment.SITE_VISITORS.getByName("site").count(digest);
      return Response.json({ count }, { headers });
    } catch {
      return new Response(null, { status: 503, headers });
    }
  },
};
