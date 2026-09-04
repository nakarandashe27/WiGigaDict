import { readFile } from "node:fs/promises";
import { createServer } from "node:http";
import { fileURLToPath } from "node:url";

const host = "127.0.0.1";
const port = 8765;
const fixtureUrl = new URL(
  "../tests/win32-spike/fixtures/browser-target.html",
  import.meta.url,
);

const server = createServer(async (request, response) => {
  if (request.method !== "GET" || request.url !== "/browser-target.html") {
    response.writeHead(404, { "Content-Type": "text/plain; charset=utf-8" });
    response.end("Not found");
    return;
  }

  try {
    const body = await readFile(fileURLToPath(fixtureUrl));
    response.writeHead(200, {
      "Cache-Control": "no-store",
      "Content-Type": "text/html; charset=utf-8",
    });
    response.end(body);
  } catch {
    response.writeHead(500, { "Content-Type": "text/plain; charset=utf-8" });
    response.end("Fixture unavailable");
  }
});

server.listen(port, host, () => {
  console.log(`WIN32_BROWSER_FIXTURE=http://${host}:${port}/browser-target.html`);
});
