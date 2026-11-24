package effects

import "context"

// Implemented outside the generated package/module.
func Call(ctx context.Context, s string) (int, error) {
	return len(s), nil
}
