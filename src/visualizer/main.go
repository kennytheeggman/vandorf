package main

import (
	"embed"
	"encoding/json"
	"flag"
	"fmt"
	"io"
	"io/fs"
	"log"
	"net/http"
	"os"
	"path/filepath"
	"sort"
	"strings"

	"github.com/kennytheeggman/vandorf/lib/compiler"
	"github.com/kennytheeggman/vandorf/src/ui/document"
)

//go:embed static
var staticFS embed.FS

func main() {
	addr := flag.String("addr", ":8080", "HTTP listen address")
	dir := flag.String("dir", ".", "base directory that source file paths are resolved against")
	flag.Parse()

	mux := http.NewServeMux()
	mux.HandleFunc("GET /api/sources", handleSources(*dir))
	mux.HandleFunc("POST /api/compile", handleCompile(*dir))

	sub, err := fs.Sub(staticFS, "static")
	if err != nil {
		log.Fatal(err)
	}
	mux.Handle("/", http.FileServer(http.FS(sub)))

	log.Printf("vandorf visualizer listening on http://localhost%s (source dir: %s)", *addr, *dir)
	log.Fatal(http.ListenAndServe(*addr, mux))
}

type compileRequest struct {
	Path   string `json:"path"`
	Source string `json:"source"`
}

func handleCompile(base string) http.HandlerFunc {
	return func(w http.ResponseWriter, r *http.Request) {
		var req compileRequest
		body, err := io.ReadAll(io.LimitReader(r.Body, 1<<20))
		if err != nil {
			writeErr(w, 400, fmt.Sprintf("bad request: %v", err))
			return
		}
		if len(body) > 0 {
			if err := json.Unmarshal(body, &req); err != nil {
				writeErr(w, 400, fmt.Sprintf("bad request: %v", err))
				return
			}
		}

		src := req.Source
		name := "<inline>"
		if src == "" {
			if req.Path == "" {
				writeErr(w, 400, `provide either "path" or "source"`)
				return
			}
			p, err := resolvePath(base, req.Path)
			if err != nil {
				writeErr(w, 400, err.Error())
				return
			}
			b, err := os.ReadFile(p)
			if err != nil {
				writeErr(w, 400, fmt.Sprintf("cannot read %s: %v", req.Path, err))
				return
			}
			src = string(b)
			name = req.Path
		}

		doc, err := compileSource(src)
		if err != nil {
			writeErr(w, 400, err.Error())
			return
		}
		writeJSON(w, 200, map[string]any{
			"ok":       true,
			"name":     name,
			"grids":    len(doc.Grids),
			"document": doc,
		})
	}
}

func compileSource(src string) (document.Document, error) {
	parseTree, err := compiler.Parse(src)
	if err != nil {
		return document.Document{}, fmt.Errorf("parse error: %v", err)
	}
	syntaxTree, err := compiler.Translate(parseTree)
	if err != nil {
		return document.Document{}, fmt.Errorf("translate error: %v", err)
	}
	if err := compiler.TypeCheck(&syntaxTree); err != nil {
		return document.Document{}, fmt.Errorf("type error: %v", err)
	}
	doc, err := document.Translate(&syntaxTree)
	if err != nil {
		return document.Document{}, fmt.Errorf("document error: %v", err)
	}
	return doc, nil
}

func handleSources(base string) http.HandlerFunc {
	return func(w http.ResponseWriter, r *http.Request) {
		files, err := listSources(base)
		if err != nil {
			writeErr(w, 400, fmt.Sprintf("cannot list sources: %v", err))
			return
		}
		writeJSON(w, 200, map[string]any{"ok": true, "root": base, "files": files})
	}
}

func listSources(base string) ([]string, error) {
	var files []string
	err := filepath.WalkDir(base, func(p string, d fs.DirEntry, err error) error {
		if err != nil {
			return err
		}
		if d.IsDir() {
			return nil
		}
		if strings.EqualFold(filepath.Ext(d.Name()), ".sys") {
			rel, err := filepath.Rel(base, p)
			if err != nil {
				return err
			}
			files = append(files, filepath.ToSlash(rel))
		}
		return nil
	})
	if err != nil {
		return nil, err
	}
	sort.Strings(files)
	return files, nil
}

func resolvePath(base, p string) (string, error) {
	if p == "" {
		return "", fmt.Errorf("empty path")
	}
	if strings.ContainsRune(p, 0) {
		return "", fmt.Errorf("invalid path")
	}
	baseAbs, err := filepath.Abs(base)
	if err != nil {
		return "", err
	}
	full := filepath.Clean(filepath.FromSlash(p))
	if !filepath.IsAbs(full) {
		full = filepath.Join(baseAbs, full)
	}
	rel, err := filepath.Rel(baseAbs, full)
	if err != nil {
		return "", fmt.Errorf("invalid path: %s", p)
	}
	if rel == ".." || strings.HasPrefix(rel, ".."+string(filepath.Separator)) {
		return "", fmt.Errorf("path escapes source directory: %s", p)
	}
	return full, nil
}

func writeJSON(w http.ResponseWriter, status int, v any) {
	w.Header().Set("Content-Type", "application/json")
	w.WriteHeader(status)
	_ = json.NewEncoder(w).Encode(v)
}

func writeErr(w http.ResponseWriter, status int, msg string) {
	writeJSON(w, status, map[string]any{"ok": false, "error": msg})
}
