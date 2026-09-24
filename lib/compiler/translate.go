package compiler

import (
	"fmt"
	"slices"

	"github.com/kennytheeggman/vandorf/lib/compiler/ast"
	"github.com/kennytheeggman/vandorf/lib/compiler/cfg"
)


func translateProgram(parse_tree *cfg.Program) (ast.Program, error) {
	num_funcs := 0
	num_procs := 0
	for _, n := range parse_tree.Nodes {
		switch {
		case n.Func != nil:
			num_funcs++
		case n.Proc != nil:
			num_procs++
		}
	}
	syntax_tree := ast.Program{
		Interfaces: make([]ast.Interface, len(parse_tree.Interfaces)),
		Funcs: make([]ast.Func, num_funcs),
		Procs: make([]ast.Proc, num_procs),
		Namespace: make(map[string]ast.Item),
	}
	for i, ifc := range parse_tree.Interfaces {
		syntax_tree.Interfaces[i] = ast.NewInterface(ifc.Name)
		for _, f := range ifc.Funcs {
			fn, err := translateFunc(&f)
			if err != nil {
				return ast.Program{}, err
			}
			if _, ok := syntax_tree.Interfaces[i].Namespace[f.Name]; ok {
				return ast.Program{}, fmt.Errorf("duplicate function name: %s", f.Name)
			}
			syntax_tree.Interfaces[i].Funcs = append(syntax_tree.Interfaces[i].Funcs, fn)
			syntax_tree.Interfaces[i].Namespace[f.Name] = &syntax_tree.Interfaces[i].Funcs[len(syntax_tree.Interfaces[i].Funcs)-1]
		}
		syntax_tree.Namespace[ifc.Name] = &syntax_tree.Interfaces[i]
	}
	proc_count := 0
	func_count := 0
	for _, n := range parse_tree.Nodes {
		switch {
		case n.Func != nil:
			fn, err := translateFunc(n.Func)
			if err != nil {
				return ast.Program{}, err
			}
			if _, ok := syntax_tree.Namespace[n.Func.Name]; ok {
				return ast.Program{}, fmt.Errorf("duplicate function name: %s", n.Func.Name)
			}
			syntax_tree.Funcs[func_count] = fn
			syntax_tree.Namespace[n.Func.Name] = &syntax_tree.Funcs[func_count]
			func_count++
		case n.Proc != nil:
			proc, err := translateProc(n.Proc)
			if err != nil {
				return ast.Program{}, err
			}
			if _, ok := syntax_tree.Namespace[n.Proc.Name]; ok {
				return ast.Program{}, fmt.Errorf("duplicate procedure name: %s", n.Proc.Name)
			}
			syntax_tree.Procs[proc_count] = proc
			syntax_tree.Namespace[n.Proc.Name] = &syntax_tree.Procs[proc_count]
			proc_count++
		default:
			return ast.Program{}, fmt.Errorf("unreachable invalid node: %v", n)
		}
	}
	for _, n := range parse_tree.Nodes {
		switch {
		case n.Func != nil:
			continue
		case n.Proc != nil:
			proc := syntax_tree.Namespace[n.Proc.Name]
			switch p := proc.(type) {
			case *ast.Proc:
				err := traceProc(p, n.Proc, &syntax_tree)
				if err != nil {
					return ast.Program{}, err
				}
			default:
				return ast.Program{}, fmt.Errorf("unreachable invalid node: %v", n)
			}
		default:
			return ast.Program{}, fmt.Errorf("unreachable invalid node: %v", n)
		}
	}
	return syntax_tree, nil
}

