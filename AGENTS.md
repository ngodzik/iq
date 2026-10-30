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

**A deterministic tool does the work before any model does.** Compilers, tests, linters, the LSP and git are cheap, fast and right. A model is called for what they cannot do, and its output is checked by them before it is kept. A feature that sends a model what a tool could have answered is a bug.

**Security is the foundation, not a layer.** Every action a model or an agent asks for (reading or writing a file, running a command, reaching the network, calling a tool, a sub-agent's request for permission) goes through one gate, which answers allow, ask or deny. Nothing reaches the machine another way, and the crate graph makes it so. The operating system's sandbox is the boundary: commands run in it, writing only where the work is, always reaching localhost, and reaching the internet when a rule gives it (fetching packages and sources), when the person allows it, or for every command once the person opened it. What needs the machine itself (the container daemon, a cluster, the forge's own command line, a cloud's, each with the person's credentials) the rules know, or an agent asks for: it runs there, as the person would run it, once the person says, or the reviewing model following the policy the person wrote for it; what changes what others share waits for the person. Rules and a reviewing model only spare the person questions; they widen nothing the person did not let them. Secrets never reach a model.

**Every cost is shown.** Each call to a model says what it cost, each agent how much context its calls read, and the costs are summed by model, by kind of task and over time, for the person to follow what the work spends. A feature that spends without showing it is a bug. iq never holds an agent back to spare money: the lead decides who does what and how hard each thinks, and the person, who sees what it costs, changes the lead or its effort.

## Architecture

Separation of concerns is **enforced by the crate dependency graph**, not by convention. The compiler refuses inverted dependencies.

```
                crates/core   ← no internal dependencies, no I/O
                ↑         ↑
      crates/llm          |        crates/gate  (no internal dependencies)
     (core)               |        ↑
          ↑         crates/agent   (core + gate, never llm)
          |               ↑
          |         crates/observe (core + agent)
          |               ↑
          |         crates/memory  (core + agent)
          |               ↑
          |         crates/ui      (core + agent, never llm, never a terminal library)
          |               ↑
          |         crates/tui     (core + agent + ui, never llm)
          |               ↑
          iq  (bin, depends on all)

      crates/sync   (gate)
      crates/plugin (no internal dependencies, no I/O)
      plugins/*     (plugin, and nothing else of iq's: iq never depends on one)
```

| Crate      | Single responsibility                                                      |
|------------|----------------------------------------------------------------------------|
| `iq-core`  | Domain types and traits, zero I/O, zero network                            |
| `iq-llm`   | Model providers, one `impl ChatModel` per protocol: Requesty's router, and any OpenAI compatible endpoint, with the models it lists and their prices, dearer past a size of conversation where its tiers say, and whether an error says the provider is out of capacity for now (a quota for the minute, an overload, a connection never made) with how long it asked to be left, from its headers or its words |
| `iq-gate`  | The one gate every action goes through (reading or writing a file, running a command line, reaching the network), judged by what it reaches and by rules on commands, written to the audit log before it is taken, a refusal written with its reason as an approval is; what waits for a login at a terminal or in a browser, which nobody can give an agent's command, refused by the rules; files read and written under a permit, where they were judged; the sandbox commands run in (Landlock and seccomp on Linux, with a network of the sandbox's own where only localhost answers; Seatbelt on macOS), their secrets kept out and their output cleared of the workspace's, git's hooks put back after them, each stopped past the time the settings give it or once it wrote nothing for as long as they say (one waiting for an answer nobody can give it), and stopped with its group when its agent is stopped; the commands the rules say need the machine itself, or an agent asks it for, run there once approved, with the person's environment but iq's own variables, their output cleared and hooks put back the same; a gate for another folder as the crew's own (its rules, its audit log, and what the person opened shared), as a worktree of the project is, the repository's own folder writable from it and the hooks it keeps put back; the variables a program of the person's gives a place, which every command run there gets, in the sandbox or on the machine, none of those the sandbox and the shell rest on nor a secret, each given written to the audit log with what that program said it ran |
| `iq-agent` | The crew: iq's agents (a model of the provider in a loop with its tools, each acting through the gate), the board they write to each other and to the person on (a message waking its reader a few times in a while), the patterns the person may start a team in (solo, a lead and the agents it starts, a writer and its reviewer, a debate), each hand-over decided by iq from how a turn ended, never by a model, the teams an agent starts with the roles, models and efforts it gives them, as the person's skills tell it (the first member working at once and answering whoever started the team, the others waiting until a teammate writes and answering the first), the rules agents follow (the person's own files, iq's `~/.iq/AGENTS.md`, else `~/.claude/CLAUDE.md`, unless the settings list others, then the project's `AGENTS.md`, `CLAUDE.md` and those of its `.claude/rules` for the whole project, a file imported twice given once, the project's read where the agent works) given only to the agents the person starts, the rules of `.claude/rules` whose head names `paths` given to any agent as it first reads or writes one of those files, the leads told which there are, as are the extensions for leads, those agents giving the ones they start what their part needs, each agent told its conversation's size each time it grows by a step the settings set, and where its model's price rises when the provider's list says, and compacting it with its own summary once its request is done, when its skills say, each agent thinking as hard as it was started with (the lead choosing its agents') or changed to by the person, the reviewer that answers for the person when the autopilot is on, thinking as hard as the settings say, given its selected local skill again for each decision, told where the action runs and, for a `make` target, what its recipe and those it depends on do, read from the Makefile where the agent works (what stays within the sandbox, what only reaches the internet and what runs on the machine itself unless the settings keep those for the person, the machine's being the reviewer's once a skill says what runs there; publishing, what changes what others share, files outside the workspace and secrets never), an agent's next action asked of the person after as many refusals in a row as the settings say, each agent the person starts given a place of its own when they want one (a folder apart from their checkout, as a worktree is, made by what the binary gives the crew for it), the agents it starts working there with it, and one that could not be given its place not started, the person told why, each chat given to a keeper as it joins and at every turn's end, and put back on the board from what was kept, resting, its instructions written again, in its place when it still has one, a call to a provider out of capacity sent again as many times as the settings say, after as long as it asks and a wait doubling at each try, each wait said, then the turn ended saying the conversation is kept to go on later or on another model, and the tools another part of iq offers some agents as an extension, whose calls act through the crew and ask the person through it. Generic over `ChatModel` |
| `iq-observe` | What iq's work goes to: the journal's tasks, the categories of work the person names in their settings with rules if they wish, the rules placing a task first, then a cheap model in the background, shown the categories and the tasks placed before so that it keeps to them, its placements kept beside the journal, a day's placing capped; nothing leaves the machine. Generic over `ChatModel` |
| `iq-memory` | What the agents the person starts know beyond their task: the project's notes, each a Markdown file with its name and what it holds, kept by the address of the project's repository so that the same project on another machine finds them, the notes kept of the project under `~/.claude` read beside them and never changed; and the person's skills, Markdown files in the folders the settings list (a folder with its `SKILL.md`, or a file), each listed by name and read whole with a tool before the work it is for. A secret is never kept nor given. The selected skill for permission reviews is read again at each decision and kept outside the workspace so commands cannot rewrite their own policy, never shown to the agents, a missing, unreadable or unsafe skill leaving the decision to the person. Extensions of the crew for leads and review policy |
| `iq-sync`  | iq's folder of data kept in the git repository the settings' `[sync]` names, a private one of any name: what changed here sent, what changed elsewhere taken, a new machine taking everything before it sends anything, this machine's lines winning where both changed; the settings, rules and skills of iq's own folder mirrored in it, and the files of the home they keep, put back on a new machine, what holds secrets never kept (in a folder of credentials of the home, such as `~/.claude`, only its Markdown files), a file of the home never removed; what a sync would send listed before it goes, for the person to say whether it does, the person's own files by name and the rest counted by folder, and shown line by line as the diff git makes of it, each file named as the repository names it, never by where it lives; a new machine taking what is there first and sending nothing until asked; what the folder's `.gitignore` leaves out, the worktrees the agents work in and the store of the assay iq once had among it, taken out of the repository. Git does it all, with the credentials the machine has |
| `iq-plugin` | What iq and its plugins say to each other, one line of JSON each way over the plugin's standard input and output: iq's hello with its protocol and folder, where each chat works, the person's commands, its goodbye; the plugin's hello with its protocol and commands, what it shows on the status line and on the Team page, the variables it gives a place's commands with the line that sets them in a terminal, what it says, and what it runs on the machine. The messages and how they are written, no I/O |
| `iq-ui`    | iq's state in the terminal: where the person writes (to an agent, or to a new chat with its model, its pattern and its role), what they write and the messages sent before, the `/` commands and what completes them, the page open over the conversation and how far down it is read (the crew, an agent's thread, the costs, the observations, the keys, what a sync would send, answered from there, every step the agents took), the agents' steps (the tools they call, what the person and the reviewer allow or refuse them) folded between two of their lines of text, a fold closed as the agent goes on, the observations (iq's tasks over a day, a week, a month, a year or all of it, in the person's time zone, summed by category, model, project or agent), the crew (each agent's thread, where it works when it has a place of its own, what it spent, the context its calls read, the questions waiting for the person), the costs of every call to a model, by model, over time and by kind of task as the person's categories place iq's tasks, over the last hours or days or since midnight in the person's time zone, and what was billed today beside what the session spent, the models the person names in a message ("with opus"), read by matching words among the provider's, the theme, the shortcuts as the settings bind them, and what the person's plugins show (their commands, completed and listed with iq's, which come first, what each shows on the status line and under a chat on the Team page, the line that sets a place's variables in a terminal), all of it gone with a plugin that stopped; every key the person presses is taken there. A new task starts on the lead's model, or on the one the person named or chose, and `/effort` sets how hard the agent written to thinks. Draws nothing |
| `iq-tui`   | Draws `iq-ui`'s state in the terminal, as terminal agents do: the conversation written once into the terminal's own scrollback as it happens (each agent's text a line at a time, as the Markdown it is, its tools, its questions answered, its messages, the end of its turns), so that the person scrolls, selects and copies it as any output; below it a live part drawn again as things move (the agents at work, each with its latest steps since its last line of text, as many as the settings say, the earlier counted, the oldest question waiting, the completions, where the person writes, the whole path of the folder of its own the agent they write to works in, for them to paste in another terminal, or the command that goes there and sets the variables a plugin gave it, the status line with what the plugins show, along which a message's spark runs from its writer to its reader); the pages on the terminal's other screen, with charts drawn in characters (curves in Braille dots, bars in half blocks, gauges by eighths); colours in 24 bits where the terminal takes them, else the nearest of its 256, the terminal's background never painted; and runs the loop, which draws only when something changes or moves |
| `iq`       | CLI entry point: iq in the terminal, `iq do` for a task handed over at once, `iq sync`, each chat the person starts in a worktree of its own when the settings or `--worktree` say (under iq's folder of data, on a branch `iq/<chat>` from what is checked out or from what the settings name, set up with the commands they list, run there on the machine, the person told where it is, what it starts from and how to take its work, their checkout left as it is; as iq starts, the worktrees untouched for as long as the settings say removed when nothing in them is uncommitted, their branch with them once merged, the person told), the person's settings, the machine's sync (what changed asked of the person before it is sent, as iq opens, while it runs and as it ends, unless the settings send it on their own and say what went; what other machines sent taken as iq opens and now and then), what iq keeps of its sessions (calls billed, each task's files, commands and costs, the files read in the place the agent works in, why a turn ended without an answer), the person's plugins, the programs `iq-<name>` on the `PATH` their settings list, started as iq opens and ended with it, each told where the chats work and handed the commands the person gives it, what they say taken to the board and the variables they give to the gate, one that stops said with its last errors and iq going on without it; the chats kept in iq's own folder, one file each under `chats/<project>/`, written whole at every turn's end, put back on the board as iq opens, each told when it was last at work, those untouched for as long as the settings say forgotten first, their worktree with them; and, run with one of its flags, the sandbox's helper |

