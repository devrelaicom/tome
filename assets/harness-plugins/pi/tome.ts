// Tome session-steering shim for Pi.
//
// Harness: Pi · directive id: --harness pi
// No-op if the `tome` binary is absent: injects nothing, never throws.
//
// Executed by Pi's own Node runtime — never by Tome. Imports nothing from npm;
// uses only the Pi extension API and node:child_process. The directive bytes
// come from `tome harness session-start --harness pi` (no --workspace: Tome
// resolves the bound workspace from the current working directory).

import { execFileSync } from "node:child_process";

function directive(): string {
  try {
    const out = execFileSync(
      "tome",
      ["harness", "session-start", "--harness", "pi"],
      { encoding: "utf8" },
    );
    return out.trim();
  } catch {
    // Missing binary (ENOENT), non-zero exit, or any spawn failure → inert.
    return "";
  }
}

export default {
  register(pi: any): void {
    pi.on("before_agent_start", () => {
      const content = directive();
      if (content.length === 0) {
        return undefined;
      }
      return {
        message: { customType: "tome", content, display: true },
      };
    });
  },
};
