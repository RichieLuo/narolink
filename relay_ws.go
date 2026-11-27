package main

import (
	"bufio"
	"context"
	"crypto/sha1"
	"encoding/base64"
	"encoding/binary"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net"
	"net/http"
	"strings"
	"sync"
	"time"
)

const (
	wsGUID = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11"

	wsOpCont   byte = 0x0
	wsOpText   byte = 0x1
	wsOpBinary byte = 0x2
	wsOpClose  byte = 0x8
	wsOpPing   byte = 0x9
	wsOpPong   byte = 0xA

	wsBinReqChunk    byte = 1
	wsBinResChunk    byte = 2
	wsBinStreamChunk byte = 3
	wsBinFlagEOF     byte = 0x01
	wsBinHeaderLen        = 18

	wsProtocolVersion = 1
	wsHeartbeatSec    = 25
	wsMaxChunkBytes   = 64 << 10
	wsMaxTextBytes    = 64 << 10
	wsMaxBinaryBytes  = wsBinHeaderLen + wsMaxChunkBytes
	wsHelloTimeout    = 10 * time.Second
	wsReadTimeout     = 65 * time.Second
	wsWriteTimeout    = 20 * time.Second

	wsResponseWindow  = 2 << 20   // 2 MiB per normal response
	wsResponseAckStep = 256 << 10 // ack every 256 KiB drained
	wsStreamWindow    = 256 << 10 // 256 KiB per SSE stream
	wsStreamAckStep   = 64 << 10  // ack every 64 KiB drained
)

type wsFrame struct {
	opcode  byte
	payload []byte
}

type wsChunk struct {
	data []byte
	eof  bool
}

type wsResponseHead struct {
	status      int
	contentType string
	setCookie   string
	eof         bool
}

type wsTextMsg struct {
	Type           string `json:"type"`
	ID             string `json:"id,omitempty"`
	Name           string `json:"name,omitempty"`
	Streaming      bool   `json:"streaming,omitempty"`
	Protocol       int    `json:"protocol,omitempty"`
	HeartbeatSec   int    `json:"heartbeat_sec,omitempty"`
	MaxChunkBytes  int    `json:"max_chunk_bytes,omitempty"`
	ResponseWindow int    `json:"response_window,omitempty"`
	StreamWindow   int    `json:"stream_window,omitempty"`
	Method         string `json:"method,omitempty"`
	Path           string `json:"path,omitempty"`
	Origin         string `json:"origin,omitempty"`
	ContentType    string `json:"content_type,omitempty"`
	Cookie         string `json:"cookie,omitempty"`
	Stream         bool   `json:"stream,omitempty"`
	BodyLen        int    `json:"body_len,omitempty"`
	Status         int    `json:"status,omitempty"`
	SetCookie      string `json:"set_cookie,omitempty"`
	EOF            bool   `json:"eof,omitempty"`
	Bytes          int    `json:"bytes,omitempty"`
	Error          string `json:"error,omitempty"`
}

type wsSession struct {
	conn   net.Conn
	brw    *bufio.ReadWriter
	target *desktop
	highCh chan wsFrame
	dataCh chan wsFrame
	done   chan struct{}
	once   sync.Once
}

func newWSSession(conn net.Conn, brw *bufio.ReadWriter, target *desktop) *wsSession {
	return &wsSession{
		conn:   conn,
		brw:    brw,
		target: target,
		highCh: make(chan wsFrame, 64),
		dataCh: make(chan wsFrame, 32),
		done:   make(chan struct{}),
	}
}

func (s *wsSession) close() {
	s.once.Do(func() {
		close(s.done)
		_ = s.conn.SetWriteDeadline(time.Now().Add(time.Second))
		_ = writeWSFrame(s.conn, wsOpClose, []byte{0x03, 0xe8}) // 1000 normal closure
		_ = s.conn.Close()
	})
}

func (s *wsSession) sendHigh(ctx context.Context, frame wsFrame) bool {
	select {
	case <-s.done:
		return false
	default:
	}
	select {
	case s.highCh <- frame:
		return true
	case <-s.done:
		return false
	case <-ctx.Done():
		return false
	}
}

func (s *wsSession) trySendHigh(frame wsFrame) bool {
	select {
	case <-s.done:
		return false
	default:
	}
	select {
	case s.highCh <- frame:
		return true
	default:
		return false
	}
}

