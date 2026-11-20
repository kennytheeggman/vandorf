package exec

// Translation of ast.Proc into Go.
//
// Model
//
// A proc is a graph whose sequential edges are tail calls, so it's compiled to
// a state machine instead of to nested Go calls: every reachable node gets a
// program counter, an input slot and an output slot in a per-proc env struct,
// and a tail call is "write the target's input slot, set pc, continue". This
// keeps `until`-style back edges (Loop -> Loop) from growing the Go stack.
//
// A `for each` edge opens a region: the subgraph reachable from the foreach
// target up to (and excluding the target of) a single `collect` edge. Each
// region compiles to its own state machine function, run once per element on
// a copy of the env (fan out), and the parent machine continues at the collect
// target with the ordered results bound to `this` (fan in).
//
// Leaf funcs are not translated; they become interface methods the user
// implements. Funcs declared in a top-level interface are effects and get one
// Go interface per vandorf interface. Procs called as nodes are invoked
// through their generated Run function. All of these are threaded through a
// per-proc Deps struct.
//
// Types are structural in vandorf, so they map onto anonymous Go types:
// structs onto structs with exported fields, enums onto a tagged struct with
// one payload field per variant. Where vandorf coerces (an enum into a wider
// enum), a generated coerceN helper does the conversion.

import (
	"bytes"
	"fmt"
	"go/format"
	"go/parser"
	"slices"
	"sort"
	"strconv"
	"strings"
	"unicode"
	"unicode/utf8"

	"github.com/kennytheeggman/vandorf/lib/compiler/ast"
)

const standalonePackage = "generated"

// TranslateProc emits one self-contained Go file for proc: the proc, its
// nested procs, any external procs it calls, the effect interfaces it uses
// and the runtime helpers. Without a program to consult, effect funcs are
// collected into a single Extern interface.
func TranslateProc(proc *ast.Proc) (src string, err error) {
	defer catch(&err)
	t := newTranslator(nil)
	t.registerProc(proc, exported(proc.Name()))
	var body writer
	for i := 0; i < len(t.queue); i++ {
		t.emitProc(&body, t.queue[i])
	}
	var w writer
	w.header(standalonePackage, "context", "fmt", "sync")
	t.emitEffects(&w)
	w.Write(body.Bytes())
	t.emitRuntime(&w)
	return w.format(proc.Name())
}

// TranslateProgram emits a package: effects.go for interfaces, one file per
// top-level proc, and runtime.go for shared helpers.
func TranslateProgram(prog *ast.Program, pkg string) (project Project, err error) {
	defer catch(&err)
	t := newTranslator(prog)
	project = make(Project)
	for _, name := range sortedKeys(prog.Namespace) {
		p, ok := prog.Namespace[name].(*ast.Proc)
		if !ok {
			continue
		}
		var w writer
		w.header(pkg, "context", "fmt")
		t.emitProc(&w, p)
		file := strings.ToLower(name) + ".go"
		project[file], err = w.format(file)
		if err != nil {
			return nil, err
		}
	}
	if len(t.ifaces) > 0 {
		var w writer
		w.header(pkg, "context")
		t.emitEffects(&w)
		if project["effects.go"], err = w.format("effects.go"); err != nil {
			return nil, err
		}
	}
	var w writer
	w.header(pkg, "context", "sync")
	t.emitRuntime(&w)
	if project["runtime.go"], err = w.format("runtime.go"); err != nil {
		return nil, err
	}
	return project, nil
}

/** Errors **/

// Translation is a deep recursive walk; failures panic with translateError
// and are recovered at the API boundary.
type translateError struct{ err error }

func failf(format string, args ...any) {
	panic(translateError{fmt.Errorf(format, args...)})
}

func catch(err *error) {
	if r := recover(); r != nil {
		te, ok := r.(translateError)
		if !ok {
			panic(r)
		}
		*err = te.err
	}
}

/** Symbols **/

