package main

import (
	"bufio"
	"bytes"
	"context"
	"crypto/sha256"
	"encoding/base64"
	"encoding/binary"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"io"
	"net"
	"net/http"
	"net/http/httptest"
	"net/url"
	"strings"
	"sync"
	"testing"
	"time"
)

const testOrigin = "https://phone.example.test"

type testWSClient struct {
	conn net.Conn
	br   *bufio.Reader
	mu   sync.Mutex
}

func dialTestWS(t *testing.T, serverURL, token, deviceID string, deviceKey [32]byte, name string) (*testWSClient, wsTextMsg) {
	t.Helper()
	u, err := url.Parse(serverURL)
	if err != nil {
		t.Fatal(err)
	}
	conn, err := net.DialTimeout("tcp", u.Host, 3*time.Second)
	if err != nil {
		t.Fatal(err)
	}
	key := base64.StdEncoding.EncodeToString([]byte("0123456789abcdef"))
	req := fmt.Sprintf(
		"GET /relay/devices/%s/ws HTTP/1.1\r\n"+
			"Host: %s\r\n"+
			"Authorization: Bearer %s\r\n"+
			"X-Naro-Device-Key: %s\r\n"+
			"Connection: Upgrade\r\n"+
			"Upgrade: websocket\r\n"+
			"Sec-WebSocket-Version: 13\r\n"+
			"Sec-WebSocket-Key: %s\r\n\r\n",
		deviceID, u.Host, token, hex.EncodeToString(deviceKey[:]), key,
	)
	if _, err := conn.Write([]byte(req)); err != nil {
		conn.Close()
		t.Fatal(err)
	}
	br := bufio.NewReader(conn)
	resp, err := http.ReadResponse(br, &http.Request{Method: http.MethodGet})
	if err != nil {
		conn.Close()
		t.Fatal(err)
	}
	if resp.StatusCode != http.StatusSwitchingProtocols {
		conn.Close()
		t.Fatalf("expected 101 Switching Protocols, got %d", resp.StatusCode)
	}
	if resp.Header.Get("Sec-WebSocket-Accept") != wsAcceptKey(key) {
		conn.Close()
		t.Fatalf("bad Sec-WebSocket-Accept: %q", resp.Header.Get("Sec-WebSocket-Accept"))
	}
	c := &testWSClient{conn: conn, br: br}
	c.writeJSON(t, wsTextMsg{Type: "hello", Name: name, Streaming: true, Protocol: 1})
	op, payload, err := c.readFrame()
	if err != nil || op != wsOpText {
		conn.Close()
		t.Fatalf("failed to read hello_ack: op=%d err=%v", op, err)
	}
	var ack wsTextMsg
	if err := json.Unmarshal(payload, &ack); err != nil || ack.Type != "hello_ack" {
		conn.Close()
		t.Fatalf("unexpected hello_ack: %s", string(payload))
	}
	return c, ack
}

func (c *testWSClient) close() {
	_ = c.conn.Close()
}

func (c *testWSClient) writeMasked(opcode byte, payload []byte) error {
	c.mu.Lock()
	defer c.mu.Unlock()
	mask := [4]byte{0x12, 0x34, 0x56, 0x78}
	var hdr [14]byte
	hdr[0] = 0x80 | (opcode & 0x0f)
	n := len(payload)
	hlen := 2
	switch {
	case n <= 125:
		hdr[1] = 0x80 | byte(n)
	case n <= 65535:
		hdr[1] = 0x80 | 126
		binary.BigEndian.PutUint16(hdr[2:4], uint16(n))
		hlen = 4
	default:
		hdr[1] = 0x80 | 127
		binary.BigEndian.PutUint64(hdr[2:10], uint64(n))
		hlen = 10
	}
	copy(hdr[hlen:hlen+4], mask[:])
	hlen += 4
	if _, err := c.conn.Write(hdr[:hlen]); err != nil {
		return err
	}
	if n > 0 {
		masked := make([]byte, n)
		for i := range payload {
			masked[i] = payload[i] ^ mask[i&3]
		}
		if _, err := c.conn.Write(masked); err != nil {
			return err
		}
	}
	return nil
}

