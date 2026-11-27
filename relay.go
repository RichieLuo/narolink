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
	scopedCookie   = "narolink_device"
)

type command struct {
	ID          string          `json:"id"`
	Method      string          `json:"method"`
	Path        string          `json:"path"`
	Origin      string          `json:"origin,omitempty"`
	ContentType string          `json:"content_type,omitempty"`
	Cookie      string          `json:"cookie,omitempty"`
	Body        json.RawMessage `json:"body,omitempty"`
	Stream      bool            `json:"stream,omitempty"`
}

type answer struct {
	Status      int    `json:"status"`
	ContentType string `json:"content_type,omitempty"`
	SetCookie   string `json:"set_cookie,omitempty"`
	Body        []byte `json:"body,omitempty"`
}

type waiting struct {
	work    command
	reply   chan answer
	stream  chan *io.PipeReader
	context context.Context
}

type relay struct {
	origin  string
	host    string
	secure  bool
	token   [32]byte
	assets  fs.FS
	slots   chan struct{}
	streams chan struct{}
	uploads chan struct{}
	legacy  *desktop

	mu      sync.Mutex
	devices map[string]*desktop
}

type desktop struct {
	key   [32]byte
	queue chan string

	mu        sync.Mutex
	name      string
	streaming bool
	lastPoll  time.Time
	waiting   map[string]*waiting
}

