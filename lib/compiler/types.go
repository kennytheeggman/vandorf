package compiler

import (
	"fmt"

	"github.com/kennytheeggman/vandorf/lib/compiler/ast"
)

func typeCheckProgram(a *ast.Program) error {
	for _, n := range a.Namespace {
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
	for _, n := range proc.Namespace {
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
	for _, edge := range proc.Tree {
		for _, call := range edge.Calls() {
			err := typeCheckCall(&call)
			if err != nil {
				return err
			}
		}
	}
	err := typeCheckTree(proc.Tree, proc.Entry, proc.Ret(), make(map[ast.Node]bool))
	if err != nil {
		return err
	}
	return nil
}

func typeCheckTree(tree map[ast.Node]ast.Edge, entry ast.Node, terminal ast.Type, visited map[ast.Node]bool) error {
	if _, ok := visited[entry]; ok {
		return nil
	}
	visited[entry] = true
	calls, ok := tree[entry]
	if !ok {
		if !entry.Ret().Match(terminal) {
			return fmt.Errorf("terminal node %s does not match return type %s for parent process", entry, terminal)
		}
		return nil
	} else {
		for _, call := range calls.Calls() {
			err := typeCheckTree(tree, call.Target, terminal, visited)
			if err != nil {
				return err
			}
		}
	}
	return nil
}

func typeCheckCall(call *ast.Call) error {
	node_args := call.Args
	if len(node_args) != len(call.Args) {
		return fmt.Errorf("found %d args, expected %d", len(node_args), len(call.Args))
	}
	for i, a := range call.Args {
		if !node_args[i].Type.Match(a.Type) {
			return fmt.Errorf("arg %s does not match type: found %s, expected %s for node %s", a.Path, a.Type, node_args[i], call.Target)
		}
	}
	return nil
}
