package main

import (
	"crypto/sha256"
	"encoding/hex"
	"fmt"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"time"
)

const testOrigin = "https://phone.example.test"

func gatewayForTest(t *testing.T) *relay {
	t.Helper()
	r, err := newRelay(testOrigin, strings.Repeat("x", 64))
	if err != nil {
		t.Fatal(err)
	}
	return r
}

func TestAnonymousUploadDoesNotReachDesktop(t *testing.T) {
	r := gatewayForTest(t)
	request := httptest.NewRequest(http.MethodPost, testOrigin+"/api/runs", strings.NewReader("{}"))
	request.Header.Set("Origin", testOrigin)
	request.AddCookie(&http.Cookie{Name: deviceCookie, Value: "forged"})
	response := httptest.NewRecorder()
	r.ServeHTTP(response, request)
	if response.Code != http.StatusUnauthorized || len(r.legacy.queue) != 0 {
		t.Fatalf("unpaired request reached relay queue: status %d, queued %d", response.Code, len(r.legacy.queue))
	}
}

func TestPairingSetsSecureCookieForHTTPS(t *testing.T) {
	r := gatewayForTest(t)
	r.legacy.touch()
	request := httptest.NewRequest(http.MethodPost, testOrigin+"/api/pair", strings.NewReader(`{"code":"12345678"}`))
	request.Header.Set("Origin", testOrigin)
	request.Header.Set("Content-Type", "application/json")
	response := httptest.NewRecorder()
	done := make(chan struct{})
	go func() {
		r.ServeHTTP(response, request)
		close(done)
	}()
	select {
	case id := <-r.legacy.queue:
		r.legacy.mu.Lock()
		job := r.legacy.waiting[id]
		r.legacy.mu.Unlock()
		cookie := fmt.Sprintf("narolink=%d.example.signature; Path=/; Secure; HttpOnly",
			time.Now().Add(time.Hour).Unix())
		job.reply <- answer{Status: 200, SetCookie: cookie}
	case <-time.After(time.Second):
		t.Fatal("pairing request did not reach the desktop")
	}
	<-done
	cookies := response.Result().Cookies()
	if len(cookies) != 1 || cookies[0].Name != deviceCookie || !cookies[0].Secure ||
		!cookies[0].HttpOnly || cookies[0].SameSite != http.SameSiteStrictMode {
		t.Fatalf("HTTPS device cookie has incorrect scope; status %d", response.Code)
	}
	read := httptest.NewRequest(http.MethodGet, testOrigin+"/api/access", nil)
	read.AddCookie(cookies[0])
	if _, paired := r.legacy.unwrap(read, deviceCookie, "/"); !paired {
		t.Fatal("paired HTTPS cookie cannot be validated")
	}
}

func TestFollowUpUploadReachesDesktop(t *testing.T) {
	r := gatewayForTest(t)
	r.legacy.touch()
	path := "/api/runs/active-run/messages"
	body := `{"mode":"queue","prompt":"review this","images":[{"name":"screenshot.png","data":"` +
		strings.Repeat("A", 5000) + `"}]}`
	request := httptest.NewRequest(http.MethodPost, testOrigin+path, strings.NewReader(body))
	request.Header.Set("Origin", testOrigin)
	request.Header.Set("Content-Type", "application/json")
	inner := fmt.Sprintf("%d.example.signature", time.Now().Add(time.Hour).Unix())
	request.AddCookie(&http.Cookie{Name: deviceCookie, Value: r.legacy.wrap(inner, "/")})
	response := httptest.NewRecorder()
	done := make(chan struct{})
	go func() {
		r.ServeHTTP(response, request)
		close(done)
	}()
	select {
	case id := <-r.legacy.queue:
		r.legacy.mu.Lock()
		job := r.legacy.waiting[id]
		r.legacy.mu.Unlock()
		job.reply <- answer{Status: http.StatusNoContent}
		if job.work.Path != path || string(job.work.Body) != body || job.work.Cookie != "narolink="+inner {
			t.Error("follow-up payload was changed before forwarding to the desktop")
		}
	case <-time.After(time.Second):
		t.Fatal("follow-up with an image did not reach the desktop")
	}
	<-done
	if response.Code != http.StatusNoContent {
		t.Fatalf("unexpected status %d: %s", response.Code, response.Body)
	}
}

func TestDeviceKeyPreventsCrossDevicePollingAndAnswers(t *testing.T) {
	r := gatewayForTest(t)
	firstKey := sha256.Sum256([]byte("first device"))
	secondKey := sha256.Sum256([]byte("second device"))
	firstDigest := sha256.Sum256(firstKey[:])
	secondDigest := sha256.Sum256(secondKey[:])
	first := hex.EncodeToString(firstDigest[:16])
	second := hex.EncodeToString(secondDigest[:16])
	request := func(id string, key [32]byte, route, body string) int {
		t.Helper()
		url := testOrigin + "/relay/devices/" + id + "/" + route
		req := httptest.NewRequest(http.MethodPost, url, strings.NewReader(body))
		req.Header.Set("Authorization", "Bearer "+strings.Repeat("x", 64))
		req.Header.Set("X-Naro-Device-Key", hex.EncodeToString(key[:]))
		response := httptest.NewRecorder()
		r.ServeHTTP(response, req)
		return response.Code
	}
	if request(first, firstKey, "hello", `{"name":"A"}`) != http.StatusNoContent ||
		request(second, secondKey, "hello", `{"name":"B"}`) != http.StatusNoContent {
		t.Fatal("could not register both devices")
	}
	if request(first, secondKey, "poll", "") != http.StatusForbidden {
		t.Fatal("second device could poll the first device's queue")
	}
	jobID := strings.Repeat("a", 32)
	job := &waiting{reply: make(chan answer, 1)}
	r.findDevice(first).waiting[jobID] = job
	if request(second, secondKey, "answer/"+jobID, `{"status":204}`) != http.StatusNotFound ||
		request(first, secondKey, "answer/"+jobID, `{"status":204}`) != http.StatusForbidden ||
		request(first, firstKey, "answer/"+jobID, `{"status":204}`) != http.StatusNoContent || len(job.reply) != 1 {
		t.Fatal("reply was not confined to its owning device")
	}
}
