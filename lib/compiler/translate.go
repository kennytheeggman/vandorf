package compiler 

import (
	"fmt"

	"github.com/kennytheeggman/vandorf/lib/compiler/ast"
	"github.com/kennytheeggman/vandorf/lib/compiler/cfg"
)


func translateProgram(parse_tree *cfg.Program) (ast.Program, error) {
	syntax_tree := ast.Program{
		Interfaces: make(map[string]ast.Interface),
		Nodes: make(map[string]ast.Node),
	}
	for _, i := range parse_tree.Interfaces {
		if _, ok := syntax_tree.Interfaces[i.Name]; ok {
			return ast.Program{}, fmt.Errorf("duplicate interface name: %s", i.Name)
		}
		syntax_tree.Interfaces[i.Name] = ast.Interface{
			Name: i.Name,
			Funcs: make(map[string]*ast.Func),
		}
		for _, f := range i.Funcs {
			fn, err := translateFunc(&f)
			if err != nil {
				return ast.Program{}, err
			}
			if _, ok := syntax_tree.Interfaces[i.Name].Funcs[f.Name]; ok {
				return ast.Program{}, fmt.Errorf("duplicate function name: %s", f.Name)
			}
			syntax_tree.Interfaces[i.Name].Funcs[f.Name] = &fn
		}
	}
	for _, n := range parse_tree.Nodes {
		switch {
		case n.Func != nil:
			fn, err := translateFunc(n.Func)
			if err != nil {
				return ast.Program{}, err
			}
			node := ast.Node(&fn)
			if _, ok := syntax_tree.Nodes[n.Func.Name]; ok {
				return ast.Program{}, fmt.Errorf("duplicate function name: %s", n.Func.Name)
			}
			syntax_tree.Nodes[n.Func.Name] = node
		case n.Proc != nil:
			proc, err := translateProc(n.Proc)
			if err != nil {
				return ast.Program{}, err
			}
			node := ast.Node(&proc)
			if _, ok := syntax_tree.Nodes[n.Proc.Name]; ok {
				return ast.Program{}, fmt.Errorf("duplicate procedure name: %s", n.Proc.Name)
			}
			syntax_tree.Nodes[n.Proc.Name] = node
		default:
			return ast.Program{}, fmt.Errorf("unreachable invalid node: %v", n)
		}
	}
	for _, n := range parse_tree.Nodes {
		var proc *ast.Proc
		switch a := syntax_tree.Nodes[n.Proc.Name].(type) {
		case *ast.Func:
			continue
		case *ast.Proc:
			proc = a
		default:
			return ast.Program{}, fmt.Errorf("unreachable invalid node: %v", n)
		}
		err := traceProc(proc, n.Proc, &syntax_tree)
		if err != nil {
			return ast.Program{}, err
		}
	}
	return syntax_tree, nil
}

