package parser

import (
	"io"
	"os"

	"github.com/alecthomas/participle/v2"
)

func Parse(s string) (*Program, error) {
	parser, err := participle.Build[Program]()
	if err != nil {
		return nil, err
	}
	ast, err := parser.ParseString("", s)
	if err != nil {
		return nil, err
	}
	return ast, nil
}

func ParseFile(path string) (*Program, error) {
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
