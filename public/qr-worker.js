importScripts("/vendor/jsqr.js");

self.addEventListener("message", ({data: {data, width, height}}) => {
  const code = self.jsQR(data, width, height, {inversionAttempts: "dontInvert"});
  self.postMessage(code?.data ?? null);
});