func traceProc(proc *ast.Proc, cfg *cfg.Proc, syntax_tree *ast.Program) error {
	for _, n := range cfg.Nodes {
		switch {
		case n.Proc != nil:
			err := traceProc(proc.Namespace[n.Proc.Name].(*ast.Proc), n.Proc, syntax_tree)
			if err != nil {
				return err
			}
		default:
			continue
		}
	}
	for _, l := range cfg.Links {
		switch {
		case l.Call != nil:
			node, d, err := resolveNode(l.Call.Source.Path, proc, syntax_tree)
			if err != nil { return err }
			if len(d) != 0 {
				return fmt.Errorf("invalid source: %s", l.Call.Source.Path)
			}
			this_binding := node.Ret()
			if err != nil { return err }
			call, err := resolveCall(l.Call.Target, l.Call.Args, this_binding, syntax_tree, proc)
			if err != nil { return err }
			if _, ok := proc.Tree[node]; ok {
				return fmt.Errorf("duplicate call: %s", node)
			}
			proc.Tree[node] = call
		case l.Match != nil:
			node, d, err := resolveNode(l.Match.Source.Path, proc, syntax_tree)
			if err != nil {
				return err
			}
			if len(d) != 0 {
				return fmt.Errorf("invalid source: %s", l.Match.Source.Path)
			}
			calls := make(map[string]ast.Call)
			for _, p := range l.Match.Paths {
				this_binding := node.Ret()
				if err != nil {
					return err
				}
				call, err := resolveCall(p.Target, p.Args, this_binding, syntax_tree, proc)
				if err != nil {
					return err
				}
				calls[p.Cond] = call
			}
			if _, ok := proc.Tree[node]; ok {
				return fmt.Errorf("duplicate call: %s", node)
			}
			proc.Tree[node] = ast.Match{Paths: calls}
		case l.Collect != nil:
			node, d, err := resolveNode(l.Collect.Source.Path, proc, syntax_tree)
			if err != nil {
				return err
			}
			if len(d) != 0 {
				return fmt.Errorf("invalid source: %s", l.Collect.Source.Path)
			}
			this_binding := node.Ret()
			if err != nil {
				return err
			}
			call, err := resolveCall(l.Collect.Target, l.Collect.Args, ast.Array{Type: this_binding}, syntax_tree, proc)
			if err != nil {
				return err
			}
			if _, ok := proc.Tree[node]; ok {
				return fmt.Errorf("duplicate call: %s", node)
			}
			proc.Tree[node] = ast.Collect{Call: call}
		case l.ForEach != nil:
			node, d, err := resolveNode(l.ForEach.Source.Path, proc, syntax_tree)
			if err != nil {
				return err
			}
			if len(d) != 0 {
				return fmt.Errorf("invalid source: %s", l.ForEach.Source.Path)
			}
			this_binding, err := resolveType(l.ForEach.Dim, syntax_tree, nil, proc)
			if err != nil {
				return err
			}
			switch b := this_binding.(type) {
			case ast.Array:
				this_binding = b.Type
			default:
				return fmt.Errorf("invalid type: %s", this_binding)
			}
			call, err := resolveCall(l.ForEach.Target, l.ForEach.Args, this_binding, syntax_tree, proc)
			if err != nil {
				return err
			}
			if _, ok := proc.Tree[node]; ok {
				return fmt.Errorf("duplicate call: %s", node)
			}
			proc.Tree[node] = ast.ForEach{Call: call, Dim: ast.Ident{Path: l.ForEach.Dim.Path, Type: this_binding}}
		default:
			continue
		}
	}
	return nil
}

func resolveIdent(path []string, namespace map[string]ast.Node) (ast.Node, []string, error) {
	if len(path) == 0 {
		return nil, nil, fmt.Errorf("tried to resolve empty path")
	}
	if n, ok := namespace[path[0]]; ok {
		if len(path) == 1 {
			return n, nil, nil;
		}
		switch t := n.(type) {
		case *ast.Proc:
			node, p, err := resolveIdent(path[1:], t.Namespace);
			if err != nil {
				return n, path[1:], nil;
			}
			return node, p, err;
		case *ast.Func:
			return n, path[1:], nil;
		default:
			return nil, nil, fmt.Errorf("unreachable invalid state");
		}
	} else {
		return nil, nil, fmt.Errorf("unknown identifier %s", path[0])
	}
}

func resolveNode(id []string, proc *ast.Proc, tree *ast.Program) (ast.Node, []string, error) {
	node, path, err := resolveIdent(id, proc.Namespace);
	if err == nil {
		return node, path, err
	}
	if n, ok := tree.Namespace[id[0]]; ok {
		switch t := n.(type) {
		case *ast.Interface:
			if len(id[1:]) == 0 {
				return nil, nil, fmt.Errorf("%s is not a node", id[0])
			}
			return resolveIdent(id[1:], t.Namespace);
		case *ast.Proc:
			node, p, err := resolveIdent(id[1:], t.Namespace);
			if err != nil {
				return t, id[1:], nil;
			}
			return node, p, err
		case *ast.Func:
			return t, id[1:], nil;
		default:
			return nil, nil, fmt.Errorf("unreachable invalid state");
		}
	} else {
		return nil, nil, fmt.Errorf("unknown identifier %s", id[0]);
	}
}

func resolveType(id cfg.Ident, syntax_tree *ast.Program, this ast.Type, proc *ast.Proc) (ast.Type, error) {
	remaining := id.Path
	remaining_type := ast.Type(nil)
	if id.Path[0] == "this" {
		remaining = remaining[1:]
		remaining_type = this
	} else {
		node, path, err := resolveNode(id.Path, proc, syntax_tree)
		if err != nil {
			return nil, err
		}
		remaining = path
		remaining_type = node.Ret()
	}
	if remaining_type == nil {
		return nil, fmt.Errorf("type not found: %s", id.Path[0])
	}
	for _, d := range remaining {
		switch p := remaining_type.(type) {
		case ast.Array:
			return nil, fmt.Errorf("array cannot be indexed")
		case ast.Enum:
			return nil, fmt.Errorf("enum cannot be indexed")
		case ast.Struct:
			if _, ok := p.Defs[d]; ok {
				remaining_type = p.Defs[d]
			} else {
				return nil, fmt.Errorf("struct not found: %s", d)
			}
		default:
			return nil, fmt.Errorf("unreachable invalid type")
		}
	}
	return remaining_type, nil
}

