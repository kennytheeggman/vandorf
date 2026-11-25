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

## Ideas

An idea is a local branch `idea/<id>` with its own worktree. The root of the
tree is the repository's main worktree; every other idea is branched from
another, at that idea's head, and its base (`refs/worktree/base`, see #16)
is set to the commit it was branched at.

- **Branch** from any idea: the branch button in the sidebar, Alt+Shift+N,
  or right-click a node. The new idea gets its parent's sections, without
  their terminals.
- **Discard** removes the idea's worktree, and those of every idea under
  it, stops their terminals and the interiors started for them, and keeps
  their branches, so each can be **restored**. If any of them has changes
  that aren't committed, nothing is discarded.
- **Delete permanently** is in a discarded node's right-click menu in the
  history view. It removes the branch, remaining worktree and sessions, and
  idea metadata. Delete any child ideas first. Git protects uncommitted
  changes in a remaining worktree.
- **Merge** an idea into the root (see [Merging](#merging)).
- **Rename** any idea, the root too.

Everything is private to the machine. grove keeps its metadata in the
common git directory (`git rev-parse --git-common-dir`), under `grove/`:

```
grove/ideas/<id>.toml     an idea: name, parent, fork point, sections
grove/root.toml           the root's name and sections
grove/worktrees/<id>/     ideas' worktrees
grove/interiors.toml      the repository's interiors (optional)
grove/lock                held while a file is written
```

The tree is rebuilt from git each time it's read: a branch `idea/<x>` made
by hand is an idea hanging from the root, an idea whose worktree was removed
outside grove shows as discarded, one whose own commits are all in the
root's branch (merged by hand) shows as merged, and the page catches up
every few seconds.

### Merging

An idea is **based** on its parent when it's built on the parent's latest
commit (its fork is the parent's head). Merging takes an idea that's on the
root's head, so it's a fast-forward and can't conflict:

1. The root's branch fast-forwards to the idea. The root's worktree is the
   repository's own checkout, so its files change too; git refuses if that
   would overwrite changes there.
2. The idea is **merged**: its worktree closes, its terminals stop, its
   branch stays. It leaves the map (the history button shows it), and the
   root's tooltip lists it.
3. Its children become the root's.
4. Every other idea is rebased onto its parent, parents first, so the whole
   tree is on the new root. An idea that can't be (changes not committed,
   or a conflict) is left exactly as it was, its rebase undone, and flagged
   for attention with why. An idea whose commits are all in its parent
   already (merged some other way, such as with git by hand) is left alone
   too: it's merged, and replaying its commits would only copy them. Its
   children go to its parent, as if grove had merged it. An idea merged
   only partly that way replays just the commits its parent hasn't.

An idea not on the root's head can't merge until it's rebased: right-click
it for **Rebase onto <parent>**, which undoes itself on a conflict. An
agent in the idea can do it with `grove rebase`, which stops on a conflict
for it to resolve, then `grove rebase --continue`. A rebase only moves the
idea's own branch, so it's safe for agents; merging and discarding are
people's.

## The window

- **The map** (left): the root, the repository's checked-out branch, in the
  middle, and each idea on a ring by its depth, with edges leading out from
  the root. There are no labels: hover any node for its name, status,
  branch and head (the root's says **HEAD**, and which ideas were merged
  into it). Click an idea to show it: the map turns so its history, from
  the root, runs straight up. Drag or scroll to pan, pinch or Ctrl+scroll to
  zoom, and double-click (or the target button) to see the whole tree
  again. Right-click a node to branch, rename, rebase, merge, discard or
  restore it. The history button shows merged and discarded ideas. The
  sidebar can be resized, and hidden (Alt+Shift+B). Merge, discard, and
  permanent deletion ask for confirmation. Enter confirms and Escape cancels.

  A node shows what the agent in its idea last said (`grove status`): a
  spinner while it's **working**, amber with a pulse when it needs
  **attention** (also when a rebase stopped or couldn't be made), green
  when it's **done**, and a quieter blue once someone views the idea
  after either report. That acknowledgment applies across windows and
  reloads until the agent reports again. A failed rebase remains amber
  until rebased successfully or retried interactively. A stopped rebase
  stays amber until it finishes or is aborted. The root, the
  checked-out branch, is the larger blue node with a ring. A dashed edge
  means the idea isn't on its parent's latest commit.
- **Sections** (right): under a slim bar with the shown idea's name (the
  root's for the root), the shown idea's interiors, stacked, each with a
  header like VS Code's sidebar views. Click a header to collapse or expand
  it (several can be open at once, sharing the height), drag headers to
  reorder them, and close one with its ×. To open another, collapse every
  section: a line under them lists the interiors to pick from.

Each idea keeps its own sections, their order and which are expanded.
Switching keeps the last four ideas' sections alive, hidden, so switching
back is instant.

| Shortcut | |
|---|---|
| Alt+Shift+N | Branch a new idea from the shown one |
| Alt+Shift+↑ / ↓ | Show the previous or next idea in the tree |
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
grove list                          # the ideas as a tree; * marks this one
grove current                       # this idea: id, name, branch, worktree
grove branch "try a cache" --run 'claude "make lookups cached"'
grove rename "cache, with eviction"
grove restore <idea>
grove run -- make test              # in this idea's terminal at its prompt
grove run --idea <idea> --new "npm run dev"
grove status working "running the tests"   # shown on this idea's node
grove rebase                        # onto its parent's latest commit
grove help
```

Each command runs in the idea whose worktree it's in (the repository's own
is the root), unless `--from` (branch) or `--idea` (rename, run) names
another. It makes the change itself, through git and grove's metadata, so
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
- **Discarding and merging are left to people**, in the tree. `grove
  discard` and `grove merge` refuse, and `$GROVE_TOKEN` is a token of its
  own that only the agent routes take, not grove's other writes. An agent
  can't discard the idea its own terminal lives in.

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

Compare, merging into an idea other than the root, ship, and restacking
onto a tracked branch (#26); restoring terminals after a restart, resume
rules for agent CLIs, and idle stops (#27); terminals that keep running
across a restart (#31); and the inbox (#29). Terminals die with grove for
now. There's no MCP server: agents work the tree with the commands above.

Writes need the per-run token the page is served with. Terminals and
interiors, which run commands, are offered only when grove listens on a
loopback address.