type translator struct {
	prog      *ast.Program
	procName  map[*ast.Proc]string   // Go name prefix
	queue     []*ast.Proc            // top-level procs awaiting emission
	owner     map[*ast.Func]string   // Go name prefix of the owning proc or interface
	effectOf  map[*ast.Func]string   // source name of the declaring interface
	ifaces    map[string][]*ast.Func // Go interface name -> effect funcs
	coercions map[string]string      // from/to type pair -> helper name
	helpers   []string
}

func newTranslator(prog *ast.Program) *translator {
	t := &translator{
		prog:      prog,
		procName:  make(map[*ast.Proc]string),
		owner:     make(map[*ast.Func]string),
		effectOf:  make(map[*ast.Func]string),
		ifaces:    make(map[string][]*ast.Func),
		coercions: make(map[string]string),
	}
	if prog == nil {
		return t
	}
	for _, name := range sortedKeys(prog.Namespace) {
		switch n := prog.Namespace[name].(type) {
		case *ast.Interface:
			goName := exported(name)
			t.ifaces[goName] = []*ast.Func{}
			for _, fname := range sortedKeys(n.Namespace) {
				if f, ok := n.Namespace[fname].(*ast.Func); ok {
					t.addEffect(f, name, goName)
				}
			}
		case *ast.Proc:
			t.registerProc(n, exported(name))
		case *ast.Func:
			// A top-level func is called from many procs; treat it as an effect
			// of an implicit interface so it's implemented once.
			t.addEffect(n, "", "Extern")
		}
	}
	return t
}

func (t *translator) addEffect(f *ast.Func, iface, goName string) {
	t.owner[f] = goName
	t.effectOf[f] = iface
	t.ifaces[goName] = append(t.ifaces[goName], f)
}

func (t *translator) registerProc(p *ast.Proc, goName string) {
	t.procName[p] = goName
	t.queue = append(t.queue, p)
	t.registerNested(p)
}

func (t *translator) registerNested(p *ast.Proc) {
	for _, name := range sortedKeys(p.Namespace) {
		switch n := p.Namespace[name].(type) {
		case *ast.Func:
			t.owner[n] = t.procName[p]
		case *ast.Proc:
			t.procName[n] = t.procName[p] + exported(name)
			t.registerNested(n)
		}
	}
}

func (t *translator) ownerOf(f *ast.Func) string {
	if o, ok := t.owner[f]; ok {
		return o
	}
	t.addEffect(f, "", "Extern")
	return "Extern"
}

func (t *translator) nameOf(p *ast.Proc) string {
	if n, ok := t.procName[p]; ok {
		return n
	}
	t.registerProc(p, exported(p.Name()))
	return t.procName[p]
}

func (t *translator) isEffect(n ast.Node) bool {
	f, ok := n.(*ast.Func)
	if !ok {
		return false
	}
	t.ownerOf(f)
	_, ok = t.effectOf[f]
	return ok
}

// inType names the input struct of a node. A proc takes its entry's input.
func (t *translator) inType(n ast.Node) string {
	switch n := n.(type) {
	case *ast.Func:
		return t.ownerOf(n) + exported(n.Name()) + "In"
	case *ast.Proc:
		return t.inType(n.Entry)
	}
	failf("unknown node kind %T", n)
	return ""
}

/** Planning **/

// frame is one state machine: the root of a proc, or a for each region.
type frame struct {
	index  int
	parent *frame
	entry  ast.Node
	elem   ast.Type             // type of `this` on entry to a region
	args   map[string]ast.Ident // foreach call args, evaluated per element
	nodes  []ast.Node
	sink   ast.Node // region node whose collect edge closes the region
}

type procPlan struct {
	proc    *ast.Proc
	name    string
	frames  []*frame
	frameOf map[ast.Node]*frame
	region  map[ast.Node]*frame // foreach source -> region it opens
	ids     map[ast.Node]string
	order   []ast.Node
}

