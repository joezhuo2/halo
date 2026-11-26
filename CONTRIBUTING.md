# Contributing to Halo

Bug reports, ideas and pull requests are welcome. By taking part you agree to
the [Code of Conduct](CODE_OF_CONDUCT.md).

## Reporting a bug

Open a [bug report](https://github.com/joezhuo2/halo/issues/new/choose) with
your operating system, the Halo version from **Help > About Halo**, and the
steps that lead to the problem. For a problem with diagnostics, completion or
navigation, add the language server and the text of the **Output** panel.

Security problems go through [SECURITY.md](SECURITY.md), not public issues.

## Building

You need Rust 1.85 or newer and `git` on `PATH`.

```bash
cargo run
cargo test
```

`cargo test -- --ignored` also runs the test that talks to a real
`rust-analyzer`, when it is installed.

## Making a change

1. For anything larger than a small fix, open an issue first so the approach
   can be agreed on.
2. Branch from `main` and keep commits small, each with a message that says
   why the change is made.
3. Follow the house conventions: no code comments or docstrings (put the why
   in the commit message); colors from `Theme`, icons from `ui::icons`, a
   tooltip on icon-only buttons; every action is a `Command`; never block the
   UI thread (run work on a thread, report back over a channel); show errors,
   never drop them; keep CRLF line endings and the surrounding style.
4. Halo is meant to stay small. Prefer the standard library and the crates
   already in `Cargo.toml` over a new dependency.
5. Add or update a test for new logic, and make sure `cargo test` passes and
   `cargo clippy --all-targets -- -D warnings` is clean, as CI checks.
6. Describe user-visible changes under **Unreleased** in
   [CHANGELOG.md](CHANGELOG.md), and update the README when a feature, key
   or setting changes.
7. Open a pull request and fill in the template.

## License

Contributions are licensed under the [MIT License](LICENSE).
