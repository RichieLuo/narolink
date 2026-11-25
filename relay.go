package main

import (
	"bytes"
	"context"
	"crypto/hmac"
	"crypto/rand"
	"crypto/sha256"
	"crypto/subtle"
	"embed"
	"encoding/base64"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"io"
	"io/fs"
	"log"
	"net"
	"net/http"
	"net/url"
	"os"
	"strconv"
	"strings"
	"sync"
	"time"
)

//go:embed public
var webAssets embed.FS

const (
	webLimit       = 18 << 20
	replyLimit     = 34 << 20
	cookieDuration = 30 * 24 * 60 * 60
	deviceCookie   = "narolink_remote"
)

type command struct {
	ID          string          `json:"id"`
	Method      string          `json:"method"`
	Path        string          `json:"path"`
	Origin      string          `json:"origin,omitempty"`
	ContentType string          `json:"content_type,omitempty"`
	Cookie      string          `json:"cookie,omitempty"`
	Body        json.RawMessage `json:"body,omitempty"`
}

type answer struct {
	Status      int    `json:"status"`
	ContentType string `json:"content_type,omitempty"`
	SetCookie   string `json:"set_cookie,omitempty"`
	Body        []byte `json:"body,omitempty"`
}

type waiting struct {
	work  command
	reply chan answer
}

type relay struct {
	origin  string
	host    string
	secure  bool
	token   [32]byte
	assets  fs.FS
	queue   chan string
	slots   chan struct{}
	uploads chan struct{}

	mu       sync.Mutex
	lastPoll time.Time
	waiting  map[string]*waiting
}

func main() {
	token := os.Getenv("NAROLINK_RELAY_TOKEN")
	if len(token) < 32 {
		log.Fatal("NAROLINK_RELAY_TOKEN must contain at least 32 characters")
	}
	gateway, err := newRelay(os.Getenv("NAROLINK_PUBLIC_URL"), token)
	if err != nil {
		log.Fatal(err)
	}
	address := os.Getenv("NAROLINK_RELAY_ADDR")
	if address == "" {
		address = "127.0.0.1:7319"
	}
	server := &http.Server{
		Addr: address, Handler: gateway, ReadHeaderTimeout: 6 * time.Second,
		ReadTimeout: 40 * time.Second, WriteTimeout: 45 * time.Second,
		IdleTimeout: 60 * time.Second, MaxHeaderBytes: 16 << 10,
	}
	cert, key := os.Getenv("NAROLINK_TLS_CERT"), os.Getenv("NAROLINK_TLS_KEY")
	log.Printf("NaroLink relay listening on %s for %s", address, gateway.origin)
	if cert != "" && key != "" {
		log.Fatal(server.ListenAndServeTLS(cert, key))
	}
	if cert != "" || key != "" {
		log.Fatal("Supply both NAROLINK_TLS_CERT and NAROLINK_TLS_KEY")
	}
	log.Fatal(server.ListenAndServe())
}

func newRelay(publicURL, token string) (*relay, error) {
	origin, err := url.Parse(publicURL)
	if err != nil || origin.Host == "" || origin.User != nil || (origin.Path != "" && origin.Path != "/") ||
		origin.RawQuery != "" || origin.Fragment != "" || origin.ForceQuery {
		return nil, fmt.Errorf("set NAROLINK_PUBLIC_URL to a standalone HTTPS origin")
	}
	secure := origin.Scheme == "https"
	if !secure && (origin.Scheme != "http" || !isLoopback(origin.Hostname())) {
		return nil, fmt.Errorf("a public relay requires HTTPS")
	}
	origin.Host = strings.ToLower(origin.Host)
	if secure && origin.Port() == "443" {
		origin.Host = strings.TrimSuffix(origin.Host, ":443")
	}
	assets, err := fs.Sub(webAssets, "public")
	if err != nil {
		return nil, err
	}
	return &relay{
		origin: origin.Scheme + "://" + origin.Host, host: origin.Host, secure: secure, token: sha256.Sum256([]byte(token)),
		assets: assets, queue: make(chan string, 32), slots: make(chan struct{}, 12), uploads: make(chan struct{}, 2),
		waiting: make(map[string]*waiting),
	}, nil
}

func isLoopback(host string) bool {
	address := net.ParseIP(host)
	return address != nil && address.IsLoopback()
}

