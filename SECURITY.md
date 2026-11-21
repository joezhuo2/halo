# Security Policy

## Supported versions

Only the latest release of Halo receives security fixes.

## Reporting a vulnerability

Please do not open a public issue for a security problem.

Report it privately through GitHub:
[open a private security advisory](https://github.com/joezhuo2/halo/security/advisories/new).

Include:

- the Halo version (**Help > About Halo**) and your operating system,
- the steps to reproduce it, or a proof of concept,
- what an attacker could do with it.

You can expect a first answer within seven days. Once a fix is released, the
advisory is published and you are credited unless you ask not to be.

## Scope

Halo runs programs on your behalf: shells in the terminal, `git` and `gh`,
the Claude and Codex CLIs, run commands and tasks, and the language servers
listed in Settings. Issues in those programs belong to their own projects.
In scope are problems in Halo itself, for example running a program the user
did not configure, or a file in an opened folder making Halo run code without
the user asking.
