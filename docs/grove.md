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

## Ideas

An idea is a local branch `idea/<id>` with its own worktree. The root of the
tree is the repository's main worktree; every other idea is branched from
another, at that idea's head, and its base (`refs/worktree/base`, see #16)
is set to the commit it was branched at.

- **Branch** from any idea: the branch button in the sidebar, Alt+Shift+N,
  or right-click a node. The new idea gets its parent's sections, without
  their terminals.
- **Discard** removes the idea's worktree, stops its terminals and the
  interiors started for it, and keeps its branch, so it can be **restored**.
  git refuses to remove a worktree with changes that aren't committed.
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
outside grove shows as discarded, and the page catches up every few
seconds.

## The window

- **The tree** (left): every idea as a small circle, with labels on leaves
  only; hover any node for its name, branch and head. Click a node to show
  it; the chevron folds its subtree. The trash button shows discarded ideas.
  The sidebar can be resized, and hidden (Alt+Shift+B).
- **Sections** (right): the shown idea's interiors, stacked, each with a
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
  itself.
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
  looked at; `run` {cmd}, typed into one of the idea's terminals (opening
  one if there's none); `keys` {code, alt, shift, ctrl, meta} for the
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
grove help
```

Each command runs in the idea whose worktree it's in (the repository's own
is the root), unless `--from` (branch) or `--idea` (rename) names another.
It makes the change itself, through git and grove's metadata, so it works
with no grove running. Then it tells grove, through `$GROVE_URL` and
`$GROVE_TOKEN`. grove sets both in its terminals and in the interiors it
starts. The page catches up at once and says which ideas are new.

- **branch** makes a new idea from this one's head. grove opens a terminal
  in it, kept as one of its sections, and types `--run`'s command there.
  An agent's session can't follow it into another worktree, so the usual
  move is to start a fresh agent there: `--run 'claude "<prompt>"'`. Pass
  `--no-terminal` for just the idea.
- **Discarding is left to people**, in the tree. `grove discard` refuses,
  and `$GROVE_TOKEN` is a token of its own that only the agent routes take,
  not grove's other writes. An agent can't discard the idea its own
  terminal lives in.

The routes behind it:

- `POST /api/agent/notify {idea, open, run}`, with the agents' token: counts
  a change, and with `open` opens a terminal in the idea, typing `run`.
- `GET /api/ideas/wait?v=`: the count of changes, as soon as it isn't `v`
  (up to 25 s). The page long-polls it.

To have Claude use these on its own, tell it in a `CLAUDE.md`, for example:
"If `$GROVE_URL` is set, you're in a grove idea: see `grove help`, and
branch with `grove branch <name> --run 'claude \"<prompt>\"'` to try an
alternative in parallel."

## Not yet

Compare, merge, ship and restack (#26); terminals that survive a restart,
resume rules for agent CLIs, and idle stops (#27); the MCP server and caps
on autonomous branching (#28, of which the commands above are a first
part); and the inbox (#29). Terminals die with grove for now.

Writes need the per-run token the page is served with. Terminals and
interiors, which run commands, are offered only when grove listens on a
loopback address.
