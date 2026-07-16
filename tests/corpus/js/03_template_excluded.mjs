// A template string may interpolate, so it is NEVER a const candidate — even without a `${...}`.
const NAME = "world";
const GREETING = `hello ${NAME}`;
const PLAIN = `no interpolation here`;
console.log(GREETING, PLAIN);
