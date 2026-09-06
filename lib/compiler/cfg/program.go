package cfg

/** Program grammar rules **/

type Program struct {
	Interfaces []Interface `parser:"@@*" json:"decls"`
	Nodes []Node `parser:"@@*" json:"nodes"`
}

type Interface struct {
	Name string `parser:"'interface' @Ident" json:"name"`
	Funcs []Func `parser:"'{' (@@ ';')* '}'" json:"funcs"`
}

type Node struct {
	Func *Func `parser:"@@" json:"func,omitempty"`
	Proc *Proc `parser:"| @@" json:"proc,omitempty"`
}

type Func struct {
	Name string `parser:"'func' @Ident '('" json:"name"`
	Args []StructTypeField `parser:"@@ (',' @@)* ')'" json:"args"`
	Ret *Type `parser:"(':' @@)?" json:"ret"`
}

type Proc struct {
	Name string `parser:"'proc' @Ident" json:"name"`
	Entry string `parser:"'(' @Ident ')'" json:"entry"`
	Ret Type `parser:"':' @@ '{'" json:"ret"`
	Nodes []Node `parser:"(@@ ';')*" json:"nodes"`
	Links []Edge `parser:"(@@ ';')* '}'" json:"links"`
}
