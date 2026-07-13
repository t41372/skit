// TypeScript-only constructs (interface / type alias / enum) that the JavaScript grammar can't
// parse — proof the ts kind selects the TypeScript grammar. The const among them stays a candidate.
interface Options {
  width: number;
  label: string;
}

type Mode = "fast" | "slow";

enum Level {
  Low,
  High,
}

const DEFAULT_WIDTH: number = 1024;
const opts: Options = { width: DEFAULT_WIDTH, label: "hi" };
let mode: Mode = "fast";
mode = "slow";
console.log(DEFAULT_WIDTH, opts, mode, Level.Low);