func (t *translator) plan(p *ast.Proc) *procPlan {
	pl := &procPlan{
		proc:    p,
		name:    t.nameOf(p),
		frameOf: make(map[ast.Node]*frame),
		region:  make(map[ast.Node]*frame),
		ids:     make(map[ast.Node]string),
	}
	t.fill(pl, pl.newFrame(nil, p.Entry))
	return pl
}

func (pl *procPlan) newFrame(parent *frame, entry ast.Node) *frame {
	f := &frame{index: len(pl.frames), parent: parent, entry: entry}
	pl.frames = append(pl.frames, f)
	return f
}

func (t *translator) fill(pl *procPlan, f *frame) {
	work := []ast.Node{f.entry}
	for len(work) > 0 {
		n := work[0]
		work = work[1:]
		if g, ok := pl.frameOf[n]; ok {
			if g != f {
				failf("%s: node %s is reachable both inside and outside a for each region", pl.name, n.Name())
			}
			continue
		}
		pl.frameOf[n] = f
		f.nodes = append(f.nodes, n)
		t.assignID(pl, n)
		edge, ok := pl.proc.Tree[n]
		if !ok {
			if f.parent != nil {
				failf("%s: path through %s ends inside a for each region without a collect", pl.name, n.Name())
			}
			continue
		}
		switch e := edge.(type) {
		case ast.Call:
			work = append(work, e.Target)
		case ast.Match:
			for _, cond := range matchOrder(e) {
				work = append(work, e.Paths[cond].Target)
			}
		case ast.Collect:
			if f.parent == nil {
				failf("%s: %s collects outside of any for each", pl.name, n.Name())
			}
			if f.sink != nil && f.sink != n {
				failf("%s: for each region has two collect points, %s and %s", pl.name, f.sink.Name(), n.Name())
			}
			f.sink = n
		case ast.ForEach:
			child := pl.newFrame(f, e.Call.Target)
			child.elem = e.Dim.Type
			child.args = e.Call.Args
			pl.region[n] = child
			t.fill(pl, child)
			if child.sink == nil {
				failf("%s: for each from %s never collects", pl.name, n.Name())
			}
			work = append(work, pl.proc.Tree[child.sink].(ast.Collect).Call.Target)
		default:
			failf("%s: unknown edge kind %s", pl.name, edge.Type())
		}
	}
}

func (t *translator) assignID(pl *procPlan, n ast.Node) {
	var id string
	switch n := n.(type) {
	case *ast.Proc:
		id = t.nameOf(n)
		if inner, ok := pl.proc.Namespace[n.Name()]; ok && inner == ast.Node(n) {
			id = exported(n.Name())
		}
	case *ast.Func:
		id = exported(n.Name())
		if t.isEffect(n) {
			id = t.ownerOf(n) + id
		}
	}
	base := id
	for i := 2; pl.hasID(id); i++ {
		id = fmt.Sprintf("%s%d", base, i)
	}
	pl.ids[n] = id
	pl.order = append(pl.order, n)
}

func (pl *procPlan) hasID(id string) bool {
	for _, v := range pl.ids {
		if v == id {
			return true
		}
	}
	return false
}

func (pl *procPlan) pc(n ast.Node) string     { return "pc" + pl.name + pl.ids[n] }
func (pl *procPlan) envType() string          { return lowerFirst(pl.name) + "Env" }
func (pl *procPlan) frameFunc(f *frame) string { return fmt.Sprintf("%sFrame%d", lowerFirst(pl.name), f.index) }

// lookup resolves the node a path starts at, mirroring compiler.resolveNode:
// the proc's own namespace shadows everything else.
func (t *translator) lookup(pl *procPlan, path []string) (ast.Node, []string) {
	if n, ok := pl.proc.Namespace[path[0]]; ok {
		return n, path[1:]
	}
	for _, n := range pl.order {
		switch n := n.(type) {
		case *ast.Proc:
			if n.Name() == path[0] {
				return n, path[1:]
			}
		case *ast.Func:
			iface, ok := t.effectOf[n]
			if !ok || len(path) < 2 {
				continue
			}
			if (iface == path[0] || iface == "") && n.Name() == path[1] {
				return n, path[2:]
			}
		}
	}
	failf("%s: %s does not name a node executed by this proc", pl.name, strings.Join(path, "."))
	return nil, nil
}

