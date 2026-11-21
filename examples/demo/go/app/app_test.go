package app

import (
	"context"
	"strings"
	"testing"

	"example.com/demo/app/types"
)

func TestForEach(t *testing.T) {
	// sum of squares of 0..4
	got, err := RunForEach(context.Background(), 5)
	if err != nil || got != 30 {
		t.Fatalf("RunForEach(5) = %v, %v; want 30", got, err)
	}
}

func TestBar(t *testing.T) {
	grown.Store(0)
	Logged = nil
	// Start: "ab"; Loop: 2+3=5 -> loops, 2+6=8 -> loops, 2+9=11 -> ForEach(11)
	// ForEach(11) = 385 -> Branch2Begin "sum=385" -> Foo.Call 7 -> Branch2End -> End(S)
	got, err := RunBar(context.Background(), 1, "ab")
	if err != nil || got != "end:S" {
		t.Fatalf("RunBar = %q, %v; want end:S", got, err)
	}
	if len(Logged) != 1 || Logged[0] != 7 {
		t.Fatalf("Branch2End saw %v, want [7]", Logged)
	}
}

func TestForEachEmpty(t *testing.T) {
	got, err := RunForEach(context.Background(), 0)
	if err != nil || got != 0 {
		t.Fatalf("RunForEach(0) = %v, %v", got, err)
	}
}

func TestStub(t *testing.T) {
	_, err := unimplementedBarBranch1(context.Background(), 0)
	if err == nil || !strings.Contains(err.Error(), "not implemented") {
		t.Fatalf("got %v", err)
	}
}

func TestCalc(t *testing.T) {
	got, err := RunCalc(context.Background(), 3)
	if err != nil || got != 115 {
		t.Fatalf("RunCalc(3) = %v, %v; want 115", got, err)
	}
}

func TestClassify(t *testing.T) {
	lit := func(i int) *types.Expr { return &types.Expr{Tag: "Lit", Lit: i} }
	neg := func(e *types.Expr) *types.Expr { return &types.Expr{Tag: "Neg", Neg: e} }
	add := func(l, r *types.Expr) *types.Expr { return &types.Expr{Tag: "Add", Add: &types.ExprAdd{L: l, R: r}} }
	for _, c := range []struct {
		e    *types.Expr
		want string
	}{
		{lit(4), "small even"},
		{lit(3), "literal"},
		{lit(400), "literal"},
		{neg(lit(1)), "negated literal"},
		{neg(neg(lit(1))), "minus negated literal"},
		{neg(neg(neg(lit(1)))), "minus minus negated literal"},
		{add(lit(1), lit(2)), "sum of literals"},
		{add(lit(1), neg(lit(2))), "something else"},
	} {
		got, err := RunClassify(context.Background(), *c.e)
		if err != nil || got != c.want {
			t.Errorf("RunClassify(%+v) = %q, %v; want %q", *c.e, got, err, c.want)
		}
	}
}
