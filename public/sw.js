const cacheName = "narolink-shell-v2";
const shell = ["/", "/app.js", "/style.css", "/manifest.webmanifest", "/icons/180.png", "/icons/192.png", "/icons/512.png"];
self.addEventListener("install", event => {
  event.waitUntil(caches.open(cacheName).then(cache => cache.addAll(shell)).then(() => self.skipWaiting()));
});
self.addEventListener("activate", event => {
  event.waitUntil(caches.keys().then(keys => Promise.all(
    keys.filter(key => key.startsWith("narolink-shell-") && key !== cacheName).map(key => caches.delete(key))
  )).then(() => self.clients.claim()));
});
self.addEventListener("fetch", event => {
  if (event.request.method !== "GET" || !shell.includes(new URL(event.request.url).pathname)) return;
  event.respondWith(fetch(event.request).catch(() => caches.match(event.request)));
});