func (c *testWSClient) writeJSON(t *testing.T, msg wsTextMsg) {
	t.Helper()
	raw, err := json.Marshal(msg)
	if err != nil {
		t.Fatal(err)
	}
	if err := c.writeMasked(wsOpText, raw); err != nil {
		t.Fatal(err)
	}
}

func (c *testWSClient) writeBin(kind, flags byte, hexID string, data []byte) error {
	rawID, err := hex.DecodeString(hexID)
	if err != nil || len(rawID) != 16 {
		return fmt.Errorf("bad id")
	}
	buf := make([]byte, wsBinHeaderLen+len(data))
	buf[0] = kind
	buf[1] = flags
	copy(buf[2:18], rawID)
	copy(buf[18:], data)
	return c.writeMasked(wsOpBinary, buf)
}

func (c *testWSClient) readFrame() (byte, []byte, error) {
	_ = c.conn.SetReadDeadline(time.Now().Add(5 * time.Second))
	var hdr [2]byte
	if _, err := io.ReadFull(c.br, hdr[:]); err != nil {
		return 0, nil, err
	}
	op := hdr[0] & 0x0f
	length := uint64(hdr[1] & 0x7f)
	if length == 126 {
		var ext [2]byte
		if _, err := io.ReadFull(c.br, ext[:]); err != nil {
			return 0, nil, err
		}
		length = uint64(binary.BigEndian.Uint16(ext[:]))
	} else if length == 127 {
		var ext [8]byte
		if _, err := io.ReadFull(c.br, ext[:]); err != nil {
			return 0, nil, err
		}
		length = binary.BigEndian.Uint64(ext[:])
	}
	payload := make([]byte, length)
	if length > 0 {
		if _, err := io.ReadFull(c.br, payload); err != nil {
			return 0, nil, err
		}
	}
	return op, payload, nil
}

func liveRelayForTest(t *testing.T) (*relay, *httptest.Server, string) {
	t.Helper()
	token := strings.Repeat("x", 64)
	var r *relay
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, req *http.Request) {
		r.ServeHTTP(w, req)
	}))
	var err error
	r, err = newRelay(srv.URL, token)
	if err != nil {
		srv.Close()
		t.Fatal(err)
	}
	t.Cleanup(srv.Close)
	return r, srv, token
}

func testDeviceIdentity(seed string) (string, [32]byte) {
	key := sha256.Sum256([]byte(seed))
	digest := sha256.Sum256(key[:])
	return hex.EncodeToString(digest[:16]), key
}

func TestAnonymousUploadDoesNotReachDesktop(t *testing.T) {
	_, srv, token := liveRelayForTest(t)
	devID, devKey := testDeviceIdentity("anon-device")
	ws, _ := dialTestWS(t, srv.URL, token, devID, devKey, "主机")
	defer ws.close()

	req, _ := http.NewRequest(http.MethodPost, srv.URL+"/devices/"+devID+"/api/runs", strings.NewReader("{}"))
	req.Header.Set("Origin", srv.URL)
	req.AddCookie(&http.Cookie{Name: scopedCookie, Value: "forged"})
	resp, err := http.DefaultClient.Do(req)
	if err != nil {
		t.Fatal(err)
	}
	resp.Body.Close()
	if resp.StatusCode != http.StatusUnauthorized {
		t.Fatalf("expected 401 for forged cookie, got %d", resp.StatusCode)
	}
	_ = ws.conn.SetReadDeadline(time.Now().Add(150 * time.Millisecond))
	var hdr [2]byte
	if _, err := io.ReadFull(ws.br, hdr[:]); err == nil {
		t.Fatal("unpaired request reached desktop WSS session")
	}
}

