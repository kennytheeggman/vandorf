package main

import (
	"fmt"

	"github.com/kennytheeggman/vandorf/lib/compiler"
	"github.com/kennytheeggman/vandorf/src/ui/document"
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
	err = compiler.TypeCheck(&ast)
	if err != nil {
		panic(err)
	}
	// fmt.Printf("%v", ast)
	doc, err := document.Translate(&ast)
	if err != nil {
		panic(err)
	}
	fmt.Printf("%v", doc)
}