func (s *wsSession) sendData(ctx context.Context, frame wsFrame) bool {
	select {
	case <-s.done:
		return false
	default:
	}
	select {
	case s.dataCh <- frame:
		return true
	case <-s.done:
		return false
	case <-ctx.Done():
		return false
	}
}

func (s *wsSession) sendJSON(ctx context.Context, msg wsTextMsg, high bool) bool {
	raw, err := json.Marshal(msg)
	if err != nil {
		return false
	}
	frame := wsFrame{opcode: wsOpText, payload: raw}
	if high {
		return s.sendHigh(ctx, frame)
	}
	return s.sendData(ctx, frame)
}

func (s *wsSession) trySendJSON(msg wsTextMsg) bool {
	raw, err := json.Marshal(msg)
	if err != nil {
		return false
	}
	return s.trySendHigh(wsFrame{opcode: wsOpText, payload: raw})
}

func (s *wsSession) sendCancel(id string) {
	s.trySendJSON(wsTextMsg{Type: "req_cancel", ID: id})
}

func (s *wsSession) sendAck(id string, n int) {
	if n <= 0 {
		return
	}
	s.trySendJSON(wsTextMsg{Type: "ack", ID: id, Bytes: n})
}

func (s *wsSession) dispatch(ctx context.Context, job *waiting, rawID [16]byte) error {
	body := []byte(job.work.Body)
	start := wsTextMsg{
		Type:        "req_start",
		ID:          job.work.ID,
		Method:      job.work.Method,
		Path:        job.work.Path,
		Origin:      job.work.Origin,
		ContentType: job.work.ContentType,
		Cookie:      job.work.Cookie,
		Stream:      job.work.Stream,
		BodyLen:     len(body),
	}
	if !s.sendJSON(ctx, start, true) {
		return errors.New("ws session unavailable")
	}
	if len(body) == 0 {
		return nil
	}
	high := len(body) <= wsMaxChunkBytes
	for offset := 0; offset < len(body); offset += wsMaxChunkBytes {
		end := offset + wsMaxChunkBytes
		flags := byte(0)
		if end >= len(body) {
			end = len(body)
			flags = wsBinFlagEOF
		}
		chunk := make([]byte, wsBinHeaderLen+(end-offset))
		chunk[0] = wsBinReqChunk
		chunk[1] = flags
		copy(chunk[2:18], rawID[:])
		copy(chunk[18:], body[offset:end])
		frame := wsFrame{opcode: wsOpBinary, payload: chunk}
		var ok bool
		if high {
			ok = s.sendHigh(ctx, frame)
		} else {
			ok = s.sendData(ctx, frame)
		}
		if !ok {
			return errors.New("ws upload interrupted")
		}
	}
	return nil
}

func (s *wsSession) writeLoop() {
	for {
		select {
		case <-s.done:
			return
		case frame := <-s.highCh:
			if err := s.writeOne(frame); err != nil {
				s.close()
				return
			}
			continue
		default:
		}
		select {
		case <-s.done:
			return
		case frame := <-s.highCh:
			if err := s.writeOne(frame); err != nil {
				s.close()
				return
			}
		case frame := <-s.dataCh:
			if err := s.writeOne(frame); err != nil {
				s.close()
				return
			}
		}
	}
}

func (s *wsSession) writeOne(frame wsFrame) error {
	if err := s.conn.SetWriteDeadline(time.Now().Add(wsWriteTimeout)); err != nil {
		return err
	}
	return writeWSFrame(s.conn, frame.opcode, frame.payload)
}

