package ast

import "fmt"

type Type interface {
	Match(other Type) bool // Can other coerce into this type?
}

type Array struct {
	Type Type
}

func (a Array) String() string {
	return fmt.Sprintf("[]%s", a.Type)
}

func (a Array) Match(other Type) bool {
	switch other := other.(type) {
	case Array:
		return a.Type.Match(other.Type)
	default:
		return false
	}
}

type Enum struct {
	Enums map[string]Type
}

func (e Enum) String() string {
	return fmt.Sprintf("{ %v }", e.Enums)
}

func (e Enum) Match(other Type) bool {
	switch other := other.(type) {
	case Enum:
		for k, v := range other.Enums {
			if _, ok := e.Enums[k]; !ok {
				return false
			}
			if !e.Enums[k].Match(v) {
				return false
			}
		}
		return true
	default:
		for _, v := range e.Enums {
			if v.Match(other) {
				return true
			}
		}
		return false
	}
}

type Struct struct {
	Defs map[string]Type
}

func (s Struct) String() string {
	return fmt.Sprintf("{ %v }", s.Defs)
}

func (s Struct) Match(other Type) bool {
	switch other := other.(type) {
	case Struct:
		for k, v := range s.Defs {
			if _, ok := other.Defs[k]; !ok {
				return false
			}
			if !v.Match(other.Defs[k]) {
				return false
			}
		}
		for k := range other.Defs {
			if _, ok := s.Defs[k]; !ok {
				return false
			}
		}
		return true
	default:
		return false
	}
}

type Primitive struct {
	Name string
}

func (p Primitive) String() string {
	return p.Name
}

func (p Primitive) Match(other Type) bool {
	switch other := other.(type) {
	case Primitive:
		return p.Name == other.Name
	default:
		return false
	}
}
