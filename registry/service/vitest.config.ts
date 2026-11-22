import { cloudflareTest, readD1Migrations } from "@cloudflare/vitest-pool-workers";
import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { defineConfig } from "vitest/config";

const here = fileURLToPath(new URL(".", import.meta.url));

export default defineConfig(async () => {
  const migrations = await readD1Migrations(`${here}migrations`);
  // The reference validator's own answers for every case the parity tests send.
  const parity = JSON.parse(
    execFileSync("python3", ["-I", `${here}test/parity.py`], { encoding: "utf8", maxBuffer: 64 << 20 }),
  );
  return {
    plugins: [
      cloudflareTest({
        wrangler: { configPath: "./wrangler.jsonc" },
        miniflare: {
          bindings: {
            RATE_KEY: "test-rate-key",
            TEST_MIGRATIONS: migrations,
            INGEST_PY: readFileSync(`${here}../ingest.py`, "utf8"),
            PARITY: parity,
          },
        },
      }),
    ],
    test: {
      // Not *.test.ts: the repo root runs `bun test`, which would pick those up.
      include: ["test/**/*.vitest.ts"],
      setupFiles: ["./test/setup.ts"],
    },
  };
});
