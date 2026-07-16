// Object and array literals are structured, not scalar — never const candidates.
const CONFIG = { width: 800, host: "localhost" };
const PORTS = [8080, 8081, 8082];
console.log(CONFIG, PORTS);
