// Destructuring binds several names from one expression — not a plain identifier, so skipped.
const { host, port } = { host: "localhost", port: 8080 };
const [first, second] = [1, 2];
const REAL = 5;
console.log(host, port, first, second, REAL);