A new crate is added when its first real code lands, not before, and this graph is updated in the same commit.

iq's state is changed only through `iq-ui`'s methods, so that it is tested without a terminal; `iq-tui` draws it, hands it the keys, the pastes and the wheel, and sends what the person asks of the crew as `Request`s. A key does something only through `iq-ui`: its `Keymap` binds the shortcuts, which the `[keys]` of the settings may change; its editor takes the keys that edit what the person writes, Emacs's, as readline and shells take them; a page takes its own keys, which its foot names. The defaults are letters held with Ctrl, which sit at the same place on QWERTY and AZERTY, none a key a terminal keeps for itself (Ctrl-I is Tab there, Ctrl-M Enter) nor one its line editors use. A new `/` command touches `command.rs` and `Board::run`. Neither does I/O: the program reads iq's journal on a thread of its own, carries out the requests, and sends what happened as `Update`s. The conversation is written once into the terminal's scrollback and never drawn again, so that a long session costs nothing to show, the files an agent read written as one line before its next step, so that what it does is not lost among them; the live part is drawn where it stands below it, and the rows above it scroll within a scrolling region that starts at the top of the screen, so that their lines go into the terminal's own scrollback. Nothing else writes to the terminal while iq runs: what is logged goes to `~/.iq/iq.log`, and every command and agent iq starts gets no terminal of its own to read or write.

