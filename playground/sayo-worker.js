import init, { evaluateJson } from "./pkg/sayo.js?v=20260806.48";

const initialized = init(new URL("./pkg/sayo_bg.wasm?v=20260806.48", import.meta.url));

self.onmessage = async (event) => {
  const envelope = event.data;
  const hasEnvelope =
    envelope && typeof envelope === "object" && "request" in envelope && "id" in envelope;
  const id = hasEnvelope ? envelope.id : null;
  const input = hasEnvelope ? envelope.request : envelope;

  try {
    await initialized;
    const request = typeof input === "string" ? input : JSON.stringify(input);
    const response = evaluateJson(request);
    self.postMessage(hasEnvelope ? { id, response } : response);
  } catch (error) {
    const response = JSON.stringify({
      format: "sayo-web-evaluation",
      version: 1,
      status: "trap",
      trap: {
        category: "host error",
        message: error instanceof Error ? error.message : String(error),
      },
    });
    self.postMessage(hasEnvelope ? { id, response } : response);
  }
};
