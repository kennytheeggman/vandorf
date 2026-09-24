package document

import (
	"fmt"
	"maps"

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

type unresolvedEdge struct {
	Type EdgeType
	Target ast.Node 
	Label *string
}

func translateProc(proc *ast.Proc) (Grid, error) {
	entry, err := translateNode(proc.Entry)
	if err != nil {
		return Grid{}, err
	}
	grid := Grid{Rows: []Row{Row{Nodes: []Node{entry}}}}
	edges := make(map[[2]int][]unresolvedEdge)
	table := make(map[Node]ast.Node)
	table[grid.Rows[0].Nodes[0]] = proc.Entry
	visited := make(map[ast.Node][2]int)
	visited[proc.Entry] = [2]int{0, 0}
	ancestors := make(map[Node]map[Node]bool)
	ancestors[entry] = make(map[Node]bool)
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
	var for_stack []ast.Ident
	for {
		if !any_left() {
			break
		}
		last := grid.Rows[len(grid.Rows)-1]
		current := Row{}
		for i, _ := range last.Nodes {
			if c, ok := proc.Tree[table[last.Nodes[i]]]; ok {
				var t EdgeType
				switch e := c.(type) {
				case ast.Call:
					if len(for_stack) != 0 {
						t = ForEachContinue
					} else {
						t = Call
					}
				case ast.Match:
					t = Match
				case ast.ForEach:
					for_stack = append(for_stack, e.Dim)
					t = ForEachExpand
				case ast.Collect:
					if len(for_stack) == 0 {
						return Grid{}, fmt.Errorf("cannot collect without for_each")
					}
					for_stack = for_stack[:len(for_stack)-1]
					t = ForEachCollect
				}
				calls := c.Calls()
				for _, call := range calls {
					if _, ok := visited[call.Target]; !ok {
						visited[call.Target] = [2]int{len(grid.Rows), len(current.Nodes)}
						node, err := translateNode(call.Target)
						if err != nil {
							return Grid{}, err
						}
						current.Nodes = append(current.Nodes, node)
						table[current.Nodes[len(current.Nodes)-1]] = call.Target
						if _, ok := ancestors[node]; !ok {
							ancestors[node] = make(map[Node]bool)
						}
						ancestors[node] = maps.Clone(ancestors[last.Nodes[i]])
						ancestors[node][last.Nodes[i]] = true
					} else {
						coords := visited[call.Target]
						node := grid.Rows[coords[0]].Nodes[coords[1]]
						ancestors[node] = maps.Clone(ancestors[last.Nodes[i]])
						ancestors[node][last.Nodes[i]] = true
						if _, ok := ancestors[last.Nodes[i]][node]; !ok {
							// delete all nodes downstream of target and all edges outgoing from all downstream nodes
							stack := []ast.Node{call.Target}
							for {
								if len(stack) == 0 {
									break
								}
								current := stack[len(stack)-1]
								stack = stack[:len(stack)-1]
								current_coords := visited[current]
								if e, ok := edges[current_coords]; ok {
									for _, n := range e {
										stack = append(stack, n.Target)
									}
									edges[current_coords] = nil
								}
								delete(visited, current)
								for i := current_coords[1]+1; i < len(grid.Rows[current_coords[0]].Nodes); i++ {
									if _, ok := edges[[2]int{current_coords[0], i}]; ok {
										edges[[2]int{current_coords[0], i-1}] = edges[[2]int{current_coords[0], i}]
									}
									node := table[grid.Rows[current_coords[0]].Nodes[i]]
									visited[node] = [2]int{visited[node][0], visited[node][1]-1}
								}
								delete(edges, [2]int{current_coords[0], len(grid.Rows[current_coords[0]].Nodes)-1})
								grid.Rows[current_coords[0]].Nodes = append(grid.Rows[current_coords[0]].Nodes[:current_coords[1]], grid.Rows[current_coords[0]].Nodes[current_coords[1]+1:]...)
							}
							// add target to new position
							current.Nodes = append(current.Nodes, node)
							// update visited with new position
							visited[call.Target] = [2]int{len(grid.Rows), len(current.Nodes)-1}
						}
					}
					edge := unresolvedEdge{Type: t, Target: call.Target}
					edges[[2]int{len(grid.Rows)-1, i}] = append(edges[[2]int{len(grid.Rows)-1, i}], edge)
				}
			}
		}
		grid.Rows = append(grid.Rows, current)
	}
	for source, e := range edges {
		for _, edge := range e {
			grid.Edges = append(grid.Edges, Edge{Type: edge.Type, Source: source, Target: visited[edge.Target]})
		}
	}
	if len(for_stack) != 0 {
		return Grid{}, fmt.Errorf("mismatching collect/for_each")
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
