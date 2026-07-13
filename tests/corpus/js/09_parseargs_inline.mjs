// A readable util.parseArgs surface: string/boolean types, a short alias, a default, and multiple.
import { parseArgs } from "node:util";

const { values } = parseArgs({
  options: {
    name: { type: "string", short: "n", default: "world" },
    verbose: { type: "boolean" },
    tag: { type: "string", multiple: true },
    "dry-run": { type: "boolean", default: false },
  },
});
console.log(values);
