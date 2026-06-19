// Tome session-steering shim for OpenCode.
//
// Harness: OpenCode · directive id: --harness opencode
// No-op if the `tome` binary is absent: injects nothing, never throws.
//
// Executed by OpenCode's own Bun runtime — never by Tome. Imports nothing from
// npm; uses Bun.spawnSync when on Bun, falling back to node:child_process under
// Node. The directive bytes come from `tome harness session-start --harness
// opencode` (no --workspace: Tome resolves the bound workspace from the current
// working directory).

import { execFileSync } from "node:child_process";

declare const Bun: any;

function directive(): string {
  const args = ["harness", "session-start", "--harness", "opencode"];
  try {
    if (typeof Bun !== "undefined") {
      const proc = Bun.spawnSync(["tome", ...args]);
      if (!proc.success) {
        return "";
      }
      return new TextDecoder().decode(proc.stdout).trim();
    }
    return execFileSync("tome", args, { encoding: "utf8" }).trim();
  } catch {
    // Missing binary (ENOENT), non-zero exit, or any spawn failure → inert.
    return "";
  }
}

export default {
  experimental: {
    chat: {
      system: {
        transform(input: { parts: string[] }): { parts: string[] } {
          const content = directive();
          if (content.length === 0) {
            return input;
          }
          return { parts: [...input.parts, content] };
        },
      },
    },
  },
};