func TestWSHandshakeAuthUpgradeAndLegacyEndpointsRemoved(t *testing.T) {
	r, srv, token := liveRelayForTest(t)
	devID, devKey := testDeviceIdentity("auth-device")
	_, wrongKey := testDeviceIdentity("other-device")

	// Wrong bearer -> 401
	req, _ := http.NewRequest(http.MethodGet, srv.URL+"/relay/devices/"+devID+"/ws", nil)
	req.Header.Set("Authorization", "Bearer "+strings.Repeat("y", 64))
	req.Header.Set("X-Naro-Device-Key", hex.EncodeToString(devKey[:]))
	resp, err := http.DefaultClient.Do(req)
	if err != nil || resp.StatusCode != http.StatusUnauthorized {
		t.Fatalf("expected 401, got err=%v resp=%v", err, resp)
	}
	resp.Body.Close()

	// Wrong device key -> 403
	req, _ = http.NewRequest(http.MethodGet, srv.URL+"/relay/devices/"+devID+"/ws", nil)
	req.Header.Set("Authorization", "Bearer "+token)
	req.Header.Set("X-Naro-Device-Key", hex.EncodeToString(wrongKey[:]))
	resp, err = http.DefaultClient.Do(req)
	if err != nil || resp.StatusCode != http.StatusForbidden {
		t.Fatalf("expected 403, got err=%v resp=%v", err, resp)
	}
	resp.Body.Close()

	// Legacy long-polling routes must return 404 even with valid auth
	for _, route := range []string{
		"/relay/hello",
		"/relay/poll",
		"/relay/devices/" + devID + "/hello",
		"/relay/devices/" + devID + "/poll",
		"/relay/devices/" + devID + "/answer/" + strings.Repeat("a", 32),
		"/relay/devices/" + devID + "/stream/" + strings.Repeat("a", 32),
	} {
		legacyReq, _ := http.NewRequest(http.MethodPost, srv.URL+route, strings.NewReader("{}"))
		legacyReq.Header.Set("Authorization", "Bearer "+token)
		legacyReq.Header.Set("X-Naro-Device-Key", hex.EncodeToString(devKey[:]))
		legacyResp, err := http.DefaultClient.Do(legacyReq)
		if err != nil || legacyResp.StatusCode != http.StatusNotFound {
			t.Fatalf("expected 404 for removed legacy route %s, got err=%v resp=%v", route, err, legacyResp)
		}
		legacyResp.Body.Close()
	}

	// Plain GET without Upgrade headers -> 426 + X-Naro-Relay-WS: 1
	req, _ = http.NewRequest(http.MethodGet, srv.URL+"/relay/devices/"+devID+"/ws", nil)
	req.Header.Set("Authorization", "Bearer "+token)
	req.Header.Set("X-Naro-Device-Key", hex.EncodeToString(devKey[:]))
	resp, err = http.DefaultClient.Do(req)
	if err != nil || resp.StatusCode != http.StatusUpgradeRequired || resp.Header.Get("X-Naro-Relay-WS") != "1" {
		t.Fatalf("expected 426 with X-Naro-Relay-WS=1, got err=%v status=%d hdr=%q", err, resp.StatusCode, resp.Header.Get("X-Naro-Relay-WS"))
	}
	resp.Body.Close()

	// Valid WS upgrade + hello
	ws, ack := dialTestWS(t, srv.URL, token, devID, devKey, "主机 A")
	defer ws.close()
	if ack.HeartbeatSec <= 0 || ack.MaxChunkBytes != wsMaxChunkBytes {
		t.Fatalf("unexpected hello_ack params: %+v", ack)
	}
	if r.activeWSCount() != 1 {
		t.Fatalf("expected 1 active WS, got %d", r.activeWSCount())
	}
	accessResp, err := http.Get(srv.URL + "/devices/" + devID + "/api/access")
	if err != nil {
		t.Fatal(err)
	}
	defer accessResp.Body.Close()
	var access map[string]any
	if err := json.NewDecoder(accessResp.Body).Decode(&access); err != nil {
		t.Fatal(err)
	}
	if access["online"] != true {
		t.Fatalf("expected online=true, got %+v", access)
	}
}

