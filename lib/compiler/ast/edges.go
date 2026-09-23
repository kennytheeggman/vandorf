package ast

type Edge interface {
	Type() string
	Calls() []Call
}

type Call struct {
	Target Node
	Args map[string]Ident
}
func (c Call) Type() string {
	return "call"
}
func (c Call) Calls() []Call {
	return []Call{c};
}

type Match struct {
	Paths map[string]Call
}
func (m Match) Type() string {
	return "match"
}
func (m Match) Calls() []Call {
	var nodes []Call;
	for _, c := range m.Paths {
		nodes = append(nodes, c.Calls()...)
	}
	return nodes
}

type Collect struct {
	Call Call
	Dim Ident
}
func (c Collect) Type() string {
	return "collect"
}
func (c Collect) Calls() []Call {
	return c.Call.Calls();
}

type ForEach struct {
	Call Call
	Dim Ident
}
func (f ForEach) Type() string {
	return "foreach"
}
func (f ForEach) Calls() []Call {
	return f.Call.Calls();
}
