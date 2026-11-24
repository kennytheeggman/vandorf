package effects

import "context"

// Counts the characters of s. Implemented outside the generated
// package/module.
func Call(ctx context.Context, s string) (int, error) {
	return len(s), nil
}
