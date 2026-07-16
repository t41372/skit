// parseArgs reads identically under the TypeScript grammar (a superset of JS).
import { parseArgs } from "node:util";

const { values } = parseArgs({
  options: {
    output: { type: "string", default: "out.txt" },
    force: { type: "boolean" },
  },
});
console.log(values);
