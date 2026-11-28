package main

import (
	"strings"
	"time"
)

const streamIOTimeout = 20 * time.Second

func runEventPath(path string) bool {
	rest, ok := strings.CutPrefix(path, "/api/runs/")
	id, route, found := strings.Cut(rest, "/")
	return ok && found && id != "" && route == "events"
}
