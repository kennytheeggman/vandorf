package app

import (
	"context"
	"fmt"
	"strings"
	"sync/atomic"
)

// Range returns 0..i-1. Its return type is the structural Go form of
// { l: []int, s: string }.
func Range(ctx context.Context, i int) (struct {
	L []int
	S string
}, error) {
	l := make([]int, i)
	for j := range l {
		l[j] = j
	}
	return struct {
		L []int
		S string
	}{L: l, S: fmt.Sprint(i)}, nil
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

type End = struct {
	Tag string
	I   string
	S   struct{}
	B   struct{ I int }
}

func Finish(ctx context.Context, r End) (string, error) {
	return "end:" + r.Tag, nil
}