func resolveCall(id cfg.Ident, args []cfg.KVIdent, this ast.Type, syntax_tree *ast.Program, proc *ast.Proc) (ast.Call, error) {
	node, d, err := resolveNode(id.Path, proc, syntax_tree)
	if err != nil {
		return ast.Call{}, err
	}
	if len(d) != 0 {
		return ast.Call{}, fmt.Errorf("invalid source: %s", id.Path)
	}
	arg_types := make(map[string]ast.Ident)
	node_args := node.Args()
	if err != nil {
		return ast.Call{}, err
	}
	if len(node_args) != len(args) {
		return ast.Call{}, fmt.Errorf("found %d args, expected %d", len(node_args), len(args))
	}
	for _, a := range args {
		switch {
		case a.Val.Ident != nil:
			t, err := resolveType(*a.Val.Ident, syntax_tree, this, proc)
			if err != nil {
				return ast.Call{}, err
			}
			arg_types[a.Key] = ast.Ident{Path: a.Val.Ident.Path, Type: t}
		case a.Val.Enum != nil:
			t, err := resolveType(a.Val.Enum.Type, syntax_tree, this, proc)
			if err != nil {
				return ast.Call{}, err
			}
			enum := ast.Enum{Enums: make(map[string]ast.Type)}
			enum.Enums[a.Val.Enum.Symbol] = t
			t = enum
			arg_types[a.Key] = ast.Ident{Path: a.Val.Enum.Type.Path, Type: t}
		}
	}
	call := ast.Call{Target: node, Args: arg_types}
	return call, nil
}

func translateFunc(parse_tree *cfg.Func) (ast.Func, error) {
	args := make(map[string]ast.Type)
	for _, v := range parse_tree.Args {
		t, err := translateType(&v.Type)
		if err != nil {
			return ast.Func{}, err
		}
		args[v.Name] = t
	}
	ret, err := translateType(parse_tree.Ret)
	if err != nil {
		return ast.Func{}, err
	}
	syntax_tree := ast.NewFunc(parse_tree.Name, args, ret)
	return syntax_tree, nil
}

func translateType(parse_tree *cfg.Type) (ast.Type, error) {
	if parse_tree == nil {
		return ast.Primitive{Name: "void"}, nil
	}
	switch {
	case parse_tree.Array != nil:
		t, err := translateType(&parse_tree.Array.Type)
		if err != nil {
			return nil, err
		}
		return ast.Array{Type: t}, nil
	case parse_tree.Enum != nil:
		enums := make(map[string]ast.Type)
		for _, e := range parse_tree.Enum.Enums {
			if e.Type == nil {
				enums[e.Name] = ast.Primitive{Name: "void"}
			} else {
				s, err := translateType(e.Type)
				if err != nil {
					return nil, err
				}
				enums[e.Name] = s
			}
		}
		return ast.Enum{Enums: enums}, nil
	case parse_tree.Struct != nil:
		defs := make(map[string]ast.Type)
		for _, d := range parse_tree.Struct.Defs {
			t, err := translateType(&d.Type)
			if err != nil {
				return nil, err
			}
			defs[d.Name] = t
		}
		return ast.Struct{Defs: defs}, nil
	case parse_tree.Name != nil:
		if !slices.Contains([]string{"void", "string", "int", "float", "bool"}, parse_tree.Name.Name) {
			return nil, fmt.Errorf("unknown type: %s", parse_tree.Name.Name)
		}
		return ast.Primitive{Name: parse_tree.Name.Name}, nil
	default:
		return nil, fmt.Errorf("unreachable invalid type")
	}
}

func translateProc(parse_tree *cfg.Proc) (ast.Proc, error) {
	var funcs []ast.Func
	var procs []ast.Proc
	namespace := make(map[string]ast.Node)
	for _, n := range parse_tree.Nodes {
		switch {
		case n.Func != nil:
			fn, err := translateFunc(n.Func)
			if err != nil {
				return ast.Proc{}, err
			}
			funcs = append(funcs, fn);
			if _, ok := namespace[fn.Name()]; ok {
				return ast.Proc{}, fmt.Errorf("duplicate name %s in namespace", fn.Name());
			}
			namespace[fn.Name()] = &funcs[len(funcs)-1]
		case n.Proc != nil:
			proc, err := translateProc(n.Proc)
			if err != nil {
				return ast.Proc{}, err
			}
			procs = append(procs, proc);
			if _, ok := namespace[proc.Name()]; ok {
				return ast.Proc{}, fmt.Errorf("duplicate name %s in namespace", proc.Name());
			}
			namespace[proc.Name()] = &procs[len(procs)-1]
		default:
			return ast.Proc{}, fmt.Errorf("unreachable invalid node")
		}
	}
	if _, ok := namespace[parse_tree.Entry]; !ok {
		return ast.Proc{}, fmt.Errorf("entry not found in namespace")
	}
	entry := namespace[parse_tree.Entry]
	ret, err := translateType(&parse_tree.Ret)
	if err != nil {
		return ast.Proc{}, err
	}
	proc := ast.NewProc(parse_tree.Name, ret)
	proc.Entry = entry
	proc.Funcs = funcs
	proc.Procs = procs
	proc.Namespace = namespace
	proc.Tree = make(map[ast.Node]ast.Edge)
	return proc, nil
}