func TestWSMultiplexingStopDuringStreamAndAttachments(t *testing.T) {
	_, srv, token := liveRelayForTest(t)
	devID, devKey := testDeviceIdentity("mux-device")
	ws, _ := dialTestWS(t, srv.URL, token, devID, devKey, "多路电脑")
	defer ws.close()

	streamStop := make(chan struct{}, 1)
	streamStarted := make(chan struct{}, 1)
	var receivedUploadSize int
	var uploadMu sync.Mutex
	go func() {
		pendingBodies := make(map[string][]byte)
		pendingPaths := make(map[string]string)
		for {
			op, payload, err := ws.readFrame()
			if err != nil {
				return
			}
			if op == wsOpText {
				var msg wsTextMsg
				if json.Unmarshal(payload, &msg) != nil {
					continue
				}
				if msg.Type == "req_start" {
					if msg.BodyLen > 0 {
						pendingPaths[msg.ID] = msg.Path
						pendingBodies[msg.ID] = make([]byte, 0, msg.BodyLen)
						continue
					}
					if msg.Stream {
						ws.writeJSON(t, wsTextMsg{Type: "stream_start", ID: msg.ID, ContentType: "text/event-stream"})
						go func(id string) {
							_ = ws.writeBin(wsBinStreamChunk, 0, id, []byte("event: snapshot\ndata: {\"status\":\"running\"}\n\n"))
							streamStarted <- struct{}{}
							<-streamStop
							_ = ws.writeBin(wsBinStreamChunk, wsBinFlagEOF, id, []byte("event: snapshot\ndata: {\"status\":\"stopped\"}\n\n"))
						}(msg.ID)
					} else {
						ws.writeJSON(t, wsTextMsg{Type: "res_start", ID: msg.ID, Status: 200, ContentType: "application/json", EOF: false})
						_ = ws.writeBin(wsBinResChunk, wsBinFlagEOF, msg.ID, []byte(`{"ok":true}`))
					}
				}
			} else if op == wsOpBinary && len(payload) >= wsBinHeaderLen && payload[0] == wsBinReqChunk {
				id := hex.EncodeToString(payload[2:18])
				pendingBodies[id] = append(pendingBodies[id], payload[wsBinHeaderLen:]...)
				if payload[1]&wsBinFlagEOF != 0 {
					path := pendingPaths[id]
					body := pendingBodies[id]
					delete(pendingBodies, id)
					delete(pendingPaths, id)
					switch {
					case path == "/api/pair":
						cookie := fmt.Sprintf("narolink=%d.nonce.sig; Path=/; HttpOnly", time.Now().Add(time.Hour).Unix())
						ws.writeJSON(t, wsTextMsg{Type: "res_start", ID: id, Status: 200, ContentType: "application/json", SetCookie: cookie, EOF: true})
					case strings.HasSuffix(path, "/stop"):
						select {
						case streamStop <- struct{}{}:
						default:
						}
						ws.writeJSON(t, wsTextMsg{Type: "res_start", ID: id, Status: 204, EOF: true})
					case path == "/api/runs":
						uploadMu.Lock()
						receivedUploadSize = len(body)
						uploadMu.Unlock()
						ws.writeJSON(t, wsTextMsg{Type: "res_start", ID: id, Status: 200, ContentType: "application/json", EOF: false})
						_ = ws.writeBin(wsBinResChunk, wsBinFlagEOF, id, []byte(`{"id":"run-1"}`))
					default:
						ws.writeJSON(t, wsTextMsg{Type: "res_start", ID: id, Status: 204, EOF: true})
					}
				}
			}
		}
	}()

	// 1. Pair
	pairReq, _ := http.NewRequest(http.MethodPost, srv.URL+"/devices/"+devID+"/api/pair", strings.NewReader(`{"code":"12345678"}`))
	pairReq.Header.Set("Origin", srv.URL)
	pairReq.Header.Set("Content-Type", "application/json")
	pairResp, err := http.DefaultClient.Do(pairReq)
	if err != nil || pairResp.StatusCode != 200 {
		t.Fatalf("pair failed: err=%v resp=%v", err, pairResp)
	}
	cookies := pairResp.Cookies()
	pairResp.Body.Close()
	if len(cookies) != 1 || cookies[0].Name != scopedCookie || !cookies[0].HttpOnly || cookies[0].SameSite != http.SameSiteStrictMode {
		t.Fatalf("expected scoped HttpOnly Strict cookie, got %+v", cookies)
	}
	cookie := cookies[0]

	// 2. Open SSE stream
	sseReq, _ := http.NewRequest(http.MethodGet, srv.URL+"/devices/"+devID+"/api/runs/r1/events", nil)
	sseReq.Header.Set("Accept", "text/event-stream")
	sseReq.AddCookie(cookie)
	sseResp, err := http.DefaultClient.Do(sseReq)
	if err != nil || sseResp.StatusCode != 200 {
		t.Fatalf("SSE failed: err=%v resp=%v", err, sseResp)
	}
	defer sseResp.Body.Close()
	<-streamStarted

	// 3. While SSE stream is open, upload a 200 KiB multi-chunk attachment AND send stop concurrently!
	largePayload := `{"prompt":"img","images":[{"name":"a.png","data":"` + strings.Repeat("A", 200*1024) + `"}]}`
	uploadReq, _ := http.NewRequest(http.MethodPost, srv.URL+"/devices/"+devID+"/api/runs", strings.NewReader(largePayload))
	uploadReq.Header.Set("Origin", srv.URL)
	uploadReq.Header.Set("Content-Type", "application/json")
	uploadReq.AddCookie(cookie)
	uploadResp, err := http.DefaultClient.Do(uploadReq)
	if err != nil || uploadResp.StatusCode != 200 {
		t.Fatalf("attachment upload failed: err=%v resp=%v", err, uploadResp)
	}
	uploadResp.Body.Close()
	uploadMu.Lock()
	gotSize := receivedUploadSize
	uploadMu.Unlock()
	if gotSize != len(largePayload) {
		t.Fatalf("attachment size mismatch: got %d want %d", gotSize, len(largePayload))
	}

	stopReq, _ := http.NewRequest(http.MethodPost, srv.URL+"/devices/"+devID+"/api/runs/r1/stop", strings.NewReader(`{}`))
	stopReq.Header.Set("Origin", srv.URL)
	stopReq.Header.Set("Content-Type", "application/json")
	stopReq.AddCookie(cookie)
	stopResp, err := http.DefaultClient.Do(stopReq)
	if err != nil || stopResp.StatusCode != http.StatusNoContent {
		t.Fatalf("stop during stream failed: err=%v resp=%v", err, stopResp)
	}
	stopResp.Body.Close()

	sseData, err := io.ReadAll(sseResp.Body)
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(string(sseData), `"running"`) || !strings.Contains(string(sseData), `"stopped"`) {
		t.Fatalf("SSE missed expected events: %q", string(sseData))
	}
}

