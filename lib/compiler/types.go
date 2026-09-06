package compiler

import (
	"fmt"

	"github.com/kennytheeggman/vandorf/lib/compiler/ast"
)

func typeCheckProgram(a *ast.Program) error {
	for _, n := range a.Nodes {
		switch t := n.(type) {
		case *ast.Proc:
			err := typeCheckProc(t)
			if err != nil {
				return err
			}
		default:
			continue
		}
	}
	return nil
}

func typeCheckProc(proc *ast.Proc) error {
	for _, n := range proc.Nodes {
		switch t := n.(type) {
		case *ast.Proc:
			err := typeCheckProc(t)
			if err != nil {
				return err
			}
		default:
			continue
		}
	}
	for _, calls := range proc.Tree {
		for _, call := range calls {
			err := typeCheckCall(&call)
			if err != nil {
				return err
			}
		}
	}
	ret, err := proc.Ret()
	if err != nil {
		return err
	}
	err = typeCheckTree(proc.Tree, proc.Entry, ret, make(map[ast.Node]bool))
	if err != nil {
		return err
	}
	return nil
}

func typeCheckTree(tree map[ast.Node][]ast.Call, entry ast.Node, terminal ast.Type, visited map[ast.Node]bool) error {
	if _, ok := visited[entry]; ok {
		return nil
	}
	visited[entry] = true
	calls, ok := tree[entry]
	if !ok {
		entry_ret, err := entry.Ret()
		if err != nil {
			return err
		}
		if !entry_ret.Match(terminal) {
			return fmt.Errorf("terminal node %s does not match return type %s for parent process", entry, terminal)
		}
		return nil
	} else {
		for _, call := range calls {
			err := typeCheckTree(tree, call.Target, terminal, visited)
			if err != nil {
				return err
			}
		}
	}
	return nil
}

func typeCheckCall(call *ast.Call) error {
	node := call.Target
	node_args, err := node.Args()
	if err != nil {
		return err
	}
	if len(node_args) != len(call.Args) {
		return fmt.Errorf("found %d args, expected %d", len(node_args), len(call.Args))
	}
	for i, a := range call.Args {
		if !node_args[i].Match(a.Type) {
			return fmt.Errorf("arg %s does not match type: found %s, expected %s for node %s", a.Path, a.Type, node_args[i], node)
		}
	}
	return nil
}