func newDesktop(key [32]byte) *desktop {
	return &desktop{key: key, queue: make(chan string, 32), waiting: make(map[string]*waiting)}
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
	log.Printf("Naro relay listening on %s for %s", address, gateway.origin)
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
	key := sha256.Sum256([]byte(token))
	return &relay{
		origin: origin.Scheme + "://" + origin.Host, host: origin.Host, secure: secure, token: key,
		assets: assets, slots: make(chan struct{}, 12), streams: make(chan struct{}, 16), uploads: make(chan struct{}, 2),
		legacy: newDesktop(key), devices: make(map[string]*desktop),
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
	w.Header().Set("Content-Security-Policy", "default-src 'none'; script-src 'self'; style-src 'self'; img-src 'self' blob:; media-src 'self' blob:; connect-src 'self'; manifest-src 'self'; worker-src 'self'; base-uri 'none'; form-action 'self'; frame-ancestors 'none'")
	if r.secure {
		w.Header().Set("Strict-Transport-Security", "max-age=86400")
	}
	switch {
	case strings.HasPrefix(req.URL.Path, "/relay/"):
		w.Header().Set("Cache-Control", "no-store")
		r.desktop(w, req)
	case strings.HasPrefix(req.URL.Path, "/api/"):
		w.Header().Set("Cache-Control", "no-store")
		r.browser(w, req, r.legacy, "/")
	case strings.HasPrefix(req.URL.Path, "/devices/"):
		r.devicePage(w, req)
	default:
		r.static(w, req, strings.TrimPrefix(req.URL.Path, "/"))
	}
}

func (r *relay) devicePage(w http.ResponseWriter, req *http.Request) {
	id, path, ok := strings.Cut(strings.TrimPrefix(req.URL.EscapedPath(), "/devices/"), "/")
	if !ok || !validDeviceID(id) {
		http.NotFound(w, req)
		return
	}
	scope := "/devices/" + id + "/"
	switch {
	case path == "":
		r.static(w, req, "index.html")
	case strings.HasPrefix(path, "api/"):
		w.Header().Set("Cache-Control", "no-store")
		r.browser(w, req, r.findDevice(id), scope)
	default:
		http.NotFound(w, req)
	}
}

func validDeviceID(id string) bool {
	_, err := hex.DecodeString(id)
	return len(id) == 32 && err == nil && id == strings.ToLower(id)
}

func (r *relay) findDevice(id string) *desktop {
	r.mu.Lock()
	defer r.mu.Unlock()
	return r.devices[id]
}

func (r *relay) registerDevice(id string, key [32]byte) *desktop {
	r.mu.Lock()
	defer r.mu.Unlock()
	target := r.devices[id]
	if target == nil {
		target = newDesktop(key)
		r.devices[id] = target
	}
	return target
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
	path := strings.TrimPrefix(req.URL.Path, "/relay/")
	target := r.legacy
	if route, scoped := strings.CutPrefix(path, "devices/"); scoped {
		id, operation, ok := strings.Cut(route, "/")
		if !ok || !validDeviceID(id) {
			http.NotFound(w, req)
			return
		}
		deviceKey, err := hex.DecodeString(req.Header.Get("X-Naro-Device-Key"))
		digest := sha256.Sum256(deviceKey)
		if err != nil || len(deviceKey) != 32 || id != hex.EncodeToString(digest[:16]) {
			fail(w, http.StatusForbidden, "设备连接密钥不匹配")
			return
		}
		path = operation
		if path == "hello" {
			var identity struct {
				Name      string `json:"name"`
				Streaming bool   `json:"streaming"`
			}
			if err := json.NewDecoder(http.MaxBytesReader(w, req.Body, 1024)).Decode(&identity); err != nil {
				fail(w, http.StatusBadRequest, "设备信息无效")
				return
			}
			target = r.registerDevice(id, [32]byte(deviceKey))
			target.mu.Lock()
			target.name = strings.TrimSpace(identity.Name)
			target.streaming = identity.Streaming
			target.lastPoll = time.Now()
			target.mu.Unlock()
			w.WriteHeader(http.StatusNoContent)
			return
		}
		target = r.findDevice(id)
		if target == nil {
			http.NotFound(w, req)
			return
		}
	}
	if path == "hello" {
		target.touch()
		w.WriteHeader(http.StatusNoContent)
		return
	}
	if path == "poll" {
		target.poll(w, req)
		return
	}
	operation, id, ok := strings.Cut(path, "/")
	if !ok || len(id) != 32 || (operation != "answer" && operation != "stream") {
		w.WriteHeader(http.StatusNotFound)
		return
	}
	target.mu.Lock()
	job := target.waiting[id]
	target.mu.Unlock()
	if job == nil {
		w.WriteHeader(http.StatusNotFound)
		return
	}
	if operation == "stream" {
		uploadStream(w, req, job)
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

func (d *desktop) touch() {
	d.mu.Lock()
	d.lastPoll = time.Now()
	d.mu.Unlock()
}

func (d *desktop) poll(w http.ResponseWriter, req *http.Request) {
	d.touch()
	ctx, cancel := context.WithTimeout(req.Context(), 20*time.Second)
	defer cancel()
	for {
		select {
		case id := <-d.queue:
			d.mu.Lock()
			task := d.waiting[id]
			d.mu.Unlock()
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

func (r *relay) browser(w http.ResponseWriter, req *http.Request, target *desktop, scope string) {
	write := req.Method == http.MethodPost
	if !write && req.Method != http.MethodGet && req.Method != http.MethodHead {
		w.WriteHeader(http.StatusMethodNotAllowed)
		return
	}
	if write && req.Header.Get("Origin") != r.origin {
		fail(w, http.StatusForbidden, "请求来源不匹配")
		return
	}
	prefix := strings.TrimSuffix(scope, "/")
	path := strings.TrimPrefix(req.URL.Path, prefix)
	pair := path == "/api/pair" && write
	cookieName := deviceCookie
	if scope != "/" {
		cookieName = scopedCookie
	}
	var inner, name string
	var paired, online, streaming bool
	if target != nil {
		inner, paired = target.unwrap(req, cookieName, scope)
		online, name, streaming = target.info()
	}
	if path == "/api/access" && !write {
		access := map[string]any{"paired": paired, "online": online}
		if paired && name != "" {
			access["name"] = name
		}
		writeJSON(w, http.StatusOK, access)
		return
	}
	if !paired && !pair {
		fail(w, http.StatusUnauthorized, "先和电脑配对")
		return
	}
	if path == "/api/logout" && write {
		http.SetCookie(w, &http.Cookie{Name: cookieName, Path: scope, MaxAge: -1, HttpOnly: true, Secure: r.secure, SameSite: http.SameSiteStrictMode})
		w.WriteHeader(http.StatusNoContent)
		return
	}
	if !online {
		writeJSON(w, http.StatusServiceUnavailable, map[string]any{
			"error": "电脑未连接。请打开 Naro。", "online": false,
		})
		return
	}
	stream := req.Method == http.MethodGet && runEventPath(path)
	slots := r.slots
	if stream {
		if !streaming {
			fail(w, http.StatusNotImplemented, "电脑端需要更新才能推送实时回复")
			return
		}
		slots = r.streams
	}
	select {
	case slots <- struct{}{}:
		defer func() { <-slots }()
	default:
		fail(w, http.StatusServiceUnavailable, "电脑请求较多，请稍后重试")
		return
	}
	limit := int64(4096)
	followUp := strings.HasPrefix(path, "/api/runs/") && strings.HasSuffix(path, "/messages")
	if write && strings.HasPrefix(path, "/api/runs/") && strings.HasSuffix(path, "/answer") {
		limit = 64 * 1024
	}
	if write && (path == "/api/runs" || followUp) {
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
	work := command{ID: key, Method: req.Method, Path: strings.TrimPrefix(req.URL.RequestURI(), prefix), Origin: req.Header.Get("Origin"),
		ContentType: req.Header.Get("Content-Type"), Body: body, Stream: stream}
	if paired {
		work.Cookie = "narolink=" + inner
	}
	job := &waiting{work: work, reply: make(chan answer, 1), context: req.Context()}
	if stream {
		job.stream = make(chan *io.PipeReader)
	}
	target.mu.Lock()
	target.waiting[key] = job
	target.mu.Unlock()
	defer func() { target.mu.Lock(); delete(target.waiting, key); target.mu.Unlock() }()
	select {
	case target.queue <- key:
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
			http.SetCookie(w, &http.Cookie{Name: cookieName, Value: target.wrap(cookie.Value, scope), Path: scope, MaxAge: cookieDuration, HttpOnly: true, Secure: r.secure, SameSite: http.SameSiteStrictMode})
			if name != "" {
				writeJSON(w, http.StatusOK, map[string]any{"paired": true, "name": name})
				return
			}
		}
		if result.ContentType != "" {
			w.Header().Set("Content-Type", result.ContentType)
		}
		w.WriteHeader(result.Status)
		_, _ = w.Write(result.Body)
	case reader := <-job.stream:
		serveStream(w, req, reader)
	case <-ctx.Done():
		fail(w, http.StatusGatewayTimeout, "等待电脑响应超时")
	}
}

func (d *desktop) info() (bool, string, bool) {
	d.mu.Lock()
	defer d.mu.Unlock()
	return time.Since(d.lastPoll) < 45*time.Second, d.name, d.streaming
}

func (d *desktop) sign(value []byte, scope string) []byte {
	tag := hmac.New(sha256.New, d.key[:])
	tag.Write([]byte("narolink/device/v1\x00"))
	if scope != "/" {
		tag.Write([]byte(scope))
	}
	tag.Write(value)
	return tag.Sum(nil)
}

func (d *desktop) wrap(inner, scope string) string {
	value := []byte(inner)
	return base64.RawURLEncoding.EncodeToString(append(value, d.sign(value, scope)...))
}

func (d *desktop) unwrap(req *http.Request, name, scope string) (string, bool) {
	cookie, err := req.Cookie(name)
	if err != nil || len(cookie.Value) > 256 {
		return "", false
	}
	decoded, err := base64.RawURLEncoding.DecodeString(cookie.Value)
	if err != nil || len(decoded) <= sha256.Size {
		return "", false
	}
	value, tag := decoded[:len(decoded)-sha256.Size], decoded[len(decoded)-sha256.Size:]
	if !hmac.Equal(d.sign(value, scope), tag) {
		return "", false
	}
	expires, _, active := strings.Cut(string(value), ".")
	deadline, err := strconv.ParseInt(expires, 10, 64)
	if !active || err != nil || deadline < time.Now().Unix() {
		return "", false
	}
	return string(value), true
}

func (r *relay) static(w http.ResponseWriter, req *http.Request, name string) {
	if req.Method != http.MethodGet && req.Method != http.MethodHead {
		w.WriteHeader(405)
		return
	}
	if name == "" {
		name = "index.html"
	}
	contentTypes := map[string]string{
		"index.html": "text/html; charset=utf-8", "app.js": "text/javascript; charset=utf-8", "style.css": "text/css; charset=utf-8",
		"theme.js": "text/javascript; charset=utf-8", "qr-worker.js": "text/javascript; charset=utf-8", "vendor/jsqr.js": "text/javascript; charset=utf-8",
		"i18n.json":            "application/json; charset=utf-8",
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
