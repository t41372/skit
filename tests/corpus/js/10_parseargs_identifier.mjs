// `options` is an identifier reference — the real option set lives elsewhere, so the reader
// degrades the whole spec honestly (passthrough escape only).
import { parseArgs } from "node:util";

const optionSpec = { name: { type: "string" } };
const { values } = parseArgs({ options: optionSpec });
console.log(values);