Only `iq-gate` makes a `Permit`, and whatever acts on the machine takes one, so the compiler refuses a way around the gate. A command runs through a helper, the iq command itself with `--iq-sandbox`, which confines its own process and then becomes the command: `main` checks for that flag before anything else starts. Rules are read from `~/.iq/rules.toml`, never from the workspace, which commands may write. Without the internet, a command on Linux joins a network of the sandbox's own, held by iq's program run with `--iq-network` for as long as the sandbox lives, so that a server one command starts answers the next; a rule gives the internet only to a line read whole whose other commands cannot send anything. A reviewing model may approve what stays within the sandbox, and, when the person lets it, what only reaches the internet and what runs on the machine itself; the rest (publishing, what changes what others share, a file outside the workspace, where code runs later) waits for the person. On macOS the helper becomes `/usr/bin/sandbox-exec` with a profile written for the command, its paths given as parameters; on other systems commands are refused until their sandbox is written. The sandbox's tests run real commands on both systems, so `scripts/check.sh` on a Mac is what tests its profile.

A plugin is a program of its own, apart from iq's binary, which iq starts only when the person's settings list it (`plugins = ["ports"]` starts `iq-ports`, found on the `PATH`) and speaks with as `iq-plugin` writes it. Plugins live in this repository under `plugins/`, each a crate and a binary that depends on `iq-plugin` and on nothing else of iq's, so that `cargo install --path .` builds iq alone and each plugin is installed apart, from the same commit: iq checks the plugin's protocol as it answers. A plugin is the person's program, as `worktree_setup` is: it runs on the machine with their rights and their environment but iq's own variables, answers to the person alone, and no agent can start one or speak to it. Its start, the variables it gives a place, and what it says it runs are written to the audit log; the variables widen nothing, and the gate refuses those its sandbox rests on. What a plugin does that only some projects need (containers, a cluster, a cloud) stays in the plugin: iq keeps the generic part, a place's folder, its variables, a command, a line of status.

`iq-agent` does not depend on `iq-llm`: the crew is generic over `ChatModel`, so that its tests run against scripted models with no network. iq's agents act only through the crew's tools, which take a permit from the gate. What the gate asks goes to the person, or to the reviewer when the autopilot is on, which may answer only what the person let it; a question nobody answers is refused. iq runs no check of its own after an agent's work: the agents run the project's tools themselves, as the lead asks them to.

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
