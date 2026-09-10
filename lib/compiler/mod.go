package compiler

import (
	"io"
	"os"

	"github.com/alecthomas/participle/v2"
	"github.com/kennytheeggman/vandorf/lib/compiler/ast"
	"github.com/kennytheeggman/vandorf/lib/compiler/cfg"
)

func Parse(s string) (*cfg.Program, error) {
	parser, err := participle.Build[cfg.Program]()
	if err != nil {
		return nil, err
	}
	parse_tree, err := parser.ParseString("", s)
	if err != nil {
		return nil, err
	}
	return parse_tree, nil
}

func Translate(parse_tree *cfg.Program) (ast.Program, error) {
	return translateProgram(parse_tree)
}

func TypeCheck(syntax_tree *ast.Program) error {
	return typeCheckProgram(syntax_tree)
}

func ParseFile(path string) (*cfg.Program, error) {
	f, err := os.Open(path)
	if err != nil {
		return nil, err
	}
	defer f.Close()
	b, err := io.ReadAll(f)
	if err != nil {
		return nil, err
	}
	return Parse(string(b))
}
