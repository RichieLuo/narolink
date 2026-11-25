package main

import (
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
	if response.Code != http.StatusUnauthorized || len(r.queue) != 0 {
		t.Fatalf("unpaired request reached relay queue: status %d, queued %d", response.Code, len(r.queue))
	}
}

func TestPairingSetsSecureCookieForHTTPS(t *testing.T) {
	r := gatewayForTest(t)
	r.lastPoll = time.Now()
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
	case id := <-r.queue:
		r.mu.Lock()
		job := r.waiting[id]
		r.mu.Unlock()
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
	if _, paired := r.unwrap(read); !paired {
		t.Fatal("paired HTTPS cookie cannot be validated")
	}
}