// matchOrder returns the arm keys of m. The AST stores arms in a map, so
// source order is lost; sort for determinism and keep the catch-all last.
func matchOrder(m ast.Match) []string {
	keys := sortedKeys(m.Paths)
	slices.SortStableFunc(keys, func(a, b string) int {
		switch {
		case condText(a) == "else":
			return 1
		case condText(b) == "else":
			return -1
		}
		return 0
	})
	return keys
}

// condText strips the string quotes the parser leaves on a match condition.
func condText(key string) string {
	if s, err := strconv.Unquote(key); err == nil {
		return s
	}
	return key
}

func hasElse(m ast.Match) bool {
	return slices.ContainsFunc(sortedKeys(m.Paths), func(k string) bool { return condText(k) == "else" })
}

/** Types **/

func (t *translator) goType(ty ast.Type) string {
	switch ty := ty.(type) {
	case ast.Primitive:
		switch ty.Name {
		case "void":
			return "struct{}"
		case "int", "string", "bool":
			return ty.Name
		case "float":
			return "float64"
		}
		failf("unknown primitive %s", ty.Name)
	case ast.Array:
		return "[]" + t.goType(ty.Type)
	case ast.Struct:
		var b strings.Builder
		b.WriteString("struct{ ")
		for _, k := range fieldNames(ty.Defs, nil) {
			fmt.Fprintf(&b, "%s %s; ", exported(k), t.goType(ty.Defs[k]))
		}
		b.WriteString("}")
		return b.String()
	case ast.Enum:
		var b strings.Builder
		b.WriteString("struct{ Tag string; ")
		for _, k := range fieldNames(ty.Enums, []string{"Tag"}) {
			fmt.Fprintf(&b, "%s %s; ", exported(k), t.goType(ty.Enums[k]))
		}
		b.WriteString("}")
		return b.String()
	}
	failf("unknown type %T", ty)
	return ""
}

func fieldNames(defs map[string]ast.Type, reserved []string) []string {
	seen := make(map[string]string)
	for _, r := range reserved {
		seen[r] = r
	}
	keys := sortedKeys(defs)
	for _, k := range keys {
		if prev, ok := seen[exported(k)]; ok {
			failf("fields %s and %s collide as Go field %s", prev, k, exported(k))
		}
		seen[exported(k)] = k
	}
	return keys
}

func isVoid(ty ast.Type) bool {
	p, ok := ty.(ast.Primitive)
	return ok && p.Name == "void"
}

// coerce converts expr from one vandorf type to another it Matches into.
func (t *translator) coerce(expr string, from, to ast.Type) string {
	src, dst := t.goType(from), t.goType(to)
	if src == dst {
		return expr
	}
	key := src + "\x00" + dst
	name, ok := t.coercions[key]
	if !ok {
		name = fmt.Sprintf("coerce%d", len(t.coercions))
		t.coercions[key] = name
		var w writer
		w.linef("func %s(v %s) (r %s) {", name, src, dst)
		t.coercionBody(&w, from, to)
		w.line("return r")
		w.line("}")
		t.helpers = append(t.helpers, w.String())
	}
	return name + "(" + expr + ")"
}

func (t *translator) coercionBody(w *writer, from, to ast.Type) {
	switch from := from.(type) {
	case ast.Array:
		to, ok := to.(ast.Array)
		if !ok {
			break
		}
		w.linef("r = make(%s, len(v))", t.goType(to))
		w.line("for i := range v {")
		w.linef("r[i] = %s", t.coerce("v[i]", from.Type, to.Type))
		w.line("}")
		return
	case ast.Struct:
		to, ok := to.(ast.Struct)
		if !ok || !to.Match(from) {
			break
		}
		for _, k := range sortedKeys(to.Defs) {
			w.linef("r.%s = %s", exported(k), t.coerce("v."+exported(k), from.Defs[k], to.Defs[k]))
		}
		return
	case ast.Enum:
		to, ok := to.(ast.Enum)
		if !ok || !to.Match(from) {
			break
		}
		w.line("r.Tag = v.Tag")
		w.line("switch v.Tag {")
		for _, k := range sortedKeys(from.Enums) {
			w.linef("case %q:", k)
			w.linef("r.%s = %s", exported(k), t.coerce("v."+exported(k), from.Enums[k], to.Enums[k]))
		}
		w.line("}")
		return
	}
	failf("cannot coerce %s into %s", t.goType(from), t.goType(to))
}

