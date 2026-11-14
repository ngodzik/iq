# Changelog

Versions follow [semantic versioning](https://semver.org). While the version is 0.x iq is an experiment: anything may change or disappear from one version to the next, commands, keys, settings and file formats included, without a way back.

## 0.1.0 (2026-11-14)

The first version: a harness for coding agents in the terminal, on your machine, its models answering through Requesty's router. It is experimental, its security has not been audited, it is barely tested in real use, and whatever it does may change or be removed in the next version.

- A lead agent takes your request and may start other agents, with the roles, models and efforts it chooses, in a few patterns (alone, a lead and those it starts, a writer and its reviewer, a debate); they write to each other and to you on a shared board. `iq do` hands a task over at once.
- Everything an agent does goes through one gate, which allows, asks or refuses; commands run in the operating system's sandbox (Landlock and seccomp on Linux, Seatbelt on macOS), and an autopilot may answer for you within the policy you write for it.
- Every cost in view: each call, each turn as it runs, by model, by kind of work and over time, with a limit a turn may reach before you are asked whether it goes on.
- Chats kept between sessions, each in a git worktree of its own when you want it; your rules and skills given to the agents; the project's notes; MCP servers and plugins of your own; iq's folder synced through a repository of yours.
- The conversation drawn by iq itself: steps folded, diagrams drawn, links opened, what agents change shown as diffs, text selected and copied.

The binaries are built for Linux (x86_64, glibc 2.35 or later) and macOS (Apple Silicon), each with its SHA-256 checksum. On macOS the binary is not signed: `xattr -d com.apple.quarantine iq` lets it open. On recent Ubuntu, the sandbox's network needs unprivileged user namespaces, which AppArmor may keep from programs by default.
