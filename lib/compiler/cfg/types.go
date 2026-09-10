package cfg

/** Type-related grammar rules **/

type Type struct {
	Array *ArrayType `parser:"@@" json:"array,omitempty"`
	Enum *EnumType `parser:"| @@" json:"enum,omitempty"`
	Struct *StructType `parser:"| @@" json:"struct,omitempty"`
	Name *PrimitiveType `parser:"| @@" json:"name,omitempty"`
}

type PrimitiveType struct {
	Name string `parser:"@Ident" json:"name"`
}

type ArrayType struct {
	Type Type `parser:"'[' ']' @@" json:"array"`
}

type StructType struct {
	Defs []StructTypeField `parser:"'{' @@ (',' @@)* ','? '}'" json:"defs"`
}

type StructTypeField struct {
	Name string `parser:"@Ident ':'" json:"name"`
	Type Type `parser:"@@" json:"type"`
}

type EnumType struct {
	Enums []EnumTypeOpt `parser:"'enum' '{' @@ (',' @@)* ','? '}'" json:"enums"`
}

type EnumTypeOpt struct {
	Name string `parser:"@Ident" json:"name"`
	Type *Type `parser:"('(' @@ ')')?" json:"struct,omitempty"`
}
