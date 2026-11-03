// Messenger without the rest of Facebook. Runs at document start in the main
// frame of a Messenger account's webview. It only restyles and steers the
// page; its one way to the app is the notifications shim (notify.js).
// window.__mcSettings holds this account's options, set by the app.
//
// Adapted from fMess (https://github.com/toanbku/fmess), MIT License,
// Copyright (c) 2026 fMess contributors.
(() => {
  if (window.top !== window || window.__masterChatMessenger) return;
  window.__masterChatMessenger = true;
  const settings = (window.__mcSettings ||= {});

  // WebView2 runs this before the page has its <html> element.
  const withRoot = (use) => {
    if (document.documentElement) return use(document.documentElement);
    const wait = new MutationObserver(() => {
      if (!document.documentElement) return;
      wait.disconnect();
      use(document.documentElement);
    });
    wait.observe(document, { childList: true });
  };

  const CHATS = "data-mc-chats";
  // A popped-out chat (its own small window): just the conversation.
  const POP = "data-mc-pop";
  if (settings.popout) withRoot((root) => root.setAttribute(POP, ""));
  const onChatsPage = () => /^\/(messages|messenger_media)(\/|$)/.test(location.pathname);
  const loggedIn = () => /(^|;\s*)c_user=/.test(document.cookie);

  // Signed in, Facebook's home page and feed have nothing to do with chats.
  const FEED = /^\/(|home\.php|feed\/?|watch\/?|reel\/.*|marketplace\/?.*)$/;
  const steer = () => {
    if (loggedIn() && FEED.test(location.pathname)) location.replace("/messages/");
  };

  // Opening a chat puts the cursor in its message box. The old chat's box is
  // still on screen for a moment, so wait for a different one.
  const composer = () => document.querySelector('[role="main"] [contenteditable="true"][role="textbox"]');
  let focusTimer;
  const focusComposer = () => {
    clearInterval(focusTimer);
    const previous = composer();
    let tries = 0;
    focusTimer = setInterval(() => {
      tries += 1;
      const box = composer();
      if ((box && (box !== previous || tries > 10)) || tries > 60) {
        clearInterval(focusTimer);
        if (box && document.activeElement !== box) box.focus();
      }
    }, 50);
  };

  // Facebook is a single-page app: routes change through the History API.
  let lastPath = null;
  const syncRoute = () => {
    document.documentElement?.toggleAttribute(CHATS, onChatsPage());
    steer();
    if (location.pathname !== lastPath) {
      lastPath = location.pathname;
      if (/^\/messages\/(e2ee\/)?t\//.test(lastPath)) focusComposer();
    }
  };
  for (const method of ["pushState", "replaceState"]) {
    const original = history[method];
    history[method] = function (...args) {
      const result = original.apply(this, args);
      syncRoute();
      return result;
    };
  }
  window.addEventListener("popstate", syncRoute);
  steer();
  withRoot(syncRoute);

  // Facebook's top bar is a role="banner" wrapper, and the page leaves room for
  // it through --header-height, which themed subtrees declare again, so it is
  // set on every element. In the chats the panes lose Facebook's inset, rounded
  // cards and sit edge to edge, as in the old desktop app.
  const LIST = "data-mc-list";
  const style = document.createElement("style");
  style.textContent = `
    html[${CHATS}] div[role="banner"] { display: none !important; }
    html[${CHATS}], html[${CHATS}] * {
      --header-height: 0px !important;
      --messenger-card-spacing: 0px !important;
      --messenger-card-corner-radius: 0px !important;
      --messenger-card-box-shadow: none !important;
    }
    html[${CHATS}] [role="main"] { box-shadow: -1px 0 0 var(--divider, var(--card-border-color)); }
    html[${CHATS}] body { background: var(--surface-background) !important; }
    /* The panes scroll on their own; the page's own scroll bar is just a grey strip. */
    html[${CHATS}] { scrollbar-width: none; }
    /* The photo viewer centres its buttons in a band made for the 56px bar. */
    html[${CHATS}] [role="dialog"], html[${CHATS}] [role="dialog"] * { --header-height: 56px !important; }
    /* The chat list's edge can be dragged to resize it. */
    [${LIST}] { position: relative; }
    /* A popped-out chat is narrow: Facebook shows just the conversation then. */
    html[${POP}] .mc-pane-edge { display: none !important; }
    .mc-pane-edge { position: absolute; top: 0; bottom: 0; right: -6px; width: 9px; z-index: 5; cursor: col-resize; touch-action: none; }
    .mc-pane-edge::after {
      content: ""; position: absolute; top: 0; bottom: 0; left: 5px; width: 2px;
      background: var(--primary-button-background, #0866ff); opacity: 0; transition: opacity .15s;
    }
    .mc-pane-edge:hover::after { opacity: .6; transition-delay: .15s; }
    .mc-pane-edge.dragging::after { opacity: .6; transition-delay: 0s; }
    html[data-mc-resizing], html[data-mc-resizing] * { cursor: col-resize !important; user-select: none !important; }
  `;
  withRoot((root) => root.append(style));

  // The chat list's width, kept in this account's own storage. Double-click
  // the edge to go back to Facebook's width.
  const MIN_CHAT = 360;
  const widthStyle = document.createElement("style");
  const readWidth = () => {
    try {
      return Number(localStorage.getItem("mc-list-width")) || 0;
    } catch {
      return 0;
    }
  };
  const showWidth = (width) => {
    widthStyle.textContent = width
      ? `html[${CHATS}] [${LIST}] { flex: 0 0 ${width}px !important; width: ${width}px !important; max-width: calc(100% - ${MIN_CHAT}px) !important; }`
      : "";
    try {
      if (width) localStorage.setItem("mc-list-width", String(width));
      else localStorage.removeItem("mc-list-width");
    } catch {}
  };
  withRoot((root) => root.append(widthStyle));
  showWidth(readWidth());
  const edge = document.createElement("div");
  edge.className = "mc-pane-edge";
  edge.addEventListener("pointerdown", (event) => {
    const pane = edge.parentElement;
    if (event.button !== 0 || !pane) return;
    event.preventDefault();
    event.stopPropagation();
    edge.setPointerCapture(event.pointerId);
    const startX = event.clientX;
    const startWidth = pane.getBoundingClientRect().width;
    const room = Math.max(0, (document.querySelector('[role="main"]')?.getBoundingClientRect().width ?? 0) - MIN_CHAT);
    const min = Math.max(parseFloat(getComputedStyle(pane).minWidth) || 0, 220);
    const max = Math.max(min, startWidth + room);
    let width = startWidth;
    document.documentElement.setAttribute("data-mc-resizing", "");
    edge.classList.add("dragging");
    const move = (e) => {
      width = Math.round(Math.min(max, Math.max(min, startWidth + e.clientX - startX)));
      pane.style.setProperty("flex", `0 0 ${width}px`, "important");
      pane.style.setProperty("width", `${width}px`, "important");
    };
    const end = () => {
      edge.removeEventListener("pointermove", move);
      edge.removeEventListener("lostpointercapture", end);
      document.documentElement.removeAttribute("data-mc-resizing");
      edge.classList.remove("dragging");
      pane.style.removeProperty("flex");
      pane.style.removeProperty("width");
      if (width !== startWidth) showWidth(width);
    };
    edge.addEventListener("pointermove", move);
    edge.addEventListener("lostpointercapture", end);
  });
  edge.addEventListener("dblclick", (event) => {
    event.stopPropagation();
    showWidth(0);
  });
  setInterval(() => {
    if (!onChatsPage()) return;
    const list = document.querySelector('[role="main"]')?.parentElement?.querySelector(':scope > [role="navigation"]');
    if (!list) return;
    if (!list.hasAttribute(LIST)) {
      document.querySelector(`[${LIST}]`)?.removeAttribute(LIST);
      list.setAttribute(LIST, "");
    }
    if (edge.parentElement !== list) list.append(edge);
  }, 1000);

  // The unread count. Facebook's title counts its own notifications and doesn't
  // drop as chats are read, so the title becomes "(N) Messenger", N being the
  // unread chats in the list, which the app reads like any other page title.
  // An unread row has, after the chat's name, text only screen readers get
  // ("Unread message:"), clipped to 1px.
  const titleProp = Object.getOwnPropertyDescriptor(Document.prototype, "title");
  let unread = 0;
  // A popped-out chat is titled with the conversation's name, for the chat bar.
  let chatName = "";
  const applyTitle = () => {
    const wanted = settings.popout ? chatName || "Messenger" : unread ? `(${unread}) Messenger` : "Messenger";
    if (titleProp.get.call(document) !== wanted) titleProp.set.call(document, wanted);
  };
  Object.defineProperty(document, "title", { configurable: true, get: () => titleProp.get.call(document), set: applyTitle });
  const isUnreadRow = (row) => {
    let seenName = false;
    const walker = document.createTreeWalker(row, NodeFilter.SHOW_TEXT);
    for (let node = walker.nextNode(); node; node = walker.nextNode()) {
      if (!node.textContent.trim()) continue;
      const box = node.parentElement.getBoundingClientRect();
      const visible = box.width > 1 && box.height > 1;
      if (visible) seenName = true;
      else if (seenName) return true;
    }
    return false;
  };
  // Facebook sends its message notifications by Web Push, which WebView2 lacks,
  // so the page shows none. Instead, when an unread chat's preview changes, this
  // raises one (through notify.js) with the chat's name and preview; clicking it
  // opens the chat. Rows are read the way screen readers get them: the visible
  // texts are the name, the preview, "·" and the time.
  const notified = new Map();
  // The list fills in over a few seconds; unread chats seen by then are old news.
  let quietUntil = 0;
  const describeRow = (row) => {
    const texts = [];
    const walker = document.createTreeWalker(row, NodeFilter.SHOW_TEXT);
    for (let node = walker.nextNode(); node; node = walker.nextNode()) {
      const text = node.textContent.trim();
      const box = node.parentElement.getBoundingClientRect();
      if (text && box.width > 1 && box.height > 1) texts.push(text);
    }
    const [name = "", ...rest] = texts;
    const dot = rest.lastIndexOf("·");
    const preview = (dot > 0 ? rest.slice(0, dot) : rest).filter((t) => t !== "·").join(" ");
    return { name, preview };
  };
  const announce = (rows) => {
    for (const row of rows) {
      const href = row.getAttribute("href");
      const { name, preview } = describeRow(row);
      if (!href || !name || notified.get(href) === preview) continue;
      notified.set(href, preview);
      if (Date.now() < quietUntil) continue;
      const notification = new Notification(name, { body: preview });
      notification.onclick = () => {
        const link = [...document.querySelectorAll('[role="grid"] [role="row"] a[href]')].find((a) => a.getAttribute("href") === href);
        if (link) link.click();
        else location.assign(href);
      };
    }
  };

  // Reading the list lays the page out, so it is read again only after it changed.
  let watchedList = null;
  let listChanged = true;
  const listWatcher = new MutationObserver(() => (listChanged = true));
  setInterval(() => {
    if (!onChatsPage()) return applyTitle();
    const list = document.querySelector('[role="grid"] [role="row"] a[href*="/messages/"]')?.closest('[role="grid"]') ?? null;
    if (list !== watchedList) {
      listWatcher.disconnect();
      if (list) listWatcher.observe(list, { subtree: true, childList: true, characterData: true, attributes: true });
      watchedList = list;
      listChanged = true;
    }
    if (listChanged) {
      listChanged = false;
      const rows = [...document.querySelectorAll('[role="grid"] [role="row"] a[href*="/messages/"]')];
      const unreadRows = rows.filter((row) => row.getBoundingClientRect().width > 0 && isUnreadRow(row));
      unread = unreadRows.length;
      if (settings.popout) {
        // The main window announces messages; here the list (hidden) just names the chat.
        const own = rows.find((row) => row.getAttribute("href")?.replace(/\/$/, "") === location.pathname.replace(/\/$/, ""));
        // The list is hidden here, so its first text is read directly: the name.
        const walker = own && document.createTreeWalker(own, NodeFilter.SHOW_TEXT);
        for (let node = walker?.nextNode(); node; node = walker.nextNode()) {
          if (node.textContent.trim()) {
            chatName = node.textContent.trim();
            break;
          }
        }
      } else {
        if (rows.length && !quietUntil) quietUntil = Date.now() + 10000;
        if (rows.length) announce(unreadRows);
      }
    }
    // Also puts the title back if Facebook rewrote the <title> element itself.
    applyTitle();
  }, 1500);

  // Block typing (account menu): nobody sees "typing…". Messenger sends
  // requests over the /ws/lightspeed WebSocket. A frame holds parts: a type
  // byte and a 2-byte stream id, then, except in the part that closes a stream
  // (0x0e), a 3-byte length and that many bytes, little endian. A data part
  // (0x0d) is two flag bytes and the request as JSON, nesting more JSON in
  // strings. Typing requests (type 4, label "3") are edited to is_typing 0
  // rather than dropped, so every stream still gets its answer. Typing in
  // encrypted chats goes to Messenger's worker (sendChatStateFromComposer).
  const parseJson = (text) => {
    try {
      return JSON.parse(text);
    } catch {
      return null;
    }
  };
  // Ids are 64-bit numbers, so only JSON that round-trips unchanged is edited.
  const editJson = (text, edit) => {
    const value = parseJson(text);
    if (!value || typeof value !== "object" || JSON.stringify(value) !== text) return null;
    return edit(value) ? JSON.stringify(value) : null;
  };
  const editNested = (object, key, edit) => {
    const value = object?.[key];
    if (value && typeof value === "object") return edit(value);
    const edited = typeof value === "string" ? editJson(value, edit) : null;
    if (edited === null) return false;
    object[key] = edited;
    return true;
  };
  const editRequest = (request) =>
    request.type === 4 &&
    editNested(
      request,
      "payload",
      (update) =>
        update.label === "3" &&
        editNested(update, "payload", (typing) => {
          if (!typing.is_typing) return false;
          typing.is_typing = 0;
          return true;
        })
    );
  const editFrame = (bytes) => {
    const parts = [];
    let edited = false;
    for (let pos = 0; pos < bytes.length; ) {
      if (bytes[pos] === 0x0e) {
        parts.push(bytes.subarray(pos, pos + 3));
        pos += 3;
        continue;
      }
      if (pos + 6 > bytes.length) return null;
      const end = pos + 6 + (bytes[pos + 3] | (bytes[pos + 4] << 8) | (bytes[pos + 5] << 16));
      if (end > bytes.length) return null;
      let part = bytes.subarray(pos, end);
      const json = bytes[pos] === 0x0d && end > pos + 8 ? new TextDecoder().decode(bytes.subarray(pos + 8, end)) : null;
      const request = json && editJson(json, editRequest);
      if (request) {
        const encoded = new TextEncoder().encode(request);
        const length = encoded.length + 2;
        part = new Uint8Array(8 + encoded.length);
        part.set(bytes.subarray(pos, pos + 3));
        part.set([length & 0xff, (length >> 8) & 0xff, length >> 16], 3);
        part.set(bytes.subarray(pos + 6, pos + 8), 6);
        part.set(encoded, 8);
        edited = true;
      }
      parts.push(part);
      pos = end;
    }
    if (!edited) return null;
    const frame = new Uint8Array(parts.reduce((size, part) => size + part.length, 0));
    parts.reduce((offset, part) => (frame.set(part, offset), offset + part.length), 0);
    return frame;
  };
  const nativeSend = WebSocket.prototype.send;
  WebSocket.prototype.send = function (data) {
    const bytes =
      !settings.blockTyping || !String(this.url).includes("/ws/lightspeed")
        ? null
        : data instanceof ArrayBuffer
          ? new Uint8Array(data)
          : ArrayBuffer.isView(data)
            ? new Uint8Array(data.buffer, data.byteOffset, data.byteLength)
            : null;
    return nativeSend.call(this, (bytes && editFrame(bytes)) || data);
  };
  const nativePostMessage = MessagePort.prototype.postMessage;
  MessagePort.prototype.postMessage = function (message, ...rest) {
    if (settings.blockTyping && Array.isArray(message) && message.some((item) => item?.content?.name === "sendChatStateFromComposer")) {
      message = message.map((item) =>
        item?.content?.name === "sendChatStateFromComposer" && item.content.arg?.state !== "idle"
          ? { ...item, content: { ...item.content, arg: { ...item.content.arg, state: "idle" } } }
          : item
      );
    }
    return nativePostMessage.call(this, message, ...rest);
  };

  // Facebook links in a chat (posts, reels, profiles) go to the browser, so the
  // window stays on the conversation: opened as a new window, which the app
  // only allows for calls and sends everywhere else to the default browser.
  const IN_APP = /^\/(messages|messenger_media|groupcall|videocall)(\/|$)/;
  const isFacebook = (host) => ["facebook.com", "messenger.com"].some((d) => host === d || host.endsWith(`.${d}`));
  document.addEventListener(
    "click",
    (event) => {
      if (!onChatsPage() || event.button !== 0 || event.metaKey || event.ctrlKey || event.shiftKey) return;
      const link = event.target instanceof Element ? event.target.closest("a[href]") : null;
      if (!link) return;
      const url = new URL(link.href, location.href);
      if (!isFacebook(url.hostname) || IN_APP.test(url.pathname)) return;
      event.preventDefault();
      event.stopImmediatePropagation();
      window.open(url.href, "_blank", "noopener");
    },
    true
  );
})();
