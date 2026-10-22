# iq

**An experimental coding agent. It is not stable, it comes with no guarantee of any kind, and it changes all the time.**

This is a personal experiment, not a product. Commands, settings and file formats change without notice, and whatever is described here may be changed or removed at any time. If you want a coding agent to rely on, use [Claude Code](https://code.claude.com/docs/en/overview) or [Codex](https://developers.openai.com/codex) instead.

## The idea

Deterministic tools before models, security as the foundation, and every cost in view.

## Usage

```bash
export REQUESTY_API_KEY=...   # without one, only Claude Code and Codex answer
iq                            # opens the window
iq do "..."                   # hands a task to the agents in the terminal
iq --help
```

## Building

On Linux the window needs Wayland's client library: `sudo apt install libwayland-dev` on Debian and Ubuntu.

```bash
cargo install --path .              # builds the iq binary into ~/.cargo/bin
scripts/check.sh                    # formatting, lints, docs, tests, dependency audit
git config core.hooksPath .githooks # run those checks before every commit
```

## Data from others

None of this data ships with the program: each install fetches it for its own use and says where it comes from.

- **Model prices** for Claude Code's and Codex's calls come from [LiteLLM](https://github.com/BerriAI/litellm)'s public list, [`model_prices_and_context_window.json`](https://github.com/BerriAI/litellm/blob/main/model_prices_and_context_window.json), under the MIT licence. They are list prices: an estimate, not a bill.
- **Model scores** come from [Artificial Analysis](https://artificialanalysis.ai/)' free API, with your own key in `ARTIFICIAL_ANALYSIS_API_KEY`, under their terms, which ask that they be credited wherever scores show.
- **Model lists, prices and context windows** for the provider's models come from the provider itself.

## License

MIT
