package main

import (
	"encoding/json"
	"fmt"

	"github.com/kennytheeggman/vandorf/lib/parser"
)

func main() {
	ast, err := parser.ParseFile("test/fixtures/test1.sys")
	if err != nil {
		panic(err)
	}
	bytes, err := json.Marshal(ast)
	fmt.Println(string(bytes))
}