func (r *relay) ServeHTTP(w http.ResponseWriter, req *http.Request) {
	if !strings.EqualFold(req.Host, r.host) {
		fail(w, http.StatusMisdirectedRequest, "此地址未配置")
		return
	}
	w.Header().Set("X-Content-Type-Options", "nosniff")
	w.Header().Set("Referrer-Policy", "no-referrer")
	w.Header().Set("Cross-Origin-Resource-Policy", "same-origin")
	w.Header().Set("Content-Security-Policy", "default-src 'none'; script-src 'self'; style-src 'self'; img-src 'self' blob:; connect-src 'self'; manifest-src 'self'; worker-src 'self'; base-uri 'none'; form-action 'self'; frame-ancestors 'none'")
	if r.secure {
		w.Header().Set("Strict-Transport-Security", "max-age=86400")
	}
	switch {
	case strings.HasPrefix(req.URL.Path, "/relay/"):
		w.Header().Set("Cache-Control", "no-store")
		r.desktop(w, req)
	case strings.HasPrefix(req.URL.Path, "/api/"):
		w.Header().Set("Cache-Control", "no-store")
		r.browser(w, req)
	default:
		r.static(w, req)
	}
}

func (r *relay) desktop(w http.ResponseWriter, req *http.Request) {
	bearer, ok := strings.CutPrefix(req.Header.Get("Authorization"), "Bearer ")
	hash := sha256.Sum256([]byte(bearer))
	if !ok || subtle.ConstantTimeCompare(hash[:], r.token[:]) != 1 {
		fail(w, http.StatusUnauthorized, "桌面连接密钥错误")
		return
	}
	if req.Method != http.MethodPost {
		w.WriteHeader(http.StatusMethodNotAllowed)
		return
	}
	if req.URL.Path == "/relay/hello" {
		r.mu.Lock()
		r.lastPoll = time.Now()
		r.mu.Unlock()
		w.WriteHeader(http.StatusNoContent)
		return
	}
	if req.URL.Path == "/relay/poll" {
		r.poll(w, req)
		return
	}
	id, ok := strings.CutPrefix(req.URL.Path, "/relay/answer/")
	if !ok || len(id) != 32 {
		w.WriteHeader(http.StatusNotFound)
		return
	}
	r.mu.Lock()
	job := r.waiting[id]
	r.mu.Unlock()
	if job == nil {
		w.WriteHeader(http.StatusNotFound)
		return
	}
	var result answer
	reader := http.MaxBytesReader(w, req.Body, replyLimit)
	if err := json.NewDecoder(reader).Decode(&result); err != nil || result.Status < 200 || result.Status > 599 {
		fail(w, http.StatusBadRequest, "桌面返回格式无效")
		return
	}
	select {
	case job.reply <- result:
	default:
	}
	w.WriteHeader(http.StatusNoContent)
}

func (r *relay) poll(w http.ResponseWriter, req *http.Request) {
	r.mu.Lock()
	r.lastPoll = time.Now()
	r.mu.Unlock()
	ctx, cancel := context.WithTimeout(req.Context(), 20*time.Second)
	defer cancel()
	for {
		select {
		case id := <-r.queue:
			r.mu.Lock()
			task := r.waiting[id]
			r.mu.Unlock()
			if task != nil {
				writeJSON(w, http.StatusOK, task.work)
				return
			}
		case <-ctx.Done():
			w.WriteHeader(http.StatusNoContent)
			return
		}
	}
}

