package ast

type Item interface {
	Name() string
}

type Program struct {
	Interfaces []Interface
	Funcs []Func
	Procs []Proc
	Namespace map[string]Item
}

type Interface struct {
	name string
	Funcs []Func
	Namespace map[string]Node
}
func (i Interface) Name() string {
	return i.name
}
func NewInterface(name string) Interface {
	return Interface{name: name, Namespace: make(map[string]Node)}
}

type Node interface {
	Item
	Args() map[string]Type
	Ret() Type
}

type Func struct {
	name string
	args map[string]Type
	ret Type
}
func (f Func) Name() string {
	return f.name
}
func (f Func) Args() map[string]Type {
	return f.args
}
func (f Func) Ret() Type {
	return f.ret
}
func NewFunc(name string, args map[string]Type, ret Type) Func {
	return Func{name: name, args: args, ret: ret}
}

type Proc struct {
	name string 
	ret Type
	Funcs []Func
	Procs []Proc
	Entry Node
	Tree map[Node]Edge
	Namespace map[string]Node
}
func (p Proc) Name() string {
	return p.name
}
func (p Proc) Args() map[string]Type {
	return p.Entry.Args()
}
func (p Proc) Ret() Type {
	return p.ret
}
func NewProc(name string, ret Type) Proc {
	return Proc{name: name, ret: ret}
}