func (r *relay) desktopWS(w http.ResponseWriter, req *http.Request, id string, deviceKey [32]byte) {
	w.Header().Set("X-Naro-Relay-WS", "1")
	key := strings.TrimSpace(req.Header.Get("Sec-WebSocket-Key"))
	if !isWebSocketUpgrade(req, key) {
		w.Header().Set("Upgrade", "websocket")
		fail(w, http.StatusUpgradeRequired, "需要 WebSocket 连接升级")
		return
	}
	hj, ok := w.(http.Hijacker)
	if !ok {
		fail(w, http.StatusInternalServerError, "服务器不支持连接升级")
		return
	}
	conn, brw, err := hj.Hijack()
	if err != nil {
		return
	}
	// Clear http.Server's ReadTimeout/WriteTimeout deadlines after hijacking.
	_ = conn.SetDeadline(time.Time{})

	accept := wsAcceptKey(key)
	handshake := "HTTP/1.1 101 Switching Protocols\r\n" +
		"Upgrade: websocket\r\n" +
		"Connection: Upgrade\r\n" +
		"Sec-WebSocket-Accept: " + accept + "\r\n" +
		"X-Naro-Relay-WS: 1\r\n\r\n"
	_ = conn.SetWriteDeadline(time.Now().Add(wsHelloTimeout))
	if _, err := brw.WriteString(handshake); err != nil {
		_ = conn.Close()
		return
	}
	if err := brw.Flush(); err != nil {
		_ = conn.Close()
		return
	}

	// Read initial hello message before registering the device.
	_ = conn.SetReadDeadline(time.Now().Add(wsHelloTimeout))
	op, payload, err := readWSMessage(brw.Reader, wsMaxTextBytes, func(ping []byte) error {
		_ = conn.SetWriteDeadline(time.Now().Add(wsWriteTimeout))
		return writeWSFrame(conn, wsOpPong, ping)
	})
	if err != nil || op != wsOpText {
		_ = conn.Close()
		return
	}
	var hello wsTextMsg
	if err := json.Unmarshal(payload, &hello); err != nil || hello.Type != "hello" {
		_ = conn.Close()
		return
	}
	target := r.registerDevice(id, deviceKey)
	session := newWSSession(conn, brw, target)
	ack := wsTextMsg{
		Type:           "hello_ack",
		Protocol:       wsProtocolVersion,
		HeartbeatSec:   wsHeartbeatSec,
		MaxChunkBytes:  wsMaxChunkBytes,
		ResponseWindow: wsResponseWindow,
		StreamWindow:   wsStreamWindow,
	}
	ackBytes, err := json.Marshal(ack)
	if err != nil {
		_ = conn.Close()
		return
	}
	_ = conn.SetWriteDeadline(time.Now().Add(wsWriteTimeout))
	if err := writeWSFrame(conn, wsOpText, ackBytes); err != nil {
		_ = conn.Close()
		return
	}

	name := strings.TrimSpace(hello.Name)
	if len(name) > 256 {
		name = string([]rune(name)[:80])
	}
	old := target.attachWS(session, name, hello.Streaming)
	if old != nil {
		old.close()
	}

	go session.writeLoop()
	session.readLoop()
	target.detachWS(session)
	session.close()
}

func (d *desktop) attachWS(session *wsSession, name string, streaming bool) *wsSession {
	d.mu.Lock()
	defer d.mu.Unlock()
	old := d.ws
	d.ws = session
	d.name = name
	d.streaming = streaming
	d.lastPoll = time.Time{}
	return old
}

func (d *desktop) detachWS(session *wsSession) {
	d.mu.Lock()
	if d.ws == session {
		d.ws = nil
	}
	var orphaned []*waiting
	for id, job := range d.waiting {
		if job.session == session {
			orphaned = append(orphaned, job)
			delete(d.waiting, id)
		}
	}
	d.mu.Unlock()

	for _, job := range orphaned {
		job.abortWS("电脑连接已断开，请确认状态后再重试")
	}
}

func (s *wsSession) readLoop() {
	for {
		if err := s.conn.SetReadDeadline(time.Now().Add(wsReadTimeout)); err != nil {
			return
		}
		op, payload, err := readWSMessage(s.brw.Reader, wsMaxBinaryBytes, func(ping []byte) error {
			s.trySendHigh(wsFrame{opcode: wsOpPong, payload: ping})
			return nil
		})
		if err != nil {
			return
		}
		switch op {
		case wsOpText:
			if len(payload) > wsMaxTextBytes {
				return
			}
			var msg wsTextMsg
			if err := json.Unmarshal(payload, &msg); err != nil {
				continue
			}
			s.handleText(msg)
		case wsOpBinary:
			s.handleBinary(payload)
		case wsOpPong:
			// Read deadline already refreshed.
		case wsOpClose:
			return
		}
	}
}

