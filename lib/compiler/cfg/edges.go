package cfg

/** Graph grammar rules **/

type Edge struct {
	Call *CallEdge `parser:"@@" json:"call,omitempty"`
	Match *MatchEdge `parser:"| @@" json:"match,omitempty"`
	Collect *CollectEdge `parser:"| @@" json:"collect,omitempty"`
	ForEach *ForEachEdge `parser:"| @@" json:"foreach,omitempty"`
}

type KVIdent struct {
	Key string `parser:"@Ident '='" json:"key"`
	Val Arg `parser:"@@" json:"val"`
}

type Arg struct {
	Enum *EnumIdent `parser:"@@" json:"enum,omitempty"`
	Ident *Ident `parser:"| @@" json:"ident,omitempty"`
}

type EnumIdent struct {
	Symbol string `parser:"@Ident" json:"symbol"`
	Type Ident `parser:"'(' @@ ')'" json:"type"`
}

type CallEdge struct {
	Source Ident `parser:"@@ '-' '>'" json:"source"`
	Target Ident `parser:"@@" json:"target"`
	Args []KVIdent `parser:"'(' @@ (',' @@)* ')'" json:"args"`
	Alias *string `parser:"('as' @Ident)?" json:"alias,omitempty"`
}

type MatchEdge struct {
	Source Ident `parser:"@@ 'switch'" json:"source"`
	Paths []MatchEdgePath `parser:"'{' @@ (',' @@)* ','? '}'" json:"paths"`
	Alias *string `parser:"('as' @Ident)?" json:"alias,omitempty"`
}

type MatchEdgePath struct {
	Cond string `parser:"'(' @String ')' '-' '>'" json:"cond"`
	Target Ident `parser:"@@" json:"sink"`
	Args []KVIdent `parser:"'(' @@ (',' @@)* ')'" json:"args"`
	Alias *string `parser:"('as' @Ident)?" json:"alias,omitempty"`
}

type CollectEdge struct {
	Source Ident `parser:"@@ 'collect'" json:"source"`
	Dim string `parser:"'each' '(' @Ident ')' '-' '>'" json:"dim"`
	Target Ident `parser:"@@" json:"sink"`
	Args []KVIdent `parser:"'(' @@ (',' @@)* ')'" json:"args"`
	Alias *string `parser:"('as' @Ident)?" json:"alias,omitempty"`
}

type ForEachEdge struct {
	Source Ident `parser:"@@ 'for' 'each'" json:"source"`
	Dim Ident `parser:"'(' @@ ')' '-' '>'" json:"dim"`
	Target Ident `parser:"@@" json:"sink"`
	Args []KVIdent `parser:"'(' @@ (',' @@)* ')'" json:"args"`
	Alias *string `parser:"('as' @Ident)?" json:"alias,omitempty"`
}