/** Expressions **/

// scope is what an argument expression can see: the env of its frame and
// its ancestors, and `this`.
type scope struct {
	pl    *procPlan
	frame *frame
	this  ast.Type
}

func (t *translator) ref(sc scope, path []string) (string, ast.Type) {
	var expr string
	var ty ast.Type
	rest := path[1:]
	if path[0] == "this" {
		expr, ty = "this", sc.this
	} else {
		var n ast.Node
		n, rest = t.lookup(sc.pl, path)
		owner := sc.pl.frameOf[n]
		visible := false
		for f := sc.frame; f != nil; f = f.parent {
			visible = visible || f == owner
		}
		if !visible {
			failf("%s: %s is not in scope here (it runs inside a for each region that has already collected, or never runs)", sc.pl.name, n.Name())
		}
		expr, ty = "env.out"+sc.pl.ids[n], n.Ret()
	}
	for _, field := range rest {
		s, ok := ty.(ast.Struct)
		if !ok {
			failf("%s: cannot select %s from %s", sc.pl.name, field, strings.Join(path, "."))
		}
		if ty, ok = s.Defs[field]; !ok {
			failf("%s: no field %s in %s", sc.pl.name, field, strings.Join(path, "."))
		}
		expr += "." + exported(field)
	}
	return expr, ty
}

func (t *translator) arg(sc scope, id ast.Ident, want ast.Type) string {
	expr, got := t.ref(sc, id.Path)
	// The AST encodes an enum constructor V(x) as an Ident of x whose type is
	// the one-variant enum { V(typeof x) }; recover the variant from that.
	if e, ok := id.Type.(ast.Enum); ok && len(e.Enums) == 1 && t.goType(got) != t.goType(id.Type) {
		for tag, payload := range e.Enums {
			if t.goType(payload) != t.goType(got) {
				break
			}
			target, ok := want.(ast.Enum)
			if !ok {
				failf("%s: enum constructor %s passed where %s is expected", sc.pl.name, tag, t.goType(want))
			}
			pt, ok := target.Enums[tag]
			if !ok {
				failf("%s: %s has no variant %s", sc.pl.name, t.goType(want), tag)
			}
			return fmt.Sprintf("%s{Tag: %q, %s: %s}", t.goType(want), tag, exported(tag), t.coerce(expr, got, pt))
		}
	}
	if !want.Match(got) {
		failf("%s: %s has type %s, expected %s", sc.pl.name, strings.Join(id.Path, "."), t.goType(got), t.goType(want))
	}
	return t.coerce(expr, got, want)
}

func (t *translator) inLiteral(sc scope, n ast.Node, args map[string]ast.Ident) string {
	params := n.Args()
	var parts []string
	for _, k := range fieldNames(params, nil) {
		id, ok := args[k]
		if !ok {
			failf("%s: call to %s is missing argument %s", sc.pl.name, n.Name(), k)
		}
		parts = append(parts, exported(k)+": "+t.arg(sc, id, params[k]))
	}
	for k := range args {
		if _, ok := params[k]; !ok {
			failf("%s: %s has no parameter %s", sc.pl.name, n.Name(), k)
		}
	}
	return t.inType(n) + "{" + strings.Join(parts, ", ") + "}"
}

