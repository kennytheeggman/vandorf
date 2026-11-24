package app

import (
	"context"
	"fmt"
	"strings"
	"sync/atomic"

	"example.com/demo/app/types"
)

// Lists 0..i-1, with i as a string.
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

func Finish(ctx context.Context, r types.Outcome) (string, error) {
	return "end:" + r.Tag, nil
}

// Passes Main's command-line arguments on to Bar, unchanged.
func Args(ctx context.Context, a int, b string) (types.BarArgs, error) {
	return types.BarArgs{A: a, B: b}, nil
}

// Builds the expression `base + 2`, of two literals.
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

func Look(ctx context.Context, e types.Expr) (types.Expr, error) { return e, nil }

// Checks whether |i| < 100.
func Small(ctx context.Context, i int) (bool, error) { return i > -100 && i < 100, nil }

// Gives each name its index as a slot.
func Declare(ctx context.Context, names []string, query string) (types.ResolveDeclareOut, error) {
	scope := types.Scope{}
	for i, n := range names {
		scope[n] = types.ScopeValue{Slot: i, Kind: "var"}
	}
	return types.ResolveDeclareOut{Scope: scope, Query: query}, nil
}

func Adjust(ctx context.Context, scope types.Scope, query string) (types.ResolveDeclareOut, error) {
	return types.ResolveDeclareOut{Scope: scope, Query: query}, nil
}

func Found(ctx context.Context, name string, slot int, kind string, total int) (string, error) {
	return fmt.Sprintf("%s: slot %d (%s) of %d", name, slot, kind, total), nil
}

func Missing(ctx context.Context, name string, known []string) (string, error) {
	return fmt.Sprintf("%s: not declared; known: %s", name, strings.Join(known, ", ")), nil
}

// ---- api.sys ----

func Shapes(ctx context.Context, shapes []types.Shape) ([]types.Shape, error) { return shapes, nil }

func Area(ctx context.Context, s types.Shape) (types.Measured, error) {
	switch s.Tag {
	case "Circle":
		return types.Measured{Kind: "circle", Area: 3.141592653589793 * s.Circle * s.Circle}, nil
	case "Rect":
		return types.Measured{Kind: "rect", Area: s.Rect.W * s.Rect.H}, nil
	default:
		return types.Measured{Kind: "dot"}, nil
	}
}

func Step(ctx context.Context, f types.Fold) (types.Fold, error) { return f, nil }

func Greet(ctx context.Context, name string, times int) (string, error) {
	return strings.TrimSpace(strings.Repeat("hello, "+name+"! ", times)), nil
}