func (s *wsSession) lookupJob(id string) *waiting {
	s.target.mu.Lock()
	job := s.target.waiting[id]
	s.target.mu.Unlock()
	if job == nil || job.session != s {
		return nil
	}
	return job
}

func (s *wsSession) handleText(msg wsTextMsg) {
	switch msg.Type {
	case "res_start":
		job := s.lookupJob(msg.ID)
		if job == nil || msg.Status < 200 || msg.Status > 599 {
			return
		}
		head := wsResponseHead{
			status:      msg.Status,
			contentType: msg.ContentType,
			setCookie:   msg.SetCookie,
			eof:         msg.EOF,
		}
		select {
		case job.wsHead <- head:
		default:
		}
	case "res_abort":
		job := s.lookupJob(msg.ID)
		if job == nil {
			return
		}
		reason := msg.Error
		if reason == "" {
			reason = "电脑响应中断"
		}
		job.abortWS(reason)
	case "stream_start":
		job := s.lookupJob(msg.ID)
		if job == nil || job.wsStream == nil {
			return
		}
		select {
		case job.wsStreamStart <- struct{}{}:
		default:
		}
	case "stream_end":
		job := s.lookupJob(msg.ID)
		if job == nil || job.wsStream == nil {
			return
		}
		job.finishWSStream()
	}
}

func (s *wsSession) handleBinary(payload []byte) {
	if len(payload) < wsBinHeaderLen {
		return
	}
	kind := payload[0]
	flags := payload[1]
	id := hex.EncodeToString(payload[2:18])
	body := payload[wsBinHeaderLen:]
	eof := (flags & wsBinFlagEOF) != 0

	job := s.lookupJob(id)
	if job == nil {
		if !eof {
			s.sendCancel(id)
		}
		return
	}
	switch kind {
	case wsBinResChunk:
		if !job.pushWSChunk(body, eof) {
			s.sendCancel(id)
		}
	case wsBinStreamChunk:
		if !job.pushWSStreamChunk(body, eof) {
			s.sendCancel(id)
		}
	}
}

func (j *waiting) abortWS(reason string) {
	j.wsMu.Lock()
	if !j.wsAborted {
		j.wsAborted = true
		j.wsAbortReason = reason
		if j.wsClosed != nil {
			close(j.wsClosed)
		}
		if j.wsChunks != nil && !j.wsChunksClosed {
			j.wsChunksClosed = true
			close(j.wsChunks)
		}
		if j.wsStream != nil && !j.wsStreamClosed {
			j.wsStreamClosed = true
			close(j.wsStream)
		}
	}
	j.wsMu.Unlock()
}

func (j *waiting) pushWSChunk(data []byte, eof bool) bool {
	j.wsMu.Lock()
	defer j.wsMu.Unlock()
	if j.wsAborted || j.wsChunks == nil || j.wsChunksClosed {
		return false
	}
	j.wsTotalBytes += len(data)
	if j.wsTotalBytes > maxWireBody || j.wsBufferedBytes+len(data) > wsResponseWindow {
		j.wsAborted = true
		j.wsAbortReason = "响应缓冲超限"
		j.wsChunksClosed = true
		close(j.wsChunks)
		return false
	}
	var copied []byte
	if len(data) > 0 {
		copied = make([]byte, len(data))
		copy(copied, data)
		j.wsBufferedBytes += len(copied)
	}
	select {
	case j.wsChunks <- wsChunk{data: copied, eof: eof}:
		if eof {
			j.wsChunksClosed = true
			close(j.wsChunks)
		}
		return true
	default:
		j.wsAborted = true
		j.wsAbortReason = "响应缓冲已满"
		j.wsChunksClosed = true
		close(j.wsChunks)
		return false
	}
}

func (j *waiting) consumeWSBytes(n int) {
	if n <= 0 {
		return
	}
	j.wsMu.Lock()
	j.wsBufferedBytes -= n
	if j.wsBufferedBytes < 0 {
		j.wsBufferedBytes = 0
	}
	j.wsMu.Unlock()
}

