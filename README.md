# iq

**An experimental coding agent. It is not stable, it comes with no guarantee of any kind, and it changes all the time.**

This is a personal experiment, not a product. Commands, settings and file formats change without notice, and whatever is described here may be changed or removed at any time. If you want a coding agent to rely on, use an established one instead.

## The idea

Deterministic tools before models, security as the foundation, and every cost in view.

## Usage

```bash
export REQUESTY_API_KEY=...   # every model answers through Requesty
iq                            # runs in the terminal
iq do "..."                   # hands a task to the agents in the terminal
iq --help
```

## Local autopilot policy

The autopilot can use a skill written for the machine and project where iq runs. Set `autopilot_skill = "local-review"` in `~/.iq/settings.toml`, and keep a skill with that name in a configured skills folder, normally `~/.iq/skills/local-review/SKILL.md`. Describe which operations serve the local workflow and which should be refused or left to the person. The reviewer reads it again for every decision, so edits take effect during a session.

The skill must be outside the workspace, including its resolved symlink target, so commands cannot rewrite their own approval policy. A missing, unreadable, oversized or secret-bearing skill leaves the decision to the person. Without this setting, the default review policy applies. A skill cannot widen the sandbox, authorize publishing or bypass the gate; access to host services still requires support in the gate.

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
