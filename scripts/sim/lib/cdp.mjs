// Minimal Chrome DevTools Protocol client for the e2e driver's --inspect
// mode: the app's WebView2 pages (main window, every overlay, the side panel)
// are reached on a loopback debugging port that only the driver's own app
// process opens (WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS, set in its temporary
// environment). Dev-only; never part of the app or its bundle.
//
// It reads the rendered DOM, renders a page to PNG and sends pointer events
// to a page. None of that needs the Windows desktop, so it also works on a
// locked session, where screenshots of the screen show the lock screen.

/** Browser arguments that open the debugging port on loopback only. */
export function inspectArguments(port) {
  if (!Number.isInteger(port) || port < 1024 || port > 65535) throw new Error(`--inspect: port ${port} is not 1024-65535`);
  // Occluded windows (a locked session, a table on top) must keep painting,
  // or a capture would wait forever for a frame.
  return [
    `--remote-debugging-port=${port}`,
    "--remote-debugging-address=127.0.0.1",
    "--disable-backgrounding-occluded-windows",
    "--disable-renderer-backgrounding",
  ].join(" ");
}

/** The page targets of the app's WebView2 browser (url, title, websocket). */
export async function listPages(port, { timeoutMs = 3000 } = {}) {
  const response = await fetch(`http://127.0.0.1:${port}/json/list`, { signal: AbortSignal.timeout(timeoutMs) });
  if (!response.ok) throw new Error(`devtools list: HTTP ${response.status}`);
  const targets = await response.json();
  return targets
    .filter((t) => t.type === "page" && typeof t.webSocketDebuggerUrl === "string")
    .map((t) => ({ id: t.id, url: t.url, title: t.title, ws: t.webSocketDebuggerUrl }))
    .filter((t) => isLoopbackWs(t.ws));
}

const isLoopbackWs = (ws) => /^ws:\/\/(127\.0\.0\.1|localhost|\[::1\]):\d+\//.test(ws);

/** Which app window a page belongs to, from its URL path. */
export function pageKind(url) {
  let path;
  try {
    path = new URL(url).pathname;
  } catch {
    return null;
  }
  if (/\/overlay\.html$/.test(path)) return "overlay";
  if (/\/panel\.html$/.test(path)) return "panel";
  if (path === "/" || /\/index\.html$/.test(path)) return "main";
  return null;
}

export class CdpPage {
  constructor(socket) {
    this.socket = socket;
    this.nextId = 1;
    this.pending = new Map();
    socket.addEventListener("message", (event) => {
      let message;
      try {
        message = JSON.parse(String(event.data));
      } catch {
        return;
      }
      const waiter = message.id !== undefined && this.pending.get(message.id);
      if (!waiter) return;
      this.pending.delete(message.id);
      if (message.error) waiter.reject(new Error(`${waiter.method}: ${message.error.message}`));
      else waiter.resolve(message.result ?? {});
    });
    socket.addEventListener("close", () => {
      for (const waiter of this.pending.values()) waiter.reject(new Error(`${waiter.method}: devtools connection closed`));
      this.pending.clear();
    });
  }

  static async connect(ws, { timeoutMs = 5000 } = {}) {
    if (!isLoopbackWs(ws)) throw new Error(`refusing a non-loopback devtools address: ${ws}`);
    const socket = new WebSocket(ws);
    await new Promise((resolve, reject) => {
      const timer = setTimeout(() => reject(new Error("devtools connect timed out")), timeoutMs);
      socket.addEventListener("open", () => (clearTimeout(timer), resolve()), { once: true });
      socket.addEventListener("error", () => (clearTimeout(timer), reject(new Error("devtools connect failed"))), { once: true });
    });
    return new CdpPage(socket);
  }

