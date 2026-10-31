# iq coding rules

Every agent that works on iq follows these rules: most read this file as it is, some through a file of their own that imports it, and iq's own agents through their instructions.

## Language

**All code, comments, doc-comments, variable names, error messages, and commit messages must be in English.**

## Commits and pull requests

**No attribution to a tool or a model**: no `Co-Authored-By` trailer for an agent, no "Generated with" line. Agents of many kinds work on iq, iq's own among them; a commit says what changed and why, not who typed it.

## Prose and Markdown

**Never use em dashes or en dashes in Markdown files (README, AGENTS.md, docs).** Use a comma, a colon, parentheses, or a new sentence instead.

**Never hard-wrap prose at a fixed column.** One paragraph is one line, one bullet is one line. Fenced blocks and table rows keep their own line structure.

## Principles

**Security is the foundation, not a layer.** Every action a model or an agent asks for (reading or writing a file, running a command, reaching the network, calling a tool, a sub-agent's request for permission) goes through one gate, which answers allow, ask or deny. Nothing reaches the machine another way, and the crate graph makes it so. The operating system's sandbox is the boundary: commands run in it, writing only where the work is, always reaching localhost, and reaching the internet when a rule gives it (fetching packages and sources), when the person allows it, or for every command once the person opened it. What needs the machine itself (the container daemon, a cluster, the forge's own command line, a cloud's, each with the person's credentials) the rules know, or an agent asks for: it runs there, as the person would run it, once the person says, or the reviewing model following the policy the person wrote for it; what changes what others share waits for the person. Rules and a reviewing model only spare the person questions; they widen nothing the person did not let them. Secrets never reach a model.

**The person's skills say how the work is done.** iq gives the mechanisms: agents that start others and write to each other, the patterns a team works in, the gate, the costs. How a kind of work proceeds (which team to start, with which roles, models and efforts, how its agents work together and review each other, what the autopilot may allow) the person writes in their skills, Markdown files the agents read before the work they serve, and each person finds their own way. iq ships no method of its own: a way of working that iq would decide for the person belongs in a skill.

**Every cost is shown.** Each call to a model says what it cost, each agent how much context its calls read, and the costs are summed by model, by kind of task and over time, for the person to follow what the work spends. A feature that spends without showing it is a bug. iq never holds an agent back to spare money: the lead decides who does what and how hard each thinks, and the person, who sees what it costs, changes the lead or its effort.

## Architecture

Separation of concerns is **enforced by the crate dependency graph**, not by convention. The compiler refuses inverted dependencies.

```
                crates/core   ← no internal dependencies, no I/O
                ↑
      crates/llm                   crates/gate  (no internal dependencies)
     (core)
```

| Crate      | Single responsibility                                                      |
|------------|----------------------------------------------------------------------------|
| `iq-core`  | Domain types and traits, zero I/O, zero network                            |
| `iq-llm`   | Model providers, one `impl ChatModel` per protocol: Requesty's router, and any OpenAI compatible endpoint, with the models it lists and their prices, dearer past a size of conversation where its tiers say, and whether an error says the provider is out of capacity for now (a quota for the minute, an overload, a connection never made) with how long it asked to be left, from its headers or its words |
| `iq-gate`  | The one gate every action goes through (reading or writing a file, running a command line, reaching the network), judged by what it reaches and by rules on commands, written to the audit log before it is taken, a refusal written with its reason as an approval is; what waits for a login at a terminal or in a browser, which nobody can give an agent's command, refused by the rules; files read and written under a permit, where they were judged; the sandbox commands run in (Landlock and seccomp on Linux, with a network of the sandbox's own where only localhost answers; Seatbelt on macOS), their secrets kept out and their output cleared of the workspace's, git's hooks put back after them, each stopped past the time the settings give it or once it wrote nothing for as long as they say (one waiting for an answer nobody can give it), and stopped with its group when its agent is stopped; the commands the rules say need the machine itself, or an agent asks it for, run there once approved, with the person's environment but iq's own variables, their output cleared and hooks put back the same; a gate for another folder as the crew's own (its rules, its audit log, and what the person opened shared), as a worktree of the project is, the repository's own folder writable from it and the hooks it keeps put back; the variables a program of the person's gives a place, which every command run there gets, in the sandbox or on the machine, none of those the sandbox and the shell rest on nor a secret, each given written to the audit log with what that program said it ran |

A new crate is added when its first real code lands, not before, and this graph is updated in the same commit.

Only `iq-gate` makes a `Permit`, and whatever acts on the machine takes one, so the compiler refuses a way around the gate. A command runs through a helper, the iq command itself with `--iq-sandbox`, which confines its own process and then becomes the command: `main` checks for that flag before anything else starts. Rules are read from `~/.iq/rules.toml`, never from the workspace, which commands may write. Without the internet, a command on Linux joins a network of the sandbox's own, held by iq's program run with `--iq-network` for as long as the sandbox lives, so that a server one command starts answers the next; a rule gives the internet only to a line read whole whose other commands cannot send anything. A reviewing model may approve what stays within the sandbox, and, when the person lets it, what only reaches the internet and what runs on the machine itself; the rest (publishing, what changes what others share, a file outside the workspace, where code runs later) waits for the person. On macOS the helper becomes `/usr/bin/sandbox-exec` with a profile written for the command, its paths given as parameters; on other systems commands are refused until their sandbox is written. The sandbox's tests run real commands on both systems, so `scripts/check.sh` on a Mac is what tests its profile.

## Idiomatic Rust patterns

1. **Traits** are the only abstraction mechanism. `impl Trait` for static dispatch, `dyn Trait` only for heterogeneous collections.
2. **Newtypes** for domain primitives: `TokenCount`, `Usd`, `ModelId`. Never a bare `u64` or `f64` where a domain type makes sense.
3. **Typestate** for lifecycles where calling a method in the wrong state is a bug the compiler can catch.
4. **Builders** for complex configuration, validated at `build()`, returning `Result`.
5. **Iterators** over indexed loops.

## Non-negotiable rules

### Error handling
- `thiserror` in library crates: explicit error types
- `anyhow` in the binary
- Never `.unwrap()` or `.expect("...")` in library code
- An error keeps its source: wrap with `#[source]`, never stringify the cause away

### Secrets
- API keys come from the environment, never from a flag value in shell history, never from a committed file
- A type that holds a key implements `Debug` by hand and redacts it

### Unsafe
- `#![deny(unsafe_code)]` in every crate, the binary included

### Visibility
- `pub` only for items that are part of the crate's public API
- `pub(crate)` to share between modules within a crate
- No `pub use *`: re-exports are explicit

### Testing
- Unit tests `#[cfg(test)]` in the relevant module
- Integration tests in `crate/tests/`
- `proptest` for invariants (a session total does not depend on how it was split into requests)
- Wire formats are tested against fixed JSON bodies, never against a live provider

### Documentation
- Every `pub` item: `///` doc-comment required
- `# Examples` for non-trivial functions
- Comments explain WHY, not what the name already says

## Commands

```bash
scripts/check.sh    # formatting, lints, docs, tests, dependency audit
```
