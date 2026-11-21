# Contributing

Thanks for helping out! Before you open a pull request, please read the part about commit messages,.

## Commit messages and versions

We don't pick version numbers manually. When something lands on `main`, the `release-plz` workflow reads the commit messages since the last release, works out the next version and writes the changelog. That only works if commits follow [Conventional Commits](https://www.conventionalcommits.org/). Examples:

```
fix: keep the input line when the port reconnects
feat: add --parity mark and space
feat!: rename --tx-log to --log-tx
```

While we're still at 0.x, a `fix:` or `feat:` bumps the patch version (0.3.1 → 0.3.2). A `!` after the type, or a `BREAKING CHANGE:` line in the commit body, bumps the minor version (0.3.1 → 0.4.0).

If your PR gets squash-merged, only its title ends up in the history. Write the title like a commit message therefore, including the `!` if the change is breaking. CI checks that the title has the right format, but it can't tell whether you forgot the `!` in the title.

## What counts as breaking

For a Rust library, cargo-semver-checks would catch breaking API changes. sermonizer is a command-line tool, so nothing unfortuantely life is a little harder for is. The interface users create their processes around is the command line itself, plus whatever scripts they've built around it.

A change therfore is breaking if someone's existing command or script would behave differently afterwards. In practice, that means you:

- renamed or removed a flag, an alias or an accepted value
- changed a default, like the baud rate, framing or line ending
- changed how typed input or control keys are handled
- changed the output of `--list` or the format of log files

Rewording a help text doesn't count, even though it changes the snapshot described below. Also, the visible output is meant only for humans, so we don't view any changes there as breaking.

## The CLI snapshot tests

To make these changes hard to miss, `cargo test` compares the full `--help` output and the hidden aliases (`--log-ts`, `lf`) against snapshots in `src/snapshots/`. If you modify the command line interface, one of these tests will fail, and that's expected. Look at the diff, decide whether the change is breaking, then accept the new snapshot and commit it with your change:

```bash
cargo install cargo-insta   # once
cargo insta accept
```

The snapshot diff will also show up in your PR, so reviewers can see the interface change at a glance.

## Before you push

CI runs these, so you can save yourself a round trip by running them first:

```bash
cargo fmt --all -- --check
cargo clippy -- -D warnings
cargo test
```