func TestWSDisconnectCleansUpWaitingWithoutRequeue(t *testing.T) {
	r, srv, token := liveRelayForTest(t)
	devID, devKey := testDeviceIdentity("drop-device")
	ws1, _ := dialTestWS(t, srv.URL, token, devID, devKey, "断线电脑")

	dev := r.findDevice(devID)
	inner := fmt.Sprintf("%d.nonce.sig", time.Now().Add(time.Hour).Unix())
	cookie := &http.Cookie{Name: scopedCookie, Value: dev.wrap(inner, "/devices/"+devID+"/")}

	gotReq := make(chan string, 1)
	go func() {
		for {
			op, payload, err := ws1.readFrame()
			if err != nil {
				return
			}
			if op == wsOpText {
				var msg wsTextMsg
				if json.Unmarshal(payload, &msg) == nil && msg.Type == "req_start" {
					gotReq <- msg.ID
					ws1.close()
					return
				}
			}
		}
	}()

	start := time.Now()
	req, _ := http.NewRequest(http.MethodPost, srv.URL+"/devices/"+devID+"/api/runs", strings.NewReader(`{"prompt":"once"}`))
	req.Header.Set("Origin", srv.URL)
	req.Header.Set("Content-Type", "application/json")
	req.AddCookie(cookie)
	resp, err := http.DefaultClient.Do(req)
	if err != nil {
		t.Fatal(err)
	}
	resp.Body.Close()
	elapsed := time.Since(start)
	<-gotReq
	if resp.StatusCode != http.StatusBadGateway {
		t.Fatalf("expected immediate 502 on mid-request disconnect, got %d", resp.StatusCode)
	}
	if elapsed > 3*time.Second {
		t.Fatalf("waiting request did not fail promptly on disconnect: %v", elapsed)
	}

	ws2, _ := dialTestWS(t, srv.URL, token, devID, devKey, "断线电脑")
	defer ws2.close()
	_ = ws2.conn.SetReadDeadline(time.Now().Add(200 * time.Millisecond))
	var hdr [2]byte
	if _, err := io.ReadFull(ws2.br, hdr[:]); err == nil {
		t.Fatal("aborted side-effecting request was re-dispatched to reconnected session")
	}
}

