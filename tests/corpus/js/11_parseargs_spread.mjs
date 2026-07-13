// A spread merges in options the reader can't see — whole-spec degrade.
import { parseArgs } from "node:util";

const common = { verbose: { type: "boolean" } };
const { values } = parseArgs({
  options: { ...common, name: { type: "string" } },
});
console.log(values);