func (j *waiting) pushWSStreamChunk(data []byte, eof bool) bool {
	j.wsMu.Lock()
	defer j.wsMu.Unlock()
	if j.wsAborted || j.wsStream == nil || j.wsStreamClosed {
		return false
	}
	if j.wsStreamBuffered+len(data) > wsStreamWindow {
		j.wsAborted = true
		j.wsAbortReason = "流式响应积压超时"
		j.wsStreamClosed = true
		close(j.wsStream)
		return false
	}
	if len(data) > 0 {
		copied := make([]byte, len(data))
		copy(copied, data)
		select {
		case j.wsStream <- copied:
			j.wsStreamBuffered += len(copied)
		default:
			j.wsAborted = true
			j.wsAbortReason = "流式响应积压"
			j.wsStreamClosed = true
			close(j.wsStream)
			return false
		}
	}
	if eof {
		j.wsStreamClosed = true
		close(j.wsStream)
	}
	return true
}

func (j *waiting) consumeWSStreamBytes(n int) {
	if n <= 0 {
		return
	}
	j.wsMu.Lock()
	j.wsStreamBuffered -= n
	if j.wsStreamBuffered < 0 {
		j.wsStreamBuffered = 0
	}
	j.wsMu.Unlock()
}

func (j *waiting) finishWSStream() {
	j.wsMu.Lock()
	if j.wsStream != nil && !j.wsStreamClosed {
		j.wsStreamClosed = true
		close(j.wsStream)
	}
	j.wsMu.Unlock()
}

func serveWSResponse(w http.ResponseWriter, req *http.Request, job *waiting, head wsResponseHead) {
	controller := http.NewResponseController(w)
	if head.contentType != "" {
		w.Header().Set("Content-Type", head.contentType)
	}
	w.WriteHeader(head.status)
	if head.eof || req.Method == http.MethodHead {
		return
	}
	drainedSinceAck := 0
	for {
		select {
		case <-req.Context().Done():
			job.session.sendCancel(job.work.ID)
			return
		case chunk, ok := <-job.wsChunks:
			if !ok {
				return
			}
			if len(chunk.data) > 0 {
				_ = controller.SetWriteDeadline(time.Now().Add(streamIOTimeout))
				if _, err := w.Write(chunk.data); err != nil {
					job.session.sendCancel(job.work.ID)
					return
				}
				job.consumeWSBytes(len(chunk.data))
				drainedSinceAck += len(chunk.data)
				if drainedSinceAck >= wsResponseAckStep {
					job.session.sendAck(job.work.ID, drainedSinceAck)
					drainedSinceAck = 0
				}
			}
			if chunk.eof {
				return
			}
		}
	}
}

func serveWSStream(w http.ResponseWriter, req *http.Request, job *waiting) {
	controller := http.NewResponseController(w)
	_ = controller.SetWriteDeadline(time.Now().Add(streamIOTimeout))
	w.Header().Set("Content-Type", "text/event-stream")
	w.Header().Set("X-Accel-Buffering", "no")
	w.WriteHeader(http.StatusOK)
	if controller.Flush() != nil {
		job.session.sendCancel(job.work.ID)
		return
	}
	drainedSinceAck := 0
	for {
		select {
		case <-req.Context().Done():
			job.session.sendCancel(job.work.ID)
			return
		case chunk, ok := <-job.wsStream:
			if !ok {
				return
			}
			_ = controller.SetWriteDeadline(time.Now().Add(streamIOTimeout))
			if _, err := w.Write(chunk); err != nil || controller.Flush() != nil {
				job.session.sendCancel(job.work.ID)
				return
			}
			job.consumeWSStreamBytes(len(chunk))
			drainedSinceAck += len(chunk)
			if drainedSinceAck >= wsStreamAckStep {
				job.session.sendAck(job.work.ID, drainedSinceAck)
				drainedSinceAck = 0
			}
		}
	}
}

func isWebSocketUpgrade(req *http.Request, key string) bool {
	if req.Method != http.MethodGet {
		return false
	}
	if !headerHasToken(req.Header, "Connection", "upgrade") ||
		!headerHasToken(req.Header, "Upgrade", "websocket") ||
		strings.TrimSpace(req.Header.Get("Sec-WebSocket-Version")) != "13" {
		return false
	}
	decoded, err := base64.StdEncoding.DecodeString(key)
	return err == nil && len(decoded) == 16
}

func headerHasToken(headers http.Header, name, token string) bool {
	for _, value := range headers.Values(name) {
		for _, part := range strings.Split(value, ",") {
			if strings.EqualFold(strings.TrimSpace(part), token) {
				return true
			}
		}
	}
	return false
}