func (t *translator) invoke(pl *procPlan, n ast.Node) string {
	in := "env.in" + pl.ids[n]
	switch n := n.(type) {
	case *ast.Proc:
		return fmt.Sprintf("Run%s(ctx, deps.%s, %s)", t.nameOf(n), t.nameOf(n), in)
	case *ast.Func:
		if t.isEffect(n) {
			return fmt.Sprintf("deps.%s.%s(ctx, %s)", t.ownerOf(n), exported(n.Name()), in)
		}
		return fmt.Sprintf("deps.Funcs.%s(ctx, %s)", exported(n.Name()), in)
	}
	failf("unknown node kind %T", n)
	return ""
}

/** Emission **/

func (t *translator) emitProc(w *writer, p *ast.Proc) {
	pl := t.plan(p)

	var funcs []*ast.Func
	deps := map[string]string{}
	for _, n := range pl.order {
		switch n := n.(type) {
		case *ast.Proc:
			deps[t.nameOf(n)] = "*" + t.nameOf(n) + "Deps"
		case *ast.Func:
			if t.isEffect(n) {
				deps[t.ownerOf(n)] = t.ownerOf(n)
			} else {
				funcs = append(funcs, n)
			}
		}
	}
	slices.SortFunc(funcs, func(a, b *ast.Func) int { return strings.Compare(a.Name(), b.Name()) })

	if len(funcs) > 0 {
		deps["Funcs"] = pl.name + "Funcs"
		w.linef("// %sFuncs are the leaf nodes of proc %s.", pl.name, p.Name())
		w.linef("type %sFuncs interface {", pl.name)
		for _, f := range funcs {
			t.emitMethod(w, f)
		}
		w.line("}")
		w.line("")
		for _, f := range funcs {
			t.emitIn(w, f)
		}
	}

	w.linef("// %sDeps supplies everything proc %s calls out to.", pl.name, p.Name())
	w.linef("type %sDeps struct {", pl.name)
	for _, k := range sortedKeys(deps) {
		w.linef("%s %s", k, deps[k])
	}
	w.line("}")
	w.line("")

	w.linef("type %s struct {", pl.envType())
	for _, n := range pl.order {
		w.linef("in%s %s", pl.ids[n], t.inType(n))
		w.linef("out%s %s", pl.ids[n], t.goType(n.Ret()))
	}
	w.line("}")
	w.line("")

	w.line("const (")
	for i, n := range pl.order {
		if i == 0 {
			w.linef("%s = iota", pl.pc(n))
		} else {
			w.line(pl.pc(n))
		}
	}
	w.line(")")
	w.line("")

	root := pl.frames[0]
	ret := t.goType(p.Ret())
	w.linef("func Run%s(ctx context.Context, deps *%sDeps, in %s) (%s, error) {", pl.name, pl.name, t.inType(p), ret)
	w.linef("var zero %s", ret)
	w.linef("env := &%s{}", pl.envType())
	w.linef("env.in%s = in", pl.ids[root.entry])
	w.linef("pc := %s", pl.pc(root.entry))
	t.emitLoop(w, pl, root)
	w.line("}")
	w.line("")

	for _, f := range pl.frames[1:] {
		sink := t.goType(f.sink.Ret())
		w.linef("// %s runs one element of a for each region entered at %s.", pl.frameFunc(f), f.entry.Name())
		w.linef("func %s(ctx context.Context, deps *%sDeps, env *%s, elem %s) (%s, error) {", pl.frameFunc(f), pl.name, pl.envType(), t.goType(f.elem), sink)
		w.linef("var zero %s", sink)
		w.line("this := elem")
		w.line("_ = this")
		w.linef("env.in%s = %s", pl.ids[f.entry], t.inLiteral(scope{pl, f, f.elem}, f.entry, f.args))
		w.linef("pc := %s", pl.pc(f.entry))
		t.emitLoop(w, pl, f)
		w.line("}")
		w.line("")
	}

	for _, name := range sortedKeys(p.Namespace) {
		if sub, ok := p.Namespace[name].(*ast.Proc); ok {
			t.emitProc(w, sub)
		}
	}
}

