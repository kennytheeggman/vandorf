package app

import (
	"context"
	"fmt"
	"strings"
	"sync/atomic"

	"example.com/demo/app/types"
)

// Range returns 0..i-1.
func Range(ctx context.Context, i int) (types.ForEachStartOut, error) {
	l := make([]int, i)
	for j := range l {
		l[j] = j
	}
	return types.ForEachStartOut{L: l, S: fmt.Sprint(i)}, nil
}

func Square(ctx context.Context, i int, l []int) (int, error) { return i * i, nil }

func Sum(ctx context.Context, l []int) (int, error) {
	n := 0
	for _, x := range l {
		n += x
	}
	return n, nil
}

func Repeat(ctx context.Context, a int, b string) (string, error) {
	return strings.Repeat(b, a), nil
}

// Grow is stateful so the `this < 10` loop terminates: each pass adds 3.
var grown atomic.Int64

func Grow(ctx context.Context, s string) (int, error) {
	return len(s) + int(grown.Add(3)), nil
}

func Describe(ctx context.Context, i int) (string, error) {
	return fmt.Sprintf("sum=%d", i), nil
}

var Logged []int

func Log(ctx context.Context, i int) error {
	Logged = append(Logged, i)
	return nil
}

func Finish(ctx context.Context, r types.BarEndR) (string, error) {
	return "end:" + r.Tag, nil
}

// Args passes Main's command-line arguments on to Bar.
func Args(ctx context.Context, a int, b string) (types.MainStartOut, error) {
	return types.MainStartOut{A: a, B: b}, nil
}

// Build returns base + 2.
func Build(ctx context.Context, base int) (types.Expr, error) {
	lit := func(i int) *types.Expr { return &types.Expr{Tag: "Lit", Lit: i} }
	return types.Expr{Tag: "Add", Add: &types.ExprAdd{L: lit(base), R: lit(2)}}, nil
}

func eval(e types.Expr) int {
	switch e.Tag {
	case "Lit":
		return e.Lit
	case "Neg":
		return -eval(*e.Neg)
	case "Add":
		return eval(*e.Add.L) + eval(*e.Add.R)
	}
	panic("bad tag " + e.Tag)
}

func Eval(ctx context.Context, e types.Expr, scale int) (int, error) {
	return eval(e) * scale, nil
}

func Total(ctx context.Context, xs []int) (int, error) {
	return Sum(ctx, xs)
}
