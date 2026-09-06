package main

import (
	"fmt"

	"github.com/kennytheeggman/vandorf/lib/compiler"
)

func main() {
	pst, err := compiler.ParseFile("test/fixtures/test1.sys")
	if err != nil {
		panic(err)
	}
	ast, err := compiler.Translate(pst)
	if err != nil {
		panic(err)
	}
	fmt.Printf("%v", ast)
}