func TestWSPerDeviceConcurrencyDoesNotStarveOtherDevices(t *testing.T) {
	r, srv, token := liveRelayForTest(t)
	idA, keyA := testDeviceIdentity("noisy-device")
	idB, keyB := testDeviceIdentity("quiet-device")
	wsA, _ := dialTestWS(t, srv.URL, token, idA, keyA, "忙碌电脑")
	defer wsA.close()
	wsB, _ := dialTestWS(t, srv.URL, token, idB, keyB, "正常电脑")
	defer wsB.close()

	go func() {
		for {
			if _, _, err := wsA.readFrame(); err != nil {
				return
			}
		}
	}()
	go func() {
		for {
			op, payload, err := wsB.readFrame()
			if err != nil {
				return
			}
			if op == wsOpText {
				var msg wsTextMsg
				if json.Unmarshal(payload, &msg) == nil && msg.Type == "req_start" {
					wsB.writeJSON(t, wsTextMsg{Type: "res_start", ID: msg.ID, Status: 200, ContentType: "application/json", EOF: false})
					_ = wsB.writeBin(wsBinResChunk, wsBinFlagEOF, msg.ID, []byte(`{"device":"B"}`))
				}
			}
		}
	}()

	inner := fmt.Sprintf("%d.nonce.sig", time.Now().Add(time.Hour).Unix())
	cookieA := &http.Cookie{Name: scopedCookie, Value: r.findDevice(idA).wrap(inner, "/devices/"+idA+"/")}
	cookieB := &http.Cookie{Name: scopedCookie, Value: r.findDevice(idB).wrap(inner, "/devices/"+idB+"/")}

	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	for range deviceSlotsLimit {
		go func() {
			req, _ := http.NewRequestWithContext(ctx, http.MethodGet, srv.URL+"/devices/"+idA+"/api/activity", nil)
			req.AddCookie(cookieA)
			resp, err := http.DefaultClient.Do(req)
			if err == nil {
				resp.Body.Close()
			}
		}()
	}
	deadline := time.Now().Add(3 * time.Second)
	for len(r.findDevice(idA).slots) < deviceSlotsLimit && time.Now().Before(deadline) {
		time.Sleep(15 * time.Millisecond)
	}
	if len(r.findDevice(idA).slots) != deviceSlotsLimit {
		t.Fatalf("expected Device A slots full (%d), got %d", deviceSlotsLimit, len(r.findDevice(idA).slots))
	}

	reqA, _ := http.NewRequest(http.MethodGet, srv.URL+"/devices/"+idA+"/api/activity", nil)
	reqA.AddCookie(cookieA)
	respA, err := http.DefaultClient.Do(reqA)
	if err != nil || respA.StatusCode != http.StatusServiceUnavailable {
		t.Fatalf("expected 503 when Device A slots full, got err=%v resp=%v", err, respA)
	}
	respA.Body.Close()

	reqB, _ := http.NewRequest(http.MethodGet, srv.URL+"/devices/"+idB+"/api/activity", nil)
	reqB.AddCookie(cookieB)
	respB, err := http.DefaultClient.Do(reqB)
	if err != nil || respB.StatusCode != http.StatusOK {
		t.Fatalf("Device B was starved by Device A: err=%v resp=%v", err, respB)
	}
	respB.Body.Close()
}