func (r *relay) browser(w http.ResponseWriter, req *http.Request) {
	write := req.Method == http.MethodPost
	if !write && req.Method != http.MethodGet && req.Method != http.MethodHead {
		w.WriteHeader(http.StatusMethodNotAllowed)
		return
	}
	if write && req.Header.Get("Origin") != r.origin {
		fail(w, http.StatusForbidden, "请求来源不匹配")
		return
	}
	path := req.URL.Path
	pair := path == "/api/pair" && write
	inner, paired := r.unwrap(req)
	if path == "/api/access" && !write {
		writeJSON(w, 200, map[string]bool{"paired": paired, "online": r.online()})
		return
	}
	if !paired && !pair {
		fail(w, http.StatusUnauthorized, "先和电脑配对")
		return
	}
	if path == "/api/logout" && write {
		http.SetCookie(w, &http.Cookie{Name: deviceCookie, Path: "/", MaxAge: -1, HttpOnly: true, Secure: r.secure, SameSite: http.SameSiteStrictMode})
		w.WriteHeader(http.StatusNoContent)
		return
	}
	if !r.online() {
		fail(w, http.StatusServiceUnavailable, "电脑未连接。请打开 NaroLink。")
		return
	}
	select {
	case r.slots <- struct{}{}:
		defer func() { <-r.slots }()
	default:
		fail(w, http.StatusServiceUnavailable, "电脑请求较多，请稍后重试")
		return
	}
	limit := int64(4096)
	if path == "/api/runs" && write {
		limit = webLimit
		select {
		case r.uploads <- struct{}{}:
			defer func() { <-r.uploads }()
		default:
			fail(w, http.StatusServiceUnavailable, "其他消息正在上传")
			return
		}
	}
	var body json.RawMessage
	if write {
		data, err := io.ReadAll(http.MaxBytesReader(w, req.Body, limit))
		if err != nil {
			fail(w, http.StatusRequestEntityTooLarge, "传输内容过大")
			return
		}
		if !json.Valid(data) {
			fail(w, http.StatusBadRequest, "JSON 内容无效")
			return
		}
		body = data
	}
	var id [16]byte
	if _, err := rand.Read(id[:]); err != nil {
		fail(w, 500, "无法创建设备请求")
		return
	}
	key := hex.EncodeToString(id[:])
	work := command{ID: key, Method: req.Method, Path: req.URL.RequestURI(), Origin: req.Header.Get("Origin"),
		ContentType: req.Header.Get("Content-Type"), Body: body}
	if paired {
		work.Cookie = "narolink=" + inner
	}
	job := &waiting{work: work, reply: make(chan answer, 1)}
	r.mu.Lock()
	r.waiting[key] = job
	r.mu.Unlock()
	defer func() { r.mu.Lock(); delete(r.waiting, key); r.mu.Unlock() }()
	select {
	case r.queue <- key:
	default:
		fail(w, 503, "电脑请求较多，请稍后重试")
		return
	}
	ctx, cancel := context.WithTimeout(req.Context(), 33*time.Second)
	defer cancel()
	select {
	case result := <-job.reply:
		if pair && result.Status == http.StatusOK {
			cookie, err := http.ParseSetCookie(result.SetCookie)
			if err != nil || cookie.Name != "narolink" || cookie.Value == "" {
				fail(w, http.StatusBadGateway, "电脑没有完成配对")
				return
			}
			http.SetCookie(w, &http.Cookie{Name: deviceCookie, Value: r.wrap(cookie.Value), Path: "/", MaxAge: cookieDuration, HttpOnly: true, Secure: r.secure, SameSite: http.SameSiteStrictMode})
		}
		if result.ContentType != "" {
			w.Header().Set("Content-Type", result.ContentType)
		}
		w.WriteHeader(result.Status)
		_, _ = w.Write(result.Body)
	case <-ctx.Done():
		fail(w, http.StatusGatewayTimeout, "等待电脑响应超时")
	}
}

func (r *relay) online() bool {
	r.mu.Lock()
	defer r.mu.Unlock()
	return time.Since(r.lastPoll) < 45*time.Second
}

func (r *relay) sign(value []byte) []byte {
	tag := hmac.New(sha256.New, r.token[:])
	tag.Write([]byte("narolink/device/v1\x00"))
	tag.Write(value)
	return tag.Sum(nil)
}

func (r *relay) wrap(inner string) string {
	value := []byte(inner)
	return base64.RawURLEncoding.EncodeToString(append(value, r.sign(value)...))
}

func (r *relay) unwrap(req *http.Request) (string, bool) {
	cookie, err := req.Cookie(deviceCookie)
	if err != nil || len(cookie.Value) > 256 {
		return "", false
	}
	decoded, err := base64.RawURLEncoding.DecodeString(cookie.Value)
	if err != nil || len(decoded) <= sha256.Size {
		return "", false
	}
	value, tag := decoded[:len(decoded)-sha256.Size], decoded[len(decoded)-sha256.Size:]
	if !hmac.Equal(r.sign(value), tag) {
		return "", false
	}
	expires, _, active := strings.Cut(string(value), ".")
	deadline, err := strconv.ParseInt(expires, 10, 64)
	if !active || err != nil || deadline < time.Now().Unix() {
		return "", false
	}
	return string(value), true
}

func (r *relay) static(w http.ResponseWriter, req *http.Request) {
	if req.Method != http.MethodGet && req.Method != http.MethodHead {
		w.WriteHeader(405)
		return
	}
	name := strings.TrimPrefix(req.URL.Path, "/")
	if name == "" {
		name = "index.html"
	}
	contentTypes := map[string]string{
		"index.html": "text/html; charset=utf-8", "app.js": "text/javascript; charset=utf-8", "style.css": "text/css; charset=utf-8",
		"manifest.webmanifest": "application/manifest+json", "sw.js": "text/javascript; charset=utf-8",
		"icons/180.png": "image/png", "icons/192.png": "image/png", "icons/512.png": "image/png",
	}
	contentType, okay := contentTypes[name]
	if !okay {
		http.NotFound(w, req)
		return
	}
	content, err := fs.ReadFile(r.assets, name)
	if err != nil {
		http.NotFound(w, req)
		return
	}
	w.Header().Set("Content-Type", contentType)
	w.Header().Set("Cache-Control", "no-cache")
	http.ServeContent(w, req, name, time.Time{}, bytes.NewReader(content))
}

func fail(w http.ResponseWriter, status int, message string) {
	writeJSON(w, status, map[string]string{"error": message})
}

func writeJSON(w http.ResponseWriter, status int, body any) {
	w.Header().Set("Content-Type", "application/json")
	w.WriteHeader(status)
	_ = json.NewEncoder(w).Encode(body)
}
