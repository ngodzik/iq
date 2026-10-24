# iq coding rules

Every agent that works on iq follows these rules: Codex and most others read this file as it is, Claude Code through `CLAUDE.md`, which imports it, and iq's own agents through their instructions.

## Language

**All code, comments, doc-comments, variable names, error messages, and commit messages must be in English.**

## Commits and pull requests

**No attribution to a tool or a model**: no `Co-Authored-By` trailer for an agent, no "Generated with" line. Agents of many kinds work on iq, iq's own among them; a commit says what changed and why, not who typed it.

## Prose and Markdown

**Never use em dashes or en dashes in Markdown files (README, AGENTS.md, docs).** Use a comma, a colon, parentheses, or a new sentence instead.

**Never hard-wrap prose at a fixed column.** One paragraph is one line, one bullet is one line. Fenced blocks and table rows keep their own line structure.

## Principles

**A deterministic tool does the work before any model does.** Compilers, tests, linters, the LSP and git are cheap, fast and right. A model is called for what they cannot do, and its output is checked by them before it is kept. A feature that sends a model what a tool could have answered is a bug.

**Security is the foundation, not a layer.** Every action a model or an agent asks for (reading or writing a file, running a command, reaching the network, calling a tool, a sub-agent's request for permission) goes through one gate, which answers allow, ask or deny. Nothing reaches the machine another way, and the crate graph makes it so. The operating system's sandbox is the boundary: commands run in it, writing only where the work is, always reaching localhost, and reaching the internet when a rule gives it (fetching packages and sources), when the person allows it, or for every command once the person opened it. Rules and a reviewing model only spare the person questions; they never widen the sandbox. Secrets never reach a model.

**Every cost is shown.** Each call to a model says what it cost, and the subscriptions of Claude Code and Codex say what is left of their windows. A feature that spends without showing it is a bug. The steward holds iq to the limits the person set: watching is counted, never asked of a model; only when the pace is too fast does it write to the agent that spends the most, and a limit reached closes its payer to new agents, the provider to new calls.

## Architecture

Separation of concerns is **enforced by the crate dependency graph**, not by convention. The compiler refuses inverted dependencies.

```
                crates/core   ← no internal dependencies, no I/O
                ↑         ↑
      crates/llm          |        crates/gate  (no internal dependencies)
     (core)               |        ↑
          ↑         crates/agent   (core + gate, never llm)
          |               ↑
          |         crates/assay   (core + gate + agent, never llm)
          |               ↑
          |         crates/ui      (core + agent + assay, never llm, never a window library)
          |               ↑
          |         crates/gui     (core + ui, never llm)
          |               ↑
          iq  (bin, depends on all)
```

| Crate      | Single responsibility                                                      |
|------------|----------------------------------------------------------------------------|
| `iq-core`  | Domain types and traits, zero I/O, zero network                            |
| `iq-llm`   | Model providers, one `impl ChatModel` per protocol (Requesty's router and any OpenAI compatible endpoint), the Claude Code and Codex `Delegate` (with the settings that make them reach iq back: Claude Code's hook and both agents' MCP server, and for Claude Code the other MCP servers the person's settings hand it), model prices, and what their subscriptions report (quotas, each call from their own files) |
| `iq-gate`  | The one gate every action goes through (reading or writing a file, running a command line, reaching the network), judged by what it reaches and by rules on commands, written to the audit log before it is taken; files read and written under a permit, where they were judged; the sandbox commands run in (Landlock and seccomp on Linux, with a network of the sandbox's own where only localhost answers; Seatbelt on macOS), their secrets kept out and their output cleared of the workspace's, git's hooks put back after them |
| `iq-agent` | The crew: iq's own agents (a model of the provider in a loop with its tools, each acting through the gate), the agents with tools of their own (Claude Code, Codex), the board they write to each other and to the person on (a message waking its reader a few times in a while), the patterns a team works in (solo, a lead and the agents it starts, a writer and its reviewer, a debate), each hand-over decided by iq from how a turn ended, never by a model, the reviewer that answers for the person when the autopilot is on (what stays within the sandbox, and what only reaches the internet unless the settings keep that for the person; publishing, files outside the workspace and secrets never), and the bridge through which the other agents reach the crew (a hook for the gate, MCP for the board, a command the gate allowed run in its sandbox), and the tools another part of iq offers some agents as an extension, whose calls act through the crew and ask the person through it. Generic over `ChatModel` and `Delegate` |
| `iq-assay` | The assay: exercises the judge writes with the person and freezes, a digest per version; each candidate (a model in iq's loop, an agent with tools of its own, a team in one of the patterns) run in a folder and a crew of its own, through a gate of its own, nobody answering for the person; its changes compared file by file, its hidden checks run in the sandbox and counted from their output, then graded blind by judges whose JSON is checked before it is kept; the results summed by iq (passes, scores, dollars per success, the cheapest that does as well as the reference), and the cost told before anything runs. The judge's tools are an extension of the crew. Generic over `ChatModel` and `Delegate` |
| `iq-ui`    | The window's state: its panes (one to four, each a chat, a chat to start, the costs, the subscriptions or the assay, and what the person writes in it, laid out by the person alone), the assay as the program read its folder and its runs as they go, the crew (each agent's thread, the questions waiting for the person), the costs of every call to a model (in dollars when billed, in tokens and in the share of its windows when a subscription paid, as it is paid by the month) and of each kind of task as the assay's rules place iq's tasks, the subscriptions' windows and requests, the steward (each limit the person set held against what is used, by iq and outside it, the pace, when it is full at that pace, and to whom it writes), the theme and opacity, and the shortcuts as the settings bind them. Draws nothing |
| `iq-gui`   | Draws `iq-ui`'s state in a see-through window, with egui on Bevy, in each theme's colours and the fonts it ships (Inter and JetBrains Mono, under the SIL Open Font License), what models write as Markdown, the panes moving quickly between layouts and at once with a line dragged, and the messages between agents flying from pane to pane; and runs its loop: it sleeps between events, the program wakes it when it sends something, and it draws often only while an agent works or something moves (no text cursor blinks, since bevy_egui draws at once any frame asked, even for later) |
| `iq`       | CLI entry point: the window, `iq do` in the terminal, `iq assay`, the person's settings, what iq keeps of its sessions (calls billed, quotas heard, each task's files, commands and costs), the assay and its judge's crew; and, run with one of its flags, the sandbox's helper, Claude Code's hook, the MCP server, a command run for Claude Code |

A new crate is added when its first real code lands, not before, and this graph is updated in the same commit.

The window's state is changed only through `iq-ui`'s methods, so that it is tested without a window; `iq-gui` draws it and turns clicks and keys into those calls, and what the person asks of the crew into `Request`s it sends. A key does something only through `iq-ui`'s `Keymap`, which the `[keys]` of the settings may change: the window takes the bound keys from egui before its widgets see them. The defaults are letters held with Ctrl, which sit at the same place on QWERTY and AZERTY. Neither does I/O: the program reads the agents' files on a thread of its own, carries out the requests, and sends what happened as `Update`s.

Only `iq-gate` makes a `Permit`, and whatever acts on the machine takes one, so the compiler refuses a way around the gate. A command runs through a helper, the iq command itself with `--iq-sandbox`, which confines its own process and then becomes the command: `main` checks for that flag before anything else starts. Rules are read from `~/.iq/rules.toml`, never from the workspace, which commands may write. Without the internet, a command on Linux joins a network of the sandbox's own, held by iq's program run with `--iq-network` for as long as the sandbox lives, so that a server one command starts answers the next; a rule gives the internet only to a line read whole whose other commands cannot send anything. A reviewing model may approve only what stays within the sandbox; what widens it (the network, a file outside the workspace, where code runs later) waits for the person. On macOS the helper becomes `/usr/bin/sandbox-exec` with a profile written for the command, its paths given as parameters; on other systems commands are refused until their sandbox is written. The sandbox's tests run real commands on both systems, so `scripts/check.sh` on a Mac is what tests its profile.

The assay's folder is out of `~/.iq`, which the gate holds as secret: its judge works there in a crew of its own, and each candidate in a folder of the store with a crew and a gate of its own, whose questions nobody answers, the same for every candidate. Its hidden checks are put in only once the candidate is done, and run in the sandbox. Its judges read the work in a folder that names nobody. Freezing, running and judging wait for the person, a run with what it should cost.

`iq-agent` does not depend on `iq-llm`: the crew is generic over `ChatModel` and `Delegate`, so that its tests run against scripted models with no network. iq's own agents act only through the crew's tools, which take a permit from the gate. Claude Code is started with a hook that has iq's program ask the running iq, over the bridge's socket in `~/.iq/run`, to judge each tool call: a command it allows comes back changed to run iq's program, which has the running iq run it in its sandbox, so that Claude Code's commands are held as iq's are. Claude Code asks for the internet for a command by starting its line with `IQ_NETWORK=1`. Codex runs its commands in its own sandbox, with the internet only once the person opened it. Both find the board's tools through iq's MCP server. The bridge acts only through the crew, so through the gate: a process that reaches it can ask what an agent may ask, never more. What the gate asks goes to the person, or to the reviewer when the autopilot is on, which may answer only what stays within the sandbox; a question nobody answers is refused.

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
