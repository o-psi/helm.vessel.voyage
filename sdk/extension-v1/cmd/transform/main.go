package main

import (
	"context"
	_ "embed"
	"encoding/json"
	"errors"
	"os"
	"strings"
	"voyage.example/extension-v1/sdk"
)

//go:embed definitions.json
var definitions []byte

func main() {
	err := sdk.Serve(definitions, []string{"execute"}, func(ctx context.Context, c *sdk.Call, kind, name string, args json.RawMessage) (any, error) {
		if kind != "tool" || name != "uppercase" {
			return nil, errors.New("unsupported")
		}
		var input struct {
			Text string `json:"text"`
		}
		if err := json.Unmarshal(args, &input); err != nil {
			return nil, err
		}
		if err := ctx.Err(); err != nil {
			return nil, err
		}
		if err := c.Progress("Transforming text"); err != nil {
			return nil, err
		}
		return map[string]string{"text": strings.ToUpper(input.Text)}, nil
	})
	if err != nil {
		os.Exit(1)
	}
}