func (t *translator) emitLoop(w *writer, pl *procPlan, f *frame) {
	w.line("for {")
	w.line("if err := ctx.Err(); err != nil {")
	w.line("return zero, err")
	w.line("}")
	w.line("switch pc {")
	for _, n := range f.nodes {
		t.emitNode(w, pl, f, n)
	}
	w.line("default:")
	w.linef("return zero, fmt.Errorf(%q, pc)", pl.name+": invalid state %d")
	w.line("}")
	w.line("}")
}

func (t *translator) emitNode(w *writer, pl *procPlan, f *frame, n ast.Node) {
	sc := scope{pl, f, n.Ret()}
	id := pl.ids[n]
	w.linef("case %s:", pl.pc(n))
	if fn, ok := n.(*ast.Func); ok && isVoid(fn.Ret()) {
		w.linef("err := %s", t.invoke(pl, n))
		w.line("this := struct{}{}")
	} else {
		w.linef("this, err := %s", t.invoke(pl, n))
	}
	w.line("if err != nil {")
	w.linef("return zero, fmt.Errorf(%q, err)", pl.proc.Name()+"."+n.Name()+": %w")
	w.line("}")
	w.linef("env.out%s = this", id)

	edge, ok := pl.proc.Tree[n]
	if !ok {
		w.linef("return %s, nil", t.coerce("this", n.Ret(), pl.proc.Ret()))
		return
	}
	switch e := edge.(type) {
	case ast.Call:
		t.emitTransition(w, sc, e)
	case ast.Match:
		t.emitMatch(w, sc, n, e)
	case ast.Collect:
		w.line("return this, nil")
	case ast.ForEach:
		child := pl.region[n]
		dim, dimType := t.ref(sc, e.Dim.Path)
		arr, ok := dimType.(ast.Array)
		if !ok {
			failf("%s: for each over %s, which is not an array", pl.name, strings.Join(e.Dim.Path, "."))
		}
		if t.goType(arr.Type) != t.goType(child.elem) {
			failf("%s: for each element type mismatch on %s", pl.name, strings.Join(e.Dim.Path, "."))
		}
		collected := child.sink.Ret()
		w.linef("results, err := fanOut(ctx, %s, func(ctx context.Context, elem %s) (%s, error) {", dim, t.goType(arr.Type), t.goType(collected))
		w.line("env := *env")
		w.linef("return %s(ctx, deps, &env, elem)", pl.frameFunc(child))
		w.line("})")
		w.line("if err != nil {")
		w.line("return zero, err")
		w.line("}")
		w.line("{")
		w.line("this := results")
		collect := pl.proc.Tree[child.sink].(ast.Collect)
		t.emitTransition(w, scope{pl, f, ast.Array{Type: collected}}, collect.Call)
		w.line("}")
	}
}

func (t *translator) emitTransition(w *writer, sc scope, c ast.Call) {
	if sc.pl.frameOf[c.Target] != sc.frame {
		failf("%s: edge into %s crosses a for each boundary", sc.pl.name, c.Target.Name())
	}
	w.linef("env.in%s = %s", sc.pl.ids[c.Target], t.inLiteral(sc, c.Target, c.Args))
	w.linef("pc = %s", sc.pl.pc(c.Target))
	w.line("continue")
}

// emitMatch branches on an enum's variant names, or otherwise treats each arm
// as a Go boolean expression over `this` (vandorf has no expression grammar
// yet). "else" is the default arm.
func (t *translator) emitMatch(w *writer, sc scope, n ast.Node, m ast.Match) {
	enum, isEnum := sc.this.(ast.Enum)
	if isEnum {
		w.line("switch this.Tag {")
	} else {
		w.line("switch {")
	}
	for _, key := range matchOrder(m) {
		cond := condText(key)
		switch {
		case cond == "else":
			w.line("default:")
		case isEnum:
			if _, ok := enum.Enums[cond]; !ok {
				failf("%s: %s has no variant %s", sc.pl.name, n.Name(), cond)
			}
			w.linef("case %q:", cond)
		default:
			if _, err := parser.ParseExpr(cond); err != nil {
				failf("%s: match arm %q on %s: %v", sc.pl.name, cond, n.Name(), err)
			}
			w.linef("case %s:", cond)
		}
		t.emitTransition(w, sc, m.Paths[key])
	}
	w.line("}")
	if !hasElse(m) {
		w.linef("return zero, fmt.Errorf(%q, this)", sc.pl.proc.Name()+"."+n.Name()+": no match arm for %v")
	}
}