func TestWSSingleConnectionPerDeviceAndIdlePingPong(t *testing.T) {
	r, srv, token := liveRelayForTest(t)
	const count = 48
	clients := make([]*testWSClient, 0, count)
	for i := range count {
		id, key := testDeviceIdentity(fmt.Sprintf("idle-host-%d", i))
		c, _ := dialTestWS(t, srv.URL, token, id, key, fmt.Sprintf("Idle-%d", i))
		clients = append(clients, c)
	}
	if got := r.activeWSCount(); got != count {
		t.Fatalf("expected %d active WSS connections, got %d", count, got)
	}
	for i, c := range clients {
		ping := []byte(fmt.Sprintf("p%d", i))
		if err := c.writeMasked(wsOpPing, ping); err != nil {
			t.Fatalf("ping %d failed: %v", i, err)
		}
		op, payload, err := c.readFrame()
		if err != nil || op != wsOpPong || string(payload) != string(ping) {
			t.Fatalf("pong %d mismatch: op=%d payload=%q err=%v", i, op, string(payload), err)
		}
	}
	id0, key0 := testDeviceIdentity("idle-host-0")
	replacement, _ := dialTestWS(t, srv.URL, token, id0, key0, "Idle-0-reconnected")
	defer replacement.close()
	deadline := time.Now().Add(2 * time.Second)
	for r.activeWSCount() != count && time.Now().Before(deadline) {
		time.Sleep(10 * time.Millisecond)
	}
	if got := r.activeWSCount(); got != count {
		t.Fatalf("expected replacement to keep activeWSCount=%d, got %d", count, got)
	}
	for _, c := range clients {
		c.close()
	}
}

func TestWSSlowStreamBufferLimitProtectsOtherRequests(t *testing.T) {
	r, srv, token := liveRelayForTest(t)
	devID, devKey := testDeviceIdentity("slow-stream-device")
	ws, _ := dialTestWS(t, srv.URL, token, devID, devKey, "慢连接电脑")
	defer ws.close()

	gotCancel := make(chan string, 1)
	streamOpened := make(chan struct{}, 1)
	go func() {
		for {
			op, payload, err := ws.readFrame()
			if err != nil {
				return
			}
			if op != wsOpText {
				continue
			}
			var msg wsTextMsg
			if json.Unmarshal(payload, &msg) != nil {
				continue
			}
			switch msg.Type {
			case "req_start":
				if msg.Stream {
					ws.writeJSON(t, wsTextMsg{Type: "stream_start", ID: msg.ID, ContentType: "text/event-stream"})
					streamOpened <- struct{}{}
					bigChunk := bytes.Repeat([]byte("a"), 32*1024)
					go func(id string) {
						for range 160 {
							if ws.writeBin(wsBinStreamChunk, 0, id, bigChunk) != nil {
								return
							}
						}
					}(msg.ID)
				} else {
					ws.writeJSON(t, wsTextMsg{Type: "res_start", ID: msg.ID, Status: 200, ContentType: "application/json", EOF: false})
					_ = ws.writeBin(wsBinResChunk, wsBinFlagEOF, msg.ID, []byte(`{"fast":true}`))
				}
			case "req_cancel":
				select {
				case gotCancel <- msg.ID:
				default:
				}
			}
		}
	}()

	dev := r.findDevice(devID)
	inner := fmt.Sprintf("%d.nonce.sig", time.Now().Add(time.Hour).Unix())
	cookie := &http.Cookie{Name: scopedCookie, Value: dev.wrap(inner, "/devices/"+devID+"/")}

	u, _ := url.Parse(srv.URL)
	rawConn, err := net.DialTimeout("tcp", u.Host, 3*time.Second)
	if err != nil {
		t.Fatal(err)
	}
	defer rawConn.Close()
	_, _ = fmt.Fprintf(rawConn, "GET /devices/%s/api/runs/slow1/events HTTP/1.1\r\nHost: %s\r\nAccept: text/event-stream\r\nCookie: %s=%s\r\n\r\n",
		devID, u.Host, scopedCookie, cookie.Value)
	<-streamOpened

	select {
	case <-gotCancel:
	case <-time.After(3 * time.Second):
		t.Fatal("relay did not send req_cancel when stream buffer exceeded wsStreamWindow")
	}

	fastReq, _ := http.NewRequest(http.MethodGet, srv.URL+"/devices/"+devID+"/api/activity", nil)
	fastReq.AddCookie(cookie)
	fastResp, err := http.DefaultClient.Do(fastReq)
	if err != nil || fastResp.StatusCode != http.StatusOK {
		t.Fatalf("normal request blocked by slow stream: err=%v resp=%v", err, fastResp)
	}
	fastResp.Body.Close()
}

