package parser

/** Identifier grammar rules **/

type Ident struct {
	Path []string `parser:"(@Ident '.')* @Ident" json:"path"`
}

/** Program grammar rules **/

type Program struct {
	Decls []TopLevelDecl `parser:"@@*" json:"decls"`
}

type TopLevelDecl struct {
	Interface *InterfaceDecl `parser:"@@" json:"interface,omitempty"`
	Node *TopLevelNodeDecl `parser:"| @@" json:"node,omitempty"`
}

type InterfaceDecl struct {
	Name string `parser:"'interface' @Ident" json:"name"`
	Funcs []FuncDecl `parser:"'{' (@@ ';')* '}'" json:"funcs"`
}

type TopLevelNodeDecl struct {
	Private bool `parser:"'private'?" json:"access"`
	Node NodeDecl `parser:"@@" json:"node"`
}

type NodeDecl struct {
	Func *FuncDecl `parser:"@@" json:"func,omitempty"`
	Proc *ProcDecl `parser:"| @@" json:"proc,omitempty"`
}

type FuncDecl struct {
	Name string `parser:"'func' @Ident '('" json:"name"`
	Args []BoundType `parser:"@@ (',' @@)* ')'" json:"args"`
	Ret  *TypeDef `parser:"(':' @@)?" json:"ret"`
}

type ProcDecl struct {
	Name string `parser:"'proc' @Ident '('" json:"name"`
	Entry string `parser:"@Ident ')' '{'" json:"entry"`
	Nodes []NodeDecl `parser:"(@@ ';')*" json:"nodes"`
	Links []LinkDecl `parser:"(@@ ';')* '}'" json:"links"`
}

/** Graph grammar rules **/

type LinkDecl struct {
	Source Ident `parser:"@@" json:"source"`
	Edge EdgeDecl `parser:"@@" json:"edge"`
}

type EdgeDecl struct {
	Edge *EdgeCallDecl `parser:"@@" json:"edge,omitempty"`
	Match *EdgeMatchDecl `parser:"| @@" json:"match,omitempty"`
	Collect *EdgeCollectDecl `parser:"| @@" json:"collect,omitempty"`
	ForEach *EdgeForEachDecl `parser:"| @@" json:"foreach,omitempty"`
}	

type EdgeForEachDecl struct {
	Input Ident `parser:"'for' 'each' @@" json:"input"`
	Name string `parser:"'(' @Ident ')' '-' '>'" json:"dim"`
	Sink Ident `parser:"@@" json:"sink"`
	Args []Ident `parser:"'(' @@ (',' @@)* ')'" json:"args"`
	Alias *string `parser:"('as' @Ident)?" json:"alias,omitempty"`
}

type EdgeCollectDecl struct {
	Name string `parser:"'collect' 'each' '(' @Ident ')' '-' '>'" json:"dim"`
	Sink string `parser:"@Ident" json:"sink"`
	Args []Ident `parser:"'(' @@ (',' @@)* ')'" json:"args"`
	Alias *string `parser:"('as' @Ident)?" json:"alias,omitempty"`
}

type EdgeMatchDecl struct {
	Arg Ident `parser:"'match' '(' @@ ')' '{'" json:"arg"`
	Paths []EdgeMatchPathDecl `parser:"@@ (',' @@)* ','?" json:"paths"`
	Alias *string `parser:"('as' @Ident)? '}'" json:"alias,omitempty"`
}

type EdgeMatchPathDecl struct {
	Cond string `parser:"'(' @String ')' '-' '>'" json:"cond"`
	Sink Ident `parser:"@@" json:"sink"`
	Args []Ident `parser:"'(' @@ (',' @@)* ')'" json:"args"`
	Alias *string `parser:"('as' @Ident)?" json:"alias,omitempty"`
}

type EdgeCallDecl struct {
	Sink Ident `parser:"'-' '>' @@" json:"sink"`
	Args []Ident `parser:"'(' @@ (',' @@)* ')'" json:"args"`
	Alias *string `parser:"('as' @Ident)?" json:"alias,omitempty"`
}

/** Type-related grammar rules **/

type TypeDecl struct {
	Name string `parser:"type @Ident" json:"name"`
	Def TypeDef `parser:"@@" json:"def"`
}

type TypeDef struct {
	Array bool `parser:"('[' ']')?" json:"array"`
	Type TypeDefType `parser:"@@" json:"type"`
}

type TypeDefType struct {
	Enum *EnumDef `parser:"@@" json:"enum,omitempty"`
	Struct *StructDef `parser:"| @@" json:"struct,omitempty"`
	Name *string `parser:"| @Ident" json:"name,omitempty"`
}

type BoundType struct {
	Name string `parser:"@Ident ':'" json:"name"`
	Type TypeDef `parser:"@@" json:"type"`
}

type StructDef struct {
	Defs []BoundType `parser:"'{' @@ (',' @@)* ','? '}'" json:"defs"`
}

type EnumDef struct {
	Enums []EnumOptDef `parser:"'enum' '{' @@ (',' @@)* ','? '}'" json:"enums"`
}

type EnumOptDef struct {
	Name string `parser:"@Ident" json:"name"`
	Struct *StructDef `parser:"('(' @@ ')')?" json:"struct,omitempty"`
}