func traceProc(proc *ast.Proc, cfg *cfg.Proc, syntax_tree *ast.Program) error {
	for _, n := range cfg.Nodes {
		switch {
		case n.Proc != nil:
			err := traceProc(proc.Nodes[n.Proc.Name].(*ast.Proc), n.Proc, syntax_tree)
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
			node, d, err := resolveNode(l.Call.Source, syntax_tree, proc)
			if err != nil {
				return err
			}
			if d != len(l.Call.Source.Path) {
				return fmt.Errorf("invalid source: %s", l.Call.Source.Path)
			}
			this_binding, err := node.Ret()
			if err != nil {
				return err
			}
			call, err := resolveCall(l.Call.Target, l.Call.Args, this_binding, syntax_tree, proc)
			if err != nil {
				return err
			}
			if _, ok := proc.Tree[node]; ok {
				return fmt.Errorf("duplicate call: %s", node)
			}
			calls := make([]ast.Call, 1)
			calls[0] = call
			proc.Tree[node] = calls
			if l.Call.Alias != nil {
				if _, ok := proc.Alias[*l.Call.Alias]; ok {
					return fmt.Errorf("duplicate alias: %s", *l.Call.Alias)
				}
				proc.Alias[*l.Call.Alias] = &call
			}
		case l.Match != nil:
			node, d, err := resolveNode(l.Match.Source, syntax_tree, proc)
			if err != nil {
				return err
			}
			if d != len(l.Match.Source.Path) {
				return fmt.Errorf("invalid source: %s", l.Match.Source.Path)
			}
			calls := make([]ast.Call, len(l.Match.Paths))
			for i, p := range l.Match.Paths {
				this_binding, err := node.Ret()
				if err != nil {
					return err
				}
				call, err := resolveCall(p.Target, p.Args, this_binding, syntax_tree, proc)
				if err != nil {
					return err
				}
				calls[i] = call
			}
			if _, ok := proc.Tree[node]; ok {
				return fmt.Errorf("duplicate call: %s", node)
			}
			proc.Tree[node] = calls
		case l.Collect != nil:
			node, d, err := resolveNode(l.Collect.Source, syntax_tree, proc)
			if err != nil {
				return err
			}
			if d != len(l.Collect.Source.Path) {
				return fmt.Errorf("invalid source: %s", l.Collect.Source.Path)
			}
			this_binding, err := proc.Nodes[l.Collect.Source.Path[0]].Ret()
			if err != nil {
				return err
			}
			call, err := resolveCall(l.Collect.Target, l.Collect.Args, ast.Array{Type: this_binding}, syntax_tree, proc)
			calls := make([]ast.Call, 1)
			calls[0] = call
			if err != nil {
				return err
			}
			if _, ok := proc.Tree[node]; ok {
				return fmt.Errorf("duplicate call: %s", node)
			}
			proc.Tree[node] = calls
		case l.ForEach != nil:
			node, d, err := resolveNode(l.ForEach.Source, syntax_tree, proc)
			if err != nil {
				return err
			}
			if d != len(l.ForEach.Source.Path) {
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
			calls := make([]ast.Call, 1)
			calls[0] = call
			if err != nil {
				return err
			}
			fmt.Printf("node: %s\n", node)
			if _, ok := proc.Tree[node]; ok {
				return fmt.Errorf("duplicate call: %s", node)
			}
			proc.Tree[node] = calls
		default:
			continue
		}
	}
	return nil
}

func resolveNode(id cfg.Ident, syntax_tree *ast.Program, proc *ast.Proc) (ast.Node, int, error) {
	if _, ok := syntax_tree.Interfaces[id.Path[0]]; ok {
		if _, ok := syntax_tree.Interfaces[id.Path[0]].Funcs[id.Path[1]]; ok {
			fn := (ast.Node)(syntax_tree.Interfaces[id.Path[0]].Funcs[id.Path[1]])
			return fn, 2, nil
		} else {
			return nil, 0, fmt.Errorf("interface not found: %s", id.Path[0])
		}
	} else {
		var cur ast.Node
		if proc != nil {
			if _, ok := proc.Nodes[id.Path[0]]; ok {
				cur = proc.Nodes[id.Path[0]]
			} else {
				cur = syntax_tree.Nodes[id.Path[0]]
			}
		} else {
			cur = syntax_tree.Nodes[id.Path[0]]
		}
		var i int = 1
		for _, d := range id.Path[1:] {
			i++
			switch p := cur.(type) {
			case *ast.Func:
				return cur, i-1, nil
			case *ast.Proc:
				if _, ok := p.Nodes[d]; ok {
					cur = p.Nodes[d]
				} else {
					return cur, i-1, nil 
				}
			default:
				return cur, i-1, nil 
			}
		}
		return cur, i, nil
	}
}

func resolveType(id cfg.Ident, syntax_tree *ast.Program, this ast.Type, proc *ast.Proc) (ast.Type, error) {
	remaining := id.Path
	remaining_type := ast.Type(nil)
	if id.Path[0] == "this" {
		remaining = remaining[1:]
		remaining_type = this
	} else {
		node, d, err := resolveNode(id, syntax_tree, proc)
		if err != nil {
			return nil, err
		}
		remaining = remaining[d:]
		remaining_type, err = node.Ret()
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
	node, d, err := resolveNode(id, syntax_tree, proc)
	if err != nil {
		return ast.Call{}, err
	}
	if d != len(id.Path) {
		return ast.Call{}, fmt.Errorf("invalid source: %s", id.Path)
	}
	arg_types := make(map[string]ast.Ident)
	node_args, err := node.Args()
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
		return ast.Primitive{Name: parse_tree.Name.Name}, nil
	default:
		return nil, fmt.Errorf("unreachable invalid type")
	}
}

func translateProc(parse_tree *cfg.Proc) (ast.Proc, error) {
	nodes := make([]ast.Node, len(parse_tree.Nodes))
	var entry ast.Node
	for i, n := range parse_tree.Nodes {
		switch {
		case n.Func != nil:
			fn, err := translateFunc(n.Func)
			if err != nil {
				return ast.Proc{}, err
			}
			nodes[i] = &fn
		case n.Proc != nil:
			proc, err := translateProc(n.Proc)
			if err != nil {
				return ast.Proc{}, err
			}
			nodes[i] = &proc
		default:
			return ast.Proc{}, fmt.Errorf("unreachable invalid node")
		}
		name, err := nodes[i].Name()
		if err != nil {
			return ast.Proc{}, err
		}
		if name == parse_tree.Entry {
			entry = nodes[i]
		}
	}
	ret, err := translateType(&parse_tree.Ret)
	if err != nil {
		return ast.Proc{}, err
	}
	proc := ast.NewProc(parse_tree.Name, ret, entry, nodes)
	proc.Tree = make(map[ast.Node][]ast.Call)
	proc.Alias = make(map[string]*ast.Call)
	return proc, nil
}