  send(method, params = {}, { timeoutMs = 8000 } = {}) {
    const id = this.nextId++;
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        this.pending.delete(id);
        reject(new Error(`${method}: timed out after ${timeoutMs} ms`));
      }, timeoutMs);
      this.pending.set(id, {
        method,
        resolve: (v) => (clearTimeout(timer), resolve(v)),
        reject: (e) => (clearTimeout(timer), reject(e)),
      });
      this.socket.send(JSON.stringify({ id, method, params }));
    });
  }

  /** Evaluates an expression in the page and returns its JSON value. */
  async evaluate(expression) {
    const { result, exceptionDetails } = await this.send("Runtime.evaluate", { expression, returnByValue: true, awaitPromise: true });
    if (exceptionDetails) throw new Error(`evaluate: ${exceptionDetails.exception?.description ?? exceptionDetails.text}`);
    return result?.value;
  }

  /** The page rendered to PNG bytes, transparent where the page is. */
  async screenshot() {
    await this.send("Emulation.setDefaultBackgroundColorOverride", { color: { r: 0, g: 0, b: 0, a: 0 } }).catch(() => {});
    const { data } = await this.send("Page.captureScreenshot", { format: "png" }, { timeoutMs: 10000 });
    return Buffer.from(data, "base64");
  }

  async mouseMove(x, y) {
    await this.send("Input.dispatchMouseEvent", { type: "mouseMoved", x, y, pointerType: "mouse" });
  }

  async click(x, y) {
    await this.mouseMove(x, y);
    await this.send("Input.dispatchMouseEvent", { type: "mousePressed", x, y, button: "left", buttons: 1, clickCount: 1 });
    await this.send("Input.dispatchMouseEvent", { type: "mouseReleased", x, y, button: "left", buttons: 0, clickCount: 1 });
  }

  close() {
    try {
      this.socket.close();
    } catch {
      // already closed
    }
  }
}

/**
 * Page script: what an overlay shows, read from its rendered DOM: the chips
 * (text, accessible name, position) and the hover card and drawer if open.
 */
export const OVERLAY_STATE = `(() => {
  const text = (el) => (el ? el.innerText.replace(/\\s+/g, " ").trim() : null);
  const chips = [...document.querySelectorAll("[data-hud-chip]")].map((wrap) => {
    const r = wrap.getBoundingClientRect();
    const named = wrap.querySelector("[aria-label]");
    return {
      id: wrap.getAttribute("data-hud-chip"),
      seat: wrap.getAttribute("data-seat-key"),
      text: text(wrap),
      label: named ? named.getAttribute("aria-label") : null,
      rect: { x: Math.round(r.x), y: Math.round(r.y), width: Math.round(r.width), height: Math.round(r.height) },
    };
  });
  const card = document.querySelector("[data-hover-card]");
  const drawer = document.querySelector("aside[aria-label$=' details']");
  return {
    url: location.href,
    size: { width: innerWidth, height: innerHeight },
    chips,
    hoverCard: text(card),
    drawer: text(drawer),
  };
})()`;

/** Page script: the side panel's rows and search result. */
export const PANEL_STATE = `(() => {
  const text = (el) => (el ? el.innerText.replace(/\\s+/g, " ").trim() : null);
  const input = document.querySelector("input[type=search]");
  const result = input && input.getAttribute("aria-describedby") ? document.getElementById(input.getAttribute("aria-describedby")) : null;
  return {
    url: location.href,
    visibility: document.visibilityState,
    size: { width: innerWidth, height: innerHeight },
    query: input ? input.value : null,
    result: text(result),
    marked: [...document.querySelectorAll("mark")].map((m) => m.innerText),
    text: text(document.body).slice(0, 4000),
  };
})()`;

/** Page script: types into the side panel's search box the way React sees it. */
export function panelSearch(query) {
  return `(() => {
  const input = document.querySelector("input[type=search]");
  if (!input) return false;
  const set = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value").set;
  set.call(input, ${JSON.stringify(String(query))});
  input.dispatchEvent(new Event("input", { bubbles: true }));
  return true;
})()`;
}
