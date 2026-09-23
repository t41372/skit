# Mutation testing policy

`cargo mutants` is a hard CI gate. The accepted result has zero surviving mutants.

```bash
cargo mutants --workspace --all-features --cargo-arg=--locked --jobs 2 --minimum-test-timeout 20
```

Do not exclude a mutant because it is difficult to test. First add a contract test, then improve the
design so the behavior is observable. An exclusion is acceptable only for generated or structural
code with no product behavior. Document each exclusion in `.cargo/mutants.toml` with the exact
reason.

Mutation records are CI artifacts. They are not committed. Line coverage and mutation testing are
separate gates: line coverage checks execution, and mutation testing checks assertions.
