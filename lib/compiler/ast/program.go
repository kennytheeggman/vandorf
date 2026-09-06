package ast

import "fmt"

type Program struct {
	Interfaces map[string]Interface
	Nodes map[string]Node
}

type Interface struct {
	Name string
	Funcs map[string]*Func
}

type Node interface {
	Name() (string, error)
	Args() (map[string]Type, error)
	Ret() (Type, error)
}


type Func struct {
	name string
	args map[string]Type
	ret Type
}

func (f *Func) String() string {
	return fmt.Sprintf("func %s(%s): %s", f.name, f.args, f.ret)
}

func NewFunc(name string, args map[string]Type, ret Type) Func {
	return Func{
		name: name,
		args: args,
		ret: ret,
	}
}

func (f Func) Name() (string, error) {
	return f.name, nil
}

func (f Func) Args() (map[string]Type, error) {
	return f.args, nil
}

func (f Func) Ret() (Type, error) {
	return f.ret, nil
}

type Proc struct {
	name string 
	ret Type
	Entry Node
	Nodes map[string]Node
	Tree map[Node][]Call 
	Alias map[string]*Call
}

func (p *Proc) String() string {
	name, err := p.Name()
	if err != nil {
		panic(err)
	}
	return fmt.Sprintf("proc %s(%v): %s { %v }", p.name, name, p.ret, p.Tree)
}

func NewProc(name string, ret Type, entry Node, nodes []Node) Proc {
	nodes_map := make(map[string]Node)
	for _, n := range nodes {
		name, err := n.Name()
		if err != nil {
			return Proc{}
		}
		nodes_map[name] = n
	}
	return Proc{
		name: name,
		ret: ret,
		Entry: entry,
		Nodes: nodes_map,
	}
}

func (p Proc) Name() (string, error) {
	return p.name, nil
}

func (p Proc) Args() (map[string]Type, error) {
	node := p.Entry
	if node == nil {
		return nil, fmt.Errorf("no entry node")
	}
	return node.Args()
}

func (p Proc) Ret() (Type, error) {
	return p.ret, nil
}

type Call struct {
	Target Node
	Args map[string]Ident
}

func (c Call) String() string {
	return fmt.Sprintf("%s(%v)", c.Target, c.Args)
}
