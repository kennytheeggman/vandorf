package document

import (
	"fmt"

	"github.com/kennytheeggman/vandorf/lib/compiler/ast"
)

func translateProgram(syntax_tree *ast.Program) (Document, error) {
	doc := Document{
		Grids: make(map[string]Grid),
	}
	for _, p := range syntax_tree.Procs {
		name := p.Name()
		grid, err := translateProc(&p)
		if err != nil {
			return Document{}, err
		}
		doc.Grids[name] = grid
	}
	return doc, nil
}

func printArgs(args map[string]ast.Ident) string {
	var s []string
	for k, v := range args {
		s = append(s, fmt.Sprintf("%s: %s", k, v.Type))
	}
	return fmt.Sprintln(s)
}

func translateProc(proc *ast.Proc) (Grid, error) {
	entry, err := translateNode(proc.Entry)
	if err != nil {
		return Grid{}, err
	}
	grid := Grid{Rows: []Row{Row{Nodes: []Node{entry}}}}
	table := make(map[Node]ast.Node)
	table[grid.Rows[0].Nodes[0]] = proc.Entry
	visited := make(map[ast.Node][2]int)
	visited[proc.Entry] = [2]int{0, 0}
	// TODO: add ancestor check, and if not ancestor, pushdown
	// incoming := make(map[ast.Node]map[ast.Node]bool)
	// incoming[proc.Entry] = make(map[ast.Node]bool)
	any_left := func() bool {
		last := grid.Rows[len(grid.Rows)-1]
		any_next := false
		for i, _ := range last.Nodes {
			if _, ok := proc.Tree[table[last.Nodes[i]]]; ok {
				any_next = true
			}
		}
		return any_next
	}
	for {
		if !any_left() {
			break
		}
		last := grid.Rows[len(grid.Rows)-1]
		current := Row{}
		for i, _ := range last.Nodes {
			if c, ok := proc.Tree[table[last.Nodes[i]]]; ok {
				calls := c.Calls()
				for _, call := range calls {
					var t EdgeType
					if _, ok := visited[call.Target]; !ok {
						visited[call.Target] = [2]int{len(grid.Rows), len(current.Nodes)}
						node, err := translateNode(call.Target)
						if err != nil {
							return Grid{}, err
						}
						current.Nodes = append(current.Nodes, node)
						table[current.Nodes[len(current.Nodes)-1]] = call.Target
						t = Call
					} else {
						t = Loop
					}
					edge := Edge{Type: t, Source: [2]int{len(grid.Rows)-1, i}, Target: visited[call.Target]}
					grid.Edges = append(grid.Edges, edge)
				}
			}
		}
		grid.Rows = append(grid.Rows, current)
	}
	return grid, nil
}

func translateNode(node ast.Node) (Node, error) {
	switch n := node.(type) {
	case *ast.Func:
		return Node{Type: Func, Label: n.Name()}, nil
	case *ast.Proc:
		return Node{Type: Proc, Label: n.Name()}, nil
	default:
		return Node{}, fmt.Errorf("unreachable invalid node")
	}
}
