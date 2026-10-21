// Rewrites src/api/generated.ts from openapi.json (`pnpm generate`).
import { readFileSync, writeFileSync } from "node:fs";
import { generateApiModule } from "../src/api/generate.ts";

const document: unknown = JSON.parse(
  readFileSync(new URL("../openapi.json", import.meta.url), "utf8"),
);
writeFileSync(
  new URL("../src/api/generated.ts", import.meta.url),
  generateApiModule(document),
);