func wsAcceptKey(clientKey string) string {
	sum := sha1.Sum([]byte(clientKey + wsGUID))
	return base64.StdEncoding.EncodeToString(sum[:])
}

// readWSMessage reads one complete (possibly fragmented) masked client WebSocket message.
// Control frames interleaved in a fragmented message are handled immediately.
func readWSMessage(r io.Reader, maxLen int, onPing func([]byte) error) (byte, []byte, error) {
	var msgOp byte
	var buf []byte
	for {
		var hdr [2]byte
		if _, err := io.ReadFull(r, hdr[:]); err != nil {
			return 0, nil, err
		}
		fin := (hdr[0] & 0x80) != 0
		rsv := hdr[0] & 0x70
		op := hdr[0] & 0x0f
		masked := (hdr[1] & 0x80) != 0
		length := uint64(hdr[1] & 0x7f)

		if rsv != 0 || !masked {
			return 0, nil, errors.New("protocol error")
		}
		if length == 126 {
			var ext [2]byte
			if _, err := io.ReadFull(r, ext[:]); err != nil {
				return 0, nil, err
			}
			length = uint64(binary.BigEndian.Uint16(ext[:]))
		} else if length == 127 {
			var ext [8]byte
			if _, err := io.ReadFull(r, ext[:]); err != nil {
				return 0, nil, err
			}
			if ext[0]&0x80 != 0 {
				return 0, nil, errors.New("invalid 64-bit length")
			}
			length = binary.BigEndian.Uint64(ext[:])
		}

		isControl := op >= 0x8
		if isControl {
			if !fin || length > 125 {
				return 0, nil, errors.New("invalid control frame")
			}
		} else if length > uint64(maxLen) || uint64(len(buf))+length > uint64(maxLen) {
			return 0, nil, fmt.Errorf("frame too large: %d", length)
		}

		var mask [4]byte
		if _, err := io.ReadFull(r, mask[:]); err != nil {
			return 0, nil, err
		}
		frame := make([]byte, int(length))
		if length > 0 {
			if _, err := io.ReadFull(r, frame); err != nil {
				return 0, nil, err
			}
			for i := range frame {
				frame[i] ^= mask[i&3]
			}
		}

		if isControl {
			switch op {
			case wsOpPing:
				if onPing != nil {
					if err := onPing(frame); err != nil {
						return 0, nil, err
					}
				}
				continue
			case wsOpPong:
				if msgOp == 0 {
					return wsOpPong, frame, nil
				}
				continue
			case wsOpClose:
				return wsOpClose, frame, nil
			default:
				return 0, nil, errors.New("unknown control opcode")
			}
		}

		switch op {
		case wsOpText, wsOpBinary:
			if msgOp != 0 {
				return 0, nil, errors.New("unexpected data frame")
			}
			msgOp = op
			if fin {
				return msgOp, frame, nil
			}
			buf = append(buf, frame...)
		case wsOpCont:
			if msgOp == 0 {
				return 0, nil, errors.New("unexpected continuation frame")
			}
			buf = append(buf, frame...)
			if fin {
				return msgOp, buf, nil
			}
		default:
			return 0, nil, errors.New("unknown data opcode")
		}
	}
}

// writeWSFrame writes an unmasked server-to-client RFC 6455 frame.
func writeWSFrame(w io.Writer, opcode byte, payload []byte) error {
	var header [10]byte
	header[0] = 0x80 | (opcode & 0x0f)
	n := len(payload)
	var hlen int
	switch {
	case n <= 125:
		header[1] = byte(n)
		hlen = 2
	case n <= 65535:
		header[1] = 126
		binary.BigEndian.PutUint16(header[2:4], uint16(n))
		hlen = 4
	default:
		header[1] = 127
		binary.BigEndian.PutUint64(header[2:10], uint64(n))
		hlen = 10
	}
	if n == 0 {
		_, err := w.Write(header[:hlen])
		return err
	}
	if n <= 1024 {
		var buf [1034]byte
		copy(buf[:hlen], header[:hlen])
		copy(buf[hlen:hlen+n], payload)
		_, err := w.Write(buf[:hlen+n])
		return err
	}
	buffers := net.Buffers{header[:hlen], payload}
	_, err := buffers.WriteTo(w)
	return err
}