func TestWSMultiDeviceCookieIsolation(t *testing.T) {
	r, srv, token := liveRelayForTest(t)
	id1, key1 := testDeviceIdentity("ws-device-1")
	id2, key2 := testDeviceIdentity("ws-device-2")

	ws1, _ := dialTestWS(t, srv.URL, token, id1, key1, "电脑 1")
	defer ws1.close()
	ws2, _ := dialTestWS(t, srv.URL, token, id2, key2, "电脑 2")
	defer ws2.close()

	respond := func(c *testWSClient, body string) {
		for {
			op, payload, err := c.readFrame()
			if err != nil {
				return
			}
			if op == wsOpText {
				var msg wsTextMsg
				if json.Unmarshal(payload, &msg) == nil && msg.Type == "req_start" {
					c.writeJSON(t, wsTextMsg{Type: "res_start", ID: msg.ID, Status: 200, ContentType: "application/json", EOF: false})
					_ = c.writeBin(wsBinResChunk, wsBinFlagEOF, msg.ID, []byte(body))
				}
			}
		}
	}
	go respond(ws1, `{"host":1}`)
	go respond(ws2, `{"host":2}`)

	inner := fmt.Sprintf("%d.nonce.sig", time.Now().Add(time.Hour).Unix())
	cookie1 := &http.Cookie{Name: scopedCookie, Value: r.findDevice(id1).wrap(inner, "/devices/"+id1+"/")}
	cookie2 := &http.Cookie{Name: scopedCookie, Value: r.findDevice(id2).wrap(inner, "/devices/"+id2+"/")}

	for _, tc := range []struct {
		id     string
		cookie *http.Cookie
		want   string
	}{
		{id1, cookie1, `{"host":1}`},
		{id2, cookie2, `{"host":2}`},
	} {
		req, _ := http.NewRequest(http.MethodGet, srv.URL+"/devices/"+tc.id+"/api/activity", nil)
		req.AddCookie(tc.cookie)
		resp, err := http.DefaultClient.Do(req)
		if err != nil || resp.StatusCode != http.StatusOK {
			t.Fatalf("device %s request failed: err=%v resp=%v", tc.id, err, resp)
		}
		body, _ := io.ReadAll(resp.Body)
		resp.Body.Close()
		if string(body) != tc.want {
			t.Fatalf("device %s body mismatch: got %s want %s", tc.id, string(body), tc.want)
		}
	}

	crossReq, _ := http.NewRequest(http.MethodGet, srv.URL+"/devices/"+id2+"/api/activity", nil)
	crossReq.AddCookie(cookie1)
	crossResp, err := http.DefaultClient.Do(crossReq)
	if err != nil || crossResp.StatusCode != http.StatusUnauthorized {
		t.Fatalf("expected 401 for cross-device cookie, got err=%v resp=%v", err, crossResp)
	}
	crossResp.Body.Close()
}
