package main

import (
	"context"
	"io"
	"net/http"
	"strings"
	"time"
)

const streamIOTimeout = 20 * time.Second

func runEventPath(path string) bool {
	rest, ok := strings.CutPrefix(path, "/api/runs/")
	id, route, found := strings.Cut(rest, "/")
	return ok && found && id != "" && route == "events"
}

func uploadStream(w http.ResponseWriter, req *http.Request, job *waiting) {
	if job.stream == nil || req.Header.Get("Content-Type") != "text/event-stream" {
		http.NotFound(w, req)
		return
	}
	reader, writer := io.Pipe()
	defer writer.Close()
	select {
	case job.stream <- reader:
	case <-job.context.Done():
		w.WriteHeader(http.StatusGone)
		return
	case <-req.Context().Done():
		return
	case <-time.After(5 * time.Second):
		w.WriteHeader(http.StatusConflict)
		return
	}
	controller := http.NewResponseController(w)
	stop := context.AfterFunc(job.context, func() { _ = controller.SetReadDeadline(time.Now()) })
	defer stop()
	buffer := make([]byte, 16*1024)
	for {
		_ = controller.SetReadDeadline(time.Now().Add(streamIOTimeout))
		_ = controller.SetWriteDeadline(time.Now().Add(streamIOTimeout))
		if job.context.Err() != nil {
			return
		}
		n, err := req.Body.Read(buffer)
		if n > 0 {
			if _, problem := writer.Write(buffer[:n]); problem != nil {
				return
			}
		}
		if err != nil {
			if err == io.EOF {
				w.WriteHeader(http.StatusNoContent)
			}
			return
		}
	}
}

func serveStream(w http.ResponseWriter, req *http.Request, reader *io.PipeReader) {
	defer reader.Close()
	stop := context.AfterFunc(req.Context(), func() { _ = reader.Close() })
	defer stop()
	controller := http.NewResponseController(w)
	_ = controller.SetWriteDeadline(time.Now().Add(streamIOTimeout))
	w.Header().Set("Content-Type", "text/event-stream")
	w.Header().Set("X-Accel-Buffering", "no")
	w.WriteHeader(http.StatusOK)
	if controller.Flush() != nil {
		return
	}
	buffer := make([]byte, 16*1024)
	for {
		n, err := reader.Read(buffer)
		if n > 0 {
			_ = controller.SetWriteDeadline(time.Now().Add(streamIOTimeout))
			if _, problem := w.Write(buffer[:n]); problem != nil || controller.Flush() != nil {
				return
			}
		}
		if err != nil {
			return
		}
	}
}
