# The IDE

`ide`, built alongside vcc in `rs/`, serves an IDE on the working directory:

```
cd examples/demo
ide                          # http://127.0.0.1:8080
ide --addr 127.0.0.1:9000
```

![The IDE](images/ide.png)

- **Explorer** (left): every `.sys` file under the directory, grouped by
  project, with the procs in each (the `ƒ` button lists funcs, types and
  interfaces too). Click a proc to open its diagram; click a project for its
  overview: its manifest, entry points, files and counts.
- **Tabs** (centre): a proc each, as a **diagram** or as **text**, compiled
  as you type. A file is compiled together with the rest of its project.
  Diagnostics show in place, with the same messages as `vcc check`.
- **Diagrams** are drawn from the graph: funcs, procs (orange), interface
  funcs (purple), switches with their arm labels, loops, for each regions
  and spawns. A proc's diagram starts with its signature node at the top,
  a pill with the proc's name and parameters; click it for the proc's
  signature and source. Click a proc in a diagram to expand it in place:
  its panel's header stands for its signature, so loops back to the
  signature end under the header. Click any node for its details,
  including its implementations' source, read out of their files. Edit in
  the text tab; the diagram follows.
- **Terminal** (bottom): shells in the project, and Check, Build and Test
  buttons that run vcc on the active file's project. Terminals are offered
  only when the IDE listens on a loopback address.

Drag terminal tabs to reorder them, or double-click a tab to rename it.
Right-click a tab for **Rename**, **Close**, **Close Others**, **Kill**, and
**Split**. Split opens a second shell side by side. Kill keeps the pane and
its output visible after the process exits. Close removes the pane and ends
its shell if it is still running. Reloading the page restores the tabs,
names, splits, and selected pane while the IDE server stays running.

Terminal titles use the working directory unless you give a tab a name.
On Linux the IDE reads the shell's working directory directly. On other
platforms it uses the launch directory and updates it when the shell
reports a directory via OSC 7.

Settings (the gear in the status bar) pick the theme, where the terminal and
details panels sit, and the compiler the buttons run: `vcc` from the `PATH`,
or a path to one.

| Shortcut | |
|---|---|
| Ctrl/⌘ + S | Save the file |
| Ctrl + \` | Show or hide the terminal |
| Ctrl + Shift + \` | New terminal |
| Ctrl + Shift + ← / → | Previous / next terminal tab |
| Ctrl + Shift + Backspace | Close the selected terminal |
| Click a proc | Expand it in place (in a diagram); open its diagram (in the explorer) |
| Shift + click | Select a proc in a diagram without expanding it |
| Ctrl/⌘ + click | Open a proc in a new tab |
| Middle-click a tab | Close it |

Other pages:

- `/viewer?path=sys/calc.sys&proc=Calc`: one proc's diagram and text, as in
  a tab.
- `/workbench`: an editor beside the generated Go and Rust, for trying
  things out.
- `/viz`: the diagram on its own, for a `.sys` file given by path.

Writes and terminals need the per-run token the pages are served with, so
other sites can't drive them. On a loopback address the IDE also requires a
loopback `Host`, against DNS rebinding.
