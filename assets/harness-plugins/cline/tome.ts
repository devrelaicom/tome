// Tome session-steering shim for Cline.
//
// Harness: Cline · directive id: --harness cline
// No-op if the `tome` binary is absent: injects nothing, never throws.
//
// Executed by Cline's own Node runtime — never by Tome. Imports nothing from
// npm; uses only the Cline plugin API and node:child_process. The directive
// bytes come from `tome harness session-start --harness cline` (no --workspace:
// Tome resolves the bound workspace from the current working directory).

import { execFileSync } from "node:child_process";

function directive(): string {
  try {
    const out = execFileSync(
      "tome",
      ["harness", "session-start", "--harness", "cline"],
      { encoding: "utf8" },
    );
    return out.trim();
  } catch {
    // Missing binary (ENOENT), non-zero exit, or any spawn failure → inert.
    return "";
  }
}

export default {
  register(api: any): void {
    api.registerMessageBuilder({
      build(): { role: string; content: string } | undefined {
        const content = directive();
        if (content.length === 0) {
          return undefined;
        }
        return { role: "system", content };
      },
    });
  },
};
