// A reassignable binding is a working variable, not a parameter: offered but demoted.
let counter = 0;
var total = 100;
const STABLE = 42;
counter = counter + 1;
console.log(counter, total, STABLE);
