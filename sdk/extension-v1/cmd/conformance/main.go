// Offline hostile/lifecycle fixture, not an example of useful extension behavior.
// Build only in the explicitly admitted verification slot.
package main

import (
	"context"
	_ "embed"
	"encoding/json"
	"errors"
	"os"
	"strings"
	"syscall"
	"time"
	"voyage.example/extension-v1/sdk"
)

//go:embed definitions.json
var definitions []byte

func main() {
	if len(os.Args) > 1 && os.Args[1] == "--held-child" {
		for {
			time.Sleep(time.Second)
		}
	}
	err := sdk.Serve(definitions, []string{"execute"}, func(ctx context.Context, c *sdk.Call, kind, name string, args json.RawMessage) (any, error) {
		if kind == "lifecycle" && (name == "run_start" || name == "run_finish") {
			return map[string]any{"event": name}, nil
		}
		var input struct {
			Mode string `json:"mode"`
			Text string `json:"text"`
		}
		if err := json.Unmarshal(args, &input); err != nil {
			return nil, err
		}
		if kind == "command" && name == "echo_command" {
			return map[string]string{"text": input.Text}, nil
		}
		if kind != "tool" || name != "probe" {
			return nil, errors.New("unsupported")
		}
		switch input.Mode {
		case "echo":
			return map[string]string{"text": input.Text}, nil
		case "block":
			if err := c.Progress("Waiting for cancellation"); err != nil {
				return nil, err
			}
			<-ctx.Done()
			return nil, ctx.Err()
		case "crash":
			os.Exit(7)
		case "flood":
			_, err := os.Stdout.WriteString(strings.Repeat("x", 1024*1024+2))
			return nil, err
		case "held_child":
			_, err := os.StartProcess("/extension", []string{"/extension", "--held-child"}, &os.ProcAttr{Files: []*os.File{os.Stdin, os.Stdout, os.Stderr}})
			if err != nil {
				return nil, err
			}
			return map[string]bool{"child_started": true}, nil
		case "wrong_id":
			json.NewEncoder(os.Stdout).Encode(map[string]any{"type": "result", "invocation": "00000000-0000-0000-0000-000000000000", "value": map[string]bool{"wrong": true}})
			return map[string]bool{"bad": true}, nil
		case "duplicate":
			json.NewEncoder(os.Stdout).Encode(map[string]any{"type": "result", "invocation": c.InvocationID(), "value": map[string]bool{"first": true}})
			return map[string]bool{"duplicate": true}, nil
		case "host_capability":
			json.NewEncoder(os.Stdout).Encode(map[string]any{"type": "host.file.read", "invocation": c.InvocationID(), "request": 1, "path": "allowed.txt", "offset": 0, "max_bytes": 4096})
			<-ctx.Done()
			return nil, ctx.Err()
		case "isolation":
			_, fileErr := os.ReadFile("/etc/passwd")
			fd, netErr := syscall.Socket(syscall.AF_INET, syscall.SOCK_STREAM, 0)
			if netErr == nil {
				syscall.Close(fd)
			}
			_, sessionErr := syscall.Setsid()
			writeErr := os.WriteFile("/tmp/probe", []byte("private"), 0600)
			descriptorsClean := true
			entries, _ := os.ReadDir("/proc/self/fd")
			for _, entry := range entries {
				target, err := os.Readlink("/proc/self/fd/" + entry.Name())
				if err == nil && (strings.Contains(target, "memfd:") || strings.HasPrefix(target, "/home/") || strings.HasPrefix(target, "/tmp/vdr-")) {
					descriptorsClean = false
				}
			}
			_, runtimeErr := os.Stat("/usr/bin")
			return map[string]bool{"descriptors_private": descriptorsClean, "system_runtime_absent": runtimeErr != nil, "host_file_denied": fileErr != nil, "network_denied": netErr != nil,
				"session_escape_denied": sessionErr != nil, "environment_empty": os.Getenv("VOYAGE_EXTENSION_CANARY") == "",
				"private_tmp_writable": writeErr == nil}, nil
		}
		return nil, errors.New("unsupported")
	})
	if err != nil {
		os.Exit(1)
	}
}