func (t *translator) emitMethod(w *writer, f *ast.Func) {
	if isVoid(f.Ret()) {
		w.linef("%s(ctx context.Context, in %s) error", exported(f.Name()), t.inType(f))
	} else {
		w.linef("%s(ctx context.Context, in %s) (%s, error)", exported(f.Name()), t.inType(f), t.goType(f.Ret()))
	}
}

func (t *translator) emitIn(w *writer, f *ast.Func) {
	w.linef("type %s struct {", t.inType(f))
	args := f.Args()
	for _, k := range fieldNames(args, nil) {
		w.linef("%s %s", exported(k), t.goType(args[k]))
	}
	w.line("}")
	w.line("")
}

func (t *translator) emitEffects(w *writer) {
	for _, name := range sortedKeys(t.ifaces) {
		funcs := t.ifaces[name]
		w.linef("// %s is an effect interface: calls that reach outside the graph.", name)
		w.linef("type %s interface {", name)
		for _, f := range funcs {
			t.emitMethod(w, f)
		}
		w.line("}")
		w.line("")
		for _, f := range funcs {
			t.emitIn(w, f)
		}
	}
}

func (t *translator) emitRuntime(w *writer) {
	w.WriteString(runtimeSrc)
	for _, h := range t.helpers {
		w.line("")
		w.WriteString(h)
	}
}

const runtimeSrc = `
// fanOut runs f over xs concurrently and returns the results in input order.
// The first error cancels the remaining branches.
func fanOut[E, R any](ctx context.Context, xs []E, f func(context.Context, E) (R, error)) ([]R, error) {
	ctx, cancel := context.WithCancel(ctx)
	defer cancel()
	out := make([]R, len(xs))
	var (
		wg    sync.WaitGroup
		once  sync.Once
		first error
	)
	for i, x := range xs {
		wg.Go(func() {
			r, err := f(ctx, x)
			if err != nil {
				once.Do(func() { first = err; cancel() })
				return
			}
			out[i] = r
		})
	}
	wg.Wait()
	return out, first
}
`

/** Output **/

// writer emits unindented lines; gofmt fixes the layout and doubles as a
// syntax check on the generated code.
type writer struct{ bytes.Buffer }

func (w *writer) line(s string) {
	w.WriteString(s)
	w.WriteByte('\n')
}

func (w *writer) linef(format string, args ...any) {
	fmt.Fprintf(w, format, args...)
	w.WriteByte('\n')
}

func (w *writer) header(pkg string, imports ...string) {
	w.line("// Code generated by vandorf. DO NOT EDIT.")
	w.line("")
	w.linef("package %s", pkg)
	w.line("")
	w.line("import (")
	for _, imp := range imports {
		w.linef("%q", imp)
	}
	w.line(")")
	w.line("")
}

func (w *writer) format(name string) (string, error) {
	src, err := format.Source(w.Bytes())
	if err != nil {
		return "", fmt.Errorf("generated invalid Go for %s: %w\n%s", name, err, w.String())
	}
	return string(src), nil
}

/** Helpers **/

func exported(s string) string {
	r, size := utf8.DecodeRuneInString(s)
	return string(unicode.ToUpper(r)) + s[size:]
}

func lowerFirst(s string) string {
	r, size := utf8.DecodeRuneInString(s)
	return string(unicode.ToLower(r)) + s[size:]
}

func sortedKeys[V any](m map[string]V) []string {
	keys := make([]string, 0, len(m))
	for k := range m {
		keys = append(keys, k)
	}
	sort.Strings(keys)
	return keys
}
