# Deterministic value-token contract

Status: implemented and contract-tested in `skit-application/tests/value_tokens.rs`.

`skit-application::tokens` expands stored intent strings from an explicit `TokenContext`; it does
not read process-global current-directory, home-directory, environment, or clock state. The caller
supplies invoke-time `cwd`, current-user `home`, child-environment values, local `today`, and local
`now`.

The scanner expands `{cwd}`, `{today}`, `{now}`, and syntactically valid `{env:NAME}` tokens. Unknown
brace expressions pass through unchanged. Missing environment variables return a structured error
that retains the exact token spelling. Brace escaping is independent: with escapes enabled, `{{`
and `}}` halve to literal braces; with escapes disabled, those pairs remain byte-identical and the
scanner skips token matching inside them, while ordinary named tokens still expand.

A leading current-user `~`, `~/`, or `~\\` form expands when the caller supplies a home value and
then composes with named tokens. Named-user tilde forms are deliberately outside this slice. Preview
never raises: on failure it returns the original text and the user-ready error. `has_tokens`
recognizes only syntax the expansion pass can change.

Not yet implemented by this contract: shell splitting, glob expansion, conversion of parameter
values into argv/environment/placeholder/inject plans, secret masking, remembered values, presets,
form assembly, persistence, or child launch.
