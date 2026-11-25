# grove

`grove`, built alongside vcc in `rs/`, is the idea tree (#17): a way to
explore several ideas at once in one git repository, each with its own
branch, worktree and terminals, and switch between them in one window. It
works on any git repository with a working directory, not just vandorf
projects.

```
cd some/repo
grove                        # http://127.0.0.1:8090
grove --addr 127.0.0.1:9000
```

> **One-time setup for agents' status.** The working / needs-attention /
> done marks on the map come from agents calling `grove status`. grove
> can't make an agent do that itself: for Claude Code, add the hooks in
> [Status from agents' hooks](#status-from-agents-hooks) to
> `~/.claude/settings.json` (merge them into any `hooks` you have), then
> restart Claude sessions (or open `/hooks`) so they load. Without them
> everything else works; nodes just show no status. The hooks do nothing
> outside grove, since they check `$GROVE_URL` first.

## Pins

The middle of the map is the repository itself: a hub that pins lead out
from. It isn't a git source, so nothing is branched from it or merged into
it. Around it, on the first ring, are the **pins**, the git sources ideas
are branched from:

- A **branch** pin follows its branch. Ideas on it are merged into it.
- A **tag** or **commit** pin stays where it is. Ideas on it can be
  rebased onto it, but there's no branch to merge them into.

A pin's id is its kind and what it pins: `branch@main`, `tag@v1.0`,
`commit@<sha>`. It keeps its own name and sections, as an idea does, and
has a worktree for its interiors and terminals: the one its branch is
checked out in (the repository's own, for the checked-out branch), or else
one of grove's in `grove/worktrees/`, made when it's pinned or first
needed, detached for a tag or a commit.

- **Pin** from the pin button in the sidebar, by right-clicking the hub,
  or with `grove pin <branch|tag|commit>`. A name is a branch if there's
  one, else a tag, else whatever commit git finds by it (`HEAD~2`,
  `origin/main`, a sha); `refs/heads/<b>` and `refs/tags/<t>` say which.
  An idea's own branch (`idea/...`) can't be pinned.
- **Unpin** by right-clicking a pin, or with `grove unpin <pin>`. It
  discards the ideas on the pin, as discarding does (below), so the tree
  asks first, listing them, and nothing is unpinned if any of them has
  changes that aren't committed (or grove's worktree for the pin has).
  grove's worktree for the pin goes; the repository's own never does. The
  branch, tag or commit itself stays in git, and the ideas keep their
  branches: they hang from the pin they fork from (see below) until it's
  pinned again. `grove unpin` refuses a pin with active ideas: discarding
  is left to people.

The page pins and unpins with `POST /api/pins/pin {ref}` and
`POST /api/pins/unpin {id}`, with the session's token.

The checked-out branch is pinned to begin with. A repository grove used
before pins is moved over the first time it's read: the checked-out branch
is pinned with the old root's name and sections (from `grove/root.toml`,
which is left as it was), and the ideas that hung from the root hang from
that pin, so the map looks the same, one ring out.

## Ideas

An idea is a local branch `idea/<id>` with its own worktree. Each is
branched from a pin or from another idea, at its head, and its base
(`refs/worktree/base`, see #16) is set to the commit it was branched at.
The pin at the top of an idea's line is **its pin**.

- **Branch** from any pin or idea: the branch button in the sidebar,
  Alt+Shift+N, or right-click a node. The new idea gets its parent's
  sections, without their terminals.
- **Discard** removes the idea's worktree, and those of every idea under
  it, stops their terminals and the interiors started for them, and keeps
  their branches, so each can be **restored**. If any of them has changes
  that aren't committed, nothing is discarded.
- **Delete permanently** is in a discarded or merged node's right-click
  menu in the history view. It removes the branch, remaining worktree and sessions, and
  idea metadata. Delete any child ideas first. Git protects uncommitted
  changes in a remaining worktree.
- **Merge** an idea into its pin (see [Merging](#merging)).
- **Rename** any pin or idea.

Everything is private to the machine. grove keeps its metadata in the
common git directory (`git rev-parse --git-common-dir`), under `grove/`:

```
grove/pins/<id>.toml      a pin: kind, ref, name, sections (a / in the id
                          is %2F in the file's name)
grove/ideas/<id>.toml     an idea: name, parent, fork point, sections
grove/worktrees/<id>/     ideas' worktrees, and pins' that grove made
grove/interiors.toml      the repository's interiors (optional)
grove/lock                held while a file is written
grove/root.toml           the root's name and sections, from before pins
```

The tree is rebuilt from git each time it's read: a branch `idea/<x>` made
by hand (or an idea whose parent is gone) hangs from the pin it forks from,
the one whose head has the most of its commits, or the checked-out
branch's pin when that's unclear; an idea whose worktree was removed
outside grove shows as discarded; one whose own commits are all in its pin
(merged by hand) shows as merged; and the page catches up every few
seconds.

### Merging

An idea is **based** on its parent (a pin or an idea) when it's built on
the parent's latest commit (its fork is the parent's head). Merging takes
an idea that's on its pin's head, so it's a fast-forward and can't
conflict:

1. The pin's branch fast-forwards to the idea, in the pin's worktree, so
   its files change too; git refuses if that would overwrite changes
   there. A tag or a commit pin has no branch, so its ideas can't be
   merged.
2. The idea is **merged**: its worktree closes, its terminals stop, its
   branch stays. It leaves the map (the history button shows it), and the
   pin's tooltip lists it.
3. Its children become the pin's.
4. Every other idea on the pin is rebased onto its parent, parents first,
   so the pin's whole tree is on its new head. An idea that can't be
   (changes not committed, or a conflict) is left exactly as it was, its
   rebase undone, and flagged for attention with why. An idea whose commits
   are all in its parent already (merged some other way, such as with git
   by hand) is left alone too: it's merged, and replaying its commits would
   only copy them. Its children go to its parent, as if grove had merged
   it. An idea merged only partly that way replays just the commits its
   parent hasn't.

An idea not on its pin's head can't merge until it's rebased: right-click
it for **Rebase onto <parent>**, which undoes itself on a conflict. An
agent in the idea can do it with `grove rebase`, which stops on a conflict
for it to resolve, then `grove rebase --continue`. A rebase only moves the
idea's own branch, so it's safe for agents; merging and discarding are
people's.

## The window

- **The map** (left): the repository in the middle, a small dot in the
  theme's ink, its pins on the first ring, and each idea on a ring by its
  depth, with edges leading out from its pin. There are no labels: hover
  any node for its name, status, branch and head (the checked-out branch's
  pin says **HEAD**; a pin's tooltip says which ideas were merged into it).
  Click a pin or an idea to show it: the map turns so its history, from its
  pin, runs straight up. Clicking the hub, double-clicking the map, or the
  target button sees the whole map again. Drag or scroll to pan, pinch or
  Ctrl+scroll to zoom. Right-click the hub to pin, a pin to branch from,
  rename or unpin it, and an idea to branch, rename, rebase, merge, discard
  or restore it. The history button shows merged and discarded ideas. The
  sidebar can be resized, and hidden (Alt+Shift+B). Merge, discard, unpin
  and permanent deletion ask for confirmation. Enter confirms and Escape
  cancels.

  A node shows what the agent in its idea last said (`grove status`): a
  spinner while it's **working**, amber with a pulse when it needs
  **attention** (also when a rebase stopped or couldn't be made), green
  when it's **done**, and a quieter blue once someone views the idea
  after either report. That acknowledgment applies across windows and
  reloads until the agent reports again. A failed rebase remains amber
  until rebased successfully or retried interactively. A stopped rebase
  stays amber until it finishes or is aborted. A pin is a larger blue node
  with a ring, dashed for a tag or a commit. A dashed edge means the idea
  isn't on its parent's latest commit.
- **Sections** (right): under a slim bar with the shown pin's or idea's
  name, its interiors, stacked, each with a
  header like VS Code's sidebar views. Click a header to collapse or expand
  it (several can be open at once, sharing the height), drag headers to
  reorder them, and close one with its ×. To open another, collapse every
  section: a line under them lists the interiors to pick from.

Each pin and idea keeps its own sections, their order and which are
expanded. Switching keeps the last four ones' sections alive, hidden, so
switching back is instant.

| Shortcut | |
|---|---|
| Alt+Shift+N | Branch a new idea from the shown pin or idea |
| Alt+Shift+↑ / ↓ | Show the previous or next pin or idea in the tree |
| Alt+Shift+B | Show or hide the tree |

## Interiors

An interior renders one worktree in an iframe. Two are built in:

- **terminal**: a shell on a pty in the idea's worktree, served by grove
  itself, with grove's directory first on the `PATH` (see
  [Finding grove](#finding-grove)).
- **ide**: vandorf's IDE on the idea's worktree, one process per idea
  (`ide --addr 127.0.0.1:{port}`). Interiors are started with grove's own
  directory first on the `PATH`, so this finds the `ide` built next to
  `grove`.

The others come from manifests, the user's (`$XDG_CONFIG_HOME/grove/interiors.toml`,
else `~/.config/grove/interiors.toml`) and then the repository's
(`grove/interiors.toml` in the common git directory), which wins for a name
both have. A manifest can replace the `ide` entry, not the terminal's:

```toml
[interior.vscode]
mode  = "per-worktree"
start = "code-server --auth none --bind-addr 127.0.0.1:{port} {worktree}"

[interior.notebook]
mode  = "shared"               # one process for every idea
start = "jupyter lab --no-browser --port {port}"
open  = "{url}/lab/tree/{idea}"
```

- `mode`: `per-worktree` (the default) starts a process for each idea, in
  its worktree; `shared` starts one, in the main worktree, for every idea.
- `start` runs with the shell. With `{port}` in it, grove picks a free port
  and waits for the process to listen on it; without, it waits for the
  process to print an `http://` or `https://` URL. Either way it waits up to
  30 seconds.
- `open`, by default `{url}`, is where each section points.
- Placeholders: `{port}`, `{worktree}`, `{idea}`, `{base}` (the commit the
  idea was branched at) and `{repo}` (the main worktree), and `{url}` in
  `open`. Values are shell-quoted in `start` and percent-encoded in `open`.

An interior must allow framing by grove's origin (no `X-Frame-Options:
deny` or `frame-ancestors` that leaves it out).

### Messages

Interiors and grove may talk by `postMessage`, all optional, each message
carrying `grove: 1`:

- **grove → interior:** `context` {idea, worktree, base, theme} when it
  loads, `visible` {visible} as its section is expanded or collapsed, and
  `focus`.
- **interior → grove:** `ready`; `title` {title} for its header;
  `attention` {message}, a dot on its header and its idea's node until it's
  looked at; `run` {cmd}, typed into one of the idea's terminals as
  `grove run` does (see below); `keys` {code, alt, shift, ctrl, meta} for the
  shortcuts it didn't use.

## Commands, for agents

`grove` with a command works on the repository from the command line, so
an agent (Claude, say) in one of grove's terminals can grow the tree itself:

```
grove list                          # the pins and ideas as a tree; * marks this one
grove current                       # this pin or idea: id, name, branch, worktree, pin
grove branch "try a cache" --run 'claude "make lookups cached"'
grove branch "on dev" --from dev    # from dev's pin
grove rename "cache, with eviction"
grove restore <idea>
grove pin release/2.0               # a branch, a tag or a commit
grove unpin release/2.0
grove run -- make test              # in this idea's terminal at its prompt
grove run --idea <idea> --new "npm run dev"
grove status working "running the tests"   # shown on this idea's node
grove rebase                        # onto its parent's latest commit
grove help
```

Each command runs in the pin or idea whose worktree it's in (the
repository's own is its checked-out branch's pin), unless `--from` (branch)
or `--idea` (rename, run, status, rebase) names another: an idea by its id,
a pin by its id or by what it pins (`main`, `v1.0`, a commit's first
digits). It makes the change itself, through git and grove's metadata, so
it works with no grove running; all but `run`, which types into grove's
terminals and fails without `$GROVE_URL`. Then it tells grove, through `$GROVE_URL` and
`$GROVE_TOKEN`. grove sets both in its terminals and in the interiors it
starts. The page catches up at once and says which ideas are new.

- **branch** makes a new idea from this one's head. grove opens a terminal
  in it, kept as one of its sections, and types `--run`'s command there.
  An agent's session can't follow it into another worktree, so the usual
  move is to start a fresh agent there: `--run 'claude "<prompt>"'`. Pass
  `--no-terminal` for just the idea.
- **run** types a command, and a return, into one of the idea's terminals:
  its first terminal section whose shell is at its prompt, else a new
  terminal, kept as one of its sections (the page shows it at once).
  `--new` always opens one. One argument is a command line for the shell
  (`grove run 'make test && echo ok'`); several, or the rest after `--`,
  are quoted as words. Flags go before the command: from its first word
  on, everything is the command's.
  - **It never types into a program.** A terminal whose foreground isn't
    its shell (an agent, an editor, a server, or the terminal `grove run`
    itself runs in) is passed over, since what's typed would go to that
    program: to an agent, as its prompt. There's no way to make it type
    into one.
  - **Only the idea's own terminals**: one counts only if grove opened it,
    since it started, in the idea's worktree, so a section's stale id (from
    before grove restarted) never reaches another idea's terminal.
  - The page's `run` messages, from interiors, pick a terminal the same
    way.
- **status** `working`, `attention` or `done`, with an optional message,
  tells grove what the agent in this idea is doing; `clear` forgets it. The
  idea's node shows it (see [The window](#the-window)) and its tooltip
  shows the message. It's quiet when grove hears it, so it can run from an
  agent's hooks, and like `run` it needs grove running. grove keeps
  statuses until it stops.
- **rebase** moves this idea onto its parent's latest commit, replaying
  its own commits there. On a conflict it stops for the agent (or you) to
  resolve it in the worktree, `git add` the files, and run
  `grove rebase --continue`. Afterwards the rebase warning clears.
- **pin** and **unpin** pin a branch, a tag or a commit, and unpin one
  (see [Pins](#pins)).
- **Discarding and merging are left to people**, in the tree. `grove
  discard` and `grove merge` refuse, as does `grove unpin` for a pin with
  active ideas, and `$GROVE_TOKEN` is a token of its own that only the
  agent routes take, not grove's other writes. An agent can't discard the
  idea its own terminal lives in.

### Status from agents' hooks

grove has nothing specific to any agent: `grove status` is a plain command.
An agent tool with hooks can call it at the right moments, guarded by
`$GROVE_URL` so it does nothing outside grove. For Claude Code, this has
to be added to its settings by hand, once: in `~/.claude/settings.json`
(for every repository), merged into any `hooks` already there. Sessions
started before the change don't have it until they're restarted or `/hooks`
is opened.

```json
{
  "hooks": {
    "UserPromptSubmit": [{ "hooks": [{ "type": "command", "command": "[ -z \"$GROVE_URL\" ] || \"$GROVE_BIN\" status working || true" }] }],
    "PostToolUse": [{ "matcher": "*", "hooks": [{ "type": "command", "command": "[ -z \"$GROVE_URL\" ] || \"$GROVE_BIN\" status working || true" }] }],
    "Notification": [{ "hooks": [{ "type": "command", "command": "[ -z \"$GROVE_URL\" ] || \"$GROVE_BIN\" status attention \"Claude is waiting for you\" || true" }] }],
    "Stop": [{ "hooks": [{ "type": "command", "command": "[ -z \"$GROVE_URL\" ] || \"$GROVE_BIN\" status done || true" }] }]
  }
}
```

Working when a turn starts and after each tool (so it's working again once
you've answered a permission prompt), attention when it waits for you, done
when it stops. `$GROVE_BIN` rather than `grove`, since an agent's tool
shells may reset `PATH`. Any other tool's hooks can do the same.

### Finding grove

grove's terminals start with grove's own directory first on the `PATH`, so
`grove` and `ide` are found there. Startup files that set `PATH` outright
(`export PATH=/usr/bin:/bin:...` in a `.zshrc`) would drop it, so for zsh
and bash grove puts it back once the user's startup files have run:

- **zsh** starts with `ZDOTDIR` pointing at a `.zshenv` of grove's (in a
  directory of its own under the temporary directory, made once per run).
  It restores the user's `ZDOTDIR` (or unsets it) at once, sources the
  user's `.zshenv`, and adds a `precmd` hook; zsh then reads the user's
  own `.zprofile`, `.zshrc` and `.zlogin` as usual. The hook runs once,
  before the first prompt, putting grove's directory first and removing
  itself, so a `PATH` changed later is left alone.
- **bash** starts with `--rcfile` set to a file of grove's that sources
  `~/.bashrc` and then puts grove's directory first.
- **Other shells** get the `PATH` as grove sets it, which their startup
  files may change.

The user's files are never changed. Whatever the shell, grove's terminals
and interiors also carry `$GROVE_BIN`, grove's executable as an absolute
path. Scripts and agents that may run where `PATH` has been reset again
(a subshell that reads the user's startup files, say) can run
`"$GROVE_BIN" branch ...`.

The routes behind it:

- `POST /api/agent/notify {idea, open, run}`, with the agents' token: counts
  a change, and with `open` opens a terminal in the idea, typing `run`.
- `POST /api/agent/run {idea, cmd, new}`, with the agents' token: types
  `cmd` into a terminal of the idea as above, answering `{idea, opened}`.
- `GET /api/ideas/wait?v=`: the count of changes, as soon as it isn't `v`
  (up to 25 s). The page long-polls it.

To have Claude use these on its own, tell it in a `CLAUDE.md`, for example:
"If `$GROVE_URL` is set, you're in a grove idea: see `"$GROVE_BIN" help`,
and branch with `"$GROVE_BIN" branch <name> --run 'claude \"<prompt>\"'` to
try an alternative in parallel."

## Not yet

Compare, merging into an idea rather than its pin, ship, and restacking
onto a tracked branch (#26); restoring terminals after a restart, resume
rules for agent CLIs, and idle stops (#27); terminals that keep running
across a restart (#31); and the inbox (#29). Terminals die with grove for
now. There's no MCP server: agents work the tree with the commands above.

Writes need the per-run token the page is served with. Terminals and
interiors, which run commands, are offered only when grove listens on a
loopback address.
