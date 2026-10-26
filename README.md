# iq

**An experimental coding agent. It is not stable, it comes with no guarantee of any kind, and it changes all the time.**

This is a personal experiment, not a product. Commands, settings and file formats change without notice, and whatever is described here may be changed or removed at any time. If you want a coding agent to rely on, use [Claude Code](https://code.claude.com/docs/en/overview) or [Codex](https://developers.openai.com/codex) instead.

## The idea

Deterministic tools before models, security as the foundation, and every cost in view.

## Usage

```bash
export REQUESTY_API_KEY=...   # every model answers through Requesty
iq                            # runs in the terminal
iq do "..."                   # hands a task to the agents in the terminal
iq --help
```

## Building

```bash
cargo install --path .              # builds the iq binary into ~/.cargo/bin
scripts/check.sh                    # formatting, lints, docs, tests, dependency audit
git config core.hooksPath .githooks # run those checks before every commit
```

## Data from others

None of this data ships with the program: each install fetches it for its own use and says where it comes from.

- **Model lists, prices and context windows** for the provider's models come from the provider itself.

## License

MIT
