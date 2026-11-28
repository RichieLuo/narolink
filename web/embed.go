package web

import (
	"embed"
	"io/fs"
)

//go:embed public
var assets embed.FS

// Assets returns the browser workspace files, rooted at the public directory.
func Assets() (fs.FS, error) {
	return fs.Sub(assets, "public")
}
