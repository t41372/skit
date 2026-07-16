// Type annotations sit on the declarator, between the name and the value — the value node is still
// found, so an annotated const is a normal candidate.
const MAX_RETRIES: number = 5;
const ENDPOINT: string = "https://example.test";
const ENABLED: boolean = true;
const TIMEOUT = 30.5;
console.log(MAX_RETRIES, ENDPOINT, ENABLED, TIMEOUT);
