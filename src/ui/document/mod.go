package document

import "github.com/kennytheeggman/vandorf/lib/compiler/ast"

type Document struct {
	Grids map[string]Grid `json:"grids"`
}

type Grid struct {
	Rows []Row `json:"rows"`
	Edges []Edge `json:"edges"`
}

type Row struct {
	Nodes []Node `json:"nodes"`
}

type NodeType int
const (
	Func NodeType = iota
	Proc
	Ellipsis
)

type Node struct {
	Type NodeType `json:"type"`
	Label string `json:"label"`
	SubLabel *string `json:"sublabel,omitempty"`
};

type EdgeType int
const (
	Call EdgeType = iota
	Match
	Loop
)

type Edge struct {
	Type EdgeType `json:"type"`
	Source [2]int `json:"source"`
	Target [2]int `json:"target"`
	Label *string `json:"label,omitempty"`
}

func Translate(p *ast.Program) (Document, error) {
	return translateProgram(p)
}
