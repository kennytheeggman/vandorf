package effects

import "context"

func Call(ctx context.Context, s string) (int, error) {
	return len(s), nil
}
